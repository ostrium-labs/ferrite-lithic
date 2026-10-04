//! Exhaustive small-width verification against the `num-bigint` oracle.
//!
//! Where `properties.rs` samples widths randomly, this enumerates **every** value
//! of a width and every operand pair, up to width 6. It is the test that actually
//! caught two bugs the random properties initially missed, both of which only
//! appear on specific operand pairs:
//!
//! - `is_ones` compared each raw word against its *masked* form, which the
//!   zero-upper-bits invariant makes always equal, so it returned `true` for every
//!   value and silently disabled the signed-division overflow guard.
//! - `cmp_signed` reversed the comparison for two negative operands, putting `-1`
//!   below `-2^128`. Subtracting `2^width` is monotonic, so it must not reverse.

#![allow(clippy::unwrap_used)]

mod common;

use common::{to_bigint, to_biguint};
use ferrite_lithic_bits::Bits;

/// Every value of a given width, in ascending order.
fn all_values(width: u32) -> Vec<Bits> {
    let count = 1u64 << width;
    (0..count)
        .map(|v| Bits::constant(v, width).unwrap())
        .collect()
}

#[test]
fn exhaustive_unsigned_division_up_to_width_6() {
    for width in 1..=6u32 {
        let values = all_values(width);
        for a in &values {
            for b in &values {
                if b.is_zero() {
                    continue;
                }
                let (x, y) = (to_biguint(a), to_biguint(b));
                let got_q = to_biguint(&a.udiv(b).unwrap());
                let got_r = to_biguint(&a.urem(b).unwrap());
                assert_eq!(got_q, &x / &y, "udiv {a:?} {b:?}");
                assert_eq!(got_r, &x % &y, "urem {a:?} {b:?}");
            }
        }
    }
}

#[test]
fn exhaustive_signed_division_up_to_width_6() {
    for width in 1..=6u32 {
        let values = all_values(width);
        for a in &values {
            for b in &values {
                if b.is_zero() {
                    continue;
                }
                if a.is_min_signed() && b.is_all_ones() {
                    assert!(a.sdiv(b).is_err(), "sdiv {a:?} {b:?} must refuse");
                    continue;
                }
                let (x, y) = (to_bigint(a), to_bigint(b));
                let got_q = to_bigint(&a.sdiv(b).unwrap());
                let got_r = to_bigint(&a.srem(b).unwrap());
                assert_eq!(got_q, &x / &y, "sdiv {a:?} / {b:?}");
                assert_eq!(got_r, &x % &y, "srem {a:?} % {b:?}");
            }
        }
    }
}

#[test]
fn exhaustive_shifts_up_to_width_6() {
    for width in 1..=6u32 {
        let values = all_values(width);
        let modulus = num_bigint::BigUint::from(1u8) << width;
        for a in &values {
            let x = to_biguint(a);
            let s = to_bigint(a);
            for by in 0..=width + 2 {
                // sll keeps the width, so bits shifted past the top are dropped.
                assert_eq!(
                    to_biguint(&a.sll(by).unwrap()),
                    (&x << by) % &modulus,
                    "sll {a:?} << {by}"
                );
                assert_eq!(
                    to_biguint(&a.srl(by).unwrap()),
                    &x >> by,
                    "srl {a:?} >> {by}"
                );
                assert_eq!(
                    to_bigint(&a.sra(by).unwrap()),
                    &s >> by,
                    "sra {a:?} >>> {by}"
                );
            }
        }
    }
}

#[test]
fn exhaustive_multiply_up_to_width_5() {
    for width in 1..=5u32 {
        let values = all_values(width);
        for a in &values {
            for b in &values {
                let product = a.mul(b).unwrap();
                assert_eq!(product.width(), width * 2);
                assert_eq!(
                    to_biguint(&product),
                    &to_biguint(a) * &to_biguint(b),
                    "mul {a:?} * {b:?}"
                );
            }
        }
    }
}

#[test]
fn exhaustive_resize_up_to_width_6() {
    for width in 1..=6u32 {
        for a in all_values(width) {
            for new_width in width..=width + 70 {
                let extended = a.sign_extend(new_width).unwrap();
                assert_eq!(extended.width(), new_width);
                // The contract is bit level: the original bits are untouched and
                // everything above them copies the sign bit.
                for i in 0..width {
                    assert_eq!(
                        extended.bit(i).unwrap(),
                        a.bit(i).unwrap(),
                        "sign_extend {a:?} to {new_width} bit {i}"
                    );
                }
                for i in width..new_width {
                    assert_eq!(
                        extended.bit(i).unwrap(),
                        a.is_negative(),
                        "sign_extend {a:?} to {new_width} bit {i}"
                    );
                }
                // Zero extension changes no bits at all.
                let zeroed = a.zero_extend(new_width).unwrap();
                assert_eq!(zeroed.width(), new_width);
                assert_eq!(to_biguint(&zeroed), to_biguint(&a), "zero_extend {a:?}");
                // And where the magnitude fits, the signed value is preserved.
                let signed = to_bigint(&a);
                if signed.magnitude().bits() < u64::from(new_width) {
                    assert_eq!(
                        to_bigint(&extended),
                        signed,
                        "sign_extend {a:?} to {new_width} changed the value"
                    );
                }
            }
        }
    }
}

#[test]
fn exhaustive_select_and_cat_up_to_width_6() {
    for width in 1..=6u32 {
        for a in all_values(width) {
            for offset in 0..width {
                for len in 1..=(width - offset) {
                    let piece = a.select(offset, len).unwrap();
                    assert_eq!(piece.width(), len);
                    let expected =
                        (to_biguint(&a) >> offset) % (num_bigint::BigUint::from(1u8) << len);
                    assert_eq!(
                        to_biguint(&piece),
                        expected,
                        "select {a:?} [{offset} +{len}]"
                    );
                }
            }
        }
    }
}

#[test]
fn is_min_signed_agrees_with_the_oracle_at_every_small_width() {
    for width in 1..=8u32 {
        let modulus = num_bigint::BigUint::from(1u8) << width;
        let minimum = &modulus >> 1;
        for a in all_values(width) {
            assert_eq!(
                a.is_min_signed(),
                to_biguint(&a) == minimum,
                "is_min_signed {a:?} at width {width}"
            );
            assert_eq!(
                a.is_negative(),
                to_biguint(&a) >= minimum,
                "is_negative {a:?} at width {width}"
            );
        }
    }
}

/// `is_ones` and `is_zero` decide whether signed division overflows, so a bug in
/// either one silently changes which divisions are refused. `is_ones` in
/// particular once compared each raw word against its *masked* form, which the
/// zero-upper-bits invariant makes always equal.
#[test]
fn is_ones_and_is_zero_agree_with_the_oracle_at_every_small_width() {
    for width in 1..=8u32 {
        let modulus = num_bigint::BigUint::from(1u8) << width;
        let all_ones = &modulus - num_bigint::BigUint::from(1u8);
        for a in all_values(width) {
            let value = to_biguint(&a);
            assert_eq!(a.is_ones(), value == all_ones, "is_ones {a:?} at {width}");
            assert_eq!(
                a.is_all_ones(),
                value == all_ones,
                "is_all_ones {a:?} at {width}"
            );
            assert_eq!(
                a.is_zero(),
                value == num_bigint::BigUint::from(0u8),
                "is_zero {a:?} at {width}"
            );
        }
        assert!(Bits::ones(width).unwrap().is_ones());
        assert!(Bits::ones(width).unwrap().is_all_ones());
    }
}

/// The signed-division overflow guard must fire on exactly the pairs it claims to:
/// the minimum value over `-1`, and nothing else.
#[test]
fn signed_division_overflow_fires_exactly_on_min_over_minus_one() {
    for width in 1..=8u32 {
        let modulus = num_bigint::BigUint::from(1u8) << width;
        let minimum = num_bigint::BigUint::from(1u8) << (width - 1);
        // -1 is all ones, which is the same as the minimum only at width 1.
        let minus_one = &modulus - num_bigint::BigUint::from(1u8);
        for a in all_values(width) {
            for b in all_values(width) {
                if b.is_zero() {
                    assert!(a.sdiv(&b).is_err());
                    continue;
                }
                let should_overflow = to_biguint(&a) == minimum && to_biguint(&b) == minus_one;
                assert_eq!(
                    a.sdiv(&b).is_err(),
                    should_overflow,
                    "sdiv overflow guard at width {width}: {} / {}",
                    a.to_hex(),
                    b.to_hex()
                );
            }
        }
    }
}
