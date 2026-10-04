//! ChaCha20: the block function, twenty rounds, unrolled.
//!
//! # Why this is the counterexample to "ROM-heavy ciphers are the ASIC ones"
//!
//! Everything else in this crate is small and word-shaped. This one is a cipher,
//! and the interesting thing about it is what it does *not* contain: **no tables at
//! all**. AES needs an 8 KiB S-box and a MixColumns matrix; ChaCha20 has nothing to
//! store. There is no lookup, no substitution step, no initial value permutation
//! driven by a key schedule that has to be evaluated and cached. Read [`CONSTANTS`]
//! and [`COLUMN_ROUND`] and you have the entire algorithm: 20 rounds of add, XOR and
//! rotate.
//!
//! That is why ChaCha20 is the *better* ASIC cipher even though AES is the one with
//! the instructions in every CPU. The datapath here is sixteen 32-bit adders and a
//! pile of wiring, so an implementation gets **one 64-byte block per cycle** for the
//! cost of adders and rotators — no SRAM, no init ROM, nothing whose area or power
//! grows with the key. §2.3's twenty rounds *are* the whole hardware specification;
//! there is nothing to fit in a memory and nothing to time-multiplex around. A CPU,
//! by contrast, runs the same eighty quarter-rounds as eighty dependent 32-bit ALU
//! operations with no vector path at all: the design has no SIMD story, it has a
//! serial dependency chain eight quarter-rounds deep per round.
//!
//! So the argument for hardware is not "a cipher is a lot of logic". It is "a
//! cipher whose primitives are add, XOR and rotate is pure ALU, and pure ALU
//! pipelines".
//!
//! # The algorithm, and where the constants come from
//!
//! RFC 8439 §2.3 defines the state as sixteen 32-bit words in four groups:
//!
//! ```text
//!  cccccccc  cccccccc  cccccccc  cccccccc
//!  kkkkkkkk  kkkkkkkk  kkkkkkkk  kkkkkkkk
//!  kkkkkkkk  kkkkkkkk  kkkkkkkk  kkkkkkkk
//!  bbbbbbbb  nnnnnnnn  nnnnnnnn  nnnnnnnn
//! ```
//!
//! | words | contents | source |
//! |---|---|---|
//! | 0–3 | `expand 32-byte k` as four little-endian words | RFC 8439 §2.3 |
//! | 4–11 | the 256-bit key, read little-endian in 4-byte chunks | RFC 8439 §2.3 |
//! | 12 | the 32-bit block counter | RFC 8439 §2.3 |
//! | 13–15 | the 96-bit nonce, little-endian | RFC 8439 §2.3 |
//!
//! Twenty rounds is ten double-rounds, and a double-round is the eight
//! quarter-rounds of [`COLUMN_ROUND`] followed by the eight of
//! [`DIAGONAL_ROUND`] (§2.3.1, `inner_block`). §2.1.1 gives the quarter round
//! itself as eight statements, and [`quarter_round`] is a line-by-line
//! transcription of them:
//!
//! ```text
//! a += b; d ^= a; d <<<= 16;
//! c += d; b ^= c; b <<<= 12;
//! a += b; d ^= a; d <<<= 8;
//! c += d; b ^= c; b <<<= 7;
//! ```
//!
//! `+` is addition modulo 2^32 (§2.1), `^` is XOR, and `<<< n` is an n-bit left
//! roll toward the high bits. The eight statements are two halves of a
//! four-step structure with a 16/12/8/7 rotation ladder: 16 and 12 are large enough
//! that `a` and `c` meet across the whole word, 8 and 7 are the diagonal, and none
//! of the four is 0 or 16, which is what stops a lane from being a copy of its
//! neighbour.
//!
//! After the twenty rounds, §2.3 adds the *original* sixteen words back in and
//! serializes little-endian. That feed-forward is why [`block`] keeps a second copy
//! of the input state rather than reaching for a register: it is the difference
//! between a block function that diffuses and one that can be inverted step by step.
//!
//! # The counter is 32 bits, and that is the IETF variant
//!
//! RFC 8439 §2.3 deliberately moved from the original ChaCha's 64-bit counter and
//! 64-bit nonce to a 32-bit counter and a 96-bit nonce, for consistency with
//! RFC 5116 §3.2, and records the trade: a single (key, nonce) pair is then good
//! for 2^32 blocks, or 256 GB. §2.8.1's note 1 gives the way back — keep the IETF
//! layout and promote word 13 to the top of a 64-bit counter, shrinking the nonce
//! to 64 bits. That is a different module with the same quarter-round core, which is
//! why [`rounds`] and [`quarter_round`] are public rather than buried inside
//! [`build`].
//!
//! # Combinational datapath, registered outputs
//!
//! The twenty rounds are combinational: no register sits inside them, because the
//! RFC does not have one and a pipeline register there would only add latency to a
//! function that already fits in a cycle. [`build`] does register the sixteen
//! output words, and that is an interface decision rather than an algorithmic one —
//! a keystream block has to be available on a clock boundary for whatever XORs it
//! against the plaintext.
//!
//! It is also the only way this design can be cosimulated, and that is worth
//! stating because it is a real gap in the toolchain rather than a preference.
//! `ferrite_lithic_rtl::Module::combinational` exists for a design with no clock,
//! and the cosimulator's generated Verilog driver knows what to do with one — it
//! settles instead of clocking, and says so in its own comment. But
//! `ferrite_lithic_cosim::simulate`, which produces the *reference* side of the
//! comparison, calls `Sim::step` on every cycle, and `Sim::step` is
//! `Error::NoClock` when `Program::clock()` is `None`. So a clockless design
//! verilates and then cannot be compared against anything. The honest response is
//! to have a clock and a register, and to say here that this design has one
//! because the reference runner needs it.
//!
//! The alternative -- declaring an `input clk` that no flip-flop latches, so that
//! `Program::clock()` is `Some` and nothing else changes -- is worse. It would put a
//! port on the port list that drives nothing and tell whoever reads the port list
//! that the block function has a cycle it does not have.
//!
//! # Thirty-two-bit ports, and why not one 512-bit port
//!
//! The state is 512 bits wide and it is exposed as **sixteen separate 32-bit words**,
//! plus 32-bit ports for the key, the nonce and the counter. That is not a
//! stylistic choice. The cosimulator's generated C++ driver refuses to build for a
//! port wider than 64 bits rather than silently truncating it, so a design that
//! wanted a 512-bit port would not cosimulate against Verilator at all. Since the
//! point of this crate is that the emitted Verilog is checked against the
//! simulator, the port list is shaped to be checkable. Thirty-two bits is also the
//! width the RFC gives every state word (§2.1), so a port is exactly a state word
//! and nothing has to be re-split or re-indexed to read the ports back.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// `"expand 32-byte k"` as four little-endian 32-bit words.
///
/// The four constant words of the ChaCha state, which is what makes a ChaCha20 block
/// recognisable in a hex dump: the first four bytes of every keystream block come
/// from here and never from the key. RFC 8439 §2.3.
pub const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

/// How many rounds ChaCha20 runs. RFC 8439 §2.1: 20 rounds is 80 quarter-rounds.
pub const ROUNDS: u32 = 20;

/// The four column-round quarter-rounds. RFC 8439 §2.3.1, `inner_block` lines 1–4.
pub const COLUMN_ROUND: [[usize; 4]; 4] =
    [[0, 4, 8, 12], [1, 5, 9, 13], [2, 6, 10, 14], [3, 7, 11, 15]];

/// The four diagonal-round quarter-rounds. RFC 8439 §2.3.1, `inner_block` lines 5–8.
pub const DIAGONAL_ROUND: [[usize; 4]; 4] =
    [[0, 5, 10, 15], [1, 6, 11, 12], [2, 7, 8, 13], [3, 4, 9, 14]];

/// The state word width, in bits. Every word of the RFC's state is a 32-bit
/// unsigned integer (§2.1).
const WORD: u32 = 32;

/// How many words the state has. RFC 8439 §2.3, the `cccc`/`kkkk`/`bbbb`/`nnnn`
/// diagram.
pub const STATE_WORDS: usize = 16;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock every output word updates on.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which zeroes the sixteen output words.
    ///
    /// A block function has no state to restore, so this does nothing the RFC asks
    /// for; it exists because a design whose output ports are registers has to say
    /// what those registers reset to.
    pub rst: Signal,
    /// The 256-bit key as eight little-endian words, `key_0` through `key_7`.
    ///
    /// Read little-endian in 4-byte chunks (RFC 8439 §2.3), so `key_0` is bytes 0–3
    /// of the key and `key_7` is bytes 28–31.
    #[bits(32)]
    #[length(8)]
    pub key: Vec<Signal>,
    /// The 96-bit nonce as three little-endian words, `nonce_0` through `nonce_2`.
    ///
    /// These land at state words 13, 14 and 15, in that order (RFC 8439 §2.3).
    #[bits(32)]
    #[length(3)]
    pub nonce: Vec<Signal>,
    /// The 32-bit block counter, which is state word 12 (RFC 8439 §2.3).
    #[bits(32)]
    pub counter: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The sixteen state words after the feed-forward addition, `word_0` through
    /// `word_15`.
    ///
    /// In RFC 8439's serialization order, so `word_0` is bytes 0–3 of the 64-byte
    /// block, little-endian (§2.3). Registered: one clock edge after the inputs are
    /// stable, the block is here.
    #[bits(32)]
    #[length(16)]
    pub word: Vec<Signal>,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The sixteen output words, which are also module output ports.
    pub words: Vec<Signal>,
}

/// Builds the ChaCha20 block function into `design`.
///
/// The twenty rounds are combinational and the sixteen output words are registered;
/// see the module docs.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return. There is no ChaCha-specific failure: the
/// key, nonce and counter widths are fixed by RFC 8439 §2.3, so a design taking any
/// other shape is a different module.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let words = block(design, &inputs.key, &inputs.nonce, &inputs.counter)?;

    // No feedback here, so unlike ghash there is nothing to route through a wire:
    // each register takes its data directly and its value is the output port.
    // `clear` is tied low, so a word updates on every edge; the only thing that
    // holds a word at zero is `rst`.
    let hold = design.constant(false);
    let words = words
        .iter()
        .map(|word| design.reg(word, &inputs.clk, &inputs.rst, &hold))
        .collect::<Result<Vec<_>, _>>()?;

    let outputs = ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            word: words.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        words: outputs.word,
    })
}

/// The ChaCha20 block function as a combinational function of key, nonce and counter.
///
/// This is the whole algorithm and nothing else: the initial state, the twenty
/// rounds, and the feed-forward addition of RFC 8439 §2.3.1's `chacha20_block`. It
/// is public and separate from [`build`] for two reasons. It is the part a
/// different module reuses — a ChaCha20-CTR mode wants the counter in a register and
/// everything else from here — and it builds no register, so a caller that wants a
/// genuinely combinational block function can drive it directly. It is also what
/// `build` registers, so there is exactly one implementation of the twenty rounds.
///
/// `key` must hold eight 32-bit signals and `nonce` three, in the order RFC 8439
/// §2.3 puts them in the state.
///
/// # Errors
///
/// Whatever [`Design`] returns. The slice lengths are taken from the signals rather
/// than checked, so a `key` of the wrong length silently produces a design whose
/// state is not 512 bits; [`build`] cannot, because the port list fixes the widths.
pub fn block(
    design: &Design,
    key: &[Signal],
    nonce: &[Signal],
    counter: &Signal,
) -> Result<Vec<Signal>, BuildError> {
    // The initial state, in RFC 8439 §2.3's order. The feed-forward at the end
    // needs this copy, so it is kept: it is the difference between twenty rounds
    // and the block function.
    let mut initial = Vec::with_capacity(STATE_WORDS);
    for word in CONSTANTS {
        initial.push(design.lit(u64::from(word), WORD)?);
    }
    initial.extend(key.iter().cloned());
    initial.push(counter.clone());
    initial.extend(nonce.iter().cloned());

    let mixed = rounds(design, &initial)?;

    // `state += initial_state` word by word. RFC 8439 §2.3: "addition in the above
    // paragraph is done modulo 2^32".
    mixed
        .iter()
        .zip(&initial)
        .map(|(mixed, initial)| add32(design, mixed, initial))
        .collect()
}

/// The twenty rounds of RFC 8439 §2.3, as ten column/diagonal double-rounds.
///
/// Takes and returns the sixteen state words by value so the caller keeps its own
/// copy of the initial state; every word is a distinct node, and the loop is the
/// algorithm run over [`Design`] nodes rather than a description of it.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn rounds(design: &Design, initial: &[Signal]) -> Result<Vec<Signal>, BuildError> {
    let mut state = initial.to_vec();
    for _ in 0..ROUNDS / 2 {
        quarter_rounds(design, &mut state, &COLUMN_ROUND)?;
        quarter_rounds(design, &mut state, &DIAGONAL_ROUND)?;
    }
    Ok(state)
}

/// Four quarter-rounds of a column or diagonal round, in place.
///
/// `state` is the sixteen words and `groups` four lists of the four indices each
/// quarter-round touches. The indices are the RFC's (RFC 8439 §2.3.1); nothing here
/// knows which of the two rounds it is running, which is why the column and diagonal
/// rounds are one function and two tables.
///
/// # Errors
///
/// Whatever [`quarter_round`] returns.
fn quarter_rounds(
    design: &Design,
    state: &mut [Signal],
    groups: &[[usize; 4]],
) -> Result<(), BuildError> {
    for group in groups {
        let (ia, ib, ic, id) = (group[0], group[1], group[2], group[3]);
        // The four words are taken out, mixed and put back rather than passed as
        // four simultaneous mutable borrows: `state` is one slice and the quarter
        // round writes all four of them, so it cannot read them in place.
        let (mut a, mut b, mut c, mut d) = (
            state[ia].clone(),
            state[ib].clone(),
            state[ic].clone(),
            state[id].clone(),
        );
        quarter_round(design, &mut a, &mut b, &mut c, &mut d)?;
        state[ia] = a;
        state[ib] = b;
        state[ic] = c;
        state[id] = d;
    }
    Ok(())
}

/// One quarter-round on four state words, in place.
///
/// A line-by-line transcription of RFC 8439 §2.1.1:
///
/// ```text
/// a += b; d ^= a; d <<<= 16;
/// c += d; b ^= c; b <<<= 12;
/// a += b; d ^= a; d <<<= 8;
/// c += d; b ^= c; b <<<= 7;
/// ```
///
/// The four arguments are reassigned rather than shadowed, because the RFC's second
/// line operates on the `a` the first line produced — shadowing would compute a
/// different function that still produces sixteen plausible words. Each line's sum
/// is carried into the line that needs it as `a1`, `c1`, `a2`, `c2` rather than
/// recomputed from the words, which is what RFC 8439 §3's implementation advice asks
/// for: its 5.5% saving comes from copying the state aside instead of rebuilding it,
/// and a quarter round that re-adds from `a` and `b` gives that back twice over.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn quarter_round(
    design: &Design,
    a: &mut Signal,
    b: &mut Signal,
    c: &mut Signal,
    d: &mut Signal,
) -> Result<(), BuildError> {
    // `a += b; d ^= a; d <<<= 16;`
    let a1 = add32(design, a, b)?;
    *d = rotl(design, &design.xor(d, &a1)?, 16)?;
    // `c += d; b ^= c; b <<<= 12;` -- `d` is the value the line above left.
    let c1 = add32(design, c, d)?;
    *b = rotl(design, &design.xor(b, &c1)?, 12)?;
    // `a += b; d ^= a; d <<<= 8;` -- `a` is `a1` and `b` the rotated one.
    let a2 = add32(design, &a1, b)?;
    *d = rotl(design, &design.xor(d, &a2)?, 8)?;
    // `c += d; b ^= c; b <<<= 7;` -- `c` is `c1` and `d` the rotl-8 one.
    let c2 = add32(design, &c1, d)?;
    *b = rotl(design, &design.xor(b, &c2)?, 7)?;
    *a = a2;
    *c = c2;
    Ok(())
}

/// Addition modulo 2^32, the RFC's `+` (RFC 8439 §2.1).
///
/// [`Design::add`] truncates to its left operand's width, so adding two 32-bit words
/// would already be modular — but only by a rule about operand order that this module
/// should not depend on. Both operands are widened to 33 bits, added, and narrowed
/// back, which computes the carry out of bit 31 and then discards it. That is what
/// "addition modulo 2^32" means, stated rather than inferred.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn add32(design: &Design, left: &Signal, right: &Signal) -> Result<Signal, BuildError> {
    let left = widen(design, left)?;
    let right = widen(design, right)?;
    Ok(design.truncate(&design.add(&left, &right)?, WORD)?)
}

/// A 33-bit zero-extended copy of a 32-bit word.
///
/// This is [`Design::zero_extend`] spelled out, and the reason it is spelled out is a
/// defect in the Verilog emitter rather than a preference. `zero_extend` is a
/// part-select of the low 32 bits followed by a `cat` with a zero pad, and
/// `Node::Select` renders as `{operand}[{offset} +: {len}]` for every operand
/// including a literal — so zero-extending one of this design's four constant words
/// emits `assign w = 32'h61707865[0 +: 32];`, which is a syntax error in Verilog and
/// stops Verilator dead. A part-select of a *net* is legal, which is why no other
/// design in the corpus has hit it: this one is the first to widen a constant.
///
/// The concat below emits `{1'h0, operand}` for a constant operand, is correct either
/// way, and costs one node instead of two. Fixing the emitter to drop the select when
/// it covers the operand whole — or to parenthesise the literal — would let this go
/// back to `zero_extend`.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn widen(design: &Design, value: &Signal) -> Result<Signal, BuildError> {
    Ok(design.concat(&[design.zeros(1)?, value.clone()])?)
}

/// An `n`-bit left roll, the RFC's `<<<` (RFC 8439 §2.1: "an n-bit left roll towards
/// the high bits").
///
/// A rotate is wiring, not arithmetic: the top `n` bits and the bottom `WORD - n`
/// bits swap places. [`Design`] has no rotate, and building one from a shift and a
/// mask would be four gates more than the two this needs.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn rotl(design: &Design, value: &Signal, by: u32) -> Result<Signal, BuildError> {
    // `by == 0` and `by == WORD` are both rotations by zero, and neither appears in
    // RFC 8439 §2.1's ladder of 16, 12, 8 and 7. Asserted rather than assumed,
    // because `by >= WORD` would silently produce a slice past the end.
    assert!(
        (1..WORD).contains(&by),
        "a rotation of {by} bits is not a rotation"
    );
    let low = design.slice(value, 0, WORD - by)?;
    let high = design.slice(value, WORD - by, by)?;
    Ok(design.concat(&[low, high])?)
}
