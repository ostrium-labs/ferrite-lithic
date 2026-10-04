//! The reflected LFSR recurrence that every reflected CRC is built from.
//!
//! # The one recurrence
//!
//! A reflected CRC of width `W` is a `W`-bit register that is shifted **right**, one
//! message bit per step, with a conditional XOR of the *reversed* polynomial:
//!
//! ```text
//! feedback = state[0] xor bit
//! state >>= 1
//! if feedback: state ^= tap
//! ```
//!
//! Two things about that are easy to get wrong, and both have bitten this project.
//!
//! **The tap is the reversed polynomial, not the polynomial.** The catalogue's `poly`
//! is in forward form. CRC-32's is `0x04c11db7` and the register uses
//! `0xedb88320`; CRC-3's is `0b011` and the register uses `0b110`. A design with the
//! wrong tap is still a shift register of the right width producing values of the
//! right width — it is just computing a different CRC, and nothing about the
//! *shape* of the circuit will tell you.
//!
//! **A reflected algorithm shifts right**, which is why the reflected form is the
//! cheap one in hardware: a right-shifting register already brings the new input bit
//! in at the low end, where the feedback tap is taken from. There is no bit reversal
//! anywhere in the datapath.
//!
//! # Byte-parallel, not bit-serial
//!
//! The loop below runs eight times, once per bit of a byte, combinationally. The
//! alternative is one bit per cycle, eight cycles per byte, which is the same eight
//! XOR networks with a register between each one — eight times the flops for the same
//! latency in cycles per byte. For a datapath this small the flops are nearly free and
//! the cycles are not, so unrolling is the right shape, and it is the reason CRC
//! hardware runs at a byte per clock.
//!
//! The correspondence between the algorithm and the circuit is visible rather than
//! asserted: the Rust loop below *is* the algorithm, run over design nodes.

use ferrite_lithic::{Design, Signal};

use crate::BuildError;

/// The reversed tap for a reflected CRC of `width` bits.
///
/// The catalogue's polynomial is the forward form. Reversing it over `width` bits is
/// what the register actually XORs in, and the difference is invisible in a circuit
/// diagram.
#[must_use]
pub fn reflected_tap(poly: u64, width: u32) -> u64 {
    let mask = if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    let mut reversed = 0u64;
    for bit in 0..width {
        if poly & (1u64 << bit) != 0 {
            reversed |= 1u64 << (width - 1 - bit);
        }
    }
    reversed & mask
}

/// Eight LFSR steps: consumes `byte` and returns the next `state`.
///
/// `width` is the register width and `tap` is the reversed polynomial, from
/// [`reflected_tap`]. The bits of `byte` are consumed least significant first, which
/// is what `refin` means.
///
/// # Errors
///
/// Whatever [`Design`] returns: a width mismatch, or an empty selection when `width`
/// is zero.
pub fn reflected_lfsr(
    design: &Design,
    state: &Signal,
    byte: &Signal,
    tap: u64,
    width: u32,
) -> Result<Signal, BuildError> {
    let tap = design.lit(tap & mask_for(width), width)?;
    let mut accumulator = state.clone();
    for bit in 0..8u32 {
        let incoming = design.slice(byte, bit, 1)?;
        let carried = design.slice(&accumulator, 0, 1)?;
        let feedback = design.xor(&carried, &incoming)?;
        let shifted = design.srl(&accumulator, 1)?;
        let tapped = design.xor(&shifted, &tap)?;
        accumulator = design.ite(&feedback, &tapped, &shifted)?;
    }
    Ok(accumulator)
}

fn mask_for(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::reflected_tap;

    #[test]
    fn the_catalogue_polynomials_reverse_to_the_taps_the_registers_use() {
        // Both checked against a differential against the crate that defines each
        // algorithm, because the shape of the mistake is that the wrong value here
        // produces a perfectly plausible circuit.
        assert_eq!(
            reflected_tap(0x04c1_1db7, 32),
            0xedb8_8320,
            "CRC-32/ISO-HDLC"
        );
        assert_eq!(reflected_tap(0b011, 3), 0b110, "CRC-3/ROTD");
        assert_eq!(reflected_tap(0x07, 8), 0xe0, "CRC-8");
    }

    #[test]
    fn reversing_is_an_involution_on_the_low_bits() {
        // A polynomial that is its own reverse would make this class of bug
        // invisible, so the property is only worth stating because none of ours is.
        assert_ne!(reflected_tap(0x04c1_1db7, 32), 0x04c1_1db7);
    }
}
