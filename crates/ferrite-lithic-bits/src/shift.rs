//! Slicing, concatenation and the three structural shifts.
//!
//! # Why there is no shift operator
//!
//! Hardcaml has **no shift primitive** in the signal graph, the simulator, or the
//! RTL backends. All three shifts desugar into `select` plus `cat` plus a constant
//! ([`kernel/comb.ml:913-947`](https://github.com/jane-street/hardcaml)), and the
//! shift amount is a compile-time OCaml `int`, not a signal. A *variable* shift is
//! a separate `log_shift`, a chain of two-input muxes
//! ([`kernel/comb.ml:973-982`](https://github.com/jane-street/hardcaml)).
//!
//! We keep that. Adding a shift node would make constant folding in the simulator
//! faster, but it would change the emitted Verilog away from the golden fixtures,
//! and shifters are hardware we would rather leave to the synthesiser than bake
//! into the IR. Everything below is therefore expressed in terms of
//! [`select`](Bits::select) and [`cat`](cat), which is also why those two are
//! correct before anything else in this crate is.

use crate::bits::{Bits, extract_u64, set_bits_u64, words_for_width};
use crate::error::{Error, Op};

impl Bits {
    /// Extracts `len` bits starting at `offset` from the least significant end.
    ///
    /// The result is `len` bits wide. This is the only way to narrow a value
    /// lossily, which is what makes truncation in [`mul`](Bits::mul) explicit.
    ///
    /// # Errors
    ///
    /// [`Error::SelectEmpty`] if `len == 0`, or
    /// [`Error::SelectOutOfRange`] if `offset + len` exceeds the width.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let v = Bits::constant(0b1101_0110, 8).unwrap();
    /// assert_eq!(v.select(4, 4).unwrap().to_u64().unwrap(), 0b1101);
    /// assert_eq!(v.select(0, 1).unwrap().to_u64().unwrap(), 0);
    /// assert!(v.select(6, 4).is_err());
    /// assert!(v.select(0, 0).is_err());
    /// ```
    pub fn select(&self, offset: u32, len: u32) -> Result<Self, Error> {
        self.check_invariant();
        if len == 0 {
            return Err(Error::SelectEmpty { width: self.width });
        }
        let end = u64::from(offset) + u64::from(len);
        if end > u64::from(self.width) {
            return Err(Error::SelectOutOfRange {
                width: self.width,
                offset,
                len,
            });
        }
        let out_words = words_for_width(len);
        let mut out = vec![0u64; out_words];
        for (i, chunk) in out.iter_mut().enumerate() {
            let start = u64::from(offset) + (i as u64) * 64;
            // `i < out_words` guarantees `start < end`, so `n` is at least 1.
            let n = ((start + 64).min(end) - start) as u32;
            *chunk = extract_u64(&self.words, start, n);
        }
        Ok(Bits::from_words(len, out))
    }

    /// Shifts left by `by` bits, filling with zeros at the bottom.
    ///
    /// Structural: `cat(select(0, width - by), zeros(by))`.
    ///
    /// A shift of at least the width yields all zeros.
    ///
    /// # Errors
    ///
    /// Never; provided so `sll` has the same shape as the other shifts.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let v = Bits::constant(0b101, 4).unwrap();
    /// assert_eq!(v.sll(1).unwrap().to_u64().unwrap(), 0b1010);
    /// assert_eq!(v.sll(4).unwrap().to_u64().unwrap(), 0);
    /// ```
    pub fn sll(&self, by: u32) -> Result<Self, Error> {
        self.check_invariant();
        if by == 0 {
            return Ok(self.clone());
        }
        if by >= self.width {
            return Bits::zeros(self.width);
        }
        let kept = self.select(0, self.width - by)?;
        let zeros = Bits::zeros(by)?;
        cat(&[&kept, &zeros])
    }

    /// Shifts right by `by` bits, filling with zeros at the top.
    ///
    /// Structural: `cat(zeros(by), select(by, width - by))`.
    ///
    /// A shift of at least the width yields all zeros.
    ///
    /// # Errors
    ///
    /// Never; provided so `srl` has the same shape as the other shifts.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let v = Bits::constant(0b1010, 4).unwrap();
    /// assert_eq!(v.srl(1).unwrap().to_u64().unwrap(), 0b0101);
    /// assert_eq!(v.srl(9).unwrap().to_u64().unwrap(), 0);
    /// ```
    pub fn srl(&self, by: u32) -> Result<Self, Error> {
        self.check_invariant();
        if by == 0 {
            return Ok(self.clone());
        }
        if by >= self.width {
            return Bits::zeros(self.width);
        }
        let kept = self.select(by, self.width - by)?;
        let zeros = Bits::zeros(by)?;
        cat(&[&zeros, &kept])
    }

    /// Shifts right arithmetically by `by` bits, replicating the sign bit.
    ///
    /// Structural: `cat(sign_fill(by), select(by, width - by))`, where
    /// `sign_fill` is all ones when the value is negative and all zeros when it is
    /// not.
    ///
    /// A shift of at least the width saturates to the sign bit.
    ///
    /// # Errors
    ///
    /// Never; provided so `sra` has the same shape as the other shifts.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// // -4 in 4 bits, arithmetic shift right by 1 gives -2
    /// let v = Bits::constant(0b1100, 4).unwrap();
    /// assert_eq!(v.sra(1).unwrap().to_u64().unwrap(), 0b1110);
    /// assert_eq!(v.sra(4).unwrap().to_u64().unwrap(), 0b1111);
    /// ```
    pub fn sra(&self, by: u32) -> Result<Self, Error> {
        self.check_invariant();
        if by == 0 {
            return Ok(self.clone());
        }
        if by >= self.width {
            // Saturate to the sign bit.
            return if self.is_negative() {
                Bits::ones(self.width)
            } else {
                Bits::zeros(self.width)
            };
        }
        let fill = if self.is_negative() {
            Bits::ones(by)?
        } else {
            Bits::zeros(by)?
        };
        // Note the asymmetry with srl's naming: here the retained part is the
        // *upper* width - by bits, at offset `by`, because the sign fill replaces
        // everything the shift would otherwise have shifted in.
        let kept = self.select(by, self.width - by)?;
        cat(&[&fill, &kept])
    }
}

/// Concatenates values, most significant part first.
///
/// The result width is the sum of the input widths. This is the building block
/// the structural shifts are defined in terms of, so it is worth being precise
/// about the part order: `cat(&[&hi, &lo])` reads like `hi` above `lo`.
///
/// # Errors
///
/// [`Error::CatEmpty`] if `parts` is empty, or [`Error::WidthOverflow`] if the
/// summed width exceeds `u32::MAX`.
///
/// ```
/// # use ferrite_lithic_bits::{Bits, cat};
/// let hi = Bits::constant(0b1010, 4).unwrap();
/// let lo = Bits::constant(0b0110, 4).unwrap();
/// assert_eq!(cat(&[&hi, &lo]).unwrap().to_u64().unwrap(), 0b1010_0110);
/// ```
pub fn cat(parts: &[&Bits]) -> Result<Bits, Error> {
    if parts.is_empty() {
        return Err(Error::CatEmpty);
    }
    let mut total: u64 = 0;
    for part in parts {
        part.check_invariant();
        total += u64::from(part.width);
    }
    let width = u32::try_from(total).map_err(|_| Error::WidthOverflow {
        op: Op::Cat,
        bits: total,
    })?;

    let mut out = vec![0u64; words_for_width(width)];
    // Walk from the least significant part, which is last in `parts`.
    let mut placed: u64 = 0;
    for part in parts.iter().rev() {
        for i in 0..part.word_count() {
            let low_bits = (part.width - (i as u32) * 64).min(64);
            let chunk = extract_u64(part.words(), (i as u64) * 64, low_bits);
            set_bits_u64(&mut out, placed + (i as u64) * 64, chunk, low_bits);
        }
        placed += u64::from(part.width);
    }
    Ok(Bits::from_words(width, out))
}

#[cfg(test)]
mod tests {
    // A failing unit test should panic with the offending values visible, so
    // `unwrap` is the right tool here rather than a lint exemption elsewhere.
    #![allow(clippy::unwrap_used)]
    use super::cat;
    use crate::bits::Bits;
    use crate::error::Error;

    #[test]
    fn cat_reads_most_significant_part_first() {
        let hi = Bits::constant(0xab, 8).unwrap();
        let lo = Bits::constant(0xcd, 8).unwrap();
        let joined = cat(&[&hi, &lo]).unwrap();
        assert_eq!(joined.width(), 16);
        assert_eq!(joined.to_u64().unwrap(), 0xabcd);
        // Order matters: reversing swaps the halves.
        assert_eq!(cat(&[&lo, &hi]).unwrap().to_u64().unwrap(), 0xcdab);
    }

    #[test]
    fn cat_of_uneven_parts_spans_word_boundaries() {
        // 1 + 63 + 2 = 66 bits, so the result must spill into a third word.
        let a = Bits::constant(1, 1).unwrap();
        let b = Bits::ones(63).unwrap();
        let c = Bits::constant(0b10, 2).unwrap();
        let joined = cat(&[&a, &b, &c]).unwrap();
        assert_eq!(joined.width(), 66);
        assert_eq!(joined.word_count(), 2);
        assert!(!joined.bit(0).unwrap());
        assert!(joined.bit(1).unwrap());
        assert!(joined.bit(65).unwrap());
        // Round trip: slicing the middle part back out gives the original.
        assert_eq!(joined.select(2, 63).unwrap(), b);
    }

    #[test]
    fn cat_rejects_no_parts() {
        assert!(cat(&[]).is_err());
    }

    #[test]
    fn select_rejects_empty_and_out_of_range() {
        let v = Bits::constant(0xff, 8).unwrap();
        assert!(matches!(v.select(0, 0), Err(Error::SelectEmpty { .. })));
        assert!(matches!(
            v.select(4, 5),
            Err(Error::SelectOutOfRange { .. })
        ));
        assert!(v.select(8, 1).is_err());
    }

    #[test]
    fn shifts_saturate_at_or_beyond_the_width() {
        let v = Bits::constant(0b101, 3).unwrap();
        assert_eq!(v.sll(3).unwrap().to_u64().unwrap(), 0);
        assert_eq!(v.srl(3).unwrap().to_u64().unwrap(), 0);
        assert_eq!(v.sra(3).unwrap().to_u64().unwrap(), 0b111);
        let neg = Bits::constant(0b101, 3).unwrap();
        assert_eq!(neg.sra(99).unwrap().to_u64().unwrap(), 0b111);
    }

    #[test]
    fn shifts_leave_the_value_unchanged_at_zero() {
        let v = Bits::constant(0x1234_5678_9abc_def0, 64).unwrap();
        assert_eq!(v.sll(0).unwrap(), v);
        assert_eq!(v.srl(0).unwrap(), v);
        assert_eq!(v.sra(0).unwrap(), v);
    }

    #[test]
    fn arithmetic_right_shift_sign_extends_across_word_boundaries() {
        // -1 as 96 bits: every word all ones, sra by 1 must stay all ones.
        let minus_one = Bits::ones(96).unwrap();
        assert_eq!(minus_one.sra(1).unwrap(), minus_one);
        assert_eq!(minus_one.sra(95).unwrap(), minus_one);
        assert_eq!(minus_one.sra(96).unwrap(), minus_one);

        // srl instead brings in a zero at the top: bit 95 clears and every lower
        // bit stays one, so the top hex digit drops from f to 7.
        let shifted = minus_one.srl(1).unwrap();
        assert_eq!(shifted.width(), 96);
        assert!(!shifted.bit(95).unwrap());
        assert!((0..95).all(|i| shifted.bit(i).unwrap()));
    }
}
