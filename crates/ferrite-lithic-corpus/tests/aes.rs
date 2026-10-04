//! AES-128, checked three ways.
//!
//! The golden model is the `aes` crate, and it is pinned twice over: once against the
//! published FIPS 197 / NIST AES-128 ECB known-answer vectors, so the crate is
//! checked rather than trusted, and once against this crate's `SBOX` and `RCON`
//! constants, which are themselves recomputed from their definitions rather than
//! transcribed. Three sources that are each derived rather than copied is the only
//! way "the design agrees with the golden model" means anything -- an AES
//! implementation that is *consistently* wrong is still a plausible-looking design
//! that encrypts with a different S-box, and only an outside vector catches it.
//!
//! The published vectors are FIPS 197 Appendix C.1 (equivalently NIST SP 800-38A
//! F.1.1) and FIPS 197 Appendix B.1:
//!
//! ```text
//! key        000102030405060708090a0b0c0d0e0f
//! plaintext  00112233445566778899aabbccddeeff
//! ciphertext 69c4e0d86a7b0430d8cdb78070b4c55a
//!
//! key        2b7e151628aed2a6abf7158809cf4f3c
//! plaintext  3243f6a8885a308d313198a2e0370734
//! ciphertext 3925841d02dc09fbdc118597196a0b32
//! ```
//!
//! Two rather than one because they fail differently: C.1's key is the byte counter
//! 0x00..0x0f, whose round keys stay unusually structured, and B.1's is not. An
//! S-box or key-schedule bug that survives one has usually been caught by the other.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{Block, BlockEncrypt, KeyInit};
use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::aes as aes_design;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

/// The key of the published known-answer vector.
const FIPS_KEY: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

/// The plaintext of the published known-answer vector.
const FIPS_PLAINTEXT: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];

/// The ciphertext of the published known-answer vector.
const FIPS_CIPHERTEXT: [u8; 16] = [
    0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5, 0x5a,
];

/// The key of the FIPS 197 Appendix B.1 example, which is also the key the
/// zero-block differential uses: a key with no symmetry in it.
const APPENDIX_B_KEY: [u8; 16] = [
    0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c,
];

/// The plaintext of the FIPS 197 Appendix B.1 example.
const APPENDIX_B_PLAINTEXT: [u8; 16] = [
    0x32, 0x43, 0xf6, 0xa8, 0x88, 0x5a, 0x30, 0x8d, 0x31, 0x31, 0x98, 0xa2, 0xe0, 0x37, 0x07, 0x34,
];

/// The ciphertext of the FIPS 197 Appendix B.1 example.
const APPENDIX_B_CIPHERTEXT: [u8; 16] = [
    0x39, 0x25, 0x84, 0x1d, 0x02, 0xdc, 0x09, 0xfb, 0xdc, 0x11, 0x85, 0x97, 0x19, 0x6a, 0x0b, 0x32,
];

/// The key the zero-block and block-sweep differentials use: [`APPENDIX_B_KEY`],
/// named separately because it is the one every test below that does not need a
/// different key reaches for.
const FIXED_KEY: [u8; 16] = APPENDIX_B_KEY;

/// Word `index` of a block, as the design's ports carry it.
fn word_of(block: &[u8; 16], index: usize) -> u64 {
    u64::from(u32::from_be_bytes(
        block[4 * index..4 * index + 4].try_into().unwrap(),
    ))
}

/// The four output words back into sixteen bytes.
fn bytes_of(words: [u64; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (index, word) in words.iter().enumerate() {
        for byte in 0..4usize {
            out[4 * index + byte] = (word >> (8 * (3 - byte))) as u8;
        }
    }
    out
}

/// The `aes` crate's ciphertext for one block.
fn golden(key: &[u8; 16], plaintext: &[u8; 16]) -> [u8; 16] {
    let cipher = Aes128::new(&GenericArray::from(*key));
    let mut block = Block::<Aes128>::clone_from_slice(plaintext);
    cipher.encrypt_block(&mut block);
    block.into()
}

/// A testbench for the design, sharing one build of it.
///
/// [`Testbench`] is cheap to clone and keeps counting cycles across `run` calls, so a
/// hundred vectors cost one design build and one simulation between them rather than
/// one of each per vector.
fn rig() -> Testbench {
    let design = Design::new();
    aes_design::build(&design).expect("the aes design builds");
    Testbench::new(&design).expect("the aes design compiles")
}

/// Drives one block through the design and returns the ciphertext register.
///
/// Reset is pulsed first, then the four key words, then the four plaintext words. The
/// last of those eight edges is the one that latches the answer, so the ciphertext is
/// read after that edge -- ADR-0018's boundary, and the only one the testbench and
/// the cosimulator agree on.
fn encrypt(tb: &Testbench, key: &[u8; 16], plaintext: &[u8; 16]) -> [u8; 16] {
    let observed = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            for index in 0..4 {
                tb.drive(&format!("key{index}"), word_of(key, index)).await;
            }
            for index in 0..4 {
                tb.drive(&format!("in{index}"), word_of(plaintext, index))
                    .await;
            }
            [
                tb.value("out0"),
                tb.value("out1"),
                tb.value("out2"),
                tb.value("out3"),
            ]
        })
        .expect("the aes design drives only ports it declares");
    bytes_of(observed)
}

/// Multiplication by `{02}` in GF(2^8), in software, for the tests below.
fn software_xtime(byte: u8) -> u8 {
    let shifted = byte << 1;
    if byte & 0x80 == 0 {
        shifted
    } else {
        shifted ^ 0x1b
    }
}

/// Multiplication in GF(2^8) modulo `0x11b`, by shift-and-add.
fn software_gf_mul(mut left: u8, mut right: u8) -> u8 {
    let mut product = 0u8;
    for _ in 0..8 {
        if right & 1 == 1 {
            product ^= left;
        }
        right >>= 1;
        left = software_xtime(left);
    }
    product
}

/// The multiplicative inverse in GF(2^8), with FIPS 197's `0 -> 0`.
fn software_inverse(byte: u8) -> u8 {
    if byte == 0 {
        return 0;
    }
    (1..=255u16)
        .map(|candidate| candidate as u8)
        .find(|candidate| software_gf_mul(byte, *candidate) == 1)
        .expect("every nonzero element of a field has an inverse")
}

/// The affine transform of FIPS 197 §5.1.3 eq. 7, over GF(2).
fn software_affine(inverse: u8) -> u8 {
    let mut out = 0u8;
    for bit in 0..8u32 {
        let term = ((inverse >> bit) & 1)
            ^ ((inverse >> ((bit + 4) % 8)) & 1)
            ^ ((inverse >> ((bit + 5) % 8)) & 1)
            ^ ((inverse >> ((bit + 6) % 8)) & 1)
            ^ ((inverse >> ((bit + 7) % 8)) & 1)
            ^ ((0x63 >> bit) & 1);
        out |= term << bit;
    }
    out
}

#[test]
fn the_sbox_is_the_multiplicative_inverse_and_the_affine_transform() {
    // Recomputed from the definition, entry by entry, with no table involved. A
    // mistyped hex digit in the design's constant fails here rather than producing a
    // design that encrypts with a slightly different S-box and agrees with nothing.
    for (index, expected) in aes_design::SBOX.iter().enumerate() {
        let derived = software_affine(software_inverse(index as u8));
        assert_eq!(
            derived, *expected,
            "S-box entry {index:#04x} is not inverse({index:#04x}) then affine"
        );
    }
}

#[test]
fn the_sbox_is_a_permutation_with_the_endpoints_the_specification_names() {
    // A substitution table that is not a permutation is not an S-box. FIPS 197 names
    // two of its entries explicitly, and they are the two a truncation error eats.
    assert_eq!(aes_design::SBOX[0x00], 0x63);
    assert_eq!(aes_design::SBOX[0x53], 0xed);
    let mut seen = [false; 256];
    for value in aes_design::SBOX {
        assert!(!seen[usize::from(value)], "0x{value:02x} appears twice");
        seen[usize::from(value)] = true;
    }
    assert!(seen.iter().all(|hit| *hit), "every value appears");
}

#[test]
fn the_rcon_table_is_ten_xtimes_applied_to_one() {
    // FIPS 197 §5.2's ten constants, and the fact that generates all of them: each is
    // the previous one multiplied by {02}. The interesting one is the ninth, where
    // 0x80 overflows eight bits and comes back as 0x1b -- a transcription that
    // "corrected" it to 0x1c would still look like a plausible table.
    assert_eq!(aes_design::RCON.len(), aes_design::ROUNDS);
    assert_eq!(aes_design::RCON[0], 0x01);
    for (round, constant) in aes_design::RCON.iter().enumerate().skip(1) {
        assert_eq!(
            *constant,
            software_xtime(aes_design::RCON[round - 1]),
            "Rcon[{round}] is not xtime of Rcon[{}]",
            round - 1
        );
    }
    assert_eq!(aes_design::REDUCTION, 0x1b);
}

#[test]
fn the_golden_model_reproduces_the_published_vectors() {
    // Before the golden model is used to judge anything, it is judged. Without this,
    // a differential test could be comparing a correct design with a broken crate.
    assert_eq!(golden(&FIPS_KEY, &FIPS_PLAINTEXT), FIPS_CIPHERTEXT);
    assert_eq!(
        golden(&APPENDIX_B_KEY, &APPENDIX_B_PLAINTEXT),
        APPENDIX_B_CIPHERTEXT
    );
}

#[test]
fn the_design_reproduces_the_published_vectors() {
    let tb = rig();
    assert_eq!(
        encrypt(&tb, &FIPS_KEY, &FIPS_PLAINTEXT),
        FIPS_CIPHERTEXT,
        "FIPS 197 Appendix C.1, the AES-128 ECB known-answer vector"
    );
    assert_eq!(
        encrypt(&tb, &APPENDIX_B_KEY, &APPENDIX_B_PLAINTEXT),
        APPENDIX_B_CIPHERTEXT,
        "FIPS 197 Appendix B.1, whose key is not a byte counter"
    );
}

#[test]
fn the_design_agrees_with_the_aes_crate_on_a_fixed_key_and_the_zero_block() {
    let tb = rig();
    assert_eq!(
        encrypt(&tb, &FIXED_KEY, &[0u8; 16]),
        golden(&FIXED_KEY, &[0u8; 16])
    );
}

#[test]
fn the_design_agrees_with_the_aes_crate_across_a_table_of_blocks() {
    // Chosen so that a mistake in one of the four steps cannot hide: all zeros and
    // all ones (every byte the same, so a byte permutation is invisible), one
    // nonzero byte in each of the four words in turn (so a word-order slip in the
    // ports shows up), and the FIPS plaintext (a known-answer vector in its own
    // right). Both an all-zero key and a real key, because the key schedule is the
    // part most likely to be wrong and an all-zero key is the degenerate one.
    let blocks: [[u8; 16]; 8] = [
        [0x00; 16],
        [0xff; 16],
        {
            let mut block = [0x00; 16];
            block[0] = 0x01;
            block
        },
        {
            let mut block = [0x00; 16];
            block[5] = 0x80;
            block
        },
        {
            let mut block = [0x00; 16];
            block[10] = 0x01;
            block
        },
        {
            let mut block = [0x00; 16];
            block[15] = 0xff;
            block
        },
        FIPS_PLAINTEXT,
        APPENDIX_B_PLAINTEXT,
    ];
    let keys: [[u8; 16]; 3] = [FIXED_KEY, [0x00; 16], [0xff; 16]];

    let tb = rig();
    for key in keys {
        for block in blocks {
            assert_eq!(
                encrypt(&tb, &key, &block),
                golden(&key, &block),
                "key {:02x?} block {:02x?}",
                key,
                block
            );
        }
    }
}

#[test]
fn the_step_testbench_sees_the_ciphertext_one_edge_after_the_block() {
    // The design is one edge behind its inputs, and a test that read the ports
    // *before* the edge would see the previous ciphertext and pass on the wrong
    // vector. So the boundary is asserted rather than assumed: a read taken while
    // the block is still arriving is the encryption of a *different* block, and the
    // same ports after the latching edge are the answer.
    //
    // Note what a reset pulse is not: it is two edges, and the second one releases
    // reset, so the register does not sit at zero afterwards -- it holds whatever the
    // ports say at that edge. `a_reset_mid_stream_clears_the_ciphertext_register` is
    // where the zero is asserted, on an edge that actually asserts reset.
    let tb = rig();
    let observed = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            for index in 0..4 {
                tb.drive(&format!("key{index}"), word_of(&FIPS_KEY, index))
                    .await;
            }
            // The key words alone are not a block, but they are an edge, so the
            // register now holds the encryption of a *partly* driven block. Reading
            // it here is what makes the next assertion meaningful rather than
            // vacuous.
            let after_key = tb.value("out0");
            for index in 0..4 {
                tb.drive(&format!("in{index}"), word_of(&FIPS_PLAINTEXT, index))
                    .await;
            }
            let after_block = [
                tb.value("out0"),
                tb.value("out1"),
                tb.value("out2"),
                tb.value("out3"),
            ];
            // One edge with the block still on the input ports: the ciphertext is
            // unchanged, which is the one-cycle latency stated in the module docs.
            tb.step().await;
            let after_hold = [
                tb.value("out0"),
                tb.value("out1"),
                tb.value("out2"),
                tb.value("out3"),
            ];
            (after_key, after_block, after_hold)
        })
        .unwrap();

    assert_ne!(
        observed.0,
        u64::from(u32::from_be_bytes(
            FIPS_CIPHERTEXT[0..4].try_into().unwrap()
        )),
        "the partly-driven read is not already the answer, so the next one is not \
         vacuous"
    );
    assert_eq!(
        bytes_of(observed.1),
        FIPS_CIPHERTEXT,
        "read after the latching edge"
    );
    assert_eq!(
        bytes_of(observed.2),
        FIPS_CIPHERTEXT,
        "and it holds while the same block stays on the input ports"
    );
}

#[test]
fn the_step_testbench_encrypts_one_block_per_cycle() {
    // The throughput claim of the unrolled design: no bubble between blocks. Two
    // consecutive blocks, each read after the edge that latched it -- which is the
    // last of the four edges that drove its words, because the register takes
    // whatever the ports say at that edge.
    let tb = rig();
    let observed = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            for index in 0..4 {
                tb.drive(&format!("key{index}"), word_of(&FIXED_KEY, index))
                    .await;
            }
            let mut out = Vec::new();
            for block in [FIPS_PLAINTEXT, [0x00; 16]] {
                for index in 0..4 {
                    tb.drive(&format!("in{index}"), word_of(&block, index))
                        .await;
                }
                out.push([
                    tb.value("out0"),
                    tb.value("out1"),
                    tb.value("out2"),
                    tb.value("out3"),
                ]);
            }
            out
        })
        .unwrap();

    assert_eq!(
        bytes_of(observed[0]),
        golden(&FIXED_KEY, &FIPS_PLAINTEXT),
        "the first block, read after its latching edge"
    );
    assert_eq!(
        bytes_of(observed[1]),
        golden(&FIXED_KEY, &[0x00; 16]),
        "and the second block four edges later, with no bubble between them"
    );
    assert_ne!(observed[0], observed[1], "two blocks are two ciphertexts");
}

#[test]
fn a_reset_mid_stream_clears_the_ciphertext_register() {
    // A reset that is never asserted while the register holds something is not a
    // reset that has been tested.
    let tb = rig();
    let observed = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            for index in 0..4 {
                tb.drive(&format!("key{index}"), word_of(&FIXED_KEY, index))
                    .await;
            }
            for index in 0..4 {
                tb.drive(&format!("in{index}"), word_of(&FIPS_PLAINTEXT, index))
                    .await;
            }
            let before = tb.value("out0");
            tb.drive(ferrite_lithic_corpus::RESET, 1).await;
            let during = [tb.value("out0"), tb.value("out1")];
            tb.drive(ferrite_lithic_corpus::RESET, 0).await;
            let after = [tb.value("out0"), tb.value("out1")];
            (before, during, after)
        })
        .unwrap();

    assert_ne!(
        observed.0, 0,
        "the register held a ciphertext before the reset"
    );
    assert_eq!(
        observed.1,
        [0, 0],
        "reset asserted on an edge clears all four ciphertext registers"
    );
    // Releasing reset is another edge and the block is still on the input ports, so
    // the design has encrypted that block again -- the same ciphertext it held before
    // the reset, because nothing about the inputs changed. Asserting it is the
    // difference between testing the release path and pretending it was exercised.
    assert_eq!(
        observed.2,
        [
            observed.0,
            u64::from(u32::from_be_bytes(
                golden(&FIXED_KEY, &FIPS_PLAINTEXT)[4..8]
                    .try_into()
                    .unwrap()
            ))
        ],
        "and the release edge re-encrypts the block still on the input ports"
    );
}

#[test]
fn the_ciphertext_ports_hold_four_bytes_each_and_the_block_is_not_one_port() {
    // The cosimulator's driver refuses a port wider than 64 bits and the testbench
    // records TooWide rather than truncating, so a 128-bit port list would not
    // elaborate. This asserts the shape that works, and it is a shape assertion
    // rather than a style one: a rename or a merge that produced a 128-bit port
    // would break every cosimulated design in the workspace.
    let design = Design::new();
    aes_design::build(&design).unwrap();
    let inputs: Vec<(String, u32)> = design
        .input_ports()
        .iter()
        .map(|signal| {
            (
                signal.name().expect("every input port is named"),
                signal.width(),
            )
        })
        .collect();
    let outputs: Vec<(String, u32)> = design
        .output_ports()
        .iter()
        .map(|signal| {
            (
                signal.name().expect("every output port is named"),
                signal.width(),
            )
        })
        .collect();

    assert_eq!(
        inputs
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "clk", "rst", "in0", "in1", "in2", "in3", "key0", "key1", "key2", "key3"
        ],
        "the clock is a port, the eight data words follow it"
    );
    assert_eq!(
        outputs
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["out0", "out1", "out2", "out3"]
    );
    assert!(
        inputs
            .iter()
            .chain(outputs.iter())
            .all(|(_, width)| *width <= 64),
        "no port is wider than the 64 bits the generated driver can carry"
    );
    assert_eq!(
        inputs[0].0,
        ferrite_lithic_corpus::CLOCK,
        "the crate's clock name"
    );
    assert_eq!(
        inputs[1].0,
        ferrite_lithic_corpus::RESET,
        "and its reset name"
    );
}

proptest! {
    /// The differential, on arbitrary keys and blocks.
    ///
    /// Sixteen cases, because the per-case cost is a full ten-round evaluation rather
    /// than a byte: this sweeps the *values*, and the table above covers the shapes
    /// a random sweep would rarely produce.
    #[test]
    fn the_design_agrees_with_the_aes_crate_on_arbitrary_keys_and_blocks(
        key in any::<[u8; 16]>(),
        block in any::<[u8; 16]>(),
    ) {
        let tb = rig();
        prop_assert_eq!(encrypt(&tb, &key, &block).to_vec(), golden(&key, &block).to_vec());
    }
}

#[test]
fn the_emitted_verilog_has_one_sbox_read_per_byte_and_per_key_word() {
    // The S-box is a 256-arm `case`, and it is read 160 times by the ten rounds of
    // SubBytes and 40 times by the ten key schedule's SubWord. Counting the emitted
    // `case` blocks pins the ROM shape -- a design that computed the S-box as logic,
    // or expanded the key schedule serially, would have a different count and would
    // still produce correct ciphertext in the simulator.
    let design = Design::new();
    let ports = aes_design::build(&design).unwrap();
    let module = ferrite_lithic_rtl::Module::new(
        "aes",
        design.build().unwrap(),
        ports.inputs.clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let verilog = module.emit().unwrap();
    let lookups = verilog.matches("case (").count();
    assert_eq!(
        lookups,
        10 * (16 + 4),
        "160 SubBytes reads plus 40 SubWord reads, each one a ROM"
    );
    assert!(
        verilog.contains("input  wire [31:0] in0"),
        "the four plaintext words are separate ports: {verilog}"
    );
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the aes equivalence did not run. The \
             differential against the `aes` crate above does not need it."
        );
        return;
    };
    println!("cosimulating aes with {}", verilator.display());

    let design = Design::new();
    let ports = aes_design::build(&design).unwrap();

    let module = ferrite_lithic_rtl::Module::new(
        "aes",
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
        [
            "rst", "in0", "in1", "in2", "in3", "key0", "key1", "key2", "key3"
        ],
        "the clock is a port but not a stimulus column, and the simulator gets the \
         design's own names"
    );
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.verilog.as_str())
            .collect::<Vec<_>>(),
        [
            "rst", "in0", "in1", "in2", "in3", "key0", "key1", "key2", "key3"
        ],
        "while the driver gets the names the Verilog declares -- none of these is a \
         reserved word, so the two lists agree"
    );
    assert_eq!(
        plan.outputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["out0", "out1", "out2", "out3"]
    );

    // A stimulus that resets and then encrypts several blocks back to back, one per
    // cycle. The register is exercised rather than sitting at its reset value, and
    // the throughput claim is checked by the cosimulator too.
    let vectors = [
        (FIPS_KEY, FIPS_PLAINTEXT),
        (FIXED_KEY, [0x00; 16]),
        ([0x00; 16], [0xff; 16]),
        ([0xff; 16], FIXED_KEY),
        (FIXED_KEY, FIPS_PLAINTEXT),
    ];

    let mut stimulus = Stimulus::new();
    let row = |reset: u64, key: &[u8; 16], block: &[u8; 16]| {
        vec![
            Bits::constant(reset, 1).unwrap(),
            Bits::constant(word_of(block, 0), 32).unwrap(),
            Bits::constant(word_of(block, 1), 32).unwrap(),
            Bits::constant(word_of(block, 2), 32).unwrap(),
            Bits::constant(word_of(block, 3), 32).unwrap(),
            Bits::constant(word_of(key, 0), 32).unwrap(),
            Bits::constant(word_of(key, 1), 32).unwrap(),
            Bits::constant(word_of(key, 2), 32).unwrap(),
            Bits::constant(word_of(key, 3), 32).unwrap(),
        ]
    };
    stimulus.push(row(1, &[0x00; 16], &[0x00; 16])).unwrap();
    stimulus.push(row(0, &[0x00; 16], &[0x00; 16])).unwrap();
    for (key, block) in vectors {
        stimulus.push(row(0, &key, &block)).unwrap();
    }
    assert_eq!(stimulus.cycles(), vectors.len() + 2);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same vectors through the testbench agree with the `aes` crate, so all
    // three of them -- simulator, Verilator and the original crate -- meet.
    let tb = rig();
    for (key, block) in vectors {
        assert_eq!(
            encrypt(&tb, &key, &block),
            golden(&key, &block),
            "the cosimulated vector, through the testbench this time"
        );
    }
}
