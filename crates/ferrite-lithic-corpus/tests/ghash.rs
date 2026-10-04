//! GHASH, checked three ways.
//!
//! # Why the golden model is the `ghash` crate, and how
//!
//! `ghash::GHash` looks at first glance like it cannot be used: its API takes
//! arbitrary-length input and applies the whole GHASH recurrence internally, and
//! GHASH is `Y = (Y ^ X) · H` over blocks *plus* the length block SP 800-38D §6.4
//! appends, so there is no obvious way to ask it for "one raw multiply".
//!
//! There is, and it is one line. `Polyval` (which `GHash` wraps) does no padding of
//! its own: `finalize` returns the accumulator, and `update` with a single
//! sixteen-byte block computes `(0 ^ block) · H`, because the state starts at zero.
//! So
//!
//! ```text
//! GHash::new(H); update(X); finalize()  ==  X · H
//! ```
//!
//! is the raw multiply this design implements, in GCM's own byte order. That is a
//! real golden model and it is used as one below, on arbitrary operand pairs -- not
//! just to confirm a published tag.
//!
//! The one thing the crate cannot do is confirm the *reduction convention*, because
//! it hides the reduction inside `polyval`'s `mulx`. So the published GCM vectors
//! are also pinned directly, and they pin it: SP 800-38D §6.3's `R =
//! 0xe1000000000000000000000000000000` is the only value that makes Test Case 2's
//! and Test Case 4's tags come out. Those vectors are transcribed here from the GCM
//! specification (McGrew & Viega §7) and each transcription is checked against the
//! `ghash` crate plus the `aes` crate's `E(K, J0)` before it is used, so a mistyped
//! hexadecimal digit fails as a transcription error rather than as a design bug.
//!
//! And the arithmetic that pins the reduction *directly* is here too, in
//! `the_reduction_constant_is_reachable_in_one_multiply`: multiplying `x^127` by `x`
//! overflows the modulus by exactly the reduction constant, so a design that
//! reduces by `0x87` -- the constant you get by reading the modulus in the other
//! bit order -- produces `0x87` here instead of `0xe1...`, and nothing else in the
//! file distinguishes the two.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::slice;

use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::ghash;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::{Handle, Testbench};
use hex::decode;
// The corpus module and the golden crate have the same name, and a `use`d name
// wins over the extern prelude in a `use` path's first segment -- so `use
// ghash::GHash` would resolve to `ferrite_lithic_corpus::ghash` and fail with "no
// `GHash` in `ghash`". The leading `::` says "the crate".
use ::ghash::GHash;
use ::ghash::universal_hash::UniversalHash;
use proptest::prelude::*;

/// GCM specification Test Case 2: an all-zero key and IV, and a one-block
/// ciphertext.
///
/// Test Case 2's plaintext is not transcribed: GHASH does not absorb it (see
/// [`ghash_blocks`]), and it happens to be sixteen zero bytes, so absorbing it by
/// mistake would make no difference here — which is exactly why only Test Case 4
/// catches that mistake.
const TC2_KEY: &str = "00000000000000000000000000000000";
/// GCM specification Test Case 2's IV.
const TC2_IV: &str = "000000000000000000000000";
/// GCM specification Test Case 2's `H`, i.e. `E(K, 0^128)`.
const TC2_H: &str = "66e94bd4ef8a2c3b884cfa59ca342b2e";
/// GCM specification Test Case 2's ciphertext.
const TC2_C: &str = "0388dace60b6a392f328c2b971b2fe78";
/// GCM specification Test Case 2's tag.
const TC2_T: &str = "ab6e47d42cec13bdf53a67b21257bddf";

/// GCM specification Test Case 4: sixty-four bytes of plaintext, no AAD.
const TC4_KEY: &str = "feffe9928665731c6d6a8f9467308308";
/// GCM specification Test Case 4's IV.
const TC4_IV: &str = "cafebabefacedbaddecaf888";
/// GCM specification Test Case 4's `H`.
const TC4_H: &str = "b83b533708bf535d0aa6e52980d53b78";
/// GCM specification Test Case 4's plaintext, sixty-four bytes: four full blocks, so
/// GHASH's length block says 512 bits. GHASH does not absorb the plaintext, so it
/// is only used to check the ciphertext, by
/// `the_transcribed_ciphertext_is_the_plaintext_xor_aes_ctr` below.
const TC4_P: &str = "d9313225f88406e5a55909c5aff5269a\
                     86a7a9531534f7da2e4c303d8a318a72\
                     1c3c0c95956809532fcf0e2449a6b525\
                     b16aedf5aa0de657ba637b391aafd255";
/// GCM specification Test Case 4's ciphertext, sixty-four bytes.
const TC4_C: &str = "42831ec2217774244b7221b784d0d49c\
                     e3aa212f2c02a4e035c17e2329aca12e\
                     21d514b25466931c7d8f6a5aac84aa05\
                     1ba30b396a0aac973d58e091473f5985";
/// GCM specification Test Case 4's tag.
const TC4_T: &str = "4d5c2af327cd64a62cf35abd2ba6fab4";

/// A sixteen-byte value from a hexadecimal literal.
fn block(hex_text: &str) -> [u8; 16] {
    bytes(hex_text, 16).try_into().expect("sixteen bytes")
}

/// A twelve-byte GCM IV from a hexadecimal literal.
fn iv(hex_text: &str) -> [u8; 12] {
    bytes(hex_text, 12).try_into().expect("twelve bytes")
}

/// A value of any length from a hexadecimal literal, whitespace and newlines ignored.
fn bytes(hex_text: &str, want: usize) -> Vec<u8> {
    let text: String = hex_text.chars().filter(|c| !c.is_whitespace()).collect();
    let decoded = decode(&text).expect("the constant is hexadecimal");
    assert_eq!(decoded.len(), want, "the constant is {want} bytes");
    decoded
}

/// The high 64 bits of a field element, read big-endian. GCM's block format is
/// sixteen bytes with the first byte in the high half.
fn hi(value: &[u8; 16]) -> u64 {
    u64::from_be_bytes(value[..8].try_into().expect("eight bytes"))
}

/// The low 64 bits of a field element, read big-endian.
fn lo(value: &[u8; 16]) -> u64 {
    u64::from_be_bytes(value[8..].try_into().expect("eight bytes"))
}

/// A sixteen-byte block in the `ghash` crate's own block type.
///
/// `ghash::Block<GHash>` is `hybrid_array::Array` and **not** the
/// `generic_array::GenericArray` that `cipher` re-exports and that the `aes` crate
/// below uses. The two crates have separate `From` impls, so handing a
/// `GenericArray` to `GHash` is a type error rather than a coercion, and the
/// conversion has to be spelled for the array type the ghash side actually uses.
fn hash_block(value: &[u8; 16]) -> ::ghash::Block {
    (*value).into()
}

/// The `ghash` crate's raw field multiply: `X · H`, with nothing else mixed in.
///
/// See the module docs for why one `update` of one block is the whole multiply.
/// `update` takes a slice of blocks, so the single block is borrowed out of a
/// one-element array.
fn golden(h: &[u8; 16], x: &[u8; 16]) -> [u8; 16] {
    let mut state = GHash::new(&hash_block(h));
    state.update(slice::from_ref(&hash_block(x)));
    state.finalize().into()
}

/// One AES-128 block encryption, `E(K, block)`.
fn aes_encrypt(key: &[u8; 16], plain: &[u8; 16]) -> [u8; 16] {
    let cipher = aes::Aes128::new(GenericArray::from_slice(key));
    let mut block = *GenericArray::from_slice(plain);
    cipher.encrypt_block(&mut block);
    block.into()
}

/// `E(K, J0)` for AES-128, the mask GCM XORs into GHASH to make the tag.
///
/// SP 800-38D §7.1: `J0 = IV || 0^31 || 1` and `T = GHASH(H, A, C) ^ E(K, J0)`.
fn gcm_mask(key: &[u8; 16], iv: &[u8; 12]) -> [u8; 16] {
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(iv);
    j0[15] = 1;
    aes_encrypt(key, &j0)
}

fn xor(left: [u8; 16], right: [u8; 16]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = left[index] ^ right[index];
    }
    out
}

/// One multiply through the design: `x · y`, with the start/busy/done handshake
/// driven the way the FSM expects it.
///
/// The count of edges is asserted rather than assumed, because 128 is the property
/// the whole design is shaped around: a counter that stopped one step early would
/// still produce a plausible accumulator and would take 127 cycles.
async fn multiply(tb: &Handle, x: [u8; 16], y: [u8; 16]) -> [u8; 16] {
    tb.drive("x_hi", hi(&x)).await;
    tb.drive("x_lo", lo(&x)).await;
    tb.drive("y_hi", hi(&y)).await;
    tb.drive("y_lo", lo(&y)).await;

    tb.drive("start", 1).await;
    assert_eq!(tb.value("busy"), 1, "the start edge loads and begins");
    assert_eq!(tb.value("done"), 0, "and does not finish on the same edge");

    // Deasserting `start` is itself the edge that performs step zero: `start` is a
    // load, so leaving it high would reload the operands every cycle instead.
    let mut edges = 1u32;
    tb.drive("start", 0).await;
    while tb.value("busy") == 1 {
        tb.step().await;
        edges += 1;
        assert!(edges <= ghash::STEPS, "the multiply does not terminate");
    }
    assert_eq!(
        edges,
        ghash::STEPS,
        "one cycle per bit of the multiplier, and no more"
    );
    assert_eq!(tb.value("done"), 1, "done is high on the finishing edge");

    let product = [
        tb.value("z_hi").to_be_bytes(),
        tb.value("z_lo").to_be_bytes(),
    ]
    .concat()
    .try_into()
    .expect("sixteen bytes");

    tb.step().await;
    assert_eq!(tb.value("done"), 0, "done is a one-cycle pulse");
    assert_eq!(
        (tb.value("z_hi"), tb.value("z_lo")),
        (hi(&product), lo(&product)),
        "and the result is held while idle"
    );
    product
}

/// Chained multiplies through one testbench: GHASH over `blocks`.
///
/// SP 800-38D §6.4's recurrence is `Y = (Y ^ X) · H`, so the design -- which
/// multiplies whatever it is handed by `H` -- is driven with the accumulator
/// already XORed with the block. Chaining is what makes a wrong step observable:
/// one bad accumulation step changes every product after it, and the only thing
/// pinned at the end is the published tag.
fn hardware_chain(h: &[u8; 16], blocks: &[[u8; 16]]) -> [u8; 16] {
    let design = Design::new();
    ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        let mut accumulator = [0u8; 16];
        for block in blocks {
            let mut operand = accumulator;
            for (index, byte) in operand.iter_mut().enumerate() {
                *byte ^= block[index];
            }
            accumulator = multiply(&tb, operand, *h).await;
        }
        accumulator
    })
    .expect("the ghash design drives only ports it declares")
}

/// Chained multiplies with the `ghash` crate as the model: GHASH over `blocks`.
fn golden_chain(h: &[u8; 16], blocks: &[[u8; 16]]) -> [u8; 16] {
    let mut state = GHash::new(&hash_block(h));
    for block in blocks {
        state.update(slice::from_ref(&hash_block(block)));
    }
    state.finalize().into()
}

/// The blocks GHASH absorbs for `aad` and `ciphertext`, each right-padded to a block
/// boundary, followed by the length block `[len(A)]_64 || [len(C)]_64`.
///
/// SP 800-38D §6.4: `GHASH_H(A || pad || C || pad || [len(A)]_64 || [len(C)]_64)`.
///
/// **The plaintext is not absorbed.** GHASH authenticates the additional data and the
/// ciphertext, never the plaintext, and a helper that takes the plaintext gets
/// Test Case 2 right by accident — its plaintext is sixteen zero bytes, so the
/// spurious block is `0 · H` and the accumulator does not move — and Test Case 4
/// wrong. That is worth recording, because "absorb the plaintext" is the natural
/// mistake to make and only one of the two published vectors here catches it.
fn ghash_blocks(aad: &[u8], ciphertext: &[u8]) -> Vec<[u8; 16]> {
    let mut blocks = Vec::new();
    for part in [aad, ciphertext] {
        for chunk in part.chunks(16) {
            let mut padded = [0u8; 16];
            padded[..chunk.len()].copy_from_slice(chunk);
            blocks.push(padded);
        }
    }
    let mut lengths = [0u8; 16];
    lengths[..8].copy_from_slice(&((aad.len() as u64) * 8).to_be_bytes());
    lengths[8..].copy_from_slice(&((ciphertext.len() as u64) * 8).to_be_bytes());
    blocks.push(lengths);
    blocks
}

/// A deterministic operand pair, so a failure is reproducible from its index.
fn derived(index: u64) -> ([u8; 16], [u8; 16]) {
    let mut h = [0u8; 16];
    let mut x = [0u8; 16];
    for offset in 0..16usize {
        h[offset] = (index as u8)
            .wrapping_mul(83)
            .wrapping_add((offset as u8).wrapping_mul(29))
            .wrapping_add(1);
        x[offset] = (index as u8)
            .wrapping_mul(47)
            .wrapping_add((offset as u8).wrapping_mul(13))
            .wrapping_add(5);
    }
    (h, x)
}

/// One stimulus row: the six input ports, in port order, with the clock omitted
/// because the clock is a port and not a stimulus column.
fn row(rst: u64, start: u64, x: [u8; 16], y: [u8; 16]) -> Vec<Bits> {
    vec![
        Bits::constant(rst, 1).expect("one bit"),
        Bits::constant(start, 1).expect("one bit"),
        Bits::constant(hi(&x), 64).expect("64 bits"),
        Bits::constant(lo(&x), 64).expect("64 bits"),
        Bits::constant(hi(&y), 64).expect("64 bits"),
        Bits::constant(lo(&y), 64).expect("64 bits"),
    ]
}

#[test]
fn the_reduction_constant_is_reachable_in_one_multiply() {
    // x^127 · x = x^128, and x^128 is not representable in 128 bits: the modulus
    // replaces it with x^7 + x^2 + x + 1, which in GCM's bit order (most significant
    // bit is x^0) is bits 120, 125, 126 and 127 -- exactly 0xe1 << 120. So this one
    // multiply's answer *is* the reduction constant, and any design that reduces by
    // a different value, or that tests the wrong bit before reducing, produces
    // something else here. It is the shortest possible check on the most
    // bug-prone line in the design.
    let x: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01];
    let y = block("40000000000000000000000000000000");
    let expected = block("e1000000000000000000000000000000");
    assert_eq!(ghash::REDUCTION_HI, hi(&expected), "and R is its high half");

    let design = Design::new();
    ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            multiply(&tb, x, y).await
        })
        .expect("the ghash design drives only ports it declares");
    assert_eq!(observed, expected, "x^127 * x must reduce to exactly R");
}

#[test]
fn a_zero_multiplicand_multiplies_to_zero() {
    // The accumulator starts at zero on every `start`, so this is the recurrence's
    // base case rather than a special one: Z = 0 ^ 0 is 0 and 0 · H is 0 whatever
    // H is. It is also the case that catches an accumulator that is cleared by
    // `start` one cycle too late.
    let design = Design::new();
    ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    for index in 0..4u64 {
        let (h, _) = derived(index);
        let observed = tb
            .run(|tb| async move {
                tb.pulse(ferrite_lithic_corpus::RESET).await;
                multiply(&tb, [0u8; 16], h).await
            })
            .expect("the ghash design drives only ports it declares");
        assert_eq!(observed, [0u8; 16], "0 * anything is 0");
    }
}

#[test]
fn the_design_agrees_with_the_ghash_crate_on_arbitrary_multiplies() {
    // The differential proper, and the reason the `ghash` crate is here at all: a
    // raw multiply, not a whole GHASH. `derived` makes every half of every operand
    // distinct, so a port wired to the wrong half -- `x_hi` and `x_lo` swapped, say
    // -- differs on nearly every case rather than on none.
    let design = Design::new();
    ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        for index in 0..12u64 {
            let (h, x) = derived(index);
            assert_eq!(
                multiply(&tb, x, h).await,
                golden(&h, &x),
                "x * h for derived case {index}"
            );
        }
    })
    .expect("the ghash design drives only ports it declares");
}

#[test]
fn the_design_agrees_with_the_ghash_crate_on_the_extremes_of_both_halves() {
    // All-zero and all-ones halves, so every bit of both ports is exercised at both
    // polarities and the 128-iteration walk reaches bit 127 and bit 0 of `Y`.
    let extremes = [
        0x0000_0000_0000_0000u64,
        u64::MAX,
        0x0000_0000_0000_0001,
        0x8000_0000_0000_0000,
        0x0000_0000_8000_0000,
        0xffff_ffff_0000_0000,
    ];
    let design = Design::new();
    ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        for (index, x_hi) in extremes.iter().enumerate() {
            for (place, y_lo) in extremes.iter().enumerate() {
                let x = [
                    x_hi.to_be_bytes(),
                    extremes[(index + 3) % extremes.len()].to_be_bytes(),
                ]
                .concat()
                .try_into()
                .expect("sixteen bytes");
                let y = [
                    extremes[(place + 2) % extremes.len()].to_be_bytes(),
                    y_lo.to_be_bytes(),
                ]
                .concat()
                .try_into()
                .expect("sixteen bytes");
                assert_eq!(
                    multiply(&tb, x, y).await,
                    golden(&y, &x),
                    "x {index:#x} * y {place:#x}"
                );
            }
        }
    })
    .expect("the ghash design drives only ports it declares");
}

#[test]
fn the_multiply_is_commutative_in_the_design_and_in_the_crate() {
    // Not a test of correctness so much as of coverage: swapping the operands
    // produces a different sequence of conditional XORs through the accumulator, so
    // a walk that dropped the first or last step would disagree here and possibly
    // nowhere else.
    let design = Design::new();
    ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        for index in 0..4u64 {
            let (h, x) = derived(index);
            let forwards = multiply(&tb, x, h).await;
            let backwards = multiply(&tb, h, x).await;
            assert_eq!(forwards, backwards, "derived case {index}");
            assert_eq!(forwards, golden(&h, &x));
        }
    })
    .expect("the ghash design drives only ports it declares");
}

#[test]
fn the_transcribed_gcm_test_case_2_agrees_with_the_ghash_and_aes_crates() {
    // A transcription check, so that a mistyped hexadecimal digit in this file is
    // reported as a transcription error rather than as a GHASH failure. The tag is
    // `GHASH ^ E(K, J0)` (SP 800-38D §7.1), and `J0 = IV || 0^31 || 1`.
    let h = block(TC2_H);
    let ciphertext = block(TC2_C);
    let blocks = ghash_blocks(&[], &ciphertext);
    let tag = golden_chain(&h, &blocks);
    assert_eq!(
        xor(tag, gcm_mask(&block(TC2_KEY), &iv(TC2_IV))),
        block(TC2_T),
        "GCM Test Case 2 does not reproduce from the transcribed vectors"
    );
}

#[test]
fn the_transcribed_gcm_test_case_4_agrees_with_the_ghash_and_aes_crates() {
    // As above, for the sixty-four-byte case: nine chained multiplies, so a single
    // wrong step anywhere in the recurrence has to survive eight more before it can
    // hide.
    let h = block(TC4_H);
    let ciphertext = bytes(TC4_C, 64);
    let blocks = ghash_blocks(&[], &ciphertext);
    assert_eq!(
        blocks.len(),
        5,
        "four ciphertext blocks and one length block"
    );
    let tag = golden_chain(&h, &blocks);
    assert_eq!(
        xor(tag, gcm_mask(&block(TC4_KEY), &iv(TC4_IV))),
        block(TC4_T),
        "GCM Test Case 4 does not reproduce from the transcribed vectors"
    );
}

#[test]
fn the_design_reproduces_the_gcm_specification_test_case_2_tag() {
    // Two chained multiplies through the design, ending on the published tag. The
    // expected value is the crate's chain, which the test above has already pinned
    // to the published tag through AES's `E(K, J0)`, so this asserts the design
    // against a published number and not merely against another implementation.
    let h = block(TC2_H);
    let blocks = ghash_blocks(&[], &block(TC2_C));
    assert_eq!(
        hardware_chain(&h, &blocks),
        golden_chain(&h, &blocks),
        "GCM Test Case 2's GHASH"
    );
    assert_eq!(
        xor(
            hardware_chain(&h, &blocks),
            gcm_mask(&block(TC2_KEY), &iv(TC2_IV))
        ),
        block(TC2_T),
        "and the tag that comes out of it"
    );
}

#[test]
fn the_transcribed_ciphertext_is_the_plaintext_xor_aes_ctr() {
    // SP 800-38D §7.1's encryption: `C = P ^ E(K, inc32(J0)) || E(K, inc32(inc32(J0)))
    // || …`, with the counter's last 32 bits incremented and its first 96 bits held
    // at `IV || 0^31`. This touches neither GHASH nor this design, so it separates
    // "these two strings are the published ones" from "the tag came out of them".
    // Without it, a typo in the plaintext and a matching typo in the ciphertext would
    // only have to be consistent with a tag that also came out right, which is one
    // coincidence too many to rely on.
    fn check(key_text: &str, iv_text: &str, plaintext: &[u8], published: &[u8]) {
        let key = block(key_text);
        let mut counter = [0u8; 16];
        counter[..12].copy_from_slice(&iv(iv_text));
        counter[15] = 1;

        let mut ciphertext = Vec::with_capacity(plaintext.len());
        for chunk in plaintext.chunks(16) {
            let next = u32::from_be_bytes(counter[12..].try_into().expect("four bytes")) + 1;
            counter[12..].copy_from_slice(&next.to_be_bytes());
            let stream = aes_encrypt(&key, &counter);
            ciphertext.extend(chunk.iter().zip(stream.iter()).map(|(p, s)| p ^ s));
        }
        assert_eq!(
            ciphertext, published,
            "C = P ^ E(K, inc32(J0)) for key {key_text}"
        );
    }

    check(TC2_KEY, TC2_IV, &[0u8; 16], &block(TC2_C));
    check(TC4_KEY, TC4_IV, &bytes(TC4_P, 64), &bytes(TC4_C, 64));
}

#[test]
fn a_short_final_block_is_padded_with_zeros_on_the_right() {
    // Neither published vector in this file ends in a short block, so the padding in
    // `ghash_blocks` is checked here against the crate. The padding is on the
    // *right*, which is GCM's convention and not the obvious one: the final block is
    // zero-extended to sixteen bytes, not shifted up to fill them, so a twenty-byte
    // ciphertext and a thirty-two-byte one containing those twenty bytes plus twelve
    // zeros of their own are different inputs.
    let h = block(TC4_H);
    let ciphertext = &bytes(TC4_C, 64)[..20];
    let blocks = ghash_blocks(&[], ciphertext);
    assert_eq!(
        blocks.len(),
        3,
        "one full block, one short block, one length"
    );
    assert_eq!(&blocks[1][4..], &[0u8; 12][..], "right-padded with zeros");
    assert_eq!(
        blocks[2],
        {
            let mut lengths = [0u8; 16];
            lengths[8..].copy_from_slice(&(160u64).to_be_bytes());
            lengths
        },
        "[len(A)]_64 || [len(C)]_64, and twenty bytes is 160 bits"
    );

    assert_eq!(
        hardware_chain(&h, &blocks),
        golden_chain(&h, &blocks),
        "GHASH over a short final block"
    );

    // And a ciphertext that already ends in twelve zeros is *not* the same input,
    // which is what "padded on the right" means and what would catch a helper that
    // left-pads.
    let mut padded = ciphertext.to_vec();
    padded.extend([0u8; 12]);
    assert_ne!(
        hardware_chain(&h, &ghash_blocks(&[], &padded)),
        hardware_chain(&h, &blocks),
        "zero-padded and already-zero-terminated are different inputs"
    );
}

#[test]
fn the_design_reproduces_the_gcm_specification_test_case_4_tag() {
    // Nine chained multiplies, the longest chain in the file, ending on a length
    // block of 512 bits. This is the case where a single wrong accumulator step has
    // eight more multiplies to hide in, and none of the eight is free: every one of
    // them has to be right for the published tag to come out.
    let h = block(TC4_H);
    let blocks = ghash_blocks(&[], &bytes(TC4_C, 64));
    assert_eq!(
        hardware_chain(&h, &blocks),
        golden_chain(&h, &blocks),
        "GCM Test Case 4's GHASH"
    );
    assert_eq!(
        xor(
            hardware_chain(&h, &blocks),
            gcm_mask(&block(TC4_KEY), &iv(TC4_IV))
        ),
        block(TC4_T),
        "and the tag that comes out of it"
    );
}

#[test]
fn a_reset_returns_the_accumulator_and_the_flags_to_their_initial_values() {
    // Registers start at zero and zero is GHASH's accumulator, so a reset is
    // exactly what a fresh multiply needs and there is nothing else to restore --
    // which is a claim worth testing rather than assuming, because it would stop
    // being true if the accumulator's reset value were ever made non-zero.
    let design = Design::new();
    ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let (after_reset, fresh) = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            assert_eq!(tb.value("busy"), 0, "idle after reset");
            assert_eq!(tb.value("done"), 0, "and not finished");
            let (h, x) = derived(0);
            let product = multiply(&tb, x, h).await;
            assert_ne!(product, [0u8; 16], "or this proves nothing");
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            let state = (
                tb.value("busy"),
                tb.value("done"),
                tb.value("z_hi"),
                tb.value("z_lo"),
            );
            // And a multiply after the reset starts from scratch, not from where
            // the interrupted one left the accumulator.
            let fresh = multiply(&tb, x, h).await;
            (state, fresh)
        })
        .expect("the ghash design drives only ports it declares");

    assert_eq!(
        after_reset,
        (0, 0, 0, 0),
        "reset returns every register to zero"
    );
    assert_ne!(
        fresh, [0u8; 16],
        "and the next multiply still produces a product"
    );
}

#[test]
fn the_handshake_is_idle_start_busy_done() {
    // The whole interface contract in one run: idle until `start`, busy for exactly
    // 128 cycles, `done` for exactly one, idle again. A design that asserted `busy`
    // on reset, or that held `done` until the next `start`, would fail here and
    // could pass every differential above.
    let design = Design::new();
    let ports = ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();

    // Nothing has been clocked yet: the simulator starts registers at zero, which is
    // the same convention Verilator's `--x-initial 0` gives (ADR-0018).
    tb.run(|tb| async move {
        assert_eq!(tb.value("busy"), 0, "idle before any edge");
        assert_eq!(tb.value("done"), 0, "and not finished");
    })
    .expect("the ghash design drives only ports it declares");

    let (h, x) = derived(7);
    let busy_series = tb
        .run(|tb| async move {
            tb.drive("x_hi", hi(&x)).await;
            tb.drive("x_lo", lo(&x)).await;
            tb.drive("y_hi", hi(&h)).await;
            tb.drive("y_lo", lo(&h)).await;
            tb.drive("start", 1).await;
            assert_eq!(tb.value("busy"), 1);
            assert_eq!(tb.value("done"), 0);
            let mut busy = vec![1u64];
            let mut done = vec![0u64];
            tb.drive("start", 0).await;
            busy.push(tb.value("busy"));
            done.push(tb.value("done"));
            while tb.value("busy") == 1 {
                tb.step().await;
                busy.push(tb.value("busy"));
                done.push(tb.value("done"));
            }
            // One more edge to see that `done` does not stick.
            tb.step().await;
            busy.push(tb.value("busy"));
            done.push(tb.value("done"));
            (busy, done)
        })
        .expect("the ghash design drives only ports it declares");

    // `busy` is high for the `start` edge and for the 127 edges that follow it, so
    // [`STEPS`] high observations in all: the 128 steps are the 128 edges *after*
    // the one that loaded the operands. The edge that performs the 128th step drops
    // `busy` and raises `done` in the same cycle, and one edge after that clears
    // `done`.
    assert_eq!(
        busy_series.0,
        {
            let mut expected = vec![1u64; ghash::STEPS as usize];
            expected.extend([0, 0]);
            expected
        },
        "busy for the start edge and the edges up to the last step"
    );
    assert_eq!(
        busy_series.1,
        {
            let mut expected = vec![0u64; ghash::STEPS as usize];
            expected.extend([1, 0]);
            expected
        },
        "and done for exactly the cycle in which the last step was performed"
    );
    assert_eq!(ports.inputs.start.width(), 1);
}

#[test]
fn a_start_while_busy_restarts_rather_than_being_ignored() {
    // `start` wins over an in-flight multiply, so a second `start` begins a new one
    // from the initial accumulator instead of being dropped or corrupting the first.
    // This is the handshake property that lets a host abandon a multiply it no longer
    // wants, and it is a choice rather than an accident: an ignored `start` would
    // need a separate abort port to get the same behaviour.
    let design = Design::new();
    ghash::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let (first, second) = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            let (h, x) = derived(3);
            tb.drive("x_hi", hi(&x)).await;
            tb.drive("x_lo", lo(&x)).await;
            tb.drive("y_hi", hi(&h)).await;
            tb.drive("y_lo", lo(&h)).await;
            tb.drive("start", 1).await;
            tb.drive("start", 0).await;
            // A few steps into the first multiply, then a new `start` with new
            // operands.
            tb.step().await;
            tb.step().await;
            tb.step().await;
            let (y, _) = derived(9);
            tb.drive("y_hi", hi(&y)).await;
            tb.drive("y_lo", lo(&y)).await;
            tb.drive("x_hi", hi(&x)).await;
            tb.drive("x_lo", lo(&x)).await;
            tb.drive("start", 1).await;
            assert_eq!(tb.value("busy"), 1, "busy never dropped");
            tb.drive("start", 0).await;
            let mut edges = 1u32;
            while tb.value("busy") == 1 {
                tb.step().await;
                edges += 1;
            }
            assert_eq!(edges, ghash::STEPS, "the second multiply ran 128 steps");
            let restarted: [u8; 16] = [
                tb.value("z_hi").to_be_bytes(),
                tb.value("z_lo").to_be_bytes(),
            ]
            .concat()
            .try_into()
            .expect("sixteen bytes");
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            let fresh = multiply(&tb, x, y).await;
            (restarted, fresh)
        })
        .expect("the ghash design drives only ports it declares");

    assert_eq!(
        first, second,
        "a start in flight restarts the multiply from scratch"
    );
}

proptest! {
    /// The differential, on arbitrary operands.
    ///
    /// Bounded because each case costs 128 simulated cycles; the deterministic cases
    /// above cover the halves' extremes and this covers the values in between.
    #[test]
    fn the_design_agrees_with_the_ghash_crate_on_arbitrary_field_elements(
        h in prop::array::uniform16(any::<u8>()),
        x in prop::array::uniform16(any::<u8>()),
    ) {
        let design = Design::new();
        ghash::build(&design).unwrap();
        let tb = Testbench::new(&design).unwrap();
        let observed = tb
            .run(|tb| async move {
                tb.pulse(ferrite_lithic_corpus::RESET).await;
                multiply(&tb, x, h).await
            })
            .expect("the ghash design drives only ports it declares");
        prop_assert_eq!(observed, golden(&h, &x));
    }
}

#[test]
fn the_emitted_ports_are_the_ones_the_testbench_drives() {
    // The testbench and the cosimulator address ports by name, so a rename that only
    // lands in one of them turns every differential above into a test that fails for
    // a reason that has nothing to do with GHASH.
    let design = Design::new();
    ghash::build(&design).unwrap();
    let inputs: Vec<String> = design
        .input_ports()
        .iter()
        .filter_map(|signal| signal.name())
        .collect();
    let outputs: Vec<String> = design
        .output_ports()
        .iter()
        .filter_map(|signal| signal.name())
        .collect();
    assert_eq!(
        inputs,
        ["clk", "rst", "start", "x_hi", "x_lo", "y_hi", "y_lo"]
    );
    assert_eq!(outputs, ["busy", "done", "z_hi", "z_lo"]);
    assert!(
        design
            .input_ports()
            .into_iter()
            .chain(design.output_ports())
            .all(|port| port.width() <= 64),
        "no port is wider than the cosimulator's driver can drive, so a 128-bit \
         value has to be two 64-bit ports"
    );
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the ghash equivalence did not run. The \
             differential against the `ghash` crate above does not need it."
        );
        return;
    };
    println!("cosimulating ghash with {}", verilator.display());

    let design = Design::new();
    let ports = ghash::build(&design).unwrap();

    let module = ferrite_lithic_rtl::Module::new(
        "ghash",
        design.build().unwrap(),
        ports.inputs.clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let plan = Plan::of(&design, &module).unwrap();
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "start", "x_hi", "x_lo", "y_hi", "y_lo"],
        "the clock is a port but not a stimulus column, and the simulator gets the \
         design's own names"
    );
    assert_eq!(
        plan.outputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["busy", "done", "z_hi", "z_lo"]
    );
    assert!(
        plan.input_widths().iter().all(|width| *width <= 64)
            && plan.outputs.iter().all(|port| port.width <= 64),
        "the generated driver refuses a port wider than 64 bits"
    );

    // Two full multiplies back to back, so the FSM's load, count, finish and
    // completion pulse are all exercised in the equivalence run and not just in the
    // simulator.
    let mut stimulus = Stimulus::new();
    let (zero, _) = ([0u8; 16], [0u8; 16]);
    stimulus.push(row(1, 0, zero, zero)).expect("one bit reset");
    stimulus
        .push(row(0, 0, zero, zero))
        .expect("reset released");
    for index in [3u64, 11] {
        let (h, x) = derived(index);
        stimulus.push(row(0, 1, x, h)).expect("start");
        for _ in 0..ghash::STEPS + 2 {
            stimulus.push(row(0, 0, x, h)).expect("stepping");
        }
    }
    assert_eq!(stimulus.cycles(), 2 + 2 * (1 + ghash::STEPS as usize + 2));

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same two multiplies through the testbench agree with the crate, so all
    // three of them -- simulator, Verilator and the original crate -- meet.
    for index in [3u64, 11] {
        let (h, x) = derived(index);
        let design = Design::new();
        ghash::build(&design).unwrap();
        let tb = Testbench::new(&design).unwrap();
        let observed = tb
            .run(|tb| async move {
                tb.pulse(ferrite_lithic_corpus::RESET).await;
                multiply(&tb, x, h).await
            })
            .expect("the ghash design drives only ports it declares");
        assert_eq!(observed, golden(&h, &x), "derived case {index}");
    }
}
