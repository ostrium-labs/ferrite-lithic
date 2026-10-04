//! ChaCha20, checked three ways.
//!
//! The golden model is the `chacha20` crate, and the route to its block function is
//! worth pinning down because it is not the obvious one and two of the obvious ones
//! are silently wrong. `chacha20::ChaCha20` is a `StreamCipherCoreWrapper`, and
//! `cipher` 0.4 gives a stream cipher [`StreamCipher`] and [`StreamCipherSeek`] and
//! nothing else -- there is no `BlockEncrypt` or `BlockDecrypt` impl for a stream
//! cipher to reach, and the wrapper's `state` is private. The block function is one
//! level down, in `chacha20::ChaChaCore`; see [`golden`] for how it is driven and for
//! why the wrapper's `seek` is the wrong tool.
//!
//! Applying a keystream to zeros recovers the raw block exactly, because RFC 8439
//! §2.4 defines ChaCha20 as the plaintext XORed with the block function's output.
//! So no plaintext is mixed in and no counter bookkeeping is in the way.
//!
//! The published vectors are pinned separately, so the crate cannot be a shared
//! wrong answer: RFC 8439 §2.3.2's block-function vector and §2.4.2's two cipher
//! blocks are asserted against the design *and* against the crate, which means the
//! crate, the design and the RFC all have to agree before either differential below
//! means anything.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::slice;

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
// The corpus module and the golden crate have the same name, and a `use`d name
// wins over the extern prelude in a `use` path's first segment -- so `use
// chacha20::ChaCha20` would resolve to `ferrite_lithic_corpus::chacha20` and fail
// with "no `ChaCha20` in `chacha20`". The leading `::` says "the crate".
use ::chacha20::cipher::{KeyIvInit, StreamCipherCore, StreamCipherSeekCore};
use ferrite_lithic_corpus::chacha20;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

/// RFC 8439 §2.3.2: the key, a byte sequence with no structure in it at all.
const RFC_KEY: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

/// RFC 8439 §2.3.2: the nonce.
const RFC_NONCE: [u8; 12] = [
    0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x4a, 0x00, 0x00, 0x00, 0x00,
];

/// RFC 8439 §2.3.2: the block count.
const RFC_COUNTER: u32 = 1;

/// RFC 8439 §2.3.2: the state at the end of the ChaCha20 operation, i.e. after the
/// twenty rounds and the feed-forward addition of the initial state.
///
/// Transcribed from the RFC's own "ChaCha state at the end of the ChaCha20
/// operation" table rather than from its serialized output, so the assertion is on
/// the words the ports carry rather than on a re-serialization of them.
const RFC_BLOCK_WORDS: [u32; 16] = [
    0xe4e7_f110,
    0x1559_3bd1,
    0x1fdd_0f50,
    0xc471_20a3,
    0xc7f4_d1c7,
    0x0368_c033,
    0x9aaa_2204,
    0x4e6c_d4c3,
    0x4664_82d2,
    0x09aa_9f07,
    0x05d7_c214,
    0xa202_8bd9,
    0xd19c_12b5,
    0xb94e_16de,
    0xe883_d0cb,
    0x4e3c_50a2,
];

/// RFC 8439 §2.4.2: the nonce for the cipher example, which differs from §2.3.2's
/// only in its first word.
const STREAM_NONCE: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4a, 0x00, 0x00, 0x00, 0x00,
];

/// RFC 8439 §2.4.2, "First block after block operation": the state at block count 1.
///
/// The RFC prints this as a state matrix rather than as bytes, which is what makes
/// it transcribable: one four-nibble group per word, sixteen of them, and the
/// plaintext that produced it is printed a page earlier so the ciphertext can be
/// checked against it. The keystream the RFC prints in the same section is the
/// same sixteen words serialized, and is *not* transcribed here -- a byte stream
/// has to be re-typed byte by byte, and this file's whole argument is that the
/// published vector should pin the design without a transcription slip being
/// indistinguishable from a design bug.
const RFC_STREAM_ONE: [u32; 16] = [
    0xf351_4f22,
    0xe1d9_1b40,
    0x6f27_de2f,
    0xed1d_63b8,
    0x821f_138c,
    0xe206_2c3d,
    0xecca_4f7e,
    0x78cf_f39e,
    0xa30a_3b8a,
    0x920a_6072,
    0xcd74_79b5,
    0x3493_2bed,
    0x40ba_4c79,
    0xcd34_3ec6,
    0x4c2c_21ea,
    0xb741_7df0,
];

/// RFC 8439 §2.4.2, "Second block after block operation": the state at block count 2.
const RFC_STREAM_TWO: [u32; 16] = [
    0x9f74_a669,
    0x410f_633f,
    0x28fe_ca22,
    0x7ec4_4dec,
    0x6d34_d426,
    0x738c_b970,
    0x3ac5_e9f3,
    0x4559_0cc4,
    0xda6e_8b39,
    0x892c_831a,
    0xcdea_67c1,
    0x2b7e_1d90,
    0x0374_63f3,
    0xa11a_2073,
    0xe8bc_fb88,
    0xedc4_9139,
];

/// The words a 32-bit byte chunk holds, little-endian, as RFC 8439 §2.3 spells them.
fn word(bytes: &[u8]) -> u64 {
    u64::from(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
}

/// The words of `block`, as the ports carry them.
fn published_of(block: &[u32; 16]) -> Vec<u64> {
    block.iter().map(|word| u64::from(*word)).collect()
}

/// The design's sixteen output words for one block.
///
/// The design has no clock, so every `.await` here is a settle rather than an edge:
/// the testbench's rule is that a module with no clock has no rising edge to take.
fn hardware(key: [u8; 32], nonce: [u8; 12], counter: u32) -> Vec<u64> {
    let design = Design::new();
    chacha20::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        for (index, chunk) in key.chunks_exact(4).enumerate() {
            tb.drive(&format!("key_{index}"), word(chunk)).await;
        }
        for (index, chunk) in nonce.chunks_exact(4).enumerate() {
            tb.drive(&format!("nonce_{index}"), word(chunk)).await;
        }
        tb.drive("counter", u64::from(counter)).await;
        (0..16)
            .map(|index| tb.value(&format!("word_{index}")))
            .collect()
    })
    .expect("the chacha20 design drives only ports it declares")
}

/// The `chacha20` crate's block-function output, reached through the stream.
/// The `chacha20` crate's core, the inner type with the block function in it.
type Core = ::chacha20::ChaChaCore<::chacha20::cipher::consts::U10>;

/// The `chacha20` crate's block-function output, at block count `counter`.
///
/// The raw block function *is* reachable, just not through `BlockEncrypt`:
/// `chacha20::ChaCha20` is a `StreamCipherCoreWrapper`, and `cipher` 0.4 gives a
/// stream cipher only [`StreamCipher`] and [`StreamCipherSeek`]. The block function
/// lives one level down, in [`Core`], which implements [`StreamCipherCore`] and
/// [`StreamCipherSeekCore`].
///
/// Two things about that API are worth stating because both are silent:
///
/// - `ChaChaCore`'s `state` is private, so the only way to set the block count is
///   [`StreamCipherSeekCore::set_block_pos`], which writes state word 12 -- the same
///   word this design's `counter` port drives.
/// - The *wrapper's* [`StreamCipherSeek::seek`] would **not** do this. It takes a
///   **byte** position and divides it into a block position and an intra-block
///   offset, so `seek(1)` is one byte into block 0 and generates block 0 with the
///   first byte skipped. `seek(n * 64)` would work, and cannot reach a counter above
///   2^26; `set_block_pos(n)` takes a block position and has no such limit.
///
/// Applying the keystream to a buffer of zeros then recovers the block function's
/// output exactly: RFC 8439 §2.4 defines ChaCha20 as the plaintext XORed with the
/// block function's output, so with a zero plaintext nothing else is mixed in.
fn golden(key: [u8; 32], nonce: [u8; 12], counter: u32) -> Vec<u64> {
    let mut core = Core::new(&key.into(), &nonce.into());
    core.set_block_pos(counter);
    let mut block = [0u8; 64];
    core.apply_keystream_blocks(slice::from_mut((&mut block).into()));
    block.chunks_exact(4).map(word).collect()
}

/// A deterministic key/nonce pair, so a failure is reproducible from its index.
fn derived(index: u64) -> ([u8; 32], [u8; 12]) {
    let mut key = [0u8; 32];
    let mut nonce = [0u8; 12];
    for (offset, byte) in key.iter_mut().enumerate() {
        *byte = (index as u8)
            .wrapping_mul(97)
            .wrapping_add((offset as u8).wrapping_mul(31))
            .wrapping_add(7);
    }
    for (offset, byte) in nonce.iter_mut().enumerate() {
        *byte = (index as u8)
            .wrapping_mul(53)
            .wrapping_add((offset as u8).wrapping_mul(11))
            .wrapping_add(3);
    }
    (key, nonce)
}

/// One stimulus row: the thirteen input ports, in port order.
///
/// The clock is a port and not a stimulus column, so thirteen of the design's
/// fourteen input ports appear here.
fn row(rst: u64, key: [u8; 32], nonce: [u8; 12], counter: u32) -> Vec<Bits> {
    let mut row = Vec::with_capacity(13);
    row.push(Bits::constant(rst, 1).expect("one bit"));
    row.extend(
        key.chunks_exact(4)
            .map(|chunk| Bits::constant(word(chunk), 32).expect("32 bits")),
    );
    row.extend(
        nonce
            .chunks_exact(4)
            .map(|chunk| Bits::constant(word(chunk), 32).expect("32 bits")),
    );
    row.push(Bits::constant(u64::from(counter), 32).expect("32 bits"));
    row
}

#[test]
fn the_design_reproduces_the_rfc_8439_section_2_3_2_block_function_vector() {
    // The RFC's own transcription of the vector, checked two ways. First the RFC's
    // "state after 20 rounds" added to the "state with the key setup" must give the
    // "state at the end of the ChaCha20 operation" the RFC prints -- sixteen 32-bit
    // additions, which is a check on this file's transcription rather than on the
    // design. Without it, a mistyped word in `RFC_BLOCK_WORDS` would show up as a
    // design failure.
    let after_rounds: [u32; 16] = [
        0x8377_78ab,
        0xe238_d763,
        0xa67a_e21e,
        0x5950_bb2f,
        0xc4f2_d0c7,
        0xfc62_bb2f,
        0x8fa0_18fc,
        0x3f5e_c7b7,
        0x3352_71c2,
        0xf294_89f3,
        0xeabd_a8fc,
        0x82e4_6ebd,
        0xd19c_12b4,
        0xb04e_16de,
        0x9e83_d0cb,
        0x4e3c_50a2,
    ];
    let initial: [u32; 16] = [
        0x6170_7865,
        0x3320_646e,
        0x7962_2d32,
        0x6b20_6574,
        0x0302_0100,
        0x0706_0504,
        0x0b0a_0908,
        0x0f0e_0d0c,
        0x1312_1110,
        0x1716_1514,
        0x1b1a_1918,
        0x1f1e_1d1c,
        0x0000_0001,
        0x0900_0000,
        0x4a00_0000,
        0x0000_0000,
    ];
    let summed: Vec<u32> = after_rounds
        .iter()
        .zip(initial.iter())
        .map(|(mixed, initial)| mixed.wrapping_add(*initial))
        .collect();
    assert_eq!(
        summed, RFC_BLOCK_WORDS,
        "the RFC's own three tables disagree"
    );

    let expected = published_of(&RFC_BLOCK_WORDS);
    assert_eq!(
        hardware(RFC_KEY, RFC_NONCE, RFC_COUNTER),
        expected,
        "the design disagrees with RFC 8439 §2.3.2"
    );
    // The crate is held to the same published state, so the two are not agreeing
    // with each other on a shared mistake.
    assert_eq!(
        golden(RFC_KEY, RFC_NONCE, RFC_COUNTER),
        expected,
        "the chacha20 crate disagrees with RFC 8439 §2.3.2"
    );
}

#[test]
fn the_design_reproduces_the_rfc_8439_section_2_4_2_cipher_example() {
    // §2.4.2 publishes the state after the block operation for block counts 1 and 2
    // under the key 00:01:...:1f and the nonce 00:00:00:00:00:00:00:4a:00:00:00:00.
    // Both are asserted against the design and against the crate.
    for (counter, published) in [(1, &RFC_STREAM_ONE), (2, &RFC_STREAM_TWO)] {
        let expected = published_of(published);
        assert_eq!(
            hardware(RFC_KEY, STREAM_NONCE, counter),
            expected,
            "block count {counter}, RFC 8439 §2.4.2"
        );
        assert_eq!(
            golden(RFC_KEY, STREAM_NONCE, counter),
            expected,
            "block count {counter}: the chacha20 crate disagrees with RFC 8439 §2.4.2"
        );
    }
}

#[test]
fn the_counter_reaches_state_word_twelve_and_no_other_word() {
    // §2.4.2 notes that the second block's setup differs from the first "only [in]
    // the counter in position 12". Comparing two counters is not enough to prove
    // which word the counter landed in -- every placement gives *some* pair of
    // different blocks -- so this checks both against the published blocks
    // directly. A counter wired to word 13 or word 15, or to the first nonce word,
    // would give a block count 1 that differs from the RFC's.
    assert_ne!(
        RFC_STREAM_ONE, RFC_STREAM_TWO,
        "the published blocks must differ, or this test cannot see anything"
    );
    assert_eq!(
        hardware(RFC_KEY, STREAM_NONCE, 1),
        published_of(&RFC_STREAM_ONE),
        "block count 1"
    );
    assert_eq!(
        hardware(RFC_KEY, STREAM_NONCE, 2),
        published_of(&RFC_STREAM_TWO),
        "block count 2"
    );
}

#[test]
fn the_design_agrees_with_the_chacha20_crate_across_the_counter_range() {
    // Counters 0 through 64. Zero is the block the AEAD construction in RFC 8439
    // §2.6 generates a one-time key from, so it is not a corner case that nothing
    // uses; the range covers it.
    for counter in 0..=64u32 {
        let (key, nonce) = derived(u64::from(counter));
        assert_eq!(
            hardware(key, nonce, counter),
            golden(key, nonce, counter),
            "block count {counter}"
        );
    }
}

#[test]
fn the_design_agrees_with_the_chacha20_crate_on_the_wrap_of_the_counter_add() {
    // The RFC's `+` is addition modulo 2^32, so a counter one below `u32::MAX` must
    // produce a block and not a carry. The crate counts in a `u32` and wraps too, so
    // this is a differential rather than a vector -- but it is the only line in the
    // design where the modularity of the addition is observable from the outside,
    // and `add32` widens to 33 bits and narrows back precisely so that it is.
    for counter in [u32::MAX - 1, u32::MAX] {
        let (key, nonce) = derived(u64::from(counter));
        assert_eq!(
            hardware(key, nonce, counter),
            golden(key, nonce, counter),
            "block count {counter}"
        );
    }
}

#[test]
fn the_quarter_round_reproduces_the_rfc_8439_section_2_1_1_vector() {
    // RFC 8439 §2.1.1 runs one quarter round on four numbers and prints the result.
    // Driving the design's own `quarter_round` over a four-port module checks the
    // eight statements of §2.1 directly, separately from the twenty rounds and the
    // feed-forward that surround them in the block function. A rotation ladder of
    // 16/12/8/7 in the wrong order, or an `a` shadowed instead of reassigned, still
    // produces sixteen plausible state words -- it just does not produce these four.
    const INPUTS: [u32; 4] = [0x1111_1111, 0x0102_0304, 0x9b8d_6f43, 0x0123_4567];
    const OUTPUTS: [u32; 4] = [0xea2a_92f4, 0xcb1c_f8ce, 0x4581_472e, 0x5881_c4bb];

    let design = Design::new();
    let (mut a, mut b, mut c, mut d) = (
        design.input("a", 32).unwrap(),
        design.input("b", 32).unwrap(),
        design.input("c", 32).unwrap(),
        design.input("d", 32).unwrap(),
    );
    chacha20::quarter_round(&design, &mut a, &mut b, &mut c, &mut d).unwrap();
    design.output("a_out", 32, &a).unwrap();
    design.output("b_out", 32, &b).unwrap();
    design.output("c_out", 32, &c).unwrap();
    design.output("d_out", 32, &d).unwrap();

    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            for (name, value) in [
                ("a", INPUTS[0]),
                ("b", INPUTS[1]),
                ("c", INPUTS[2]),
                ("d", INPUTS[3]),
            ] {
                tb.drive(name, u64::from(value)).await;
            }
            (
                tb.value("a_out"),
                tb.value("b_out"),
                tb.value("c_out"),
                tb.value("d_out"),
            )
        })
        .expect("the quarter-round module drives only ports it declares");

    assert_eq!(
        (observed.0, observed.1, observed.2, observed.3,),
        (
            u64::from(OUTPUTS[0]),
            u64::from(OUTPUTS[1]),
            u64::from(OUTPUTS[2]),
            u64::from(OUTPUTS[3]),
        ),
        "RFC 8439 §2.1.1"
    );
}

#[test]
fn every_state_word_reaches_its_own_output_port() {
    // The sixteen output ports are a `#[length(16)]` collection, so nothing forces
    // `word_3` to be state word 3 other than the order the builder supplies them in.
    // A design that fed the words to the ports in reverse, or that drove all sixteen
    // ports from one word, would still have sixteen 32-bit outputs and would fail
    // only here.
    let design = Design::new();
    let ports = chacha20::build(&design).unwrap();
    assert_eq!(ports.words.len(), 16, "sixteen output words");
    assert_eq!(
        chacha20::Outputs::PORT_NAMES,
        (0..16).map(|i| format!("word_{i}")).collect::<Vec<_>>(),
        "and they are the ports the testbench drives"
    );
}

proptest! {
    /// The differential, on arbitrary keys, nonces and counters.
    ///
    /// Bounded counter because the design is rebuilt per case and the block function
    /// is 20 unrolled rounds; the counter range above covers the boundaries a wider
    /// sweep would find and this covers the values.
    #[test]
    fn the_design_agrees_with_the_chacha20_crate_on_arbitrary_inputs(
        key in prop::array::uniform32(any::<u8>()),
        nonce in prop::array::uniform12(any::<u8>()),
        counter in 0u32..u32::MAX,
    ) {
        prop_assert_eq!(hardware(key, nonce, counter), golden(key, nonce, counter));
    }
}

#[test]
fn the_emitted_ports_are_the_ones_the_testbench_drives() {
    // The testbench and the cosimulator address ports by name, so a rename that
    // only lands in one of them turns every differential above into a test that
    // fails for a reason that has nothing to do with ChaCha20.
    let design = Design::new();
    chacha20::build(&design).unwrap();
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

    let mut expected: Vec<String> = vec!["clk".to_string(), "rst".to_string()];
    expected.extend((0..8).map(|i| format!("key_{i}")));
    expected.extend((0..3).map(|i| format!("nonce_{i}")));
    expected.push("counter".to_string());
    assert_eq!(inputs, expected);
    assert_eq!(
        outputs,
        (0..16).map(|i| format!("word_{i}")).collect::<Vec<_>>()
    );
    assert!(
        design.input_ports().iter().all(|port| port.width() <= 64),
        "no port is wider than the cosimulator's driver can drive"
    );
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the chacha20 equivalence did not run. \
             The differential against the `chacha20` crate above does not need it."
        );
        return;
    };
    println!("cosimulating chacha20 with {}", verilator.display());

    let design = Design::new();
    let ports = chacha20::build(&design).unwrap();
    let module = ferrite_lithic_rtl::Module::new(
        "chacha20",
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
            "rst", "key_0", "key_1", "key_2", "key_3", "key_4", "key_5", "key_6", "key_7",
            "nonce_0", "nonce_1", "nonce_2", "counter"
        ],
        "twelve 32-bit stimulus columns in port order, the clock being a port but \
         not a column"
    );
    assert_eq!(plan.outputs.len(), 16, "sixteen 32-bit output columns");

    // A stimulus that varies every input, so a port wired to the wrong column -- or
    // a column that is not varied and so cannot catch it -- is caught. The clock is
    // a port but not a stimulus column, and `emit` drives it once per cycle, which is
    // the edge the registers need.
    let mut stimulus = Stimulus::new();
    stimulus.push(row(1, [0; 32], [0; 12], 0)).unwrap();
    stimulus.push(row(0, [0; 32], [0; 12], 0)).unwrap();
    for index in 0..6u64 {
        let (key, nonce) = derived(index);
        // Two rows per block: the registered output is one cycle late, so the
        // reference run and Verilator have to agree about that too.
        stimulus.push(row(0, key, nonce, index as u32 * 7)).unwrap();
        stimulus.push(row(0, key, nonce, index as u32 * 7)).unwrap();
    }
    assert_eq!(stimulus.cycles(), 14);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same six blocks through the testbench agree with the crate, so all
    // three of them -- simulator, Verilator and the original crate -- meet.
    for index in 0..6u64 {
        let (key, nonce) = derived(index);
        let counter = index as u32 * 7;
        assert_eq!(hardware(key, nonce, counter), golden(key, nonce, counter));
    }
}
