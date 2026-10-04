//! The stimulus both backends are driven with.
//!
//! # One list, two backends
//!
//! A stimulus is a cycle-indexed list of input values, in input-port order. It
//! is the *only* thing that crosses between the simulator and the emitted
//! Verilog: the harness reads this file, the simulator is fed from this list, and
//! any disagreement downstream is therefore about the design rather than about
//! two different input sequences.
//!
//! # The file format
//!
//! A decimal cycle count on the first line, then whitespace-separated hex words,
//! one per input port per cycle, with no other separators. Not because a terse
//! format is nicer to read but because it has no failure modes: a `fscanf("%s")`
//! in the generated C++ cannot mis-parse it, and the Rust side knows each word's
//! width from the port list, so a value that does not fit is caught before the file
//! is written rather than silently truncated by a reader that stopped caring.
//!
//! The count is not decoration. Without it the harness's loop has to end at
//! end-of-file, which cannot express a module with no input ports at all — a
//! counter, or an LFSR with no data in — and a file whose contents disagree with
//! its length is then indistinguishable from one that simply ended. The count
//! makes both of those an error.
//!
//! The clock is *not* one of the columns, for the reason `Plan::new` gives.

use ferrite_lithic_bits::Bits;

use crate::error::Error;

/// Cycle-indexed input values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Stimulus {
    rows: Vec<Vec<Bits>>,
}

impl Stimulus {
    /// An empty stimulus.
    #[must_use]
    pub const fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Appends one cycle's inputs, in input-port order.
    ///
    /// # Errors
    ///
    /// [`Error::Arity`] if this cycle has a different number of values than the
    /// first, and [`Error::Width`] if a value is not its port's width. Both are
    /// refused here rather than at the point of use, because a stimulus whose
    /// rows disagree is a stimulus where "cycle 5" means two different things.
    pub fn push(&mut self, inputs: Vec<Bits>) -> Result<(), Error> {
        if let Some(first) = self.rows.first()
            && first.len() != inputs.len()
        {
            return Err(Error::Arity {
                cycle: self.rows.len() as u64,
                expected: first.len(),
                got: inputs.len(),
            });
        }
        self.rows.push(inputs);
        Ok(())
    }

    /// How many cycles are in the stimulus.
    #[must_use]
    pub fn cycles(&self) -> usize {
        self.rows.len()
    }

    /// One cycle's values.
    #[must_use]
    pub fn cycle(&self, cycle: u64) -> Option<&[Bits]> {
        self.rows.get(cycle as usize).map(Vec::as_slice)
    }

    /// The stimulus as the harness's input: a cycle count, then hex words.
    ///
    /// # Errors
    ///
    /// [`Error::Width`] if a value is not the width of the input port at its
    /// position. The check is repeated here because this is the boundary where a
    /// value becomes text.
    ///
    /// The first line is the number of cycles. `widths` must not include the
    /// clock: the clock is a port but both backends drive it themselves, and a
    /// column for it would mean a word that the generated driver applies and then
    /// overwrites the clock with.
    pub fn encode(&self, widths: &[u32]) -> Result<String, Error> {
        let mut out = format!("{}\n", self.rows.len());
        for (cycle, row) in self.rows.iter().enumerate() {
            if row.len() != widths.len() {
                return Err(Error::Arity {
                    cycle: cycle as u64,
                    expected: widths.len(),
                    got: row.len(),
                });
            }
            for (index, value) in row.iter().enumerate() {
                if value.width() != widths[index] {
                    return Err(Error::Width {
                        cycle: cycle as u64,
                        port: index,
                        expected: widths[index],
                        got: value.width(),
                    });
                }
                out.push_str(&hex(value));
                out.push(if index + 1 == row.len() { '\n' } else { ' ' });
            }
        }
        Ok(out)
    }
}

/// One value as a fixed-width hex word, no prefix.
///
/// Fixed width because the reader knows how many nibbles to expect, and a
/// variable-width word would make `0x1` and `0x01` two spellings of the same
/// value in a file whose whole point is being unambiguous.
#[must_use]
pub fn hex(value: &Bits) -> String {
    let digits = value.width().div_ceil(4) as usize;
    let text = value.to_hex();
    let trimmed = text.trim_start_matches("0x");
    if trimmed.len() >= digits {
        return trimmed.to_string();
    }
    format!("{:0>width$}", trimmed, width = digits)
}

/// Parses one hex word back into a value of `width` bits.
///
/// The inverse of [`hex`], and separate from it because a round trip through
/// text is exactly where a width bug would hide: a value that came back a
/// different width would make every comparison downstream fail for a reason that
/// has nothing to do with the design.
///
/// # Errors
///
/// [`Error::HexWord`] if the word is not hex, and [`Error::Width`] if it does
/// not fit the width.
pub fn parse_hex(word: &str, width: u32) -> Result<Bits, Error> {
    let digits = word.trim();
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::HexWord {
            word: word.to_string(),
        });
    }
    // The nibble count is checked *before* parsing. `Bits::constant` truncates a
    // value that does not fit, by design, so parsing first would turn an
    // over-wide word into a plausible wrong answer rather than an error.
    let expected_digits = width.div_ceil(4) as usize;
    if digits.len() > expected_digits {
        return Err(Error::Width {
            cycle: 0,
            port: 0,
            expected: width,
            got: digits.len() as u32 * 4,
        });
    }
    let value = u64::from_str_radix(digits, 16).map_err(|_| Error::HexWord {
        word: word.to_string(),
    })?;
    Bits::constant(value, width).map_err(|_| Error::HexWord {
        word: word.to_string(),
    })
}
