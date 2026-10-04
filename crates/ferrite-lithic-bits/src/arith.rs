//! Integer arithmetic on [`Bits`].
//!
//! # The asymmetry that matters
//!
//! **Addition, subtraction and negation truncate; multiplication does not.**
//!
//! `add` and `sub` return the left operand's width, so the carry out of the top
//! bit is discarded. This is inherited from Hardcaml, which asserts equal input
//! widths and masks the result
//! ([`kernel/comb.ml:542-545`](https://github.com/jane-street/hardcaml)) while
//! offering no automatic widening. `mul` by contrast returns `wa + wb` bits and
//! cannot overflow
//! ([`kernel/bits.ml:605-609`](https://github.com/jane-street/hardcaml)).
//!
//! The asymmetry is deliberate and kept: the dangerous operation is the one that
//! silently loses bits, and making `mul` widen is how the type system stops
//! multiplication from being the accident. Truncation stays available, but you
//! have to ask for it by name with [`select`](crate::Bits::select).
//!
//! # Division is not cheap
//!
//! [`udiv`](Bits::udiv), [`sdiv`](Bits::sdiv), [`urem`](Bits::urem) and
//! [`srem`](Bits::srem) do not exist anywhere in Hardcaml
//! ([ADR-0009](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0009-add-integer-division-and-remainder.md)).
//! They are real primitives here, but they are **not** cheap: each infers a large
//! divider. For the constant-reciprocal patterns a datapath usually wants, build
//! the reciprocal by hand with shifts and a multiply instead.

use crate::bits::Bits;
use crate::error::{Error, Op};

impl Bits {
    /// Adds two values, **truncating to the left operand's width**.
    ///
    /// Bits carried out of the top are discarded. `add` requires equal widths;
    /// widen explicitly with [`zero_extend`](Bits::zero_extend) or
    /// [`sign_extend`](Bits::sign_extend) first.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let a = Bits::constant(0xff, 8).unwrap();
    /// assert_eq!(a.add(&Bits::constant(1, 8).unwrap()).unwrap().to_u64().unwrap(), 0);
    /// ```
    pub fn add(&self, rhs: &Self) -> Result<Self, Error> {
        self.check_invariant();
        rhs.check_invariant();
        self.require_same_width(Op::Add, rhs)?;
        let mut out = Vec::with_capacity(self.words.len());
        let mut carry = 0u64;
        for i in 0..self.words.len() {
            let (s1, c1) = self.words[i].overflowing_add(rhs.words[i]);
            let (s2, c2) = s1.overflowing_add(carry);
            out.push(s2);
            carry = u64::from(c1) + u64::from(c2);
        }
        // The carry out of the top word is discarded on purpose: `add` truncates.
        Ok(Bits::from_words(self.width, out))
    }

    /// Subtracts two values, **truncating to the left operand's width**.
    ///
    /// Modular two's complement: if `rhs > self` the result wraps, it does not
    /// saturate.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let a = Bits::constant(1, 4).unwrap();
    /// assert_eq!(a.sub(&Bits::constant(2, 4).unwrap()).unwrap().to_u64().unwrap(), 0b1111);
    /// ```
    pub fn sub(&self, rhs: &Self) -> Result<Self, Error> {
        self.check_invariant();
        rhs.check_invariant();
        self.require_same_width(Op::Sub, rhs)?;
        let mut out = Vec::with_capacity(self.words.len());
        let mut borrow = 0u64;
        for i in 0..self.words.len() {
            let (d1, b1) = self.words[i].overflowing_sub(rhs.words[i]);
            let (d2, b2) = d1.overflowing_sub(borrow);
            out.push(d2);
            borrow = u64::from(b1) + u64::from(b2);
        }
        Ok(Bits::from_words(self.width, out))
    }

    /// Two's complement negation at the same width.
    ///
    /// # Errors
    ///
    /// Never; provided so `neg` has the same shape as the other arithmetic.
    pub fn neg(&self) -> Result<Self, Error> {
        self.check_invariant();
        let mut out = self.words.clone();
        let mut carry = 1u64;
        for w in out.iter_mut() {
            let (v, c) = (!*w).overflowing_add(carry);
            *w = v;
            carry = u64::from(c);
        }
        Ok(Bits::from_words(self.width, out))
    }

    /// Multiplies, returning **`self.width + rhs.width` bits**.
    ///
    /// Full precision: the product of a `wa`-bit and a `wb`-bit value always fits
    /// in `wa + wb` bits, so this cannot overflow and never truncates.
    /// Operands need not have equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthOverflow`] if the sum of the widths exceeds `u32::MAX`, which
    /// requires operands near 2^31 bits wide.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let a = Bits::constant(0b1111, 4).unwrap();
    /// let b = Bits::constant(0b11, 2).unwrap();
    /// let p = a.mul(&b).unwrap();
    /// assert_eq!(p.width(), 6);
    /// assert_eq!(p.to_u64().unwrap(), 0b101101); // 15 * 3 = 45, in 6 bits
    /// ```
    pub fn mul(&self, rhs: &Self) -> Result<Self, Error> {
        self.check_invariant();
        rhs.check_invariant();
        let total = u64::from(self.width) + u64::from(rhs.width);
        let out_width = u32::try_from(total).map_err(|_| Error::WidthOverflow {
            op: Op::Mul,
            bits: total,
        })?;
        let out_words = crate::bits::words_for_width(out_width);
        let mut out = vec![0u64; out_words];

        for i in 0..self.words.len() {
            let a = self.words[i];
            if a == 0 {
                continue;
            }
            let mut carry: u128 = 0;
            for j in 0..rhs.words.len() {
                // Max value here is (2^64-1)^2 + (2^64-1) + (2^64-1) = 2^128-1,
                // so this is exact in u128 with no overflow and no wraparound.
                let t = u128::from(a) * u128::from(rhs.words[j]) + u128::from(out[i + j]) + carry;
                out[i + j] = t as u64;
                carry = t >> 64;
            }
            let mut k = i + rhs.words.len();
            while carry != 0 && k < out_words {
                let t = u128::from(out[k]) + carry;
                out[k] = t as u64;
                carry = t >> 64;
                k += 1;
            }
        }
        Ok(Bits::from_words(out_width, out))
    }

    /// Unsigned division, truncating toward zero. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ, or
    /// [`Error::DivisionByZero`] if `rhs` is zero.
    pub fn udiv(&self, rhs: &Self) -> Result<Self, Error> {
        self.check_invariant();
        rhs.check_invariant();
        self.require_same_width(Op::UDiv, rhs)?;
        if rhs.is_zero() {
            return Err(Error::DivisionByZero {
                op: Op::UDiv,
                width: self.width,
            });
        }
        Ok(div_rem_unsigned(self, rhs).0)
    }

    /// Unsigned remainder. Requires equal widths.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ, or
    /// [`Error::DivisionByZero`] if `rhs` is zero.
    pub fn urem(&self, rhs: &Self) -> Result<Self, Error> {
        self.check_invariant();
        rhs.check_invariant();
        self.require_same_width(Op::URem, rhs)?;
        if rhs.is_zero() {
            return Err(Error::DivisionByZero {
                op: Op::URem,
                width: self.width,
            });
        }
        Ok(div_rem_unsigned(self, rhs).1)
    }

    /// Signed division, truncating toward zero. Requires equal widths.
    ///
    /// Matches Verilog's `signed /`, not Rust's panicking behaviour.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ,
    /// [`Error::DivisionByZero`] if `rhs` is zero, or
    /// [`Error::SignedDivisionOverflow`] for the most negative value divided by
    /// `-1`, whose exact quotient needs one more bit than the operands have.
    pub fn sdiv(&self, rhs: &Self) -> Result<Self, Error> {
        self.check_invariant();
        rhs.check_invariant();
        self.require_same_width(Op::SDiv, rhs)?;
        if rhs.is_zero() {
            return Err(Error::DivisionByZero {
                op: Op::SDiv,
                width: self.width,
            });
        }
        if self.is_min_signed() && rhs.is_all_ones() {
            return Err(Error::SignedDivisionOverflow {
                op: Op::SDiv,
                width: self.width,
            });
        }
        let quotient_negated = self.is_negative() != rhs.is_negative();
        let (quotient, _) = div_rem_unsigned(&self.magnitude()?, &rhs.magnitude()?);
        let quotient = quotient.truncate(self.width)?;
        if quotient_negated {
            quotient.neg()
        } else {
            Ok(quotient)
        }
    }

    /// Signed remainder, with the sign of the dividend. Requires equal widths.
    ///
    /// Note that unlike [`sdiv`](Self::sdiv) this cannot overflow: the most
    /// negative value modulo `-1` is zero, which is representable.
    ///
    /// # Errors
    ///
    /// [`Error::WidthMismatch`] if the widths differ, or
    /// [`Error::DivisionByZero`] if `rhs` is zero.
    pub fn srem(&self, rhs: &Self) -> Result<Self, Error> {
        self.check_invariant();
        rhs.check_invariant();
        self.require_same_width(Op::SRem, rhs)?;
        if rhs.is_zero() {
            return Err(Error::DivisionByZero {
                op: Op::SRem,
                width: self.width,
            });
        }
        let remainder_negated = self.is_negative();
        let (_, remainder) = div_rem_unsigned(&self.magnitude()?, &rhs.magnitude()?);
        let remainder = remainder.truncate(self.width)?;
        if remainder_negated {
            remainder.neg()
        } else {
            Ok(remainder)
        }
    }

    /// Shifts left by one bit in place, discarding the bit shifted out.
    pub(crate) fn shl1_in_place(&mut self) {
        let mut carry = 0u64;
        for w in self.words.iter_mut() {
            let next = *w >> 63;
            *w = (*w << 1) | carry;
            carry = next;
        }
        self.mask_top();
    }
}

/// Unsigned restoring division: shift-and-subtract, most significant bit first.
///
/// Requires equal operand widths and a non-zero divisor. The remainder is kept in
/// `rhs.width + 1` bits because after `rem = rem << 1 | bit` it can reach
/// `2 * rhs`, which does not fit in `rhs.width` bits.
fn div_rem_unsigned(lhs: &Bits, rhs: &Bits) -> (Bits, Bits) {
    debug_assert_eq!(lhs.width, rhs.width, "div_rem_unsigned needs equal widths");
    debug_assert!(!rhs.is_zero(), "div_rem_unsigned needs a non-zero divisor");

    let width = lhs.width;
    let remainder_width = rhs.width + 1;
    let rhs_wide = rhs
        .zero_extend(remainder_width)
        .expect("remainder_width exceeds rhs width");
    let mut remainder = Bits::zeros(remainder_width).expect("remainder_width >= 2");
    let mut quotient = Bits::zeros(width).expect("width >= 1");

    for i in (0..width).rev() {
        remainder.shl1_in_place();
        if lhs.bit(i).expect("i < width") {
            remainder.set_bit_unchecked(0);
        }
        if remainder.uge(&rhs_wide).expect("equal widths") {
            remainder = remainder.sub(&rhs_wide).expect("equal widths");
            quotient.set_bit_unchecked(i);
        }
    }
    debug_assert!(
        remainder
            .truncate(width)
            .expect("remainder_width > width")
            .ult(rhs)
            .expect("equal widths"),
        "the running remainder must stay below the divisor"
    );
    // remainder < rhs < 2^width, so truncating to the operand width loses nothing.
    (
        quotient,
        remainder.truncate(width).expect("remainder_width > width"),
    )
}
