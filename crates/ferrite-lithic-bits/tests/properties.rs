//! Property tests for [`Bits`] against a `num-bigint` oracle.
//!
//! The width strategy deliberately concentrates on values near word boundaries
//! (1, 63, 64, 65, 127, 128, 129) because that is where word-index arithmetic
//! goes wrong, and it includes widths below 64 where the whole value lives in one
//! word and the "two words minimum" assumption that Hardcaml's multiply makes
//! ([`kernel/bits.ml:449`](https://github.com/jane-street/hardcaml)) would have
//! broken. Small widths are additionally swept exhaustively in
//! `small_width_sweep.rs`.
//!
//! Binop tests draw **both operands from one width** rather than filtering for
//! equal widths afterwards. Rejecting mismatched pairs instead would reject
//! almost every case, since two independently drawn widths rarely coincide.

// A failing property must panic loudly and immediately, so `unwrap` is the right
// tool here. The workspace keeps `unwrap_used` as a warning for library code.
#![allow(clippy::unwrap_used)]

mod common;

use common::{from_bigint, invariant_holds, modulus, to_bigint, to_biguint, zero};
use ferrite_lithic_bits::{Bits, cat};
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use proptest::prelude::*;

/// Widths biased towards word and sub-word boundaries.
fn arb_width() -> impl Strategy<Value = u32> {
    prop_oneof![
        1u32..=40,
        41u32..=63,
        Just(64),
        65u32..=67,
        95u32..=97,
        126u32..=130,
        190u32..=260,
    ]
}

/// Builds a value of `width` from raw bytes, optionally forcing every high bit to
/// one so negative values are exercised about half the time.
fn make_bits(width: u32, high: bool, bytes: Vec<u8>) -> Bits {
    let want = ferrite_lithic_bits::bytes_for_width(width);
    let fill = if high { 0xffu8 } else { 0x00 };
    let mut buffer: Vec<u8> = bytes.into_iter().map(|b| b | fill).collect();
    buffer.resize(want, fill);
    buffer.truncate(want);
    Bits::from_bytes_le(width, &buffer).expect("buffer length is exact")
}

/// A single value of an arbitrary width.
fn arb_bits() -> impl Strategy<Value = Bits> {
    (arb_width(), any::<bool>(), any::<Vec<u8>>())
        .prop_map(|(width, high, bytes)| make_bits(width, high, bytes))
}

/// Two values sharing one width, which is what every binop needs.
fn arb_pair() -> impl Strategy<Value = (Bits, Bits)> {
    (
        arb_width(),
        any::<bool>(),
        any::<bool>(),
        any::<Vec<u8>>(),
        any::<Vec<u8>>(),
    )
        .prop_map(|(width, high_a, high_b, a, b)| {
            (make_bits(width, high_a, a), make_bits(width, high_b, b))
        })
}

/// Test configuration.
///
/// Persistence is left on deliberately: the seeds in
/// `properties.proptest-regressions` are the ones that found the `cmp_signed` and
/// `is_min_signed` bugs, so they are replayed before any novel case is generated
/// and stay replayed for the life of the crate.
fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 128,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    /// `add` truncates to the left operand's width, discarding the carry.
    #[test]
    fn add_truncates_to_the_left_width((a, b) in arb_pair()) {
        let width = a.width();
        let result = a.add(&b).unwrap();
        prop_assert_eq!(result.width(), width);
        let expected = (&to_biguint(&a) + &to_biguint(&b)) % &modulus(width);
        prop_assert_eq!(to_biguint(&result), expected);
        prop_assert!(invariant_holds(&result));
    }

    /// `sub` is modular two's complement, so it wraps rather than saturating.
    #[test]
    fn sub_wraps_modularly((a, b) in arb_pair()) {
        let width = a.width();
        let result = a.sub(&b).unwrap();
        let (x, y) = (to_biguint(&a), to_biguint(&b));
        let expected = if x >= y { &x - &y } else { &modulus(width) - (&y - &x) };
        prop_assert_eq!(to_biguint(&result), expected);
    }

    /// `neg` is two's complement at the same width, and is its own inverse.
    #[test]
    fn neg_is_twos_complement_and_involutive(a in arb_bits()) {
        let negated = a.neg().unwrap();
        prop_assert_eq!(negated.width(), a.width());
        let modulus = modulus(a.width());
        let expected = (&modulus - &to_biguint(&a)) % &modulus;
        prop_assert_eq!(to_biguint(&negated), expected);
        prop_assert_eq!(negated.neg().unwrap(), a.clone());
    }

    /// `mul` widens to the sum of the widths and cannot overflow. The operands
    /// need not share a width.
    #[test]
    fn mul_widens_and_never_overflows(a in arb_bits(), b in arb_bits()) {
        let product = a.mul(&b).unwrap();
        prop_assert_eq!(product.width(), a.width() + b.width());
        prop_assert_eq!(to_biguint(&product), &to_biguint(&a) * &to_biguint(&b));
        prop_assert!(invariant_holds(&product));
    }

    /// The truncating asymmetry, made concrete: when `add` drops a carry,
    /// `mul` of the same values still holds the full product.
    #[test]
    fn mul_keeps_bits_that_add_would_drop((a, b) in arb_pair()) {
        let width = a.width();
        prop_assume!(to_biguint(&a) + &to_biguint(&b) >= modulus(width));
        let product = a.mul(&b).unwrap();
        prop_assert_eq!(product.width(), width * 2);
        prop_assert_eq!(a.add(&b).unwrap().width(), width);
        prop_assert_eq!(to_biguint(&product), &to_biguint(&a) * &to_biguint(&b));
    }

    /// Unsigned division and remainder agree with the oracle and satisfy
    /// `lhs == quotient * divisor + remainder` with `remainder < divisor`.
    #[test]
    fn unsigned_division_and_remainder_match_the_oracle((a, b) in arb_pair()) {
        prop_assume!(!b.is_zero());
        let (x, y) = (to_biguint(&a), to_biguint(&b));
        let quotient = a.udiv(&b).unwrap();
        let remainder = a.urem(&b).unwrap();
        prop_assert_eq!(to_biguint(&quotient), &x / &y);
        prop_assert_eq!(to_biguint(&remainder), &x % &y);
        prop_assert!(to_biguint(&remainder) < y);
        prop_assert_eq!(to_biguint(&quotient) * &y + &to_biguint(&remainder), x);
    }

    /// Signed division truncates toward zero, like Verilog's `signed /`, and the
    /// quotient and remainder reconstruct the dividend exactly.
    #[test]
    fn signed_division_truncates_toward_zero((a, b) in arb_pair()) {
        prop_assume!(!b.is_zero());
        prop_assume!(!(a.is_min_signed() && b.is_all_ones()));
        let (x, y) = (to_bigint(&a), to_bigint(&b));
        let quotient = a.sdiv(&b).unwrap();
        let remainder = a.srem(&b).unwrap();
        prop_assert_eq!(to_bigint(&quotient), &x / &y);
        prop_assert_eq!(to_bigint(&remainder), &x % &y);
        // The remainder takes the sign of the dividend, never the divisor's —
        // except that an exact division leaves zero, which has no sign.
        prop_assert_eq!(
            remainder.is_negative(),
            a.is_negative() && !remainder.is_zero()
        );
        prop_assert_eq!(
            &to_bigint(&quotient) * &y + &to_bigint(&remainder),
            x
        );
    }

    /// The division identity also holds unsigned, stated separately so a failure
    /// names which family broke.
    #[test]
    fn unsigned_division_reconstructs_the_dividend((a, b) in arb_pair()) {
        prop_assume!(!b.is_zero());
        prop_assert_eq!(
            to_biguint(&a.udiv(&b).unwrap()) * &to_biguint(&b)
                + &to_biguint(&a.urem(&b).unwrap()),
            to_biguint(&a)
        );
    }

    /// Bitwise ops match the oracle, which is bit-exact so no wrapping applies.
    #[test]
    fn bitwise_ops_match_the_oracle((a, b) in arb_pair()) {
        let full = modulus(a.width()) - BigUint::from(1u8);
        let (x, y) = (to_biguint(&a), to_biguint(&b));
        prop_assert_eq!(to_biguint(&a.and(&b).unwrap()), &x & &y);
        prop_assert_eq!(to_biguint(&a.or(&b).unwrap()), &x | &y);
        prop_assert_eq!(to_biguint(&a.xor(&b).unwrap()), &x ^ &y);
        prop_assert_eq!(to_biguint(&a.not().unwrap()), &full ^ &x);
    }

    /// Unsigned comparisons match `BigUint` ordering.
    #[test]
    fn unsigned_comparisons_match_the_oracle((a, b) in arb_pair()) {
        let (x, y) = (to_biguint(&a), to_biguint(&b));
        prop_assert_eq!(a.ult(&b).unwrap(), x < y);
        prop_assert_eq!(a.ule(&b).unwrap(), x <= y);
        prop_assert_eq!(a.ugt(&b).unwrap(), x > y);
        prop_assert_eq!(a.uge(&b).unwrap(), x >= y);
        prop_assert_eq!(a.eq(&b).unwrap(), x == y);
    }

    /// Signed comparisons match `BigInt` ordering. For two values of the *same*
    /// width, subtracting `2^width` is monotonic, so signed and unsigned order
    /// agree whenever the signs agree and disagree exactly when they differ.
    #[test]
    fn signed_comparisons_match_the_oracle((a, b) in arb_pair()) {
        let (x, y) = (to_bigint(&a), to_bigint(&b));
        prop_assert_eq!(a.slt(&b).unwrap(), x < y);
        prop_assert_eq!(a.sle(&b).unwrap(), x <= y);
        prop_assert_eq!(a.sgt(&b).unwrap(), x > y);
        prop_assert_eq!(a.sge(&b).unwrap(), x >= y);
        prop_assert_eq!(
            a.slt(&b).unwrap() == a.ult(&b).unwrap(),
            a.is_negative() == b.is_negative()
        );
    }

    /// `sll` and `srl` match the oracle and saturate at or beyond the width.
    #[test]
    fn logical_shifts_match_the_oracle(a in arb_bits(), raw_by in any::<u32>()) {
        let width = a.width();
        let by = raw_by % (width + 8);
        let x = to_biguint(&a);
        let left = a.sll(by).unwrap();
        prop_assert_eq!(left.width(), width);
        prop_assert_eq!(to_biguint(&left), (&x << by) % &modulus(width));
        let right = a.srl(by).unwrap();
        prop_assert_eq!(to_biguint(&right), &x >> by);
    }

    /// `sra` matches `BigInt`'s arithmetic shift, i.e. floor division by 2^by,
    /// and saturates to the sign bit for shifts at or beyond the width.
    #[test]
    fn arithmetic_shift_matches_the_oracle_and_saturates(a in arb_bits(), raw_by in any::<u32>()) {
        let width = a.width();
        let by = raw_by % (width + 8);
        let shifted = a.sra(by).unwrap();
        prop_assert_eq!(shifted.width(), width);
        prop_assert_eq!(to_bigint(&shifted), to_bigint(&a) >> by);
        if by >= width {
            prop_assert_eq!(shifted.is_negative(), a.is_negative());
        }
    }

    /// Splitting a value at any point and concatenating the halves back is the
    /// identity. The split at 0 and at `width` has an empty side, which has no
    /// zero-width representation, so those cases are the value itself.
    #[test]
    fn select_and_cat_round_trip(a in arb_bits(), first in any::<u32>()) {
        let width = a.width();
        let split = first % (width + 1);
        let rejoined = if split == 0 || split == width {
            a.clone()
        } else {
            let low = a.select(0, split).unwrap();
            let high = a.select(split, width - split).unwrap();
            cat(&[&high, &low]).unwrap()
        };
        prop_assert_eq!(rejoined, a.clone());
    }

    /// Each half carries exactly the bits it should. Both halves are non-empty, so
    /// the split stays in `1..width`.
    #[test]
    fn select_pieces_carry_the_right_bits(a in arb_bits(), first in any::<u32>()) {
        let width = a.width();
        prop_assume!(width >= 2);
        let split = 1 + first % (width - 1);
        let low = a.select(0, split).unwrap();
        let high = a.select(split, width - split).unwrap();
        for i in 0..split {
            prop_assert_eq!(low.bit(i).unwrap(), a.bit(i).unwrap());
        }
        for i in 0..(width - split) {
            prop_assert_eq!(high.bit(i).unwrap(), a.bit(split + i).unwrap());
        }
    }

    /// `select` takes an arbitrary window, not just the halves.
    #[test]
    fn select_takes_an_arbitrary_window(a in arb_bits(), raw_offset in any::<u32>(), raw_len in any::<u32>()) {
        let width = a.width();
        let offset = raw_offset % width;
        let len = 1 + raw_len % (width - offset);
        let piece = a.select(offset, len).unwrap();
        prop_assert_eq!(piece.width(), len);
        let expected = (&to_biguint(&a) >> offset) % &modulus(len);
        prop_assert_eq!(to_biguint(&piece), expected);
    }

    /// Zero extension, sign extension and truncation agree with the oracle, and
    /// sign extension matches `from_bigint`.
    #[test]
    fn resizing_agrees_with_the_oracle(a in arb_bits(), raw_extra in any::<u32>()) {
        let width = a.width();
        let extra = 1 + raw_extra % 96;
        let new_width = width + extra;

        let zeroed = a.zero_extend(new_width).unwrap();
        prop_assert_eq!(zeroed.width(), new_width);
        prop_assert_eq!(to_biguint(&zeroed), to_biguint(&a));

        let signed = a.sign_extend(new_width).unwrap();
        prop_assert_eq!(signed.width(), new_width);
        prop_assert_eq!(signed, from_bigint(&to_bigint(&a), new_width));

        let narrowed_width = extra.min(width);
        let narrowed = a.truncate(narrowed_width).unwrap();
        prop_assert_eq!(
            to_biguint(&narrowed),
            to_biguint(&a) % &modulus(narrowed_width)
        );
    }

    /// `replicate` is `cat` of copies.
    #[test]
    fn replicate_matches_concatenation(a in arb_bits(), raw_times in any::<u32>()) {
        let times = 1 + raw_times % 4;
        let replicated = a.replicate(times).unwrap();
        prop_assert_eq!(replicated.width(), a.width() * times);
        let parts: Vec<Bits> = (0..times).map(|_| a.clone()).collect();
        let refs: Vec<&Bits> = parts.iter().collect();
        prop_assert_eq!(replicated, cat(&refs).unwrap());
    }

    /// Byte conversions round-trip, and `bit` agrees with the bytes.
    #[test]
    fn byte_round_trip_and_bit_agree(a in arb_bits()) {
        let bytes = a.to_bytes_le();
        prop_assert_eq!(bytes.len(), ferrite_lithic_bits::bytes_for_width(a.width()));
        prop_assert_eq!(Bits::from_bytes_le(a.width(), &bytes).unwrap(), a.clone());
        for i in 0..a.width() {
            let expected = bytes[(i / 8) as usize] & (1u8 << (i % 8)) != 0;
            prop_assert_eq!(a.bit(i).unwrap(), expected);
        }
    }

    /// `to_u64` agrees with the oracle whenever the width allows it.
    #[test]
    fn to_u64_matches_the_oracle(a in arb_bits()) {
        if a.width() <= 64 {
            prop_assert_eq!(
                a.to_u64().unwrap(),
                to_biguint(&a).to_u64().expect("fits in 64 bits")
            );
        } else {
            prop_assert!(a.to_u64().is_err());
        }
    }

    /// The zero-upper-bits invariant survives arbitrary construction and every
    /// operation applied to it.
    #[test]
    fn the_zero_upper_bits_invariant_always_holds((a, b) in arb_pair()) {
        prop_assert!(invariant_holds(&a));
        prop_assert!(invariant_holds(&a.add(&b).unwrap()));
        prop_assert!(invariant_holds(&a.sub(&b).unwrap()));
        prop_assert!(invariant_holds(&a.mul(&b).unwrap()));
        prop_assert!(invariant_holds(&a.not().unwrap()));
        prop_assert!(invariant_holds(&a.neg().unwrap()));
        prop_assert!(invariant_holds(&a.sll(1).unwrap()));
        prop_assert!(invariant_holds(&a.srl(1).unwrap()));
        prop_assert!(invariant_holds(&a.sra(1).unwrap()));
    }

    /// Constructors are well formed at every width, including the awkward ones.
    #[test]
    fn constructors_are_well_formed(width in arb_width()) {
        let ones = Bits::ones(width).unwrap();
        for value in [zero(width), ones.clone(), Bits::constant(u64::MAX, width).unwrap()] {
            prop_assert_eq!(value.width(), width);
            prop_assert_eq!(value.word_count(), ferrite_lithic_bits::words_for_width(width));
            prop_assert!(invariant_holds(&value));
        }
        let round_tripped =
            Bits::from_bytes_le(width, &ones.to_bytes_le()).unwrap();
        prop_assert_eq!(round_tripped, ones);
    }

    /// Ordering is by width first, then by unsigned value, so a narrower value is
    /// always "less than" a wider one regardless of value.
    #[test]
    fn ordering_is_width_then_value(a in arb_bits(), b in arb_bits()) {
        match a.width().cmp(&b.width()) {
            std::cmp::Ordering::Less => {
                prop_assert!(a < b);
                prop_assert!(a.lt(&b));
            }
            std::cmp::Ordering::Greater => {
                prop_assert!(a > b);
            }
            std::cmp::Ordering::Equal => {
                prop_assert_eq!(a.cmp(&b), to_biguint(&a).cmp(&to_biguint(&b)));
            }
        }
    }

    /// `mul` followed by `select` is the truncating product: `mul` does not
    /// truncate, asking for the low half does. This is the explicit truncation
    /// story the design rests on.
    #[test]
    fn mul_then_select_is_the_truncating_product(a in arb_bits()) {
        let width = a.width();
        let product = a.mul(&a).unwrap();
        let low = product.select(0, width).unwrap();
        let expected = (&to_biguint(&a) * &to_biguint(&a)) % &modulus(width);
        prop_assert_eq!(to_biguint(&low), expected);
    }
}
