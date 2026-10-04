//! The VCD rendering of a [`WaveData`].
//!
//! # What the standard actually allows
//!
//! VCD's value-change syntax has three scalar forms: a one-bit vector as a bare
//! `0`/`1`, a wider vector as a binary string (`b1010 !`), a real as `r1.5 !`,
//! and a string as `s"text" !`. There is **no** hex or decimal form. So a
//! `#[wave_format("hex")]` port is written as binary — because that is the only
//! vector encoding a viewer will read — and the *rendering* of it as hex lives in
//! [`WaveData::render`](crate::WaveData::render), where a test asserts on it. The
//! only thing the file does with the format is the `$var` range annotation, which
//! is a hint a viewer is free to ignore.
//!
//! Two further consequences of writing a real file rather than a pretty string:
//!
//! - **No timestamp line.** A wall-clock date would make two runs of the same
//!   design produce different bytes, which defeats a golden file. A `$comment`
//   says so in the file, so a reader is not left wondering why.
//! - **Identifiers are allocated, not derived from names.** VCD refers to a
//!   signal by an identifier code, and the *name* is documentation. Deriving the
//!   identifier from the name would mean two ports whose names legalise alike
//!   cannot both be dumped, so identifiers are positional: `!`, `"`, `#`, ...
//!
//! ```
//! use ferrite_lithic_bits::Bits;
//! use ferrite_lithic_wave::{Vcd, WaveData};
//!
//! let mut wave = WaveData::new("top");
//! let acc = wave.register("acc", 4).unwrap();
//! for (cycle, value) in [1u64, 2, 3].into_iter().enumerate() {
//!     wave.set(cycle as u64, &acc, &Bits::constant(value, 4).unwrap()).unwrap();
//! }
//!
//! let text = Vcd::new(&wave).render();
//! assert!(text.contains("$var wire 4 ! acc [3:0] $end"));
//! assert!(text.contains("b0001 !"), "{text}");
//! ```

use std::io::Write;

use ferrite_lithic::WaveFormat;
use ferrite_lithic_bits::Bits;

use crate::data::WaveData;
use crate::error::Error;

/// The default timescale, in the VCD's own units.
pub const DEFAULT_TIMESCALE: u32 = 1;

/// The characters VCD allows as an identifier, `!` through `~`.
const IDENTIFIER_ALPHABET: usize = 94;

/// A VCD rendering of a [`WaveData`].
///
/// Build one and write it:
///
/// ```no_run
/// use std::fs::File;
/// use ferrite_lithic_wave::{Vcd, WaveData};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let wave = WaveData::new("top");
/// let mut file = File::create("wave.vcd")?;
/// Vcd::new(&wave).write(&mut file)?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Vcd<'a> {
    wave: &'a WaveData,
    timescale: u32,
}

impl<'a> Vcd<'a> {
    /// Renders `wave` with the default timescale.
    #[must_use]
    pub const fn new(wave: &'a WaveData) -> Self {
        Self {
            wave,
            timescale: DEFAULT_TIMESCALE,
        }
    }

    /// The timescale, in VCD units.
    ///
    /// This scales the timestamps only. One recorded cycle is one `#` timestamp
    /// regardless, because a cycle is a discrete event in this waveform and
    /// inventing sub-cycle time for the clock would be claiming resolution the
    /// simulator does not have.
    #[must_use]
    pub const fn timescale(mut self, timescale: u32) -> Self {
        self.timescale = timescale;
        self
    }

    /// Writes the whole file.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the writer fails. The whole file is formatted first and
    /// written in one call, so a failure leaves either nothing or everything: a
    /// truncated VCD is a file a viewer will read as if it were complete.
    pub fn write<W: Write>(self, writer: &mut W) -> Result<(), Error> {
        writer.write_all(self.render().as_bytes())?;
        Ok(())
    }

    /// The whole file as text.
    ///
    /// Named `render` as well as `to_string` on purpose: an inherent `to_string`
    /// that takes `self` by value shadows [`Display`](core::fmt::Display), and a
    /// `&Vcd` would then format through neither. `Display` is the canonical path.
    #[must_use]
    pub fn render(self) -> String {
        let mut out = String::new();
        let wave = self.wave;
        let identifiers = wave.identifiers();

        out.push_str("$comment ferrite-lithic-wave\n");
        out.push_str(
            "$comment no timestamp line, so the same waveform always renders to \
             the same bytes\n",
        );
        out.push_str(&format!("$timescale {}ns $end\n", self.timescale));
        out.push_str(&format!("$scope module {} $end\n", wave.top()));
        for (index, port) in wave.ports().iter().enumerate() {
            out.push_str(&format!(
                "$var wire {} {} {}{} $end\n",
                port.width(),
                identifiers[index],
                port.name(),
                range(port.width(), port.format()),
            ));
        }
        out.push_str("$upscope $end\n");
        out.push_str("$enddefinitions $end\n");

        // `$dumpvars` carries the values at time zero. A viewer that opens the
        // file before the first timestamp would otherwise see every signal
        // uninitialised, and "no value yet" and "zero" are different facts.
        out.push_str("#0\n$dumpvars\n");
        for (index, port) in wave.ports().iter().enumerate() {
            let zero = Bits::zeros(port.width()).expect("a declared width is never zero");
            let value = wave.value(0, port).unwrap_or(&zero);
            out.push_str(&change(value, &identifiers[index]));
        }
        out.push_str("$end\n");

        // One timestamp per cycle, and only what changed: a VCD file is a change
        // log, so repeating an unchanged value would grow the file for nothing
        // and hide the cycles where something moved.
        for cycle in 1..wave.cycles() as u64 {
            let mut body = String::new();
            for (index, port) in wave.ports().iter().enumerate() {
                let Some(value) = wave.value(cycle, port) else {
                    continue;
                };
                let previous = wave.value(cycle - 1, port);
                if previous == Some(value) {
                    continue;
                }
                body.push_str(&change(value, &identifiers[index]));
            }
            if body.is_empty() {
                continue;
            }
            out.push_str(&format!("#{cycle}\n"));
            out.push_str(&body);
        }
        out
    }
}

impl core::fmt::Display for Vcd<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.render())
    }
}

/// The VCD identifier code for the `index`th signal.
///
/// One printable character for the first 94, then two, then three. VCD's
/// convention is most-significant first, so the codes sort in the same order as
/// the ports they stand for.
#[must_use]
pub fn identifier_for(index: usize) -> String {
    let mut width = 1;
    while index >= IDENTIFIER_ALPHABET.pow(width as u32) {
        width += 1;
    }
    let mut out = String::with_capacity(width);
    for place in (0..width).rev() {
        let digit = (index / IDENTIFIER_ALPHABET.pow(place as u32)) % IDENTIFIER_ALPHABET;
        out.push((33 + digit) as u8 as char);
    }
    out
}

/// One value-change line, without its newline.
///
/// A one-bit value is bare `0`/`1`, which is what the grammar wants; anything
/// wider is a binary string, which is the only vector form VCD has.
fn change(value: &Bits, identifier: &str) -> String {
    if value.width() == 1 {
        let digit = if value.bit(0).unwrap_or(false) {
            '1'
        } else {
            '0'
        };
        return format!("{digit}{identifier}\n");
    }
    format!("b{} {identifier}\n", value.to_binary_digits())
}

/// The bracketed range a `$var` declaration carries.
///
/// Hex gets hex bounds, because that is a viewer-facing hint and hex is what the
/// user asked to read. Decimal gets decimal bounds for the same reason. Neither
/// changes how the value is written.
fn range(width: u32, format: WaveFormat) -> String {
    // No range for a single bit: `[0:0]` is noise on a one-bit signal, and every
    // viewer reads a scalar without one.
    if width == 1 {
        return String::new();
    }
    let top = width - 1;
    match format {
        WaveFormat::Hex => format!(" [0x{top:x}:0x0]"),
        WaveFormat::Binary | WaveFormat::Decimal => format!(" [{top}:0]"),
        // `#[non_exhaustive]`: an unknown format gets the plain range, which is
        // the same range binary would get and is what a viewer can always read.
        _ => format!(" [{top}:0]"),
    }
}
