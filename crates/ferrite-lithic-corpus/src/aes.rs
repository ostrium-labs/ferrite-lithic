//! AES-128: one 128-bit block in, one 128-bit block out, one clock.
//!
//! # Why this is the best hardware target in the corpus
//!
//! `crc3` is three flops and a parity tap; this is the opposite end of the range.
//! AES-128 is ten rounds of the same four steps -- SubBytes, ShiftRows, MixColumns,
//! AddRoundKey -- and every one of those steps is *either* an XOR of bits that are
//! already in the design *or* a lookup in a fixed 256-entry table. There is no
//! multiplier, no divider, no comparator chain and no variable-width anything. So
//! where `crc3` stresses the toolchain's streaming side (a register, a reset, a
//! per-cycle dependency chain), this stresses the side a small design cannot reach:
//! **a 256-arm table lookup in the middle of a combinational cone, two hundred
//! times over**, and a graph with a couple of thousand nodes in it.
//!
//! Both are things a hand-written simulator handles trivially and a real toolchain
//! gets wrong in interesting ways: a `case` whose arms are not scheduled, a constant
//! that is not emitted because nothing named it, a 128-bit value that a 64-bit I/O
//! path quietly truncates. Those are the bugs this design is here to find.
//!
//! # The S-box is a ROM, and it is built as a `case`
//!
//! [`SBOX`] is 256 entries of eight bits. In silicon that is not a function, it is
//! a table: every standard-cell library and every FPGA fabric has a lookup primitive
//! for it, and a synthesiser asked to compute the S-box algebraically will produce
//! something *much* larger than the table. So the honest shape is a 256-entry,
//! 8-bit-wide read-only memory addressed by the byte.
//!
//! [`Design::mem`] cannot express it, and the reason is worth recording rather than
//! working around quietly. A memory in this IR starts zero-filled -- both backends
//! agree on that, the simulator zero-fills and Verilator is pinned to
//! `--x-initial 0` -- and there is no initial-value mechanism to put the table in:
//! `Sim::initialize_to` refuses a memory outright ("memories start zero-filled,
//! which is the only initial state they have"), and a memory has no reset, so it
//! cannot be filled by a reset cycle either. A front end that wanted AES would need
//! a ROM node, or a memory with initial contents, and today it has neither.
//!
//! What it does have is [`Design::case_`], and a 256-arm `case` over a literal
//! scrutinee *is* a combinational ROM: the emitted Verilog is one `always @*` with a
//! `case` and 256 items, which is exactly how a LUT gets written down. So that is
//! what [`SBox::lookup`] builds, and it is a faithful shape rather than a
//! substitute. What it costs against a real ROM is duplication -- 200 lookups means
//! 200 copies of the table in the netlist, where a shared block-RAM S-box would be
//! read by all of them -- and that cost is the honest price of the front end's
//! missing ROM, not of the algorithm.
//!
//! ## Where the table itself comes from
//!
//! [`SBOX`] is not transcribed from a table of numbers on faith. It is the AES
//! S-box as FIPS 197 §5.1.3 defines it: for each byte `x`, take the **multiplicative
//! inverse of `x` in GF(2^8)** modulo the AES field polynomial `x^8 + x^4 + x^3 +
//! x + 1` (`0x11b`), with `0` defined to map to `0`, and then apply an **affine
//! transform over GF(2)**
//!
//! ```text
//! b'_i = b_i ^ b_(i+4) ^ b_(i+5) ^ b_(i+6) ^ b_(i+7) ^ c_i     (indices mod 8)
//! ```
//!
//! with `b` the inverse's bits and `c = 0x63` (FIPS 197 §5.1.3, eq. 7). The test
//! `the_sbox_is_the_multiplicative_inverse_and_the_affine_transform` in
//! `tests/aes.rs` computes all 256 entries from that definition -- a multiply and a
//! search of the field's 255 nonzero elements per entry, no table involved -- and
//! asserts they equal [`SBOX`] entry for entry. So the table's *provenance* is
//! checked, not its spelling, and a single mistyped hex digit in the constant below
//! fails a test rather than producing a plausible design that computes the wrong
//! cipher.
//!
//! # Unrolled, not serial
//!
//! Ten rounds are unrolled into the graph: one block is encrypted per cycle, with
//! latency one (the output register). The alternative is the textbook serial core --
//! one round of logic, the state fed back through a register, ten cycles of latency
//! for the same ten rounds of work.
//!
//! The serial core is the smaller one, by about ten times on the datapath, and it is
//! what an ASIC that encrypts one block every ten cycles should build. This design
//! is the other trade: **a block per cycle, bought with ten blocks' worth of logic**.
//! That is the right trade for a streaming encryptor (a disk or a network engine
//! encrypting continuously has a new block every cycle and no bubble to put a bubble
//! in), and it is the right trade *here* for a third reason: the ten-round unrolled
//! form is the one whose combinational depth has to survive an emitter, a Verilog
//! backend and a synthesiser, and a serial form would hide all of it behind a
//! register.
//!
//! The gate cost is stated rather than hidden, and the emitted netlist is where the
//! numbers come from rather than an estimate: 1,890 named nodes, of which 200 are
//! S-box ROM reads (160 SubBytes, 40 key-schedule SubWord), 144 are `xtime`'s mux on
//! bit 7, 1,066 are eight-bit XORs, 320 are byte selects and 4 are the 32-bit
//! ciphertext registers. So the S-box is most of the *node* count and none of the
//! logic: with a shared block RAM in front of it instead, this is a design of 144
//! XOR gates and 320 byte selects.
//!
//! # The key schedule is combinational too
//!
//! The ten round keys are expanded from the cipher key in the same cycle as the
//! block, by [`expand_key`]: `RotWord`, `SubWord` -- which is *the same* S-box ROM,
//! four more lookups -- and an XOR with `Rcon[i/4]` (FIPS 197 §5.2). Doing it this
//! way costs eleven round keys' worth of XOR logic and four extra ROM reads, and it
//! is what "a new key can arrive on any cycle" costs. A core that changes key
//! rarely would hold the round keys in registers and spend ten cycles expanding
//! them, trading the key schedule's area for a cycle of latency on the key change.
//!
//! `Rcon` is [`RCON`], FIPS 197 §5.2: `0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40,
//! 0x80, 0x1b, 0x36`. All ten are `xtime` applied repeatedly to `0x01`, with no
//! special case anywhere -- including `0x80 -> 0x1b`, which is the `0x11b`
//! reduction rather than a correction -- and a test asserts exactly that, because
//! "the ninth constant is special" is the sort of thing a transcription gets wrong
//! by writing `0x1c`.
//!
//! # MixColumns and ShiftRows
//!
//! **ShiftRows** is a byte permutation and nothing else: row `r` of the state is
//! rotated left by `r` bytes, so `state'[r][c] = state[r][(c + r) mod 4]` (FIPS 197
//! §5.2). On this graph it is *free* -- sixteen wires permuted -- because the state
//! is carried as sixteen byte-wide signals rather than as one 128-bit value and a
//! slice out of it. Which is the reason the state is carried that way and not as a
//! single `Signal`: ShiftRows is then an index permutation in a Rust array instead
//! of sixteen 8-bit selects, and the select nodes it would have created are nodes
//! the emitter has to name.
//!
//! **MixColumns** is multiplication in GF(2^8) modulo `0x11b` by the fixed
//! coefficients `{02}`, `{03}` written as a matrix (FIPS 197 §5.2):
//!
//! ```text
//! | 02 03 01 01 |   | s0 |        | s0 |   2*s0
//! | 01 02 03 01 | * | s1 |   =    | s1 | = 3*s1 ^ s0      (as a sum of columns)
//! | 01 01 02 03 |   | s2 |        | s2 |     ...
//! | 03 01 01 02 |   | s3 |        | s3 |
//! ```
//!
//! `xtime` ([`xtime`]) is multiplication by `{02}`: a one-bit left shift, then XOR
//! with `0x1b` **only if bit 7 was set**. That conditional is the whole of the
//! reduction, and it is a mux on bit 7 rather than a guess: `x << 1` overflows out of
//! eight bits precisely when `x & 0x80` was set, and `0x11b ^ 0x100 = 0x1b`. `{03}`
//! is `xtime(x) ^ x`, so `MixColumns` needs no multiplier at all -- eight `xtime`s
//! and sixteen XORs per column, which is why it is cheap here and would be a waste
//! in software.
//!
//! # The ports are four words, not one
//!
//! A 128-bit input would be the obvious port list and it cannot be built here: the
//! cosimulator's generated driver `static_assert`s that every port is 1 to 64 bits
//! wide, and the testbench records a `TooWide` error rather than truncating. So the
//! block and the key arrive as `in0..in3` and `key0..key3`, four 32-bit words each,
//! and the ciphertext leaves as `out0..out3`. A word carries its first byte in its
//! most significant position, so `in0` bits 31:24 are plaintext byte 0 and bits 7:0
//! are byte 3; the same for the key and for the output. That is the only
//! awkwardness the width limit costs, and it is the port list of a real 32-bit
//! datapath anyway.
//!
//! # Reading the result
//!
//! The ciphertext appears in the output ports one edge after the block and key are
//! presented, and a test reads it *after* that edge, per ADR-0018: the same boundary
//! the testbench and the cosimulator both use. Reading the ports *before* the edge
//! gives the previous ciphertext, and a test asserts that too, because "the output
//! is combinational" and "the output is one edge behind" are different designs and
//! only one of them is this one.
//!
//! One consequence worth stating because it surprised this design's own first
//! version: each input word takes an edge to drive, and the register takes whatever
//! the ports say *at* that edge. So presenting a 128-bit block over four 32-bit ports
//! is four edges, and only the last of them sees a complete block -- the three before
//! it encrypt a partly-updated input. That is why the differential harness here
//! drives the key, then the block, and reads after the eighth edge.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The AES S-box, FIPS 197 §5.1.3 and Figure 7.
///
/// 256 entries, indexed by the byte being substituted. Every entry is the
/// multiplicative inverse in GF(2^8) modulo `0x11b` followed by the affine
/// transform of §5.1.3 eq. 7; see the module docs and the test that recomputes all
/// 256 entries from that definition.
pub const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

/// The AES round constants, FIPS 197 §5.2.
///
/// `RCON[i]` is the constant XORed into byte 0 of word `4 * (i + 1)` of the key
/// schedule, so there are ten of them for ten rounds. Every one is the previous one
/// multiplied by `{02}` in GF(2^8), `0x80 -> 0x1b` included, which is why the
/// reduction constant is the same `0x1b` that [`xtime`] uses.
pub const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

/// The AES field polynomial `x^8 + x^4 + x^3 + x + 1`, truncated to its low eight
/// bits: what an overflowing shift is XORed with.
pub const REDUCTION: u8 = 0x1b;

/// The number of rounds AES-128 applies, FIPS 197 §5.1.
pub const ROUNDS: usize = 10;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge latches one ciphertext.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which clears the ciphertext register.
    pub rst: Signal,
    /// Plaintext bytes 0 to 3, most significant byte first.
    #[bits(32)]
    pub in0: Signal,
    /// Plaintext bytes 4 to 7.
    #[bits(32)]
    pub in1: Signal,
    /// Plaintext bytes 8 to 11.
    #[bits(32)]
    pub in2: Signal,
    /// Plaintext bytes 12 to 15.
    #[bits(32)]
    pub in3: Signal,
    /// Cipher key bytes 0 to 3.
    #[bits(32)]
    pub key0: Signal,
    /// Cipher key bytes 4 to 7.
    #[bits(32)]
    pub key1: Signal,
    /// Cipher key bytes 8 to 11.
    #[bits(32)]
    pub key2: Signal,
    /// Cipher key bytes 12 to 15.
    #[bits(32)]
    pub key3: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// Ciphertext bytes 0 to 3, as they stood after the last edge.
    #[bits(32)]
    pub out0: Signal,
    /// Ciphertext bytes 4 to 7.
    #[bits(32)]
    pub out1: Signal,
    /// Ciphertext bytes 8 to 11.
    #[bits(32)]
    pub out2: Signal,
    /// Ciphertext bytes 12 to 15.
    #[bits(32)]
    pub out3: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The output ports, each driven by its own ciphertext register.
    pub outputs: Outputs,
}

/// The AES S-box as a 256-entry, eight-bit-wide ROM in the graph.
///
/// One of these is built per [`build`] call and shared by every lookup in the
/// design, so the 256 table entries are 256 nodes however many times the design
/// reads them. The *lookups* are not shared -- each [`lookup`](Self::lookup) is its
/// own 256-arm `case`, because a `case` node has one scrutinee and the design has
/// 200 different bytes to substitute.
#[derive(Clone, Debug)]
pub struct SBox {
    entries: Vec<Signal>,
}

impl SBox {
    /// Builds the table, one eight-bit constant per [`SBOX`] entry.
    ///
    /// # Errors
    ///
    /// Whatever [`Design::lit`] returns. There is no S-box-specific failure: the
    /// table is 256 constants of a width the DSL has.
    pub fn build(design: &Design) -> Result<Self, BuildError> {
        let mut entries = Vec::with_capacity(SBOX.len());
        for value in SBOX {
            entries.push(design.lit(u64::from(value), 8)?);
        }
        Ok(Self { entries })
    }

    /// The substitution of one byte: a 256-arm ROM read.
    ///
    /// `byte` must be eight bits wide. Every one of the 256 possible values has an
    /// arm, so the default arm cannot be taken by an eight-bit input; it is
    /// `SBOX[0]`, the substitution of zero, because that is the value FIPS 197
    /// gives `0` and it is a better answer than a zero the table never contains.
    ///
    /// # Errors
    ///
    /// Whatever [`Design::case_`] returns -- a scrutinee or arm width mismatch.
    pub fn lookup(&self, design: &Design, byte: &Signal) -> Result<Signal, BuildError> {
        let arms = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (index as u64, entry.clone()))
            .collect::<Vec<_>>();
        let zero = self.entries[0].clone();
        Ok(design.case_(byte, &arms, &zero)?)
    }
}

/// Builds an AES-128 block encryptor into `design`.
///
/// Ten rounds are unrolled into the graph, so a block presented on the input ports
/// at one edge is in the output ports at the next. The clock is named `clk` and the
/// reset `rst`, as the crate-level [`crate::CLOCK`] and [`crate::RESET`] say.
///
/// # Errors
///
/// Whatever [`Design`] returns and whatever the port-list helpers return. There is
/// no AES-specific failure mode: every width in the cipher is fixed by FIPS 197, so
/// there is nothing to configure and nothing that can be configured wrongly.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;
    let sbox = SBox::build(design)?;

    // The key schedule reads the *key* ports and the datapath reads the *plaintext*
    // ports, and they are two different lists of four words. Sharing one variable
    // between them is the mistake this comment exists to prevent: the result is a
    // design that encrypts the block under itself, is perfectly deterministic, and
    // fails every vector.
    let keys = expand_key(design, &sbox, &load(design, &key_words(&inputs))?)?;

    // The initial AddRoundKey with the key itself is outside the round loop and easy
    // to leave out: FIPS 197 §5.1 draws it as a separate step before round 1, and a
    // design that starts straight at SubBytes produces a plausible, entirely
    // self-consistent cipher that nothing but an outside vector will catch.
    let mut state = add_round_key(design, &load(design, &input_words(&inputs))?, &keys[0])?;
    // Ten unrolled rounds, one per round key after the first: `index` is FIPS 197's
    // round number, and `round_key` is `w[4 * index .. 4 * index + 4]`.
    for (index, round_key) in keys.iter().enumerate().skip(1) {
        state = sub_bytes(design, &sbox, &state)?;
        // ShiftRows is a permutation of sixteen byte signals, so it costs nothing.
        state = shift_rows(&state);
        // The last round of AES-128 has no MixColumns (FIPS 197 §5.1).
        if index < ROUNDS {
            state = mix_columns(design, &state)?;
        }
        state = add_round_key(design, &state, round_key)?;
    }

    // One register per output word. `clear` is tied low, so the register updates on
    // every edge and the design presents one ciphertext per cycle; `rst` is a real
    // input rather than a constant, because a reset that cannot be exercised is a
    // reset that is not tested.
    let clear = design.constant(false);
    let mut held = Vec::with_capacity(4);
    for word in store(design, &state)? {
        held.push(design.reg(&word, &inputs.clk, &inputs.rst, &clear)?);
    }
    let outputs = ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            out0: held[0].clone(),
            out1: held[1].clone(),
            out2: held[2].clone(),
            out3: held[3].clone(),
        },
    )?;

    Ok(Ports { inputs, outputs })
}

/// The four 32-bit plaintext words, in port order.
fn input_words(inputs: &Inputs) -> [Signal; 4] {
    [
        inputs.in0.clone(),
        inputs.in1.clone(),
        inputs.in2.clone(),
        inputs.in3.clone(),
    ]
}

/// The four 32-bit key words, in port order.
fn key_words(inputs: &Inputs) -> [Signal; 4] {
    [
        inputs.key0.clone(),
        inputs.key1.clone(),
        inputs.key2.clone(),
        inputs.key3.clone(),
    ]
}

/// The state as sixteen bytes: index `4 * column + row`, FIPS 197 §5.2.
///
/// Column-major, because that is the order the four input words arrive in and the
/// order `MixColumns` wants a column in. Carrying the state as sixteen bytes rather
/// than one 128-bit value is what makes [`shift_rows`] free.
type State = [Signal; 16];

/// Splits four 32-bit words into the sixteen state bytes, byte 0 first.
///
/// # Errors
///
/// Whatever [`Design::slice`] returns, which is nothing for four 32-bit words and
/// a byte index of 0, 8, 16 or 24.
fn load(design: &Design, words: &[Signal; 4]) -> Result<State, BuildError> {
    let mut state = Vec::with_capacity(16);
    for column in 0..4u32 {
        for row in 0..4u32 {
            // Row `r` of column `c` is byte `r` of word `c`, and a word carries its
            // first byte in its most significant position -- so byte `r` is bits
            // `8 * (3 - r)`, not `8 * r`. Taking `8 * r` instead reverses all four
            // bytes of every word, which is a design that encrypts something and
            // agrees with no published vector.
            state.push(design.slice(&words[column as usize], 8 * (3 - row), 8)?);
        }
    }
    Ok(state
        .try_into()
        .unwrap_or_else(|_| unreachable!("sixteen pushes")))
}

/// The four 32-bit output words, byte 0 in the most significant position.
///
/// # Errors
///
/// Whatever [`Design::concat`] returns, which is nothing for four eight-bit bytes.
fn store(design: &Design, state: &State) -> Result<[Signal; 4], BuildError> {
    let mut words = Vec::with_capacity(4);
    for column in 0..4usize {
        words.push(design.concat(&[
            state[4 * column].clone(),
            state[4 * column + 1].clone(),
            state[4 * column + 2].clone(),
            state[4 * column + 3].clone(),
        ])?);
    }
    Ok(words
        .try_into()
        .unwrap_or_else(|_| unreachable!("four pushes")))
}

/// Multiplication by `{02}` in GF(2^8) modulo `x^8 + x^4 + x^3 + x + 1`.
///
/// A one-bit left shift, then XOR with [`REDUCTION`] if and only if bit 7 was set:
/// `x << 1` leaves the eight-bit field exactly when `x & 0x80` was set, and the
/// polynomial's low byte is `0x1b`. The conditional is a mux on bit 7, not a
/// guess -- an unconditional XOR would be wrong for 255 of the 256 inputs.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn xtime(design: &Design, byte: &Signal) -> Result<Signal, BuildError> {
    let carried = design.slice(byte, 7, 1)?;
    let shifted = design.sll(byte, 1)?;
    let reduced = design.xor(&shifted, &design.lit(u64::from(REDUCTION), 8)?)?;
    Ok(design.ite(&carried, &reduced, &shifted)?)
}

/// SubBytes: every byte through the S-box ROM (FIPS 197 §5.1.3).
///
/// # Errors
///
/// Whatever [`SBox::lookup`] returns.
pub fn sub_bytes(design: &Design, sbox: &SBox, state: &State) -> Result<State, BuildError> {
    let mut out = Vec::with_capacity(state.len());
    for byte in state {
        out.push(sbox.lookup(design, byte)?);
    }
    Ok(out
        .try_into()
        .unwrap_or_else(|_| unreachable!("sixteen pushes")))
}

/// ShiftRows: row `r` rotated left by `r` bytes (FIPS 197 §5.2).
///
/// A permutation of the sixteen byte signals, so it builds no nodes at all:
/// `state'[4 * c + r] = state[4 * ((c + r) % 4) + r]` wires the output to a
/// different input. This is why the state is carried as bytes; a single 128-bit
/// signal would make it sixteen selects.
#[must_use]
pub fn shift_rows(state: &State) -> State {
    let mut out = Vec::with_capacity(state.len());
    for column in 0..4usize {
        for row in 0..4usize {
            out.push(state[4 * ((column + row) % 4) + row].clone());
        }
    }
    out.try_into()
        .unwrap_or_else(|_| unreachable!("sixteen pushes"))
}

/// MixColumns: a GF(2^8) matrix multiply by `{02}` and `{03}` (FIPS 197 §5.2).
///
/// Column `c` is `2*s0 ^ 3*s1 ^ s2 ^ s3` and so on down the column. `{03}` is
/// `xtime(s) ^ s`, so the whole matrix needs no multiplier at all: per column, four
/// `xtime`s, four XORs for the `{03}` bytes and twelve for the four sums.
///
/// # Errors
///
/// Whatever [`xtime`] returns.
pub fn mix_columns(design: &Design, state: &State) -> Result<State, BuildError> {
    let mut out = Vec::with_capacity(state.len());
    for column in 0..4usize {
        let s = [
            state[4 * column].clone(),
            state[4 * column + 1].clone(),
            state[4 * column + 2].clone(),
            state[4 * column + 3].clone(),
        ];
        let doubled = [
            xtime(design, &s[0])?,
            xtime(design, &s[1])?,
            xtime(design, &s[2])?,
            xtime(design, &s[3])?,
        ];
        // `{03}` bytes, so each output byte below is a four-input XOR rather than a
        // five-input one.
        let mut tripled = doubled.clone();
        for row in 0..4usize {
            tripled[row] = design.xor(&doubled[row], &s[row])?;
        }
        for row in 0..4usize {
            out.push(xor_all(
                design,
                &[
                    doubled[row].clone(),
                    tripled[(row + 1) % 4].clone(),
                    s[(row + 2) % 4].clone(),
                    s[(row + 3) % 4].clone(),
                ],
            )?);
        }
    }
    Ok(out
        .try_into()
        .unwrap_or_else(|_| unreachable!("sixteen pushes")))
}

/// AddRoundKey: a sixteen-byte XOR (FIPS 197 §5.1).
///
/// # Errors
///
/// Whatever [`Design::xor`] returns.
pub fn add_round_key(design: &Design, state: &State, key: &State) -> Result<State, BuildError> {
    let mut out = Vec::with_capacity(state.len());
    for (byte, round_byte) in state.iter().zip(key.iter()) {
        out.push(design.xor(byte, round_byte)?);
    }
    Ok(out
        .try_into()
        .unwrap_or_else(|_| unreachable!("sixteen pushes")))
}

/// The key schedule: `ROUNDS + 1` round keys of sixteen bytes (FIPS 197 §5.2).
///
/// `w[i] = w[i - 4] ^ w[i - 1]`, except every fourth word, where `w[i - 1]` is
/// first `RotWord`ed (a one-byte left rotation of the word), passed through the same
/// S-box ROM as the data (`SubWord`), and XORed with `RCON[i / 4]` into byte 0.
/// Round key `r` is words `4 * r .. 4 * r + 4`, so the result is
/// `round_keys[0] = key` and `round_keys[1..=ROUNDS]` are the ten XORed into the
/// rounds.
///
/// # Errors
///
/// Whatever [`SBox::lookup`] and [`Design::xor`] return.
pub fn expand_key(design: &Design, sbox: &SBox, key: &State) -> Result<Vec<State>, BuildError> {
    let mut words: Vec<[Signal; 4]> = key
        .chunks(4)
        .map(|chunk| {
            [
                chunk[0].clone(),
                chunk[1].clone(),
                chunk[2].clone(),
                chunk[3].clone(),
            ]
        })
        .collect();

    // `w` is 4 * (ROUNDS + 1) = 44 words, of which the first four are the key, so
    // FIPS 197's loop runs from w[4] to w[43] inclusive.
    for index in 4..=(ROUNDS * 4 + 3) {
        let mut temp = words[index - 1].clone();
        if index % 4 == 0 {
            // RotWord: [a, b, c, d] -> [b, c, d, a].
            let rotated = [
                temp[1].clone(),
                temp[2].clone(),
                temp[3].clone(),
                temp[0].clone(),
            ];
            let mut substituted = [
                temp[0].clone(),
                temp[0].clone(),
                temp[0].clone(),
                temp[0].clone(),
            ];
            for (byte, source) in substituted.iter_mut().zip(rotated.iter()) {
                *byte = sbox.lookup(design, source)?;
            }
            // Rcon goes into byte 0 only; as an eight-bit XOR into the substituted
            // byte, that is the whole of it.
            let rcon = design.lit(u64::from(RCON[index / 4 - 1]), 8)?;
            substituted[0] = design.xor(&substituted[0], &rcon)?;
            temp = substituted;
        }
        let mut word = temp.clone();
        for (byte, earlier) in word.iter_mut().zip(words[index - 4].iter()) {
            *byte = design.xor(earlier, byte)?;
        }
        words.push(word);
    }

    Ok(words
        .chunks(4)
        .map(|chunk| {
            let mut round_key = Vec::with_capacity(16);
            for word in chunk {
                round_key.extend_from_slice(word);
            }
            round_key
                .try_into()
                .unwrap_or_else(|_| unreachable!("four words of four bytes"))
        })
        .collect())
}

/// An eight-bit XOR of four terms, folded.
///
/// There is no n-ary XOR in the DSL, and [`Design::xor`] truncates to its left
/// operand's width, so the fold has to start from a term of the final width. All
/// four are eight bits here.
///
/// # Errors
///
/// Whatever [`Design::xor`] returns.
fn xor_all(design: &Design, terms: &[Signal]) -> Result<Signal, BuildError> {
    let mut terms = terms.iter();
    let mut accumulator = terms
        .next()
        .expect("xor_all is only ever called with four terms")
        .clone();
    for term in terms {
        accumulator = design.xor(&accumulator, term)?;
    }
    Ok(accumulator)
}
