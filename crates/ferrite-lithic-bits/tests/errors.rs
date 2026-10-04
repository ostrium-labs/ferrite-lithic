//! Error message quality.
//!
//! [ADR-0003](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0003-runtime-widths-not-const-generics.md)
//! trades compile-time width checking for runtime widths, and requires the error
//! message to carry the compensation a compiler would have provided. These tests
//! are the enforcement mechanism for that requirement: if someone shortens a
//! message to "width mismatch", this file fails.
//!
//! The rule every width error must satisfy: **name both widths, and name the
//! operation that fixes it.**

#![allow(clippy::unwrap_used)]

mod common;

use common::to_biguint;
use ferrite_lithic_bits::{Bits, Error, Op};

fn narrow() -> Bits {
    Bits::constant(0b1010, 8).unwrap()
}

fn wide() -> Bits {
    Bits::constant(0b1010, 16).unwrap()
}

/// A binary operation that requires equal widths, reduced to `()`.
type EqualWidthBinop = (&'static str, fn(&Bits, &Bits) -> Result<(), Error>);

/// Every binary operation that requires equal widths, for exhaustive checking.
fn equal_width_binops() -> Vec<EqualWidthBinop> {
    vec![
        ("add", |a, b| a.add(b).map(|_| ())),
        ("sub", |a, b| a.sub(b).map(|_| ())),
        ("and", |a, b| a.and(b).map(|_| ())),
        ("or", |a, b| a.or(b).map(|_| ())),
        ("xor", |a, b| a.xor(b).map(|_| ())),
        ("udiv", |a, b| a.udiv(b).map(|_| ())),
        ("sdiv", |a, b| a.sdiv(b).map(|_| ())),
        ("urem", |a, b| a.urem(b).map(|_| ())),
        ("srem", |a, b| a.srem(b).map(|_| ())),
        ("eq", |a, b| a.eq(b).map(|_| ())),
        ("ult", |a, b| a.ult(b).map(|_| ())),
        ("ule", |a, b| a.ule(b).map(|_| ())),
        ("ugt", |a, b| a.ugt(b).map(|_| ())),
        ("uge", |a, b| a.uge(b).map(|_| ())),
        ("slt", |a, b| a.slt(b).map(|_| ())),
        ("sle", |a, b| a.sle(b).map(|_| ())),
        ("sgt", |a, b| a.sgt(b).map(|_| ())),
        ("sge", |a, b| a.sge(b).map(|_| ())),
    ]
}

#[test]
fn every_equal_width_binop_names_both_widths_and_a_fix() {
    for (name, op) in equal_width_binops() {
        let error = op(&narrow(), &wide()).expect_err(name);
        let message = error.to_string();
        assert!(message.contains(name), "{name}: {message}");
        assert!(message.contains("8 bits"), "{name} must name 8: {message}");
        assert!(
            message.contains("16 bits"),
            "{name} must name 16: {message}"
        );
        assert!(
            message.contains("zero_extend")
                || message.contains("truncate")
                || message.contains("sign_extend"),
            "{name} must name a fix: {message}"
        );
    }
}

#[test]
fn the_error_itself_carries_both_widths_structurally() {
    for (_, op) in equal_width_binops() {
        match op(&narrow(), &wide()).expect_err("mismatch") {
            Error::WidthMismatch { op, lhs, rhs } => {
                assert_eq!(lhs, 8);
                assert_eq!(rhs, 16);
                assert!(!op.name().is_empty());
            }
            other => panic!("expected WidthMismatch, got {other:?}"),
        }
    }
}

#[test]
fn mul_is_the_operation_that_does_not_need_equal_widths() {
    // Unequal widths must be fine for `mul`, which widens. If this ever starts
    // failing, the widening behaviour has regressed.
    let product = narrow().mul(&wide()).unwrap();
    assert_eq!(product.width(), 24);
    assert_eq!(
        to_biguint(&product),
        &to_biguint(&narrow()) * &to_biguint(&wide())
    );
}

#[test]
fn resizing_in_the_wrong_direction_says_which_direction_is_wrong() {
    // Legal resizes in both directions succeed.
    assert_eq!(narrow().truncate(4).unwrap().width(), 4);
    assert_eq!(narrow().zero_extend(24).unwrap().width(), 24);
    assert_eq!(narrow().sign_extend(24).unwrap().width(), 24);

    // Widening via `truncate` is refused, and the message says to widen properly.
    let widen_by_truncating = narrow().truncate(16).unwrap_err().to_string();
    assert!(
        widen_by_truncating.contains("truncate"),
        "{widen_by_truncating}"
    );
    assert!(
        widen_by_truncating.contains("zero_extend"),
        "{widen_by_truncating}"
    );
    assert!(
        widen_by_truncating.contains("8 bits"),
        "{widen_by_truncating}"
    );
    assert!(
        widen_by_truncating.contains("16 bits"),
        "{widen_by_truncating}"
    );

    // Narrowing via `zero_extend` is refused, and names both widths.
    let narrow_by_extending = wide().zero_extend(4).unwrap_err().to_string();
    assert!(
        narrow_by_extending.contains("16 bits"),
        "{narrow_by_extending}"
    );
    assert!(
        narrow_by_extending.contains("4 bits"),
        "{narrow_by_extending}"
    );
    assert!(
        narrow_by_extending.contains("zero_extend"),
        "{narrow_by_extending}"
    );
}

#[test]
fn a_zero_width_construct_explains_why_zero_is_refused() {
    let message = Bits::zeros(0).unwrap_err().to_string();
    assert!(message.contains("at least 1 bit"), "{message}");
    assert!(message.contains("select"), "{message}");
}

#[test]
fn an_out_of_range_select_shows_the_arithmetic() {
    let message = narrow().select(6, 4).unwrap_err().to_string();
    assert!(message.contains("6"), "{message}");
    assert!(message.contains("4"), "{message}");
    assert!(message.contains("8 bits"), "{message}");
    assert!(message.contains("zero_extend"), "{message}");
    assert!(
        message.contains("12") || message.contains("6 + 4"),
        "the message should show the sum: {message}"
    );
}

#[test]
fn division_by_zero_points_at_a_workaround_rather_than_a_number() {
    let zero = Bits::zeros(8).unwrap();
    for error in [
        narrow().udiv(&zero).unwrap_err(),
        narrow().sdiv(&zero).unwrap_err(),
        narrow().urem(&zero).unwrap_err(),
        narrow().srem(&zero).unwrap_err(),
    ] {
        let message = error.to_string();
        assert!(message.contains("no defined result"), "{message}");
        assert!(message.contains("8 bits"), "{message}");
        assert!(message.contains("constant"), "{message}");
    }
}

#[test]
fn signed_overflow_names_the_operation_and_the_one_bit_widening() {
    let minimum = Bits::constant(0x80, 8).unwrap();
    let minus_one = Bits::constant(0xff, 8).unwrap();
    let message = minimum.sdiv(&minus_one).unwrap_err().to_string();
    assert!(message.contains("sdiv"), "{message}");
    assert!(message.contains("8 bits"), "{message}");
    assert!(message.contains("9"), "{message}");
    assert!(message.contains("zero_extend"), "{message}");
    // srem cannot overflow, so it must still work and give zero.
    assert_eq!(minimum.srem(&minus_one).unwrap().to_u64().unwrap(), 0);
}

#[test]
fn a_too_wide_conversion_suggests_the_word_level_escape_hatch() {
    // 16 bits still fits, so the interesting case needs more than 64.
    assert!(wide().to_u64().is_ok());
    let very_wide = Bits::constant(0, 128).unwrap();
    let message = very_wide.to_u64().unwrap_err().to_string();
    assert!(message.contains("128 bits"), "{message}");
    assert!(message.contains("64 bits"), "{message}");
    assert!(message.contains("word"), "{message}");
}

#[test]
fn a_wrong_length_byte_buffer_is_called_out_exactly() {
    let message = Bits::from_bytes_le(16, &[0u8; 3]).unwrap_err().to_string();
    assert!(message.contains('2'), "{message}");
    assert!(message.contains('3'), "{message}");
}

#[test]
fn an_out_of_range_bit_index_shows_the_valid_range() {
    let message = narrow().bit(8).unwrap_err().to_string();
    assert!(message.contains("bit 8"), "{message}");
    assert!(message.contains('7'), "{message}");
}

#[test]
fn cat_with_no_parts_is_refused_with_a_way_forward() {
    let message = ferrite_lithic_bits::cat(&[]).unwrap_err().to_string();
    assert!(message.contains("at least one"), "{message}");
    assert!(message.contains("zeros"), "{message}");
}

#[test]
fn every_op_renders_its_dsl_spelling_in_errors() {
    // Spot-check that the Op carried in an error is the one the user called.
    let mismatch = narrow().add(&wide()).unwrap_err();
    assert!(mismatch.to_string().contains(Op::Add.name()), "{mismatch}");
    let zero_width = narrow().select(0, 0).unwrap_err();
    assert!(
        zero_width.to_string().contains(Op::Select.name()),
        "{zero_width}"
    );
    let out_of_range = narrow().select(9, 4).unwrap_err();
    assert!(
        out_of_range.to_string().contains("select"),
        "{out_of_range}"
    );
}
