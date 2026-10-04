//! The structural operations, checked against the `bits` crate's semantics.
//!
//! A DSL that desugars `srl` into `select` plus `cat` plus a constant can be
//! wrong in a way no width test would notice: the widths all come out right and
//! the bits are wrong. The only check that catches that is to evaluate the
//! graph, which `-sim` will eventually do properly. Until then this file
//! evaluates the handful of node kinds the structural operations are built from,
//! over random inputs, and compares against `Bits`.
//!
//! The interpreter is deliberately partial: it handles only what
//! `sll`/`srl`/`sra`/`zero_extend`/`sign_extend`/`truncate`/`concat`/`replicate`
//! emit. Anything else is an error rather than a wrong answer.

#![allow(clippy::unwrap_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{Circuit, Node, NodeId};
use proptest::prelude::*;

/// Folds a graph of constants into a `Bits`.
fn fold(circuit: &Circuit, id: NodeId) -> Result<Bits, String> {
    match circuit.get(id).map_err(|e| e.to_string())? {
        Node::Constant { value } => Ok(value.clone()),
        Node::Not { arg } => Ok(fold(circuit, *arg)?.not().map_err(|e| e.to_string())?),
        Node::Add { left, right } => fold(circuit, *left)?
            .add(&fold(circuit, *right)?)
            .map_err(|e| e.to_string()),
        Node::Mul { left, right } => fold(circuit, *left)?
            .mul(&fold(circuit, *right)?)
            .map_err(|e| e.to_string()),
        Node::Select {
            value, offset, len, ..
        } => fold(circuit, *value)?
            .select(*offset, *len)
            .map_err(|e| e.to_string()),
        Node::Cat { high, low } => {
            let high = fold(circuit, *high)?;
            let low = fold(circuit, *low)?;
            ferrite_lithic_bits::cat(&[&high, &low]).map_err(|e| e.to_string())
        }
        Node::Replicate { value, count } => fold(circuit, *value)?
            .replicate(*count)
            .map_err(|e| e.to_string()),
        other => Err(format!("the interpreter cannot fold a {}", other.kind())),
    }
}

/// A `Bits` and the width it should be built at.
fn lit(value: u64, width: u32) -> Bits {
    Bits::constant(value, width).unwrap()
}

proptest! {
    #[test]
    fn a_left_shift_matches_the_bits_crate(
        value in any::<u64>(),
        width in 1u32..=64,
        by in any::<u32>(),
    ) {
        let input = lit(value, width);
        let expected = input.sll(by).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let signal = d.lit(value, width).unwrap();
        let shifted = d.sll(&signal, by).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(shifted.width(), expected.width());
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, shifted.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }

    #[test]
    fn a_logical_right_shift_matches_the_bits_crate(
        value in any::<u64>(),
        width in 1u32..=64,
        by in any::<u32>(),
    ) {
        let input = lit(value, width);
        let expected = input.srl(by).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let signal = d.lit(value, width).unwrap();
        let shifted = d.srl(&signal, by).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(shifted.width(), expected.width());
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, shifted.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }

    #[test]
    fn an_arithmetic_right_shift_matches_the_bits_crate(
        value in any::<u64>(),
        width in 1u32..=64,
        by in any::<u32>(),
    ) {
        let input = lit(value, width);
        let expected = input.sra(by).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let signal = d.lit(value, width).unwrap();
        let shifted = d.sra(&signal, by).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(shifted.width(), expected.width());
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, shifted.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }

    #[test]
    fn a_zero_extension_matches_the_bits_crate(
        value in any::<u64>(),
        width in 1u32..=64,
        extra in any::<u32>(),
    ) {
        let new_width = u64::from(width) + u64::from(extra);
        let new_width = u32::try_from(new_width.min(128)).unwrap();
        let input = lit(value, width);
        let expected = input
            .zero_extend(new_width)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let signal = d.lit(value, width).unwrap();
        let wide = d.zero_extend(&signal, new_width)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, wide.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }

    #[test]
    fn a_sign_extension_matches_the_bits_crate(
        value in any::<u64>(),
        width in 1u32..=64,
        extra in any::<u32>(),
    ) {
        let new_width = u64::from(width) + u64::from(extra);
        let new_width = u32::try_from(new_width.min(128)).unwrap();
        let input = lit(value, width);
        let expected = input
            .sign_extend(new_width)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let signal = d.lit(value, width).unwrap();
        let wide = d.sign_extend(&signal, new_width)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, wide.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }

    #[test]
    fn a_truncation_matches_the_bits_crate(
        value in any::<u64>(),
        width in 1u32..=64,
        new_width in 1u32..=64,
    ) {
        prop_assume!(new_width <= width);
        let input = lit(value, width);
        let expected = input
            .truncate(new_width)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let signal = d.lit(value, width).unwrap();
        let narrow = d.truncate(&signal, new_width)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, narrow.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }

    #[test]
    fn a_slice_matches_the_bits_crate(
        value in any::<u64>(),
        // The width is derived from the window rather than generated alongside it:
        // three independent ranges with an `assume` reject almost every case.
        (offset, len) in (0u32..32, 1u32..16),
    ) {
        let width = offset + len;
        let input = lit(value, width);
        let expected = input
            .select(offset, len)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let signal = d.lit(value, width).unwrap();
        let sliced = d
            .slice(&signal, offset, len)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, sliced.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }

    #[test]
    fn a_concatenation_matches_the_bits_crate(
        high in any::<u64>(),
        low in any::<u64>(),
        high_width in 1u32..=40,
        low_width in 1u32..=40,
    ) {
        let expected = ferrite_lithic_bits::cat(&[&lit(high, high_width), &lit(low, low_width)])
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let both = d
            .concat(&[
                d.lit(high, high_width).unwrap(),
                d.lit(low, low_width).unwrap(),
            ])
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(both.width(), expected.width());
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, both.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }

    #[test]
    fn a_replication_matches_the_bits_crate(
        value in any::<u64>(),
        width in 1u32..=20,
        count in 1u32..=6,
    ) {
        let input = lit(value, width);
        let expected = input
            .replicate(count)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let d = Design::new();
        let signal = d.lit(value, width).unwrap();
        let repeated = d
            .replicate(&signal, count)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(repeated.width(), expected.width());
        let circuit = d.build().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = fold(&circuit, repeated.id()).map_err(TestCaseError::fail)?;
        prop_assert_eq!(got.to_hex(), expected.to_hex());
    }
}

/// A three-way concat is the case a binary fold could get backwards, so it is
/// checked explicitly rather than left to the random single-cat cases.
#[test]
fn a_three_way_concatenation_keeps_its_order() {
    let d = Design::new();
    let all = d
        .concat(&[
            d.lit(0xa, 4).unwrap(),
            d.lit(0xb, 4).unwrap(),
            d.lit(0xc, 4).unwrap(),
        ])
        .unwrap();
    let circuit = d.build().unwrap();
    assert_eq!(all.width(), 12);
    assert_eq!(
        fold(&circuit, all.id()).unwrap().to_hex(),
        lit(0xabc, 12).to_hex()
    );
}

#[test]
fn a_five_way_concatenation_keeps_its_order() {
    let d = Design::new();
    let all = d
        .concat(&[
            d.lit(1, 4).unwrap(),
            d.lit(2, 4).unwrap(),
            d.lit(3, 4).unwrap(),
            d.lit(4, 4).unwrap(),
            d.lit(5, 4).unwrap(),
        ])
        .unwrap();
    let circuit = d.build().unwrap();
    assert_eq!(all.width(), 20);
    assert_eq!(
        fold(&circuit, all.id()).unwrap().to_hex(),
        lit(0x12345, 20).to_hex()
    );
}
