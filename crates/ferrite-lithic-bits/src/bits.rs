//! The fixed-width bitvector: representation, conversions and ordering.

use core::cmp::Ordering;
use core::fmt;

use crate::error::{Error, Op};

/// Bits per storage word.
pub const WORD_BITS: u32 = 64;

/// Number of `u64` words needed to hold `width` bits.
///
/// A 1-bit value occupies one word, but a 65-bit value occupies two rather than
/// three. Hardcaml rounds every width up to a full word and additionally wastes a
/// whole word on the width field itself
/// ([`kernel/bits0.mli:8-11`](https://github.com/jane-street/hardcaml)); we size the
/// vector exactly and keep only the invariant that actually matters.
///
/// ```
/// # use ferrite_lithic_bits::words_for_width;
/// assert_eq!(words_for_width(0), 0);
/// assert_eq!(words_for_width(1), 1);
/// assert_eq!(words_for_width(64), 1);
/// assert_eq!(words_for_width(65), 2);
/// ```
#[must_use]
pub const fn words_for_width(width: u32) -> usize {
    (width as u64).div_ceil(64) as usize
}

/// Number of bytes needed to hold `width` bits.
///
/// ```
/// # use ferrite_lithic_bits::bytes_for_width;
/// assert_eq!(bytes_for_width(1), 1);
/// assert_eq!(bytes_for_width(8), 1);
/// assert_eq!(bytes_for_width(9), 2);
/// ```
#[must_use]
pub const fn bytes_for_width(width: u32) -> usize {
    (width as u64).div_ceil(8) as usize
}

/// The mask that clears the unused bits above `width` in the top word.
///
/// Returns zero for a zero width, and all ones on a word boundary. Both are
/// cases where a naive `1 << (width % 64)` shifts out of range or wraps to zero
/// for the wrong reason.
#[must_use]
pub const fn mask_for_width(width: u32) -> u64 {
    if width == 0 {
        return 0;
    }
    let rem = width % WORD_BITS;
    if rem == 0 {
        u64::MAX
    } else {
        (1u64 << rem) - 1
    }
}

/// A fixed-width vector of bits, stored little-endian in `u64` words.
///
/// # The invariant
///
/// **Unused bits above `width` in the top word are always zero.** This is not a
/// suggestion; every operation depends on it, and it is the one sharp edge
/// inherited from Hardcaml
/// ([`kernel/bits0.ml:113-122`](https://github.com/jane-street/hardcaml)). All
/// construction funnels through one private masking constructor, and every public
/// method debug-asserts the invariant at its own entry.
///
/// With that invariant held, `width` alone determines the value domain: there is
/// no separate X/Z, no signedness tag, and no notion of "don't care". This is a
/// strictly two-state type, matching Hardcaml's synthesizable graph.
///
/// # Widths are runtime values
///
/// See [ADR-0003](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0003-runtime-widths-not-const-generics.md).
/// Width mismatches are therefore runtime errors, not compile errors, and the
/// messages in [`Error`] carry the compensation.
///
/// # Ordering
///
/// [`Ord`] is **by width first, then by unsigned value**, so values of different
/// widths are ordered rather than rejected. This reproduces Hardcaml's
/// [`kernel/bits0.ml:46-61`](https://github.com/jane-street/hardcaml) because it
/// is observable: a `Bits` collection sorted with `sort` groups by width.
///
/// # Examples
///
/// ```
/// # use ferrite_lithic_bits::Bits;
/// let a = Bits::constant(0b1010, 4).unwrap();
/// let b = Bits::constant(0b0110, 4).unwrap();
/// assert_eq!((a.add(&b).unwrap()).to_u64().unwrap(), 0b0000); // truncated
/// assert_eq!((a.mul(&b).unwrap()).width(), 8);               // widened
/// assert_eq!((a.xor(&b).unwrap()).to_u64().unwrap(), 0b1100);
/// ```
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct Bits {
    pub(crate) width: u32,
    pub(crate) words: Vec<u64>,
}

impl Bits {
    /// Builds a value from words, masking off the unused bits above `width`.
    ///
    /// The single point where the zero-upper-bits invariant is established. Every
    /// other constructor and every arithmetic result goes through here.
    pub(crate) fn from_words(width: u32, mut words: Vec<u64>) -> Self {
        debug_assert!(
            width >= 1,
            "a zero-width value must be refused before it reaches from_words"
        );
        debug_assert_eq!(
            words.len(),
            words_for_width(width),
            "word count must match the width"
        );
        if let Some(top) = words.last_mut() {
            *top &= mask_for_width(width);
        }
        Bits { width, words }
    }

    /// Debug-asserts the zero-upper-bits invariant.
    ///
    /// Called at the top of every operation. Cheap in release (it compiles to
    /// nothing) and catches a violated invariant at the operation that broke it
    /// rather than several operations later.
    pub(crate) fn check_invariant(&self) {
        debug_assert!(self.width >= 1, "zero-width value escaped construction");
        debug_assert_eq!(
            self.words.len(),
            words_for_width(self.width),
            "word count must match the width"
        );
        if let Some(top) = self.words.last() {
            debug_assert_eq!(
                *top & !mask_for_width(self.width),
                0,
                "zero-upper-bits invariant violated: unused bits above width {} \
                 are not clear",
                self.width
            );
        }
    }

    /// The width of this value, in bits. Always at least 1.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The number of storage words. Always at least 1.
    #[must_use]
    pub const fn word_count(&self) -> usize {
        self.words.len()
    }

    /// The storage words, least significant first.
    ///
    /// The top word may contain bits above [`width`](Self::width) cleared to zero
    /// as an invariant, but callers must not rely on that: use
    /// [`word`](Self::word) for a width-correct value.
    #[must_use]
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    /// One storage word, masked to the width.
    ///
    /// Returns `None` if `index` is past the end. The returned word has its upper
    /// bits cleared for the top word, so it is safe to accumulate into a wider
    /// value.
    #[must_use]
    pub fn word(&self, index: usize) -> Option<u64> {
        let raw = *self.words.get(index)?;
        if index + 1 == self.words.len() {
            Some(raw & mask_for_width(self.width))
        } else {
            Some(raw)
        }
    }

    /// Whether every bit is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// Whether every bit is one.
    #[must_use]
    pub fn is_ones(&self) -> bool {
        self.check_invariant();
        let last = self.words.len() - 1;
        self.words.iter().enumerate().all(|(i, &w)| {
            if i == last {
                w == mask_for_width(self.width)
            } else {
                w == u64::MAX
            }
        })
    }

    /// Whether the most significant bit is set.
    ///
    /// For a two's complement interpretation this is the sign bit. Uniform across
    /// widths, including width 1, where the only bit *is* the sign bit.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.bit(self.width - 1).unwrap_or(false)
    }

    /// Whether this is the most negative value in two's complement, `-2^(w-1)`.
    ///
    /// That is "sign bit set, every bit below it clear". The sign bit is the only
    /// bit in the top word that is allowed to be set, which at width 1 means the
    /// whole word — so at width 1 the single value `1` *is* the minimum, and
    /// dividing it by `-1` overflows.
    #[must_use]
    pub fn is_min_signed(&self) -> bool {
        self.check_invariant();
        if !self.is_negative() {
            return false;
        }
        let last = self.words.len() - 1;
        // Position of the sign bit inside the top word.
        let sign_in_top = (self.width - 1) % WORD_BITS;
        self.words
            .iter()
            .enumerate()
            .all(|(i, &w)| i != last || w & !(1u64 << sign_in_top) == 0)
    }

    /// All bits one, i.e. `-1` in two's complement.
    #[must_use]
    pub fn is_all_ones(&self) -> bool {
        self.is_ones()
    }

    /// Whether this value's magnitude is zero, ignoring sign interpretation.
    #[must_use]
    pub fn magnitude_is_zero(&self) -> bool {
        self.is_zero()
    }

    /// Reads a single bit, counting from the least significant end.
    ///
    /// # Errors
    ///
    /// [`Error::BitIndexOutOfRange`] if `index >= width`.
    pub fn bit(&self, index: u32) -> Result<bool, Error> {
        if index >= self.width {
            return Err(Error::BitIndexOutOfRange {
                width: self.width,
                index,
            });
        }
        let w = (index / WORD_BITS) as usize;
        let b = index % WORD_BITS;
        Ok(self.words[w] & (1u64 << b) != 0)
    }

    /// Sets a single bit in place, for internal use by algorithms that know the
    /// index is in range.
    pub(crate) fn set_bit_unchecked(&mut self, index: u32) {
        let w = (index / WORD_BITS) as usize;
        let b = index % WORD_BITS;
        self.words[w] |= 1u64 << b;
        self.mask_top();
    }

    pub(crate) fn mask_top(&mut self) {
        if let Some(top) = self.words.last_mut() {
            *top &= mask_for_width(self.width);
        }
    }

    /// Asserts equal widths, producing the compensation-bearing error.
    pub(crate) fn require_same_width(&self, op: Op, rhs: &Self) -> Result<(), Error> {
        if self.width != rhs.width {
            return Err(Error::WidthMismatch {
                op,
                lhs: self.width,
                rhs: rhs.width,
            });
        }
        Ok(())
    }
}

/// Word-level helper: read `n` bits (1..=64) starting at bit `bit_offset` as a
/// `u64`, shifted so bit 0 of the result is the first requested bit.
pub(crate) fn extract_u64(words: &[u64], bit_offset: u64, n: u32) -> u64 {
    debug_assert!((1..=64).contains(&n), "extract_u64 needs 1..=64 bits");
    let w = (bit_offset / 64) as usize;
    let b = (bit_offset % 64) as u32;
    debug_assert!(w < words.len(), "extract_u64 offset past the end");
    let mut v = words[w] >> b;
    if b > 0 && w + 1 < words.len() {
        v |= words[w + 1] << (64 - b);
    }
    if n < 64 {
        v &= mask_for_width(n);
    }
    v
}

/// Word-level helper: OR the low `n` bits of `value` into `words` at `bit_offset`.
pub(crate) fn set_bits_u64(words: &mut [u64], bit_offset: u64, value: u64, n: u32) {
    debug_assert!((1..=64).contains(&n), "set_bits_u64 needs 1..=64 bits");
    let w = (bit_offset / 64) as usize;
    let b = (bit_offset % 64) as u32;
    let lo = if n >= 64 {
        value
    } else {
        value & mask_for_width(n)
    };
    words[w] |= lo << b;
    if b > 0 {
        let spill = n.saturating_sub(64 - b);
        if spill > 0 {
            words[w + 1] |= lo >> (64 - b);
        }
    }
}

/// Compares word slices as unsigned big-endian integers, most significant word
/// first. Both slices must be the same length.
pub(crate) fn cmp_words(a: &[u64], b: &[u64]) -> Ordering {
    debug_assert_eq!(a.len(), b.len(), "cmp_words needs equal lengths");
    for i in (0..a.len()).rev() {
        match a[i].cmp(&b[i]) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

impl Ord for Bits {
    /// By width first, then by unsigned value.
    fn cmp(&self, other: &Self) -> Ordering {
        self.width
            .cmp(&other.width)
            .then_with(|| cmp_words(&self.words, &other.words))
    }
}

impl PartialOrd for Bits {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for Bits {
    /// Prints the width and hex digits, e.g. `Bits { 12, 0xabc }`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bits {{ {}, {} }}", self.width, self.to_hex())
    }
}

impl fmt::Display for Bits {
    /// Prints the bits as binary, most significant first, grouped four per digit
    /// with `_` separators.
    ///
    /// ```
    /// # use ferrite_lithic_bits::Bits;
    /// assert_eq!(Bits::constant(0b1010_1010_1010, 12).unwrap().to_string(), "1010_1010_1010");
    /// assert_eq!(Bits::constant(0b1, 12).unwrap().to_string(), "0000_0000_0001");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bits = self.to_binary_digits();
        let mut out = String::with_capacity(bits.len() + bits.len() / 4);
        for (i, ch) in bits.chars().enumerate() {
            if i > 0 && (bits.len() - i).is_multiple_of(4) {
                out.push('_');
            }
            out.push(ch);
        }
        f.write_str(&out)
    }
}

#[cfg(test)]
mod tests {
    // A failing unit test should panic with the offending values visible, so
    // `unwrap` is the right tool here rather than a lint exemption elsewhere.
    #![allow(clippy::unwrap_used)]
    use super::{Bits, bytes_for_width, mask_for_width, words_for_width};

    #[test]
    fn mask_is_all_ones_on_word_boundaries() {
        assert_eq!(mask_for_width(0), 0);
        assert_eq!(mask_for_width(64), u64::MAX);
        assert_eq!(mask_for_width(128), u64::MAX);
        assert_eq!(mask_for_width(1), 1);
        assert_eq!(mask_for_width(63), (1u64 << 63) - 1);
        assert_eq!(mask_for_width(65), 1);
    }

    #[test]
    fn word_and_byte_counts_handle_boundaries() {
        assert_eq!(words_for_width(63), 1);
        assert_eq!(words_for_width(64), 1);
        assert_eq!(words_for_width(65), 2);
        assert_eq!(bytes_for_width(7), 1);
        assert_eq!(bytes_for_width(8), 1);
        assert_eq!(bytes_for_width(64), 8);
    }

    #[test]
    fn ordering_is_by_width_then_value() {
        let mut v = [
            Bits::constant(5, 8).unwrap(),
            Bits::constant(1, 3).unwrap(),
            Bits::constant(9, 8).unwrap(),
            Bits::constant(255, 4).unwrap(),
        ];
        v.sort();
        let widths: Vec<u32> = v.iter().map(Bits::width).collect();
        assert_eq!(widths, vec![3, 4, 8, 8]);
        assert_eq!(v[2].to_u64().unwrap(), 5);
        assert_eq!(v[3].to_u64().unwrap(), 9);
    }

    #[test]
    fn equality_ignores_padding_but_not_width() {
        assert_eq!(
            Bits::constant(0, 64).unwrap(),
            Bits::constant(0, 65).unwrap().truncate(64).unwrap()
        );
        assert_ne!(Bits::constant(0, 8).unwrap(), Bits::constant(0, 9).unwrap());
    }
}
