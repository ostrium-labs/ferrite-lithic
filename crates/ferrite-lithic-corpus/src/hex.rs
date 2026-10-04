//! Lowercase hexadecimal: one byte in, two ASCII characters out, one a cycle.
//!
//! # Why this shape
//!
//! [`crc3`](crate::crc3) is a design with a serial dependency chain: one byte per
//! cycle, one result per byte, and the only thing the hardware buys is a flopper
//! version of a table lookup. This design is the other end of the spectrum. It has
//! **no dependency chain at all** -- the output character for one nibble does not
//! depend on the output character for any other -- so the only interesting hardware
//! question left is *rate*, and the rate is fixed by the 8:4 split rather than by
//! the design.
//!
//! One byte carries eight bits of information and becomes two characters of four
//! bits each. So the datapath is **8 bits in per cycle and 4 bits out per cycle**,
//! and no amount of pipelining changes that: a hex encoder is information-preserving
//! and therefore cannot emit more than four bits per cycle, ever. The useful design
//! question is not "how fast can this go" but "does it reach the 4 bits per cycle
//! limit at all", because a design that accepts a byte a cycle and emits a character
//! a cycle would be cheating -- it would have to be doing two lookups in one cycle,
//! which is a 16-entry ROM addressed by eight bits, i.e. sixteen times the
//! combinational logic for no reason.
//!
//! So this design accepts a byte every two cycles and emits a character *every*
//! cycle, which is the same 4 bits per cycle expressed the other way round, and it
//! saturates the output bus: from the first accepted byte to the last, there is no
//! cycle in which `valid` is low. The test
//! `the_output_bus_saturates_when_the_input_is_fed_at_its_maximum_rate` is the
//! assertion that this design actually reaches the limit.
//!
//! # Why a lookup and not a gate network
//!
//! A CRC is an affine function of its inputs over GF(2), so it is XORs and nothing
//! else. The hex digit is **not**: the map from a nibble to its character is not
//! affine, and the cheapest way to see that is to note that its second difference
//! over the nibble is not constant. `crc3` can be built out of `xor` and `and`
//! alone and this cannot, so the shape has to be a 16-entry lookup -- which is
//! [`Design::case_`], and which a synthesizer turns into a 16x8 ROM or a
//! four-level mux network. Nothing in this module is a cleverer encoding of the
//! same table; the table is the design.
//!
//! # The alphabet, and where it comes from
//!
//! [`ALPHABET`] is the 16 characters of RFC 4648 s2 ("Representations of Numeric
//! Values"), which specifies the digits `0`-`9` followed by `A`-`F`. RFC 4648 spells
//! them uppercase; **this design emits the lowercase form**, because lowercase is
//! what the `hex` crate's `hex::encode` produces and this corpus checks against that
//! crate rather than against a self-written model. The difference is one bit: ASCII
//! bit 5 (`0x20`), so `0x61..=0x66` for `a`-`f` rather than `0x41..=0x46`. The
//! `hex` crate has a second entry point, `hex::encode_upper`, for the other case;
//! picking lowercase is a decision about which golden model to be checked against,
//! not a claim about what the RFC requires.
//!
//! # Cycles per byte, and why the byte is held in a register
//!
//! Two bits of state, eight bits of byte register, and a 16-entry ROM. The byte
//! register is not there for the pipeline's sake, it is there because the low nibble
//! has to be emitted a whole cycle *after* the high nibble, and after the accept edge
//! the input port is free to change. The alternative is to demand that the upstream
//! hold the byte stable across two cycles, which pushes the hold into someone else's
//! interface and makes the module unable to accept a new byte on the cycle it emits
//! the previous byte's low nibble -- exactly the cycle the design needs free in
//! order to run at one character per cycle.
//!
//! # Why the output port is called `ascii` and not `char`
//!
//! Because `char` is a C++ keyword, and the cosimulator's generated driver assigns every
//! output port as `dut-><name>`. A Verilog port named `char` is legal Verilog and the
//! emitter correctly leaves it alone, so the failure appears two crates later as a C++
//! compile error in generated code: `error: expected unqualified-id before 'char'`.
//! `ascii` is accurate -- the output is a code point, not a `char` -- and legal in both
//! languages. This is the same shape of defect as `byte`, which was missing from the
//! emitter's reserved-word list for the whole of A1 and A2; the difference is that `char`
//! is *not* a Verilog keyword, so no amount of extending that list would have caught it.
//!
//! Two bits of phase rather than one: the design accepts a byte on the idle cycle and
//! again on the cycle it emits the low nibble, which is what makes the two-cycles-per-
//! byte input rate possible. With a one-bit "busy" that is not expressible, because the
//! accept decision and the nibble select would both read the same bit.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The hex digits, indexed by nibble.
///
/// RFC 4648 s2 in its lowercase spelling, which is what `hex::encode` emits. The
/// index is the nibble's numeric value, so the table *is* the mapping and there is
/// no arithmetic in the design beyond the lookup.
pub const ALPHABET: [u8; 16] = *b"0123456789abcdef";

/// The number of characters one byte encodes to.
///
/// Two, always: hex has no padding, so a byte is always exactly [`DIGITS_PER_BYTE`]
/// characters and there is no length-dependent behaviour to get wrong. Stated as a
/// constant because the tests and the shift amounts in this module both need it.
pub const DIGITS_PER_BYTE: u32 = 2;

/// The idle phase: no byte latched and no character being emitted.
pub const IDLE: u64 = 0b00;

/// The phase in which the high nibble of the latched byte is emitted.
pub const HIGH: u64 = 0b01;

/// The phase in which the low nibble of the latched byte is emitted.
pub const LOW: u64 = 0b10;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge consumes at most one byte and produces at most one
    /// character.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which drops the latched byte and returns the design to
    /// [`IDLE`] so `valid` falls.
    pub rst: Signal,
    /// A byte is on `data` and may be latched on this edge.
    pub in_valid: Signal,
    /// The next byte to encode.
    #[bits(8)]
    pub data: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// `ascii` is a character of the output and not stale data.
    pub valid: Signal,
    /// The next character, as its ASCII code point. Meaningful only while `valid` is
    /// high. Not called `char`, which is a C++ keyword; see the module docs.
    #[bits(8)]
    pub ascii: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The output-valid flag, which is also the output.
    pub valid: Signal,
    /// The output character, which is also the output.
    pub ascii: Signal,
}

/// Builds a lowercase hex encoder into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say; naming them rather than leaving them
/// positional is what lets the emitted Verilog, the testbench and the cosimulator
/// agree.
///
/// `in_valid` is a real input rather than assumed, because a design that consumed a
/// byte on every edge would have to be fed at the design's own rate with no way to
/// stop it: the golden comparison drives a buffer of unknown length, and the
/// alternative is a test that pads the input with junk and hopes.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return. There is no hex-specific failure: the
/// alphabet is a compile-time table and the phase machine has no configuration, so a
/// different alphabet or a different case would be a different module.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    // The phase and the byte both have to be read by the logic that computes their
    // own next values, and a register cannot be an input to itself, so both are
    // wires driven from a register afterwards. Same reason, and same pattern, as
    // `crc3`'s state wire.
    let phase = design.wire(2)?;
    let latched = design.wire(8)?;

    let take = accepts(design, &phase, &inputs.in_valid)?;
    let next_phase = advance(design, &phase, &take)?;

    let phase_q = design.reg(
        &next_phase,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;
    design.drive(&phase, &phase_q)?;

    // The byte register holds unless this edge accepts one. `clear` is tied low so
    // the register updates on every edge; the `ite` is what makes "hold" the
    // alternative, and reset is a real input because a reset that cannot be
    // exercised is a reset that is not tested.
    let next_byte = design.ite(&take, &inputs.data, &latched)?;
    let byte_q = design.reg(
        &next_byte,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;
    design.drive(&latched, &byte_q)?;

    let ascii = encode(design, &byte_q, &phase_q)?;
    let valid = busy(design, &phase_q)?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            valid: valid.clone(),
            ascii: ascii.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        valid,
        ascii,
    })
}

/// Whether this edge latches a new byte.
///
/// One bit. The condition is phase-aware rather than just `in_valid`, because the
/// design accepts a byte on [`IDLE`] *and* on [`LOW`]: accepting on [`LOW`] is what
/// lets a byte be taken on the same cycle the previous byte's second character goes
/// out, which is what keeps `valid` high on every cycle once the pipeline is full.
///
/// The accept test is therefore "the phase's low bit is clear", which is true for
/// [`IDLE`] and [`LOW`] and false for [`HIGH`].
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn accepts(design: &Design, phase: &Signal, in_valid: &Signal) -> Result<Signal, BuildError> {
    let low = design.slice(phase, 0, 1)?;
    Ok(design.and(in_valid, &design.not(&low))?)
}

/// The next value of the phase register.
///
/// [`IDLE`] and [`LOW`] both go to [`HIGH`] if a byte arrives and back to [`IDLE`]
/// if one does not; [`HIGH`] always goes to [`LOW`]. Built as a `case_` because it
/// is a three-way choice on a literal-ranged signal, which is what `case_` is for.
///
/// The default arm is unreachable and is [`IDLE`] rather than something like a
/// sentinel, because the phase register can only ever hold `0b00`, `0b01` or `0b10`
/// -- `0b11` is never assigned -- and the IR requires a default for a case to have a
/// value at all. [`IDLE`] is the right choice for an unreachable arm because it is
/// the only one of the three that is self-consistent if it ever *were* reached.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn advance(design: &Design, phase: &Signal, take: &Signal) -> Result<Signal, BuildError> {
    let idle = design.lit(IDLE, 2)?;
    let high = design.lit(HIGH, 2)?;
    let low = design.lit(LOW, 2)?;
    let taken = design.ite(take, &high, &idle)?;
    Ok(design.case_(
        phase,
        &[(IDLE, taken.clone()), (HIGH, low), (LOW, taken)],
        &idle,
    )?)
}

/// Whether a character is being emitted this cycle, as `phase != IDLE`.
///
/// `valid` is `phase[0] or phase[1]`, which is one OR rather than a comparison,
/// because the phase register is two bits and only three of the four values are
/// reachable -- `IDLE` is the only zero, so the OR *is* the non-zero test and there
/// is no need for the unused `0b11` state to be excluded.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn busy(design: &Design, phase: &Signal) -> Result<Signal, BuildError> {
    let low = design.slice(phase, 0, 1)?;
    let high = design.slice(phase, 1, 1)?;
    Ok(design.or(&low, &high)?)
}

/// The character for the nibble this phase calls for.
///
/// `HIGH` selects bits 4..8 of the latched byte and everything else selects bits 0..4.
/// The idle phase therefore reads the *low* nibble of a stale byte, which is why the
/// selection is written as a single `ite` on the phase's low bit rather than a
/// three-way `case_`: only two of the three reachable phases have a defined answer
/// and the third is gated off by `valid`, so a case would be two arms and a default
/// to express one bit.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn encode(design: &Design, byte: &Signal, phase: &Signal) -> Result<Signal, BuildError> {
    let low = design.slice(byte, 0, 4)?;
    let high = design.slice(byte, 4, 4)?;
    let on_high = design.slice(phase, 0, 1)?;
    let nibble = design.ite(&on_high, &high, &low)?;
    nibble_to_ascii(design, &nibble)
}

/// The hex character for `nibble`: the 16-entry ROM.
///
/// One `case_` arm per [`ALPHABET`] entry, addressed by the nibble itself. A
/// `case_` and not a `design.mem` read, because a memory's contents are *written*
/// and a ROM's are not: there is no write port to attach, no enable to tie, and no
/// reset value to argue about. Sixteen literal arms say "this is a table" in a way
/// that a memory declaration cannot.
///
/// The default is [`ALPHABET`]'s first entry. It is unreachable -- `nibble` is four
/// bits and all sixteen values are enumerated above -- but a `case_` needs one, and
/// `Design::case_` will not build the node without it.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn nibble_to_ascii(design: &Design, nibble: &Signal) -> Result<Signal, BuildError> {
    let arms = ALPHABET
        .iter()
        .enumerate()
        .map(|(value, character)| {
            design
                .lit(u64::from(*character), 8)
                .map(|lit| (value as u64, lit))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let default = design.lit(u64::from(ALPHABET[0]), 8)?;
    Ok(design.case_(nibble, &arms, &default)?)
}
