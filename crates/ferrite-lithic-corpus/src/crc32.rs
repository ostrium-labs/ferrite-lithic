//! CRC-32/ISO-HDLC: a 32-bit LFSR, one byte per cycle.
//!
//! # The same shape as [`crate::crc3`], thirty-two times wider
//!
//! This is the algorithm behind zlib, PNG and Ethernet, and it is the reason CRC
//! hardware is a commodity: 32 flops, a shift, and two conditional XORs, consuming a
//! whole byte per clock. The software version everyone uses (`crc32fast`) is a table
//! of 256 `u32`s — 8 kB of lookup that a hardware design simply does not have, which
//! is why an ASIC gets a byte per cycle from logic that a CPU cannot afford to
//! simulate at all.
//!
//! It is `crc3` with a 32-bit register instead of a 3-bit one, and the recurrence is
//! shared: see [`crate::lfsr`] for the reflection and the tap, and why the tap is
//! `0xedb88320` rather than the catalogue's `0x04c11db7`.
//!
//! # Parameters
//!
//! CRC-32/ISO-HDLC as the RevEng catalogue defines it, which is what
//! `crc32fast::Hasher` implements:
//!
//! | parameter | value |
//! |---|---|
//! | width | 32 |
//! | poly | `0x04c11db7` |
//! | init | `0xffffffff` |
//! | refin | true |
//! | refout | true |
//! | xorout | `0xffffffff` |
//! | check | `0xcbf43926` |
//!
//! **Init and xorout are all ones**, which is the whole reason this CRC is not just
//! `crc3` with a bigger register. A CRC whose init and xorout are zero is the
//! remainder of dividing the message by the polynomial and composes across a
//! concatenation; one whose init is all ones does not compose, and that is precisely
//! what a hardware CRC wants — the transmitted message can simply have the CRC
//! appended and the receiver runs the identical register over message-then-CRC to get
//! zero. So `init` and `xorout` here are not decoration, they are the property that
//! makes the error check work at line rate.
//!
//! # Why both `state` and `crc` come out
//!
//! `state` is the raw register, which is what a receiver XORs the received word into
//! and feeds back in to get zero. `crc` is `state ^ xorout`, the value software
//! prints. Both are outputs because they are two different answers to two different
//! questions, and a port list that only had the first would make every caller apply
//! the mask itself.
//!
//! # `init`, and why it is a port
//!
//! A CRC has to start from `0xffffffff`, not from whatever the register happened to
//! hold, and on a bus that happens every packet. So `init` is an input: asserted, it
//! loads the init value on the next edge and *discards the byte that edge carries*.
//! That is not an implementation detail to paper over — the byte is genuinely
//! consumed by the stream and a receiver will never know it was dropped — so it is
//! stated here and asserted in the tests rather than left for a caller to discover.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;
use crate::lfsr::{reflected_lfsr, reflected_tap};

/// The catalogue polynomial, in forward form.
pub const POLY: u64 = 0x04c1_1db7;

/// The register's init value.
pub const INIT: u64 = 0xffff_ffff;

/// The value XORed in at the output.
pub const XOROUT: u64 = 0xffff_ffff;

/// The catalogue's check value: the register after `"123456789"`, with xorout applied.
pub const CHECK: u64 = 0xcbf4_3926;

/// The reversed polynomial, which is what the register actually XORs in.
pub const TAP: u64 = 0xedb8_8320;

/// The register width.
pub const WIDTH: u32 = 32;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge consumes one byte.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which loads [`INIT`].
    pub rst: Signal,
    /// Load [`INIT`] on this edge, discarding the byte that edge carries.
    pub init: Signal,
    /// The next byte of the message.
    #[bits(8)]
    pub byte: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The raw register, which is what a receiver feeds back to get zero.
    #[bits(32)]
    pub state: Signal,
    /// `state` with `xorout` applied: the value software prints.
    #[bits(32)]
    pub crc: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The raw register, also the source of `crc`.
    pub state: Signal,
}

/// Builds a CRC-32/ISO-HDLC design into `design`.
///
/// # Errors
///
/// Whatever [`Design`] returns, or [`BuildError`] from the port-list helpers.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let state = design.wire(WIDTH)?;
    let advanced = reflected_lfsr(design, &state, &inputs.byte, TAP, WIDTH)?;

    // `init` and `rst` both load the init value, so a mux rather than the register's
    // reset port alone: the register's own reset is also `init`, and a design that
    // can only be initialised by a reset cannot start a second packet.
    let init_value = design.lit(INIT, WIDTH)?;
    let reloading = design.or(&inputs.rst, &inputs.init)?;
    let next = design.ite(&reloading, &init_value, &advanced)?;

    let held = design.reg(
        &next,
        &inputs.clk,
        &design.constant(false),
        &design.constant(false),
    )?;
    design.drive(&state, &held)?;

    let crc = design.xor(&held, &design.lit(XOROUT, WIDTH)?)?;
    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            state: held.clone(),
            crc,
        },
    )?;

    Ok(Ports {
        inputs,
        state: held,
    })
}

/// The tap, derived from the catalogue polynomial rather than written down.
///
/// # Panics
///
/// Never. [`reflected_tap`] is total.
#[must_use]
pub fn tap() -> u64 {
    reflected_tap(POLY, WIDTH)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{TAP, tap};

    #[test]
    fn the_tap_is_the_reversed_polynomial_not_the_polynomial() {
        assert_eq!(tap(), TAP);
        assert_ne!(
            tap(),
            super::POLY,
            "a CRC with the forward polynomial is a different CRC that looks identical"
        );
    }
}
