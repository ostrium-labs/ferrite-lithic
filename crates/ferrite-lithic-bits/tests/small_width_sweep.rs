//! The explicit small-width sweep.
//!
//! Hardcaml's multiply assumes its operands span at least two words, with no
//! assertion to that effect — [`kernel/bits.ml:449`](https://github.com/jane-street/hardcaml)
//! and `smul_core` rely on it, and upstream covers it only with a width `1..40`
//! sweep whose comment calls the concern out
//! ([`test/lib/test_bits.ml:285`](https://github.com/jane-street/hardcaml)).
//!
//! Our `Vec<u64>` carries no such assumption, so the sweep is no longer strictly
//! required for correctness. It is kept anyway, because it is cheap and it is the
//! exact test that would catch a regression if someone ever "optimised" `mul`
//! into a fixed two-word fast path.
//!
//! Unlike the randomised properties, this is deterministic: a fixed seed LCG
//! generates the values, so a failure reproduces exactly.

#![allow(clippy::unwrap_used)]

mod common;

use common::{Lcg, from_biguint, modulus, to_biguint, zero};
use ferrite_lithic_bits::Bits;
use num_bigint::BigUint;

/// Operand patterns that have historically broken word-level arithmetic:
/// all-ones carries, alternating patterns, isolated high bits, and the signed
/// extremes.
fn edge_patterns(width: u32) -> Vec<BigUint> {
    let full = &modulus(width) - BigUint::from(1u8);
    let half = width / 2;
    let high_bit = BigUint::from(1u8) << (width - 1);
    let alternating = {
        // 0b1010... or 0b0101... depending on width parity.
        let mut v = BigUint::from(0u8);
        for i in 0..width {
            if i % 2 == 0 {
                v |= BigUint::from(1u8) << i;
            }
        }
        v
    };
    vec![
        BigUint::from(0u8),
        BigUint::from(1u8),
        full.clone(),
        &full - BigUint::from(1u8),
        high_bit.clone(),
        &high_bit - BigUint::from(1u8),
        alternating.clone(),
        full.clone() - &alternating,
        BigUint::from(1u8) << half,
        &full >> half,
    ]
}

fn all_patterns(width: u32) -> Vec<Bits> {
    let mut values: Vec<Bits> = edge_patterns(width)
        .iter()
        .map(|v| from_biguint(v, width))
        .collect();
    let mut rng = Lcg::new(0x5eed_0000 + u64::from(width));
    for _ in 0..24 {
        values.push(rng.bits(width));
    }
    values.push(Bits::ones(width).expect("width >= 1"));
    values.push(zero(width));
    values
}

#[test]
fn multiply_is_exact_at_every_width_from_1_to_40() {
    for width in 1..=40u32 {
        let values = all_patterns(width);
        for a in &values {
            for b in &values {
                let product = a.mul(b).unwrap();
                assert_eq!(
                    product.width(),
                    width * 2,
                    "mul must widen to wa + wb at width {width}"
                );
                assert_eq!(
                    to_biguint(&product),
                    &to_biguint(a) * &to_biguint(b),
                    "mul mismatch at width {width}: {} * {}",
                    a.to_hex(),
                    b.to_hex()
                );
            }
        }
    }
}

#[test]
fn add_and_sub_agree_with_the_oracle_at_every_width_from_1_to_40() {
    for width in 1..=40u32 {
        let values = all_patterns(width);
        let m = modulus(width);
        for a in &values {
            for b in &values {
                let sum = a.add(b).unwrap();
                assert_eq!(sum.width(), width);
                assert_eq!(
                    to_biguint(&sum),
                    (&to_biguint(a) + &to_biguint(b)) % &m,
                    "add mismatch at width {width}"
                );
                let difference = a.sub(b).unwrap();
                let (x, y) = (to_biguint(a), to_biguint(b));
                let expected = if x >= y { &x - &y } else { &m - (&y - &x) };
                assert_eq!(
                    to_biguint(&difference),
                    expected,
                    "sub mismatch at width {width}"
                );
            }
        }
    }
}

#[test]
fn division_agrees_with_the_oracle_at_every_width_from_1_to_40() {
    for width in 1..=40u32 {
        let values = all_patterns(width);
        for a in &values {
            for b in &values {
                if b.is_zero() {
                    assert!(a.udiv(b).is_err(), "division by zero must be refused");
                    assert!(a.urem(b).is_err(), "division by zero must be refused");
                    continue;
                }
                let (x, y) = (to_biguint(a), to_biguint(b));
                assert_eq!(to_biguint(&a.udiv(b).unwrap()), &x / &y, "udiv at {width}");
                assert_eq!(to_biguint(&a.urem(b).unwrap()), &x % &y, "urem at {width}");
            }
        }
    }
}

#[test]
fn signed_division_agrees_with_the_oracle_at_every_width_from_1_to_40() {
    for width in 1..=40u32 {
        let values = all_patterns(width);
        for a in &values {
            for b in &values {
                if b.is_zero() {
                    continue;
                }
                // The most negative value over -1 is refused rather than wrapped.
                if a.is_min_signed() && b.is_all_ones() {
                    assert!(a.sdiv(b).is_err());
                    // srem cannot overflow, so it still succeeds and yields zero.
                    assert_eq!(to_biguint(&a.srem(b).unwrap()), BigUint::from(0u8));
                    continue;
                }
                let (x, y) = (common::to_bigint(a), common::to_bigint(b));
                assert_eq!(
                    common::to_bigint(&a.sdiv(b).unwrap()),
                    &x / &y,
                    "sdiv at width {width}: {} / {}",
                    a.to_hex(),
                    b.to_hex()
                );
                assert_eq!(
                    common::to_bigint(&a.srem(b).unwrap()),
                    &x % &y,
                    "srem at width {width}: {} % {}",
                    a.to_hex(),
                    b.to_hex()
                );
            }
        }
    }
}

#[test]
fn shifts_agree_with_the_oracle_at_every_width_from_1_to_40() {
    for width in 1..=40u32 {
        let values = all_patterns(width);
        let m = modulus(width);
        for a in &values {
            let x = to_biguint(a);
            let s = common::to_bigint(a);
            for by in 0..=width + 4 {
                assert_eq!(
                    to_biguint(&a.sll(by).unwrap()),
                    (&x << by) % &m,
                    "sll at width {width} by {by}"
                );
                assert_eq!(
                    to_biguint(&a.srl(by).unwrap()),
                    &x >> by,
                    "srl at width {width} by {by}"
                );
                assert_eq!(
                    common::to_bigint(&a.sra(by).unwrap()),
                    &s >> by,
                    "sra at width {width} by {by}"
                );
            }
        }
    }
}

#[test]
fn a_one_bit_value_is_a_full_word_but_does_not_behave_like_one() {
    // The property Hardcaml cannot state: at width 1 there is exactly one word and
    // no second word to borrow from, which is where a two-word multiply
    // assumption would read out of bounds.
    let one = Bits::ones(1).unwrap();
    assert_eq!(one.width(), 1);
    assert_eq!(one.word_count(), 1);
    assert_eq!(one.to_u64().unwrap(), 1);
    assert_eq!(to_biguint(&one.mul(&one).unwrap()), BigUint::from(1u8));
    assert_eq!(one.mul(&one).unwrap().width(), 2);
    assert_eq!(zero(1).mul(&one).unwrap().to_u64().unwrap(), 0);
    // Shifts keep the width, so at width 1 every shift of at least 1 saturates.
    assert_eq!(one.sll(1).unwrap().to_u64().unwrap(), 0);
    assert_eq!(one.srl(1).unwrap().to_u64().unwrap(), 0);
    assert_eq!(one.sra(1).unwrap().to_u64().unwrap(), 0b1); // saturated to the sign
}

#[test]
fn multiply_carries_propagate_past_the_second_word() {
    // (2^64 - 1)^2 needs 128 bits and its top word is non-zero, so a
    // implementation that stopped after two partial products would be wrong here.
    let max = Bits::constant(u64::MAX, 64).unwrap();
    let product = max.mul(&max).unwrap();
    assert_eq!(product.width(), 128);
    assert_eq!(product.word_count(), 2);
    let expected = &to_biguint(&max) * &to_biguint(&max);
    assert_eq!(to_biguint(&product), expected);
    assert!(product.word(1).unwrap() != 0);
}
