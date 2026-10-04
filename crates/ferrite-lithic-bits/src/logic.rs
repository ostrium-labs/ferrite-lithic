//! Bitwise logic and comparisons.
//!
//! Every binary operation here requires equal widths. That is stricter than
//! necessary for `and`, `or` and `xor`, where zero-extending the narrower operand
//! would be a reasonable guess — but a guess is exactly what a hardware DSL should
//! not make silently, because `wide & narrow_mask` and `wide | narrow_mask` want
//! opposite padding. Make it explicit.

use core::cmp::Ordering;

use crate::bits::{Bits, cmp_words};
use crate::error::{Error, Op};

impl Bits {
    /// Bitwise complement, at the same width.
    pub fn not(&self) -> Result<Self, Error> {
        self.check_invariant();
        Ok(Bits::from_words(
            self.width,
            self.words.iter().map(|w| !w).collect(),
        ))
    }

    /// Bitwise and. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn and(&self, rhs: &Self) -> Result<Self, Error> {
        self.zip_words(Op::And, rhs, |a, b| a & b)
    }

    /// Bitwise or. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn or(&self, rhs: &Self) -> Result<Self, Error> {
        self.zip_words(Op::Or, rhs, |a, b| a | b)
    }

    /// Bitwise exclusive or. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn xor(&self, rhs: &Self) -> Result<Self, Error> {
        self.zip_words(Op::Xor, rhs, |a, b| a ^ b)
    }

    /// Equality. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn eq(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Eq, rhs)?;
        Ok(self.words == rhs.words)
    }

    /// Unsigned `<`. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn ult(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Ult, rhs)?;
        Ok(cmp_words(&self.words, &rhs.words) == Ordering::Less)
    }

    /// Unsigned `<=`. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn ule(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Ule, rhs)?;
        Ok(cmp_words(&self.words, &rhs.words) != Ordering::Greater)
    }

    /// Unsigned `>`. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn ugt(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Ugt, rhs)?;
        Ok(cmp_words(&self.words, &rhs.words) == Ordering::Greater)
    }

    /// Unsigned `>=`. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn uge(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Uge, rhs)?;
        Ok(cmp_words(&self.words, &rhs.words) != Ordering::Less)
    }

    /// Signed `<`, two's complement. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn slt(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Slt, rhs)?;
        Ok(cmp_signed(self, rhs) == Ordering::Less)
    }

    /// Signed `<=`, two's complement. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn sle(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Sle, rhs)?;
        Ok(cmp_signed(self, rhs) != Ordering::Greater)
    }

    /// Signed `>`, two's complement. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn sgt(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Sgt, rhs)?;
        Ok(cmp_signed(self, rhs) == Ordering::Greater)
    }

    /// Signed `>=`, two's complement. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    pub fn sge(&self, rhs: &Self) -> Result<bool, Error> {
        self.require_same_width(Op::Sge, rhs)?;
        Ok(cmp_signed(self, rhs) != Ordering::Less)
    }

    fn zip_words(&self, op: Op, rhs: &Self, f: impl Fn(u64, u64) -> u64) -> Result<Self, Error> {
        self.check_invariant();
        rhs.check_invariant();
        self.require_same_width(op, rhs)?;
        let out: Vec<u64> = self
            .words
            .iter()
            .zip(rhs.words.iter())
            .map(|(&a, &b)| f(a, b))
            .collect();
        Ok(Bits::from_words(self.width, out))
    }
}

/// Signed comparison of equal-width values.
///
/// Implemented directly by dispatching on the sign bits, rather than by
/// Hardcaml's trick of flipping the sign bit into the LSB and doing an unsigned
/// compare ([`kernel/comb.ml:728-746`](https://github.com/jane-street/hardcaml)).
/// We keep the same observable results and skip the special case for width 1 that
/// the trick requires, because dispatching on the sign bit is already correct at
/// width 1 — the single bit *is* the sign bit.
///
/// The subtle case is two values that are *both* negative. They must not have
/// their comparison reversed: within one sign, interpreting the raw pattern as
/// `raw - 2^width` subtracts a constant, and subtracting a constant preserves
/// order. So the unsigned order of the raw patterns is already the signed order,
/// and reversing it would put `-1` *below* `-2^128`. It is magnitudes that order
/// inversely, which is what Hardcaml's LSB trick goes on to compare.
fn cmp_signed(a: &Bits, b: &Bits) -> Ordering {
    debug_assert_eq!(a.width, b.width, "cmp_signed needs equal widths");
    match (a.is_negative(), b.is_negative()) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) | (true, true) => cmp_words(&a.words, &b.words),
    }
}

#[cfg(test)]
mod tests {
    // A failing unit test should panic with the offending values visible, so
    // `unwrap` is the right tool here rather than a lint exemption elsewhere.
    #![allow(clippy::unwrap_used)]
    use super::cmp_signed;
    use crate::bits::Bits;
    use core::cmp::Ordering;

    #[test]
    fn signed_comparison_at_width_one_matches_ordering_by_sign() {
        // At width 1 the only bit is the sign bit: 0 is positive, 1 is negative.
        let zero = Bits::zeros(1).unwrap();
        let one = Bits::ones(1).unwrap();
        assert!(!zero.is_negative());
        assert!(one.is_negative());
        assert_eq!(cmp_signed(&zero, &one), Ordering::Greater);
        assert_eq!(cmp_signed(&one, &zero), Ordering::Less);
        assert_eq!(cmp_signed(&zero, &zero), Ordering::Equal);
    }

    #[test]
    fn signed_comparison_across_word_boundaries() {
        let min = Bits::constant(0x8000_0000_0000_0000, 64).unwrap();
        let max = Bits::constant(0x7fff_ffff_ffff_ffff, 64).unwrap();
        assert!(min.slt(&max).unwrap());
        assert!(max.sgt(&min).unwrap());
        // Unsigned, the same pair orders the other way round.
        assert!(min.ugt(&max).unwrap());
    }

    #[test]
    fn bitwise_ops_require_equal_widths() {
        let a = Bits::constant(0b1100, 4).unwrap();
        let b = Bits::constant(0b1010, 4).unwrap();
        assert_eq!(a.and(&b).unwrap().to_u64().unwrap(), 0b1000);
        assert_eq!(a.or(&b).unwrap().to_u64().unwrap(), 0b1110);
        assert_eq!(a.xor(&b).unwrap().to_u64().unwrap(), 0b0110);
        assert_eq!(a.not().unwrap().to_u64().unwrap(), 0b0011);
        let narrow = Bits::constant(0b11, 2).unwrap();
        assert!(a.and(&narrow).is_err());
    }
}
