//! Base64: six bits in, one ASCII character out, and the `=` padding that goes
//! with it.
//!
//! # Why this shape
//!
//! The interesting object in base64 is not a pipeline, it is **the alphabet**. Base64
//! is defined as a remapping of 6-bit groups onto 64 printable ASCII characters
//! (RFC 4648 s4), and that table is the whole algorithm. There is no arithmetic to
//! unroll and no serial dependency to break: the character for a sextet depends only
//! on that sextet. So this design is [`crc3`](crate::crc3) again in the sense that the
//! interesting hardware is a lookup, and in this case the lookup is genuinely a
//! **64-entry ROM** rather than the three XOR taps `crc3` gets away with.
//!
//! The table is not affine in the index, so it cannot be gates. Concretely: it is four
//! runs of consecutive code points -- `A`-`Z`, `a`-`z`, `0`-`9`, then `+` and `/` -- and
//! the gaps between the runs are 6, 6 and 0, so no single add or XOR of the index
//! produces it. A 64-entry `case_` is the honest expression, and it is what a
//! synthesizer will turn into a 64x8 ROM.
//!
//! A good demonstration of *why* the alphabet has to be a table is that the entire
//! difference between standard base64 and the URL-safe variant is two entries: `+`
//! becomes `-` and `/` becomes `_`. There is no structure to those two entries
//! relative to the other sixty-two, which is precisely the argument that the table is
//! the design. This module implements the standard alphabet, so it matches the
//! `base64` crate's `engine::general_purpose::STANDARD`.
//!
//! # Why `=` is not in the alphabet, and where it comes from anyway
//!
//! A 6-bit index has exactly 64 values and the alphabet has exactly 64 characters, so
//! **no sextet maps to `=`** -- the padding character is not a 65th symbol, it is
//! *framing*: RFC 4648 s4 pads the output with `=` up to a multiple of four
//! characters. That is a statement about the length of the message, not about any
//! input group.
//!
//! The padding count is nonetheless derivable from something the hardware already has.
//! If a message is `n` sextets long, the number of `=` characters is
//! `(4 - (n mod 4)) mod 4`, which in two-bit arithmetic is the **two's-complement
//! negation of `n mod 4`**. So the design carries a two-bit counter of accepted
//! sextets, and the padding count falls out of it: one adder, one subtractor, no
//! sideband, and nothing to be told.
//!
//! That is a deliberate choice against a `pad` input. An encoder handed its own padding
//! count as a sideband is trusting something outside itself, and every bug in the layer
//! that computes the sideband becomes a bug in this module's output that no test of this
//! module can distinguish from its own. The `n mod 4 == 1` case is unreachable for real
//! base64 -- a message whose bit count leaves one sextet and three padding characters
//! does not exist, since `n mod 4` is always 0, 2 or 3 -- and it is *defined* rather
//! than left as a hole: two's-complement negation gives 3, which is the coherent answer
//! for a lone sextet followed by three `=`.
//!
//! # Rates, cycles per character, and flops
//!
//! Unlike [`hex`](crate::hex), where the 8:4 split forces the output to run at half the
//! input rate, base64 is **rate-balanced**: one sextet in per cycle and one character
//! out per cycle, with no queue between them. That is only true if the character is
//! *registered*. A combinational character fed from a latched sextet costs the design an
//! extra cycle per group -- the sextet has to be latched on one edge and read on the
//! next, and the cycle in between cannot accept anything -- which halves the output rate
//! for no benefit. So the output character is a register, and the accept condition is
//! "nothing is queued for padding", which is true again on the very cycle the last `=`
//! leaves.
//!
//! Thirteen flops: eight for the output character, one for its valid flag, six for the
//! latched sextet and two each for the sextet counter and the padding countdown. One
//! cycle of latency. The alternative -- a byte-wide input with an internal bit packer --
//! would be a byte per three cycles out, strictly worse, and would put a second
//! variable-width shift register in the design for no benefit.
//!
//! # Why the output port is called `ascii` and not `char`
//!
//! Because `char` is a C++ keyword, and the cosimulator's generated driver assigns
//! every output port as `dut-><name>`. A Verilog port named `char` is legal Verilog and
//! the emitter correctly leaves it alone, so the failure appears two crates later as a
//! C++ compile error in generated code: `error: expected unqualified-id before 'char'`.
//! The port is named `ascii`, which is accurate -- the output is an ASCII code point,
//! not a `char` -- and legal in both languages. This is the same shape of defect as
//! `byte`, which was missing from the emitter's reserved-word list for the whole of A1
//! and A2; the difference is that `char` is *not* a Verilog keyword, so no amount of
//! extending that list would have caught it.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The base64 alphabet of RFC 4648 s4, indexed by sextet.
///
/// `A`-`Z`, `a`-`z`, `0`-`9`, then `+` and `/` -- the standard, *not* the URL-safe
/// variant, which is the same table with entries 62 and 63 replaced by `-` and `_`.
pub const ALPHABET: [u8; 64] = *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The padding character, which is not part of [`ALPHABET`].
///
/// RFC 4648 s4: the output is padded to a multiple of four characters with `=`. Every
/// 6-bit index is already spoken for by [`ALPHABET`], so this character is selected by
/// the framing logic rather than by a sextet.
pub const PADDING: u8 = b'=';

/// The value [`sextet_to_ascii`] gives a sextet it was never asked about.
///
/// Unreachable: the address is six bits and all sixty-four are enumerated. It exists
/// because a `case_` needs a default and [`Design::case_`] will not build the node
/// without one. It is *not* what `ascii` reads on an idle cycle -- the output register
/// simply holds its last value there, and reset clears it to zero -- so nothing in the
/// design or its tests depends on this byte being visible.
pub const IDLE_CHARACTER: u8 = b'0';

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge accepts at most one sextet.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which empties the padding countdown and the sextet counter
    /// and so drops `valid`.
    pub rst: Signal,
    /// A sextet is on `sextet` and may be accepted on this edge.
    ///
    /// The design has no backpressure: a sextet offered while it is still emitting
    /// padding is **dropped**, not queued. The contract is that the upstream offers one
    /// sextet per cycle and then stops, which for a streaming encoder means stopping
    /// after the last one -- the drain takes at most three further cycles and no new
    /// sextets arrive during them.
    pub in_valid: Signal,
    /// This is the last sextet of the message, so the padding for it follows.
    pub in_last: Signal,
    /// The next sextet, six bits, most significant bit first as it appears in the
    /// encoded text.
    #[bits(6)]
    pub sextet: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// `ascii` is a character of the encoding and not stale data.
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

/// Builds a base64 encoder into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say; naming them rather than leaving them
/// positional is what lets the emitted Verilog, the testbench and the cosimulator
/// agree.
///
/// `in_last` exists so the design can emit [`PADDING`] rather than being told how much
/// padding to emit: the sextet counter already holds the only fact the count depends on,
/// and see the module docs for why that is not a sideband.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and whatever
/// the port-list helpers return. There is no base64-specific failure: the alphabet is a
/// compile-time table and the counter has no configuration, so a different alphabet
/// would be a different module.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    // The five state elements: the padding countdown, the sextet counter, the latched
    // sextet, the output character and the output-valid flag. Each is read by the logic
    // that computes its own next value, and a register cannot be an input to itself, so
    // each is a wire driven from its register afterwards. Same reason, and same pattern,
    // as `crc3`'s state wire.
    let pads = design.wire(2)?;
    let count = design.wire(2)?;
    let latched = design.wire(6)?;
    let emitted = design.wire(8)?;
    let live = design.wire(1)?;

    let accept = accepts(design, &pads, &inputs.in_valid)?;
    let queued = queue(design, &count, &inputs.in_last, &accept)?;
    let next_pads = advance(design, &pads, &queued, &accept)?;

    let increment = design.add(&count, &design.lit(1, 2)?)?;
    let next_count = design.ite(&accept, &increment, &count)?;
    let next_sextet = design.ite(&accept, &inputs.sextet, &latched)?;

    // The output register is loaded on every cycle that produces a character and held
    // otherwise, which is what keeps the input and the output rates equal: `accept` is
    // `pads == 0`, and that is true again on the very cycle the last `=` leaves.
    //
    // The ROM is addressed by `inputs.sextet`, *not* by `latched`. This is the next value
    // of the character register, and on an accepting edge the sextet being encoded is
    // still on the input port -- `latched` updates at this very edge, so the wire reads
    // the *previous* group. Passing the wire here instead emits every group one late.
    let next_character = encode(design, &inputs.sextet, &emitted, &pads, &accept)?;
    let next_live = design.or(&accept, &draining(design, &pads)?)?;

    let mut regs = Vec::with_capacity(5);
    for (wire, data) in [
        (&pads, next_pads),
        (&count, next_count),
        (&latched, next_sextet),
        (&emitted, next_character),
        (&live, next_live),
    ] {
        let reg = design.reg(&data, &inputs.clk, &inputs.rst, &design.constant(false))?;
        design.drive(wire, &reg)?;
        regs.push(reg);
    }
    // The two output registers, in the order the loop above registered them.
    let ascii = regs[3].clone();
    let valid = regs[4].clone();

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

/// Whether this edge accepts a new sextet.
///
/// One bit: `in_valid` and nothing queued for padding. There is deliberately no
/// `ready` output -- a streaming encoder whose only consumer is the next stage in a
/// pipe does not need to report back-pressure, and an encoder that *did* would have to
/// define what happens to a sextet offered during the drain.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn accepts(design: &Design, pads: &Signal, in_valid: &Signal) -> Result<Signal, BuildError> {
    Ok(design.and(in_valid, &design.not(&draining(design, pads)?))?)
}

/// Whether padding characters are still queued, as `pads != 0`.
///
/// The two bits are ORed rather than compared: zero is the only value that means
/// "nothing queued", so the OR is the test and there is nothing for a comparison to add.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn draining(design: &Design, pads: &Signal) -> Result<Signal, BuildError> {
    let low = design.slice(pads, 0, 1)?;
    let high = design.slice(pads, 1, 1)?;
    Ok(design.or(&low, &high)?)
}

/// The number of `=` characters to queue when a sextet is accepted.
///
/// `count` is the number of sextets accepted *before* this edge, so this is the padding
/// for the sextet being accepted now: `(4 - (count + 1) mod 4) mod 4`. In two-bit
/// arithmetic that is the two's-complement negation of `count + 1`, which is why this is
/// a negate rather than a three-way `case_` over the four possible counts.
///
/// Queued only when the sextet being accepted is also the last one; otherwise zero,
/// because a message that is not finished yet does not know its own length.
///
/// | sextets, mod 4 | padding |
/// |---|---|
/// | 0 | 0 |
/// | 1 | 3 (unreachable for real base64; see the module docs) |
/// | 2 | 2 |
/// | 3 | 1 |
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn queue(
    design: &Design,
    count: &Signal,
    in_last: &Signal,
    accept: &Signal,
) -> Result<Signal, BuildError> {
    let including_this_one = design.add(count, &design.lit(1, 2)?)?;
    let pad = design.sub(&design.lit(0, 2)?, &including_this_one)?;
    let closing = design.ite(&design.and(accept, in_last)?, &pad, &design.lit(0, 2)?)?;
    Ok(closing)
}

/// The next value of the padding countdown.
///
/// Two bits, counting `=` characters still to emit. An accept loads the queued count;
/// otherwise a non-empty countdown decrements, because this cycle emits one `=` and the
/// rest are for the following cycles; otherwise it stays zero.
///
/// The decrement is guarded on *non-zero* rather than trusting `pads` to be at least one.
/// `Design::sub` truncates to the operands' width, so `0 - 1` in two bits is `0b11`,
/// not `-1`, and a countdown that wrapped would emit three spurious `=` characters.
/// Guarding makes the unreachable case safe rather than merely unlikely.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn advance(
    design: &Design,
    pads: &Signal,
    queued: &Signal,
    accept: &Signal,
) -> Result<Signal, BuildError> {
    let one = design.lit(1, 2)?;
    let decremented = design.ite(
        &draining(design, pads)?,
        &design.sub(pads, &one)?,
        &design.lit(0, 2)?,
    )?;
    Ok(design.ite(accept, queued, &decremented)?)
}

/// The character loaded into the output register this cycle.
///
/// Three cases, in priority order: a sextet accepted on this edge goes through the
/// alphabet ROM; otherwise anything still queued is a [`PADDING`] character; otherwise
/// the register holds, because there is no character to produce.
///
/// The first case is an `ite` on `accept` rather than on the countdown, and the
/// distinction matters: on the accept edge the countdown has *already* been loaded with
/// the padding count, so testing the countdown would emit a `=` in place of the sextet
/// that was just offered.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn encode(
    design: &Design,
    sextet: &Signal,
    held: &Signal,
    pads: &Signal,
    accept: &Signal,
) -> Result<Signal, BuildError> {
    // `sextet` is the *input port*: this is the value being loaded into the output
    // register, not the latched copy, which on an accepting edge is still the previous
    // group.
    let character = sextet_to_ascii(design, sextet)?;
    let pad = design.lit(u64::from(PADDING), 8)?;
    let queued = design.ite(&draining(design, pads)?, &pad, held)?;
    Ok(design.ite(accept, &character, &queued)?)
}

/// The base64 character for `sextet`: the 64-entry ROM.
///
/// One `case_` arm per [`ALPHABET`] entry, addressed by the sextet itself. A `case_` and
/// not a `design.mem` read, because a memory's contents are *written* and a ROM's are
/// not: there is no write port to attach, no enable to tie, and no reset value to argue
/// about. Sixty-four literal arms say "this is a table" in a way that a memory
/// declaration cannot.
///
/// The default is [`IDLE_CHARACTER`]. It is unreachable -- `sextet` is six bits and all
/// sixty-four values are enumerated above -- but a `case_` needs one, and
/// [`Design::case_`] will not build the node without it.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn sextet_to_ascii(design: &Design, sextet: &Signal) -> Result<Signal, BuildError> {
    let arms = ALPHABET
        .iter()
        .enumerate()
        .map(|(value, character)| {
            design
                .lit(u64::from(*character), 8)
                .map(|lit| (value as u64, lit))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let default = design.lit(u64::from(IDLE_CHARACTER), 8)?;
    Ok(design.case_(sextet, &arms, &default)?)
}
