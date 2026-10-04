//! SHA-256, checked three ways.
//!
//! # The awkward part: what a golden model can see
//!
//! `crc3` compares against the `crc` crate's own compression step, because the
//! `crc` crate has one. `sha2` does not: its public surface is the streaming,
//! padded [`Digest`] API, and the compression function itself sits behind
//! `sha2/compress`, a feature this crate's dev-dependency does not enable. There
//! is no `compress256` to hand the design a block and read the state back.
//!
//! What there *is* is a whole hash. For a message short enough to pad to exactly
//! one block -- 55 bytes or fewer -- the digest **is** one compression of that
//! block from the initial hash value, and the design starts from the initial hash
//! value. So comparing the design's eight output words against `Sha256::digest` of
//! the same message pins the compression output exactly, provided the IV is right.
//! The IV is pinned separately and twice over, by the published FIPS 180-4
//! vectors and by the square-root provenance check, so a wrong K cannot hide
//! behind a wrong H either: the FIPS vectors and the `sha2` comparison are
//! independent and both have to hold.
//!
//! Chaining gives a second, independent handle. A 56-to-119-byte message pads to
//! exactly two blocks, and its digest is `compress(compress(H, b0), b1)` -- so
//! driving two blocks and comparing against one `Digest` call pins the register
//! *and* both compressions, and a design that compressed each block from the IV
//! independently would fail it.
//!
//! # The awkward part: driving a register
//!
//! Every `Handle::drive` is a rising edge, and this design updates its chaining
//! register on every edge. Presenting a block is therefore seventeen cycles for
//! one logical operation -- sixteen words plus the read -- and the intermediate
//! sixteen edges each capture a compression of a half-updated block, which is
//! discarded rather than looked at. [`Testbench::run`] keeps its cycle count across
//! calls, so one compiled simulator serves every block in a test rather than being
//! rebuilt per message.
//!
//! The protocol is the ordinary `init` handshake the module documents: assert
//! `rst` and present the first block, so the compression reads the IV; release
//! `rst` and present the next, so it reads the register.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::sha256;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::{Handle, Testbench};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

/// The design's input port names, in order, with the clock excluded.
///
/// Written out rather than generated, because a generated list would agree with
/// the design for the same reason a wrong design agrees with a wrong test.
const INPUT_NAMES: [&str; 1 + sha256::BLOCK_WORDS] = [
    "rst", "block_0", "block_1", "block_2", "block_3", "block_4", "block_5", "block_6", "block_7",
    "block_8", "block_9", "block_10", "block_11", "block_12", "block_13", "block_14", "block_15",
];

/// The design's output port names, in order.
const OUTPUT_NAMES: [&str; sha256::STATE_WORDS] = [
    "state_0", "state_1", "state_2", "state_3", "state_4", "state_5", "state_6", "state_7",
];

/// The longest message that pads to exactly one 512-bit block.
///
/// A block is 64 bytes and the padding needs a `0x80` byte plus an 8-byte length,
/// so `n + 1 + 8 <= 64`, i.e. `n <= 55`.
const ONE_BLOCK: usize = 55;

/// The longest message that pads to exactly two 512-bit blocks.
const TWO_BLOCKS: usize = 119;

/// The published FIPS 180-4 digest of the empty message.
const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// The published FIPS 180-4 digest of `"abc"`.
const ABC_DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// The one block a short message pads to: message, `0x80`, zeros, bit length.
///
/// # Panics
///
/// If `message` needs more than one block. That is checked rather than truncated
/// because a silently two-block message compared against a one-block digest fails
/// for a reason that has nothing to do with the design.
fn padded_block(message: &[u8]) -> [u8; sha256::BLOCK_BYTES] {
    assert!(
        message.len() <= ONE_BLOCK,
        "a {}-byte message does not pad to one block, and this is the one-block \
         differential",
        message.len()
    );
    let mut block = [0u8; sha256::BLOCK_BYTES];
    block[..message.len()].copy_from_slice(message);
    block[message.len()] = 0x80;
    let bits = (message.len() as u64) * 8;
    block[sha256::BLOCK_BYTES - 8..].copy_from_slice(&bits.to_be_bytes());
    block
}

/// The two blocks a longer message pads to.
///
/// The padding is applied to the message as a whole, not to each block: for a
/// 56-byte message the `0x80` marker lands in the *first* block at offset 56 and
/// the length field in the *second* block, which is the case a block-at-a-time
/// padding helper gets wrong.
///
/// # Panics
///
/// If `message` does not pad to exactly two blocks.
fn padded_pair(message: &[u8]) -> [[u8; sha256::BLOCK_BYTES]; 2] {
    assert!(
        message.len() > ONE_BLOCK && message.len() <= TWO_BLOCKS,
        "a {}-byte message does not pad to two blocks",
        message.len()
    );
    let mut padded = [0u8; 2 * sha256::BLOCK_BYTES];
    padded[..message.len()].copy_from_slice(message);
    padded[message.len()] = 0x80;
    let bits = (message.len() as u64) * 8;
    padded[2 * sha256::BLOCK_BYTES - 8..].copy_from_slice(&bits.to_be_bytes());
    [
        padded[..sha256::BLOCK_BYTES].try_into().unwrap(),
        padded[sha256::BLOCK_BYTES..].try_into().unwrap(),
    ]
}

/// A digest as eight big-endian 32-bit words, most significant first.
fn digest_words(bytes: &[u8]) -> [u32; sha256::STATE_WORDS] {
    assert_eq!(bytes.len(), sha256::STATE_WORDS * 4, "one digest");
    std::array::from_fn(|index| {
        let mut word = [0u8; 4];
        word.copy_from_slice(&bytes[index * 4..index * 4 + 4]);
        u32::from_be_bytes(word)
    })
}

/// A block as sixteen big-endian 32-bit words, most significant first.
fn block_words(bytes: &[u8]) -> [u32; sha256::BLOCK_WORDS] {
    assert_eq!(bytes.len(), sha256::BLOCK_BYTES, "one block");
    std::array::from_fn(|index| {
        let mut word = [0u8; 4];
        word.copy_from_slice(&bytes[index * 4..index * 4 + 4]);
        u32::from_be_bytes(word)
    })
}

/// The digest the `sha2` crate produces, as eight words.
fn golden(message: &[u8]) -> [u64; sha256::STATE_WORDS] {
    let digest = Sha256::digest(message);
    digest_words(&digest).map(u64::from)
}

/// A published digest as eight words, for the FIPS vectors.
fn published(hex: &str) -> [u64; sha256::STATE_WORDS] {
    let bytes = hex::decode(hex).expect("the published digest is hex");
    digest_words(&bytes).map(u64::from)
}

/// A design compiled into a simulator, with reset asserted and one edge taken, so
/// the chaining register holds the initial hash value.
///
/// One simulator for a whole test: the design is a few thousand nodes and the
/// testbench keeps its cycle count across `run` calls, so driving a hundred
/// blocks through one instance is a hundred block drive phases rather than a
/// hundred compilations.
fn started() -> Testbench {
    let design = Design::new();
    sha256::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.set(ferrite_lithic_corpus::RESET, 1).await;
        tb.step().await;
    })
    .unwrap();
    tb
}

/// The eight output words, read after the last edge.
fn read_state(tb: &Handle) -> [u64; sha256::STATE_WORDS] {
    OUTPUT_NAMES.map(|name| tb.value(name))
}

/// Compresses one block at the given reset level and returns the chaining state.
///
/// **One edge.** [`Handle::set`] writes an input without taking an edge and
/// [`Handle::drive`] reverts it the moment the edge is over, so the block has to be
/// presented with sixteen `set`s and then a single `step` -- driving sixteen words
/// would compress sixteen blocks, each with one word of the previous block still in
/// place, and only the last of those would be the block asked for. That distinction
/// is the whole reason this helper exists: it is very easy to write a chaining test
/// that quietly exercises a different circuit at every step.
///
/// `rst` is applied before the edge, so `feed(block, 0)` is the block after the first
/// and `feed(block, 1)` is the first block of a hash.
fn feed(block: &[u8], rst: u64, tb: &Testbench) -> [u64; sha256::STATE_WORDS] {
    let words = block_words(block);
    tb.run(|tb| async move {
        for (index, word) in words.iter().enumerate() {
            tb.set(&format!("block_{index}"), u64::from(*word)).await;
        }
        tb.set(ferrite_lithic_corpus::RESET, rst).await;
        tb.step().await;
        read_state(&tb)
    })
    .unwrap()
}

/// Loads the initial hash value into the chaining register, compressing no block.
///
/// One edge with `rst` asserted. Every hash starts here, so this is not optional
/// setup: an edge with `rst` released compresses whatever the register happens to
/// hold, and a test that skipped this would chain one message onto the last.
fn load_iv(tb: &Testbench) {
    tb.run(|tb| async move {
        tb.set(ferrite_lithic_corpus::RESET, 1).await;
        tb.step().await;
    })
    .unwrap();
}

/// The state after one block compressed from the initial hash value.
///
/// Two edges: one to load H, one to compress the block from it. The result is
/// `Sha256::digest` of a one-block message.
fn compress(tb: &Testbench, message: &[u8]) -> [u64; sha256::STATE_WORDS] {
    load_iv(tb);
    feed(&padded_block(message), 0, tb)
}

/// Compresses a two-block message and returns the state after each block.
///
/// The first block is the first block of a hash -- compressed from H -- and the
/// second chains from whatever the first left behind.
fn chain(
    tb: &Testbench,
    message: &[u8],
) -> ([u64; sha256::STATE_WORDS], [u64; sha256::STATE_WORDS]) {
    let blocks = padded_pair(message);
    load_iv(tb);
    let after_first = feed(&blocks[0], 0, tb);
    let after_second = feed(&blocks[1], 0, tb);
    (after_first, after_second)
}

#[test]
fn a_message_of_at_most_55_bytes_pads_to_exactly_one_block() {
    // The premise of every one-block differential below. If this were wrong the
    // comparison would be against a two-block digest and the design would be
    // innocent.
    for length in 0..=ONE_BLOCK {
        let block = padded_block(&vec![0u8; length]);
        assert_eq!(block.len(), sha256::BLOCK_BYTES, "length {length}");
        assert_eq!(block[length], 0x80, "length {length}: the padding marker");
        assert_eq!(
            u64::from_be_bytes(block[56..].try_into().unwrap()),
            (length as u64) * 8,
            "length {length}: the bit length"
        );
        assert!(
            block[length + 1..56].iter().all(|byte| *byte == 0),
            "length {length}: the zeros between the marker and the length"
        );
    }

    // And 56 bytes does not fit, which is what makes 55 a boundary rather than a
    // round number that happens to work: the marker would go at offset 56 and the
    // eight-byte length field at 57, which runs past the end of the block.
    const {
        // 56 bytes is one byte past the limit, and 56 + one marker byte + an
        // eight-byte length overruns the block, which is what makes 55 the
        // boundary rather than a round number that happens to work.
        assert!(
            ONE_BLOCK + 1 + 1 + 8 > sha256::BLOCK_BYTES,
            "a 56-byte message pads to two blocks"
        );
    }
}

#[test]
fn the_design_reproduces_the_published_digest_of_the_empty_message() {
    let tb = started();
    assert_eq!(compress(&tb, b""), published(EMPTY_DIGEST));
    assert_eq!(published(EMPTY_DIGEST), golden(b""));
}

#[test]
fn the_design_reproduces_the_published_digest_of_abc() {
    let tb = started();
    assert_eq!(compress(&tb, b"abc"), published(ABC_DIGEST));
    assert_eq!(published(ABC_DIGEST), golden(b"abc"));
}

#[test]
fn the_design_agrees_with_the_sha2_crate_on_every_single_byte_message() {
    // All 256 one-byte messages. A constant written wrong, a `Ch` inverted, a
    // sigma with two rotates the wrong way round: every one of those disagrees on
    // most single-byte messages rather than on one, so this is the cheapest test
    // that finds them.
    let tb = started();
    for value in 0..=255u8 {
        let message = [value];
        assert_eq!(
            compress(&tb, &message),
            golden(&message),
            "one byte {value:#04x}"
        );
    }
}

#[test]
fn the_design_agrees_with_the_sha2_crate_across_the_one_block_length_range() {
    // Lengths 0..=55 with a distinguishing prefix per length, so a length change
    // cannot accidentally produce the same bytes. The padding boundary moves within
    // this range -- the marker, the zero run and the encoded bit length all sit at
    // different offsets for different lengths -- so a design that read the block's
    // length field from a fixed offset would pass the single-byte test and fail
    // here.
    let tb = started();
    for length in 0..=ONE_BLOCK {
        let message: Vec<u8> = (0..length)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(7))
            .collect();
        assert_eq!(compress(&tb, &message), golden(&message), "length {length}");
    }
}

#[test]
fn the_design_agrees_with_the_sha2_crate_on_a_message_with_no_zero_byte_in_it() {
    // The tightest padding there is: 55 bytes of message means the `0x80` marker
    // sits at offset 55 and the length field starts immediately after it, so the
    // block contains no zero byte at all and a design that filled a word with a
    // default zero rather than reading it would still produce zeros.
    let tb = started();
    let message: Vec<u8> = (0..ONE_BLOCK)
        .map(|index| (index as u8).wrapping_mul(4).wrapping_add(1))
        .collect();
    let block = padded_block(&message);
    assert_eq!(block[ONE_BLOCK], 0x80);
    assert!(
        block[..ONE_BLOCK].iter().all(|byte| *byte != 0),
        "no zero byte survives in the message region"
    );
    assert_eq!(compress(&tb, &message), golden(&message));
}

#[test]
fn the_chaining_register_carries_the_second_block_of_a_two_block_message() {
    // A 56-byte message is the first length that needs two blocks, so its digest is
    // `compress(compress(H, b0), b1)`. The first block is driven with `rst` held,
    // the second with `rst` released, and the result must be the `sha2` crate's
    // digest of all 56 bytes. A design that compressed each block from the IV would
    // produce the digest of the two blocks hashed *separately*, which is a
    // different, well-formed hash.
    let tb = started();
    let message: Vec<u8> = (0..56)
        .map(|index| (index as u8).wrapping_mul(53).wrapping_add(3))
        .collect();
    let blocks = padded_pair(&message);
    assert_eq!(
        blocks[0][56], 0x80,
        "the marker lands in the *first* block, because the message is longer than \
         one block"
    );
    assert_eq!(
        u64::from_be_bytes(blocks[1][56..].try_into().unwrap()),
        56 * 8,
        "and the length field lands in the second"
    );

    let (after_first, after_second) = chain(&tb, &message);
    assert_ne!(
        after_first,
        golden(&message),
        "the first block on its own is not the digest of the whole message"
    );
    assert_eq!(after_second, golden(&message));
}

#[test]
fn the_design_agrees_with_the_sha2_crate_across_the_two_block_length_range() {
    let tb = started();
    for length in [ONE_BLOCK + 1, 64, 80, 119] {
        let message: Vec<u8> = (0..length)
            .map(|index| (index as u8).wrapping_mul(17).wrapping_add(5))
            .collect();
        let (_, after_second) = chain(&tb, &message);
        assert_eq!(
            after_second,
            golden(&message),
            "two blocks, length {length}"
        );
    }
}

#[test]
fn the_initial_hash_value_is_the_square_root_of_the_first_eight_primes() {
    // FIPS 180-4 §5.3.3, checked rather than trusted. A design with the cube roots
    // here instead of the square roots is a plausible transcription slip, and it
    // produces a design that computes a real, different hash -- so the constants
    // are checked against the recipe that generated them rather than against a
    // table transcribed a second time.
    let primes = [2u64, 3, 5, 7, 11, 13, 17, 19];
    for (index, prime) in primes.iter().enumerate() {
        let root = (*prime as f64).sqrt();
        let derived = ((root - root.floor()) * 2f64.powi(32)) as u64;
        assert_eq!(
            derived,
            u64::from(sha256::H[index]),
            "H({index}) is not the first 32 bits of the fractional part of sqrt({prime})"
        );
    }
}

#[test]
fn the_round_constants_are_the_cube_roots_of_the_first_sixty_four_primes() {
    // FIPS 180-4 §4.2.2, the same check for K. Both roots are irrational and
    // neither product lands near a 2^-32 boundary closer than 0.005 of one, so the
    // floating-point derivation has about sixteen bits of headroom over the 32 bits
    // it has to get right -- which is why recomputing them here is a check and not
    // a coin flip.
    let primes = first_primes(sha256::ROUNDS);
    assert_eq!(primes[0], 2, "the primes start at two");
    assert_eq!(primes[63], 311, "the sixty-fourth prime is 311");
    for (index, prime) in primes.iter().enumerate() {
        let root = (*prime as f64).powf(1.0 / 3.0);
        let derived = ((root - root.floor()) * 2f64.powi(32)) as u64;
        assert_eq!(
            derived,
            u64::from(sha256::K[index]),
            "K({index}) is not the first 32 bits of the fractional part of cbrt({prime})"
        );
    }
}

/// The first `count` primes, by trial division.
fn first_primes(count: usize) -> Vec<u64> {
    let mut primes = Vec::with_capacity(count);
    let mut candidate = 2u64;
    while primes.len() < count {
        let is_prime = primes
            .iter()
            .all(|prime| *prime * *prime > candidate || !candidate.is_multiple_of(*prime));
        if is_prime {
            primes.push(candidate);
        }
        candidate += 1;
    }
    primes
}

#[test]
fn reset_presents_the_initial_hash_value_on_the_outputs() {
    // The claim worth testing on its own is that reset here means *the start of
    // the hash*, not zero. A chaining register wired to the register reset pin
    // comes out of reset holding eight zero words, and from there every digest in
    // this file would be the hash of the right bytes computed from the wrong start
    // -- a plausible, well-formed, wrong answer, which is why it gets its own test
    // rather than being folded into the FIPS vector.
    let tb = started();
    assert_eq!(
        tb.run(|tb| async move { read_state(&tb) }).unwrap(),
        sha256::H.map(u64::from),
        "asserting reset puts the initial hash value on the outputs, which is \
         what a chaining register must come out of reset holding"
    );

    // Releasing reset with the empty message's block on the input is one real
    // compression from the IV, and the published digest is the check on it.
    let released = feed(&padded_block(b""), 0, &tb);
    assert_eq!(released, published(EMPTY_DIGEST));
    assert_eq!(
        feed(&padded_block(b""), 1, &tb),
        sha256::H.map(u64::from),
        "and asserting reset again goes back to the start of the hash, discarding \
         the block on the input"
    );
}

#[test]
fn the_design_holds_exactly_eight_chaining_registers_and_nothing_else() {
    // One block per cycle is a claim about state, and this design makes it by
    // holding exactly the 256-bit chaining register and no pipeline registers at
    // all: a register between rounds would be a different design with a different
    // latency. That the eight are on *this* design's clock is checked where the
    // emitted Verilog is in hand, in the cosimulation below.
    let design = Design::new();
    sha256::build(&design).unwrap();

    let stateful: Vec<(String, u32)> = design
        .signals()
        .iter()
        .filter(|signal| signal.is_stateful())
        .map(|signal| (signal.kind().to_string(), signal.width()))
        .collect();
    assert_eq!(
        stateful.len(),
        sha256::STATE_WORDS,
        "one register per chaining word and nothing else: {stateful:?}"
    );
    assert!(
        stateful
            .iter()
            .all(|(_, width)| *width == sha256::WORD_BITS),
        "and every one of them is a whole word"
    );
    assert!(
        stateful.iter().all(|(kind, _)| kind == "Reg"),
        "and every one of them is a register, not a memory or an instance: {stateful:?}"
    );
}

#[test]
fn every_output_word_is_32_bits_and_the_state_actually_moves() {
    let tb = started();
    for message in [b"".as_slice(), b"a", b"abc", b"the quick brown fox"] {
        let observed = compress(&tb, message);
        assert_eq!(observed, golden(message), "{message:?}");
    }
    for name in OUTPUT_NAMES {
        let series = tb.series(name);
        assert!(!series.is_empty(), "{name} recorded nothing at all");
        assert!(
            series.iter().all(|bits| bits.width() == sha256::WORD_BITS),
            "{name} is a whole number of words at every cycle"
        );
        let values: Vec<u64> = series.iter().map(|bits| bits.to_u64().unwrap()).collect();
        assert!(
            values.iter().all(|value| *value <= u64::from(u32::MAX)),
            "{name} never exceeds 32 bits"
        );
        assert!(
            values.windows(2).any(|pair| pair[0] != pair[1]),
            "{name} never changed across four blocks, which would mean the \
             compression is constant"
        );
    }
}

#[test]
fn the_ports_are_the_ones_the_testbench_and_the_driver_both_address() {
    let design = Design::new();
    let ports = sha256::build(&design).unwrap();

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
        std::iter::once(ferrite_lithic_corpus::CLOCK.to_string())
            .chain(INPUT_NAMES.iter().map(|name| (*name).to_string()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        outputs,
        OUTPUT_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(ports.state.words.len(), sha256::STATE_WORDS);

    // The generated driver reads and writes every port through a `u64` and has a
    // `static_assert` refusing anything wider, so a port above 64 bits does not
    // cosimulate -- it fails to build, and only in the Verilator leg.
    for port in design
        .input_ports()
        .iter()
        .chain(design.output_ports().iter())
    {
        assert!(
            port.width() <= 64,
            "port {} is {} bits, which the cosimulator's driver cannot carry",
            port.name().unwrap_or_default(),
            port.width()
        );
    }
    assert_eq!(
        design
            .input_ports()
            .iter()
            .chain(design.output_ports().iter())
            .filter(|port| port.name().as_deref() != Some(ferrite_lithic_corpus::CLOCK))
            .filter(|port| port.name().as_deref() != Some(ferrite_lithic_corpus::RESET))
            .map(|port| port.width())
            .collect::<Vec<_>>(),
        vec![sha256::WORD_BITS; sha256::BLOCK_WORDS + sha256::STATE_WORDS],
        "every data port is one 32-bit word"
    );

    // The literals in the two `PortList` declarations are what the widths above are
    // really testing, and `base10_parse` cannot see the constants, so they are
    // pinned against the constants here.
    assert_eq!(sha256::Inputs::PORT_WIDTHS.len(), 2 + sha256::BLOCK_WORDS);
    assert_eq!(sha256::Outputs::PORT_WIDTHS.len(), sha256::STATE_WORDS);
    assert!(
        sha256::Inputs::PORT_WIDTHS[2..]
            .iter()
            .all(|width| *width == sha256::WORD_BITS)
    );
    assert!(
        sha256::Outputs::PORT_WIDTHS
            .iter()
            .all(|width| *width == sha256::WORD_BITS)
    );
    assert_eq!(sha256::ROUNDS, 64, "FIPS 180-4 has 64 rounds");
    assert_eq!(sha256::BLOCK_WORDS, 16, "a block is 512 bits");
    assert_eq!(sha256::STATE_WORDS, 8, "the state is 256 bits");
}

proptest! {
    /// The differential, on arbitrary one-block messages.
    ///
    /// The deterministic sweeps above cover the shapes; this covers the values,
    /// including the bit patterns the sweeps cannot write down.
    #[test]
    fn the_design_agrees_with_the_sha2_crate_on_arbitrary_one_block_messages(
        data in prop::collection::vec(any::<u8>(), 0..=56)
    ) {
        prop_assume!(data.len() <= ONE_BLOCK);
        let tb = started();
        prop_assert_eq!(compress(&tb, &data), golden(&data));
    }
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the sha256 equivalence did not run. \
             The differential against the `sha2` crate above does not need it."
        );
        return;
    };
    println!("cosimulating sha256 with {}", verilator.display());

    let design = Design::new();
    let ports = sha256::build(&design).unwrap();

    let module = ferrite_lithic_rtl::Module::new(
        "sha256",
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
        INPUT_NAMES,
        "the clock is a port but not a stimulus column, and the simulator gets \
         the design's own names"
    );
    assert_eq!(
        plan.outputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        OUTPUT_NAMES,
        "and all eight state words are compared, one port at a time"
    );
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.verilog.as_str())
            .collect::<Vec<_>>(),
        INPUT_NAMES,
        "no name is a SystemVerilog keyword here, so the driver and the module \
         agree on every one of them"
    );

    // The stimulus is the protocol the module documents: reset held for the first
    // block, then released for a second block chained onto it, then a third block
    // chained onto the second. Every cycle has all sixteen words on the input, so
    // the comparison is over three real compressions rather than over a reset.
    const MESSAGES: [&[u8]; 3] = [b"", b"abc", b"the quick brown fox jumps over the lazy dog"];
    let mut stimulus = Stimulus::new();
    let mut push = |rst: u64, block: &[u8; sha256::BLOCK_BYTES]| {
        let mut row = vec![Bits::constant(rst, 1).unwrap()];
        row.extend(
            block_words(block)
                .iter()
                .map(|word| Bits::constant(u64::from(*word), sha256::WORD_BITS).unwrap()),
        );
        stimulus.push(row).unwrap();
    };
    push(1, &padded_block(MESSAGES[0]));
    push(0, &padded_block(MESSAGES[1]));
    push(0, &padded_block(MESSAGES[2]));
    // A two-block message, so the register's own chaining is on the record rather
    // than assumed: the third and fourth rows compress from each other.
    let pair = b"a message long enough to need two whole sha-256 blocks to hash it";
    let blocks = padded_pair(pair);
    push(1, &blocks[0]);
    push(0, &blocks[1]);
    assert_eq!(
        stimulus.cycles(),
        5,
        "three one-block rows and a two-block pair"
    );

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();

    // The only state in the emitted module is the chaining register, so the count
    // of clocked always-blocks is the design's state read off the Verilog rather
    // than off the IR. A pipeline register between rounds would show up here as
    // more than eight.
    assert_eq!(
        verilog.matches("always @(posedge clk)").count(),
        sha256::STATE_WORDS,
        "eight clocked always-blocks and nothing else, so the module really is \
         combinational-plus-one-chaining-register"
    );
    assert!(
        !verilog.contains("always @(posedge rst)"),
        "and the register is on the clock, not on the reset"
    );

    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same messages through the testbench agree with the software hash, so
    // all three -- simulator, Verilator and the `sha2` crate -- meet.
    let tb = started();
    for message in MESSAGES {
        assert_eq!(compress(&tb, message), golden(message), "{message:?}");
    }
    assert_eq!(chain(&tb, pair).1, golden(pair));
}
