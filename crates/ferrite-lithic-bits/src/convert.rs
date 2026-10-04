//! Construction, conversion and resizing of [`Bits`].

use crate::bits::{Bits, bytes_for_width, mask_for_width, words_for_width};
use crate::error::{Error, Op};

impl Bits {
    /// A value of `width` bits, all zero.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroWidth`] if `width == 0`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// assert_eq!(Bits::zeros(8).unwrap().to_u64().unwrap(), 0);
    /// assert!(Bits::zeros(0).is_err());
    /// ```
    pub fn zeros(width: u32) -> Result<Self, Error> {
        Self::checked_width(Op::Construct, width)?;
        Ok(Bits::from_words(width, vec![0; words_for_width(width)]))
    }

    /// A value of `width` bits, all one.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroWidth`] if `width == 0`.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// assert_eq!(Bits::ones(8).unwrap().to_u64().unwrap(), 0xff);
    /// assert_eq!(Bits::ones(13).unwrap().to_u64().unwrap(), 0x1fff);
    /// ```
    pub fn ones(width: u32) -> Result<Self, Error> {
        Self::checked_width(Op::Construct, width)?;
        Ok(Bits::from_words(
            width,
            vec![u64::MAX; words_for_width(width)],
        ))
    }

    /// A value of `width` bits holding the low bits of `value`.
    ///
    /// Bits of `value` above `width` are discarded, matching a sized Verilog
    /// constant. To detect that loss instead of accepting it, compare
    /// [`width`](Self::width) against `u64::BITS` first, or take a `u128` and
    /// split it yourself.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroWidth`] if `width == 0`.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// assert_eq!(Bits::constant(0xff, 4).unwrap().to_u64().unwrap(), 0xf);
    /// assert_eq!(Bits::constant(0b101, 3).unwrap().to_u64().unwrap(), 0b101);
    /// ```
    pub fn constant(value: u64, width: u32) -> Result<Self, Error> {
        Self::checked_width(Op::Construct, width)?;
        let mut words = vec![0u64; words_for_width(width)];
        words[0] = value;
        Ok(Bits::from_words(width, words))
    }

    /// Reads `width` bits from a little-endian byte buffer.
    ///
    /// The buffer length is fully determined by the width, so it must be exact;
    /// see [`bytes_for_width`]. Bits above `width` in the final byte are ignored,
    /// which lets a whole-byte buffer be reused for a narrower width.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroWidth`] if `width == 0`, or [`Error::BufferLength`] if
    /// `bytes.len()` is not exactly `bytes_for_width(width)`.
    pub fn from_bytes_le(width: u32, bytes: &[u8]) -> Result<Self, Error> {
        Self::checked_width(Op::Construct, width)?;
        let expected = bytes_for_width(width);
        if bytes.len() != expected {
            return Err(Error::BufferLength {
                op: Op::Construct,
                expected,
                got: bytes.len(),
            });
        }
        let mut words = vec![0u64; words_for_width(width)];
        for (i, &b) in bytes.iter().enumerate() {
            words[i / 8] |= (b as u64) << ((i % 8) * 8);
        }
        Ok(Bits::from_words(width, words))
    }

    /// The value as a `u64`.
    ///
    /// # Errors
    ///
    /// [`Error::TooWide`] if the width exceeds 64. For wider values, use
    /// [`words`](Self::words) or [`word`](Self::word).
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// assert_eq!(Bits::constant(7, 3).unwrap().to_u64().unwrap(), 7);
    /// assert!(Bits::constant(7, 65).unwrap().to_u64().is_err());
    /// ```
    pub fn to_u64(&self) -> Result<u64, Error> {
        self.check_invariant();
        if self.width > 64 {
            return Err(Error::TooWide {
                op: Op::ToU64,
                width: self.width,
                limit: 64,
            });
        }
        Ok(self.words[0] & mask_for_width(self.width))
    }

    /// The value as a little-endian byte buffer of exactly
    /// [`bytes_for_width`](crate::bytes_for_width) bytes.
    #[must_use]
    pub fn to_bytes_le(&self) -> Vec<u8> {
        self.check_invariant();
        let mut out = vec![0u8; bytes_for_width(self.width)];
        for (i, chunk) in out.iter_mut().enumerate() {
            *chunk = ((self.words[i / 8] >> ((i % 8) * 8)) & 0xff) as u8;
        }
        out
    }

    /// The value as lowercase hex digits, most significant first, zero-padded to
    /// a whole number of nibbles.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// assert_eq!(Bits::constant(0xabc, 12).unwrap().to_hex(), "abc");
    /// assert_eq!(Bits::constant(0x1, 12).unwrap().to_hex(), "001");
    /// ```
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.check_invariant();
        let digits = (self.width as usize).div_ceil(4);
        let mut out = String::with_capacity(digits);
        for d in (0..digits).rev() {
            let mut nibble = 0u32;
            for b in 0..4 {
                if self.bit((d * 4 + b) as u32).unwrap_or(false) {
                    nibble |= 1 << b;
                }
            }
            out.push(char::from_digit(nibble, 16).expect("nibble < 16"));
        }
        out
    }

    /// The bits as `'0'`/`'1'` characters, most significant first, no separators.
    #[must_use]
    pub fn to_binary_digits(&self) -> String {
        self.check_invariant();
        let mut out = String::with_capacity(self.width as usize);
        for i in (0..self.width).rev() {
            out.push(if self.bit(i).unwrap_or(false) {
                '1'
            } else {
                '0'
            });
        }
        out
    }

    /// Concatenates copies of this value, `times` times.
    ///
    /// # Errors
    ///
    /// [`Error::CatEmpty`] if `times == 0`, or [`Error::WidthOverflow`] if the
    /// result would exceed `u32::MAX` bits.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let one = Bits::constant(1, 1).unwrap();
    /// assert_eq!(one.replicate(4).unwrap().to_u64().unwrap(), 0b1111);
    /// assert!(one.replicate(0).is_err());
    /// ```
    pub fn replicate(&self, times: u32) -> Result<Self, Error> {
        self.check_invariant();
        if times == 0 {
            return Err(Error::CatEmpty);
        }
        let parts: Vec<&Self> = std::iter::repeat_n(self, times as usize).collect();
        crate::shift::cat(&parts)
    }

    /// Widens to `new_width` by adding zero bits above the most significant bit.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroWidth`] if `new_width == 0`, or [`Error::WidthMismatch`] if
    /// `new_width < width`.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let n = Bits::constant(0b11, 2).unwrap();
    /// assert_eq!(n.zero_extend(6).unwrap().to_u64().unwrap(), 0b000011);
    /// assert!(n.zero_extend(1).is_err());
    /// ```
    pub fn zero_extend(&self, new_width: u32) -> Result<Self, Error> {
        self.check_invariant();
        Self::checked_width(Op::ZeroExtend, new_width)?;
        self.require_not_narrower(Op::ZeroExtend, new_width)?;
        let mut words = self.words.clone();
        words.resize(words_for_width(new_width), 0);
        Ok(Bits::from_words(new_width, words))
    }

    /// Widens to `new_width` by replicating the most significant bit.
    ///
    /// The right choice when the value is a two's complement signed integer.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroWidth`] if `new_width == 0`, or [`Error::WidthMismatch`] if
    /// `new_width < width`.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let n = Bits::constant(0b11, 2).unwrap(); // -1
    /// assert_eq!(n.sign_extend(6).unwrap().to_u64().unwrap(), 0b111111);
    /// ```
    pub fn sign_extend(&self, new_width: u32) -> Result<Self, Error> {
        self.check_invariant();
        Self::checked_width(Op::SignExtend, new_width)?;
        self.require_not_narrower(Op::SignExtend, new_width)?;
        let fill_word = if self.is_negative() { u64::MAX } else { 0 };
        let mut out = vec![0u64; words_for_width(new_width)];
        for (dst, src) in out.iter_mut().zip(self.words.iter()) {
            *dst = *src;
        }
        // The region above `self.width` is still zero, so OR is enough. Note the
        // boundary word: when both widths fit in one word the fill is *inside* it,
        // not in any later word, which a plain `resize` would miss entirely.
        let first_word = (self.width / 64) as usize;
        let first_bit = self.width % 64;
        for (i, word) in out.iter_mut().enumerate().skip(first_word) {
            let mut fill = fill_word;
            if i == first_word && first_bit > 0 {
                fill &= !mask_for_width(first_bit);
            }
            *word |= fill;
        }
        Ok(Bits::from_words(new_width, out))
    }

    /// Narrows to `new_width` by discarding the high bits.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroWidth`] if `new_width == 0`, or [`Error::WidthMismatch`] if
    /// `new_width > width`.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// let n = Bits::constant(0b1011_0110, 8).unwrap();
    /// assert_eq!(n.truncate(4).unwrap().to_u64().unwrap(), 0b0110);
    /// ```
    pub fn truncate(&self, new_width: u32) -> Result<Self, Error> {
        self.check_invariant();
        Self::checked_width(Op::Truncate, new_width)?;
        if new_width > self.width {
            return Err(Error::WidthMismatch {
                op: Op::Truncate,
                lhs: self.width,
                rhs: new_width,
            });
        }
        let mut words = self.words.clone();
        words.truncate(words_for_width(new_width));
        Ok(Bits::from_words(new_width, words))
    }

    /// The absolute value in `width + 1` bits, for signed division and
    /// remainder.
    ///
    /// Order matters: negate **within the original width first**, then widen.
    /// Widening a negative pattern with zeros and negating afterwards computes
    /// `2^(w+1) - v` instead of `2^w - v`, which is wrong for every negative value
    /// except `-1`. `Bits { 2, 0b11 }` is `-1`, whose magnitude is `1`; the
    /// reversed order yields `5`.
    pub(crate) fn magnitude(&self) -> Result<Self, Error> {
        let wide = self.width.checked_add(1).ok_or(Error::WidthOverflow {
            op: Op::Neg,
            bits: u64::from(self.width) + 1,
        })?;
        if self.is_negative() {
            self.neg()?.zero_extend(wide)
        } else {
            self.zero_extend(wide)
        }
    }

    pub(crate) fn checked_width(op: Op, width: u32) -> Result<(), Error> {
        if width == 0 {
            return Err(Error::ZeroWidth { op });
        }
        Ok(())
    }

    fn require_not_narrower(&self, op: Op, new_width: u32) -> Result<(), Error> {
        if new_width < self.width {
            return Err(Error::WidthMismatch {
                op,
                lhs: self.width,
                rhs: new_width,
            });
        }
        Ok(())
    }
}
