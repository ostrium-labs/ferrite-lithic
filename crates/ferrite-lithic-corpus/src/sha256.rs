//! SHA-256: one 512-bit block in, one 256-bit chaining state out, every cycle.
//!
//! # Why this is the shape hardware wants
//!
//! SHA-256 is one of the best hardware targets in existence, and the reason is
//! almost entirely in the dependency chain. Every one of the 64 rounds reads the
//! eight working words the round before it produced, so there is nothing to
//! overlap: no lanes to fill, no message parallelism inside a block, and no SIMD
//! story at all. A CPU compressing one block does 64 serial rounds of four adds,
//! three rotates, a `Ch` and a `Maj`, and no amount of vector width changes that.
//! Hardware gets exactly the same answer for exactly the same 64 rounds, for the
//! price of 64 copies of a few hundred gates.
//!
//! So the datapath here is the **unrolled, one block per cycle** form. All 64
//! rounds are combinational logic fed from the 16 message words and the current
//! chaining state, and the only register in the module is the 256-bit chaining
//! state itself. There is no ROM, no message schedule table and no lookup of any
//! kind: the round constant `K[t]` is a literal and the message word `w[t]` is
//! arithmetic on the input. A table-driven SHA-256 is a CPU design; a
//! rounds-of-gates SHA-256 is an ASIC design, and this is the latter.
//!
//! # The chaining register is the point of `rst`
//!
//! SHA-256 is iterated: `H(1) = compress(H(0), block)`, `H(2) =
//! compress(H(1), block)`, and so on, starting from a fixed initial value. That
//! register is inside this module, and its *initial* value is a specification
//! constant rather than a reset convention — so a register that resets to zero is
//! actively wrong here, and the difference is invisible: the circuit would
//! compress the first block from all zeros and produce a well-formed 256-bit
//! number that is not the hash of anything.
//!
//! So `rst` does not drive the register's reset pin at all. A register reset can
//! only load zero, and this register must come out of reset holding H(0)..H(7).
//! Instead `rst` selects on **both** sides of the register:
//!
//! - the register's data input is the initial hash value while `rst` is asserted,
//!   so the outputs read H on every cycle of reset, and
//! - the datapath's chaining input is the initial hash value too, so a block
//!   presented on the cycle `rst` is released is the *first* block of a hash.
//!
//! In use that is the ordinary `init` handshake of a hash core: assert `rst`,
//! present the first block, and release `rst` on the edge that consumes it. Every
//! later block chains from the register with `rst` low. [`compress`] itself is the
//! bare compression function and knows nothing about any of this, which is what
//! lets a caller put the register somewhere else.
//!
//! # Where the constants come from
//!
//! Both constant lists are irrational-number digits, and FIPS 180-4 gives the
//! recipe rather than just the table. They come from **different roots**, which is
//! the one detail worth being explicit about, because it is very easy to write
//! "cube roots" in both places and be half right:
//!
//! - [`H`] is FIPS 180-4 §5.3.3: the first 32 bits of the *fractional parts of the
//!   square roots of the first eight primes* (2, 3, 5, 7, 11, 13, 17, 19).
//! - [`K`] is FIPS 180-4 §4.2.2: the first 32 bits of the *fractional parts of the
//!   cube roots of the first sixty-four primes* (2 through 311).
//!
//! Neither list is guessable from a hash, which is what makes them worth carrying
//! as literals: `2^32 * frac(sqrt(5))` is a constant an ASIC cannot generate, so
//! it is a constant an ASIC must be told. The tests recompute both lists from the
//! recipe and assert they agree, so a transcription slip in either direction is
//! caught here rather than showing up as an unattributable disagreement with the
//! `sha2` crate.
//!
//! # What is *not* here
//!
//! - **No padding.** SHA-256's Merkle-Damgard padding (append `0x80`, zeros, and
//!   the message length in bits) is a byte-shuffling step that a hardware
//!   implementation puts in the surrounding logic, not in the round datapath. The
//!   tests build the padded block themselves, which is also what lets them compare
//!   against the `sha2` crate at the level of a whole hash.
//! - **No pipelining.** Sixty-four rounds in series is a long combinational path and
//!   a real block would cut it with a register every few rounds. A pipeline
//!   register here would put 64 identical round slices behind a state machine and
//!   hide the whole message schedule from the toolchain, which is the opposite of
//!   what this corpus is for. The round boundary is [`round`], so adding a register
//!   is a one-line change per boundary.
//! - **No `valid`.** The module consumes a block every cycle unconditionally.
//!   Real hash cores have a `valid` because real buses have idle cycles, and it is
//!   a one-line change: the register's `clear` input is already a pin on this
//!   design, tied low.
//!
//! # Ports
//!
//! The block arrives as sixteen 32-bit words (`block_0`..`block_15`) and the state
//! leaves as eight (`state_0`..`state_7`). Big-endian, most significant word
//! first, which is how FIPS 180-4 §6.2.1 numbers them. Splitting into words
//! rather than one 256-bit port is not a style choice: the cosimulator's generated
//! driver reads and writes every port through a `u64`, so a port wider than 64
//! bits cannot be driven at all. Thirty-two-bit words are also what a real
//! implementation's message schedule register is cut into.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// How many bits are in one hash word. Fixed by the specification, not a choice.
pub const WORD_BITS: u32 = 32;

/// How many words the chaining state is, which is also the digest width in words.
pub const STATE_WORDS: usize = 8;

/// How many 32-bit words one 512-bit message block is.
pub const BLOCK_WORDS: usize = 16;

/// How many bytes one message block is.
pub const BLOCK_BYTES: usize = BLOCK_WORDS * (WORD_BITS as usize / 8);

/// How many rounds SHA-256 has.
pub const ROUNDS: usize = 64;

/// The initial hash value H(0)..H(7): the eight words a hash starts from.
///
/// FIPS 180-4 §5.3.3: "These words were obtained by taking the first thirty-two
/// bits of the fractional parts of the **square** roots of the first eight prime
/// numbers." Square roots, not cube roots -- the round constants are the cube
/// roots, and the two are easy to conflate.
///
/// They are literals because a fraction of a square root is not a value any
/// hardware can generate on the fly, so a design that computed one instead would
/// be computing a different algorithm at great expense. They are also the value
/// the chaining register must *start* from; see the module docs on `rst`.
pub const H: [u32; STATE_WORDS] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// The 64 round constants K(0)..K(63), one per round.
///
/// FIPS 180-4 §4.2.2: the first 32 bits of the fractional parts of the **cube**
/// roots of the first 64 prime numbers (2 through 311). A round reads `K[t]` and
/// nothing else -- there is no table and no index, because the rounds are
/// unrolled, so round `t`'s constant is a literal baked into round `t`'s adder.
pub const K: [u32; ROUNDS] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// The eight words of the chaining state, most significant word first.
///
/// `words[0]`..`words[7]` are `a`..`h` in FIPS 180-4 §6.2.2. A plain array rather
/// than eight named fields because the eight are rotated rather than replaced:
/// every round produces one new word and shifts the other seven along by one, and
/// eight named fields would turn that shift into seven assignments a reader has to
/// check against the specification.
#[derive(Clone, Debug)]
pub struct State {
    /// The eight 32-bit words, `a` through `h`.
    pub words: [Signal; STATE_WORDS],
}

impl State {
    /// Eight 32-bit constants, in word order.
    ///
    /// # Errors
    ///
    /// Whatever [`Design::lit`] reports. A `u32` at width 32 always fits, so there
    /// is no failure specific to the constants themselves.
    pub fn constants(design: &Design, values: &[u32; STATE_WORDS]) -> Result<Self, BuildError> {
        let mut literals = Vec::with_capacity(STATE_WORDS);
        for value in values {
            literals.push(design.lit(u64::from(*value), WORD_BITS)?);
        }
        // Indexed rather than converted: the vector was pushed to exactly the
        // array's length one line above, so the index cannot be out of range, and a
        // fallible conversion here would mean inventing a length error for a length
        // this function controls.
        Ok(Self {
            words: std::array::from_fn(|index| literals[index].clone()),
        })
    }

    /// Each word taken from `then_value` when `condition` is set, and from `self`
    /// otherwise.
    ///
    /// A word-at-a-time select rather than a 256-bit one, for the same reason the
    /// ports are split: a width above 64 bits is outside what the cosimulator's
    /// driver can carry, and a design that only had 256-bit values to hand would
    /// have to be split again at the boundary anyway.
    ///
    /// # Errors
    ///
    /// Whatever [`Design::ite`] reports.
    pub fn mux(
        &self,
        design: &Design,
        condition: &Signal,
        then_value: &State,
    ) -> Result<Self, BuildError> {
        let mut chosen = Vec::with_capacity(STATE_WORDS);
        for index in 0..STATE_WORDS {
            chosen.push(design.ite(condition, &then_value.words[index], &self.words[index])?);
        }
        Ok(Self {
            words: std::array::from_fn(|index| chosen[index].clone()),
        })
    }
}

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge compresses one block and advances the chain.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which means "go back to the start of the hash".
    ///
    /// Asserted, the chaining register holds the initial hash value H(0)..H(7) and
    /// the compression reads that rather than the register, so a block presented
    /// on the edge that releases `rst` is the first block of a hash. It is *not*
    /// wired to the register's reset pin: that can only load zero, and a chaining
    /// register that starts at zero computes a well-formed hash of nothing.
    ///
    /// A one-bit *port* rather than a constant because a reset that cannot be
    /// exercised is a reset that is not tested.
    pub rst: Signal,
    /// The sixteen message words of the block, most significant first.
    ///
    /// Collected rather than written out sixteen times, because `PortList`
    /// expands a `Vec<Signal>` into `block_0`..`block_15` and that keeps the
    /// declaration and the sixteen names in one place. The lengths are literals
    /// because `PortList` reads them with `base10_parse`, and a test pins each one
    /// against the constant it stands for.
    #[bits(32)]
    #[length(16)]
    pub block: Vec<Signal>,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The chaining state as it stands after the last block consumed, most
    /// significant word first.
    #[bits(32)]
    #[length(8)]
    pub state: Vec<Signal>,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The eight chaining registers, which are also the module outputs.
    pub state: State,
}

/// Builds a one-block-per-cycle SHA-256 into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say.
///
/// # Errors
///
/// Whatever [`Design`] returns and whatever the port-list helpers return. There
/// is no SHA-specific failure, because a block width and a round count are fixed
/// by the specification: a design with either of them configurable is a design
/// for a different algorithm.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;
    let initial = State::constants(design, &H)?;
    let block = std::array::from_fn(|index| inputs.block[index].clone());

    // The chaining register's outputs have to be wires, because the datapath reads
    // them and the register is fed from the datapath: the graph is built forwards
    // and there is no cycle in it, so the loop is closed by driving the wires from
    // the registers once both exist. This is the pattern for any feedback through
    // state.
    let mut chain = Vec::with_capacity(STATE_WORDS);
    for _ in 0..STATE_WORDS {
        chain.push(design.wire(WORD_BITS)?);
    }
    let chained = State {
        words: std::array::from_fn(|index| chain[index].clone()),
    };

    // Reset selects the initial hash value on both sides of the register: into it,
    // and into the datapath's chaining input. Without the second mux the module
    // would only be correct if the caller always asserted `rst` for a cycle first,
    // and with only the first mux the register would never come out of reset
    // holding H at all -- which is the same wrong hash by a different route.
    let state_in = chained.mux(design, &inputs.rst, &initial)?;
    let next = compress(design, &state_in, &block)?;
    let loaded = next.mux(design, &inputs.rst, &initial)?;

    let mut held = Vec::with_capacity(STATE_WORDS);
    for index in 0..STATE_WORDS {
        // Both control pins are tied low, so the register updates on every edge:
        // this design has no `valid` and consumes a block per cycle
        // unconditionally. The initialisation is on the data input above rather
        // than on the reset pin for the reason the module docs give.
        held.push(design.reg(
            &loaded.words[index],
            &inputs.clk,
            &design.constant(false),
            &design.constant(false),
        )?);
    }
    let held: [Signal; STATE_WORDS] = std::array::from_fn(|index| held[index].clone());

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            state: held.to_vec(),
        },
    )?;

    for index in 0..STATE_WORDS {
        design.drive(&chain[index], &held[index])?;
    }

    Ok(Ports {
        inputs,
        state: State { words: held },
    })
}

/// Compresses one block into `state`, all [`ROUNDS`] rounds unrolled.
///
/// `state` is the chaining value the block is compressed from -- for SHA-256
/// that is [`H`] at the start of a hash -- and `block` is the block's sixteen
/// words, most significant first. This is FIPS 180-4 §6.2.2 in full, including
/// the final feed-forward, and it is the bare compression function: it reads and
/// writes no state of its own, so a caller may put the chaining register
/// wherever it likes.
///
/// The message schedule is the first half of the work: `w[0..16]` is the block
/// itself, and `w[t]` for `t >= 16` is
/// `w[t-16] + sigma0(w[t-15]) + w[t-7] + sigma1(w[t-2])`. Every word is built,
/// not looked up, so the schedule is 48 copies of two shifts-and-xor clusters and
/// three adds rather than a 64-entry ROM.
///
/// # Errors
///
/// Whatever the sigma functions and [`round`] report.
pub fn compress(
    design: &Design,
    state: &State,
    block: &[Signal; BLOCK_WORDS],
) -> Result<State, BuildError> {
    let mut schedule: Vec<Signal> = Vec::with_capacity(ROUNDS);
    schedule.extend(block.iter().cloned());

    for index in BLOCK_WORDS..ROUNDS {
        let low = small_sigma0(design, &schedule[index - 15])?;
        let high = small_sigma1(design, &schedule[index - 2])?;
        // Left to right, and every operand is 32 bits, so every intermediate is a
        // truncating 32-bit add. That truncation *is* the modular arithmetic the
        // specification asks for, so nothing is lost by not widening: SHA-256
        // explicitly reduces modulo 2^32 after every addition.
        let mut word = design.add(&schedule[index - 16], &low)?;
        word = design.add(&word, &schedule[index - 7])?;
        word = design.add(&word, &high)?;
        schedule.push(word);
    }

    let mut carried = state.clone();
    for index in 0..ROUNDS {
        carried = round(design, &carried, K[index], &schedule[index])?;
    }

    // FIPS 180-4 §6.2.2 step 4: the new chaining words are the *sum* of the ones
    // the block started from and the working words the 64 rounds left behind.
    // Leaving this out is the classic SHA-256 transcription slip and it is a
    // remarkably plausible one -- a round loop that hands back `carried` looks
    // like a finished state -- so it is written out here rather than folded into
    // [`round`]. It belongs to the block rather than to a round: with no chaining
    // state the sum would be against zero, and a design that hard-coded that
    // would be correct for exactly one block.
    let mut sum = Vec::with_capacity(STATE_WORDS);
    for index in 0..STATE_WORDS {
        sum.push(design.add(&state.words[index], &carried.words[index])?);
    }
    Ok(State {
        words: std::array::from_fn(|index| sum[index].clone()),
    })
}

/// One round: the eight working words in, the eight working words out.
///
/// FIPS 180-4 §6.2.2 step 3, with `k` this round's [`K`] constant and `w` this
/// round's scheduled message word:
///
/// ```text
/// t1 = h + Sigma1(e) + Ch(e, f, g) + k + w
/// t2 = Sigma0(a) + Maj(a, b, c)
/// (a, b, c, d, e, f, g, h) = (t1 + t2, a, b, c, d + t1, e, f, g)
/// ```
///
/// Note what is *not* recomputed: `a` through `d` and `e` through `g` are carried
/// across unchanged, which is the rotation the last line performs. Only one new
/// word is produced and the other seven are the same nodes as the round before, so
/// a design that unrolls this 64 times builds 64 new big-Sigma clusters rather
/// than 64 sets of eight.
///
/// These are the *working* words, not the chaining state: the feed-forward back to
/// the chaining words belongs to [`compress`], not to a round.
///
/// # Errors
///
/// Whatever [`Design::lit`] and [`Design::add`] report.
pub fn round(design: &Design, state: &State, k: u32, w: &Signal) -> Result<State, BuildError> {
    let a = &state.words[0];
    let b = &state.words[1];
    let c = &state.words[2];
    let d = &state.words[3];
    let e = &state.words[4];
    let f = &state.words[5];
    let g = &state.words[6];
    let h = &state.words[7];

    let big1 = big_sigma1(design, e)?;
    let choose = ch(design, e, f, g)?;
    let constant = design.lit(u64::from(k), WORD_BITS)?;
    let mut t1 = design.add(h, &big1)?;
    t1 = design.add(&t1, &choose)?;
    t1 = design.add(&t1, &constant)?;
    t1 = design.add(&t1, w)?;

    let big0 = big_sigma0(design, a)?;
    let majority = maj(design, a, b, c)?;
    let t2 = design.add(&big0, &majority)?;

    let new_a = design.add(&t1, &t2)?;
    let new_e = design.add(d, &t1)?;

    Ok(State {
        words: [
            new_a,
            a.clone(),
            b.clone(),
            c.clone(),
            new_e,
            e.clone(),
            f.clone(),
            g.clone(),
        ],
    })
}

/// `Ch(e, f, g)`: the conditional that is the round's one non-linear step.
///
/// `(e and f) xor (not e and g)`, which is a bit-select between `f` and `g` keyed
/// on `e` -- and a bit-select is exactly what a mux is, so this is two ANDs, an
/// inverter and an XOR rather than anything with a lookup in it. It is the whole
/// reason SHA-256 resists differential cryptanalysis, and it is also the cheapest
/// thing in the round.
///
/// # Errors
///
/// Whatever [`Design`] returns. [`Design::not`] is the only operation here that
/// cannot fail, so the fallible ones are the two ANDs and the XOR.
pub fn ch(design: &Design, e: &Signal, f: &Signal, g: &Signal) -> Result<Signal, BuildError> {
    let taken = design.and(e, f)?;
    let untaken = design.and(&design.not(e), g)?;
    Ok(design.xor(&taken, &untaken)?)
}

/// `Maj(a, b, c)`: the majority function over three words.
///
/// `(a and b) xor (a and c) xor (b and c)`, which is one if two or three of the
/// inputs are one. The three pairwise ANDs are not redundancy: an XOR-only majority
/// would be `(a xor b) xor (b xor c) xor a`, three fewer gates, and it would not
/// be the majority function. SHA-256 needs the gate that costs the four extra
/// ANDs.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn maj(design: &Design, a: &Signal, b: &Signal, c: &Signal) -> Result<Signal, BuildError> {
    let ab = design.and(a, b)?;
    let ac = design.and(a, c)?;
    let bc = design.and(b, c)?;
    Ok(design.xor(&design.xor(&ab, &ac)?, &bc)?)
}

/// `Sigma0(x)`, the *big* sigma: `ROTR 2 xor ROTR 13 xor ROTR 22`.
///
/// It feeds `t2` and depends on `a`, which the round has just overwritten, so it
/// is the part of the round with the deepest path into the state. Three rotates
/// and two XORs; there is no way to factor it, which is why an implementation that
/// wanted fewer gates per round would have to pipeline rather than rearrange.
///
/// # Errors
///
/// Whatever [`rotate_right`] and [`Design::xor`] report.
pub fn big_sigma0(design: &Design, x: &Signal) -> Result<Signal, BuildError> {
    let two = rotate_right(design, x, 2)?;
    let thirteen = rotate_right(design, x, 13)?;
    let twenty_two = rotate_right(design, x, 22)?;
    Ok(design.xor(&design.xor(&two, &thirteen)?, &twenty_two)?)
}

/// `Sigma1(x)`, the other *big* sigma: `ROTR 6 xor ROTR 11 xor ROTR 25`.
///
/// # Errors
///
/// Whatever [`rotate_right`] and [`Design::xor`] report.
pub fn big_sigma1(design: &Design, x: &Signal) -> Result<Signal, BuildError> {
    let six = rotate_right(design, x, 6)?;
    let eleven = rotate_right(design, x, 11)?;
    let twenty_five = rotate_right(design, x, 25)?;
    Ok(design.xor(&design.xor(&six, &eleven)?, &twenty_five)?)
}

/// `sigma0(x)`, a *small* sigma: `ROTR 7 xor ROTR 18 xor SHR 3`.
///
/// The message-schedule sigma, and the only place SHA-256 uses a plain logical
/// shift. It is not `Sigma0`: the small sigmas are a different function of a
/// different argument, and writing the schedule with the big ones would produce a
/// design that computes a real, different, wrong hash.
///
/// # Errors
///
/// Whatever [`rotate_right`] and [`Design`] return.
pub fn small_sigma0(design: &Design, x: &Signal) -> Result<Signal, BuildError> {
    let seven = rotate_right(design, x, 7)?;
    let eighteen = rotate_right(design, x, 18)?;
    let shifted = design.srl(x, 3)?;
    Ok(design.xor(&design.xor(&seven, &eighteen)?, &shifted)?)
}

/// `sigma1(x)`: `ROTR 17 xor ROTR 19 xor SHR 10`.
///
/// # Errors
///
/// Whatever [`rotate_right`] and [`Design`] return.
pub fn small_sigma1(design: &Design, x: &Signal) -> Result<Signal, BuildError> {
    let seventeen = rotate_right(design, x, 17)?;
    let nineteen = rotate_right(design, x, 19)?;
    let shifted = design.srl(x, 10)?;
    Ok(design.xor(&design.xor(&seventeen, &nineteen)?, &shifted)?)
}

/// `ROTR(x, by)`, as an OR of the two shifts.
///
/// SHA-256 uses only rotates and logical shifts -- never an arithmetic shift and
/// never a sign-extending one -- so `srl` rather than `sra` on the way down is not
/// an approximation. The two shifts are built structurally (`sll` and `srl` are a
/// `select` and a `concat`), so a rotate costs three wires and a gate rather than
/// needing the shifter to infer a barrel rotator.
///
/// A `by` of zero works without a special case: `srl` by zero is the whole value
/// and `sll` by the word width is all zeros, so the OR is the value again.
///
/// # Errors
///
/// Whatever [`Design::sll`], [`Design::srl`] and [`Design::or`] report.
pub fn rotate_right(design: &Design, x: &Signal, by: u32) -> Result<Signal, BuildError> {
    let down = design.srl(x, by)?;
    let up = design.sll(x, WORD_BITS - by)?;
    Ok(design.or(&down, &up)?)
}
