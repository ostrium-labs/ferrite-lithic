//! The two tiers must not drift apart.
//!
//! Each operator delegates to its builder, so the risk is not a wrong
//! implementation but a stale one. Every test here builds the same operation
//! twice, through each tier, and compares the resulting node's kind and width —
//! and where the graph shape is interesting, the shape too.

#![allow(clippy::unwrap_used)]

use std::panic::AssertUnwindSafe;

use ferrite_lithic::{Design, Signal};

/// Builds the same operation through both tiers and checks they agree.
fn agree(build: impl Fn(&Design, &Signal, &Signal) -> Signal, concise: Signal) -> Signal {
    let checked_design = Design::new();
    let a = checked_design.lit(0b1010_1010, 8).unwrap();
    let b = checked_design.lit(0x0f, 8).unwrap();
    let checked = build(&checked_design, &a, &b);

    assert_eq!(checked.kind(), concise.kind(), "kinds should match");
    assert_eq!(checked.width(), concise.width(), "widths should match");
    checked
}

#[test]
fn the_arithmetic_operators_agree_with_their_builders() {
    let d = Design::new();
    let a = d.lit(200, 8).unwrap();
    let b = d.lit(7, 8).unwrap();

    agree(|d, x, y| d.add(x, y).unwrap(), &a + &b);
    agree(|d, x, y| d.sub(x, y).unwrap(), &a - &b);
    agree(|d, x, y| d.mul(x, y).unwrap(), &a * &b);
    agree(|d, x, y| d.udiv(x, y).unwrap(), &a / &b);
    agree(|d, x, y| d.urem(x, y).unwrap(), &a % &b);
    agree(|d, x, y| d.and(x, y).unwrap(), &a & &b);
    agree(|d, x, y| d.or(x, y).unwrap(), &a | &b);
    agree(|d, x, y| d.xor(x, y).unwrap(), &a ^ &b);
}

#[test]
fn the_comparison_methods_agree_with_their_builders() {
    let d = Design::new();
    let a = d.lit(200, 8).unwrap();
    let b = d.lit(7, 8).unwrap();

    agree(|d, x, y| d.eq(x, y).unwrap(), a.equals(&b));
    agree(|d, x, y| d.ult(x, y).unwrap(), a.unsigned_lt(&b));
    agree(|d, x, y| d.ule(x, y).unwrap(), a.unsigned_le(&b));
    agree(|d, x, y| d.ugt(x, y).unwrap(), a.unsigned_gt(&b));
    agree(|d, x, y| d.uge(x, y).unwrap(), a.unsigned_ge(&b));
    agree(|d, x, y| d.slt(x, y).unwrap(), a.signed_lt(&b));
    agree(|d, x, y| d.sle(x, y).unwrap(), a.signed_le(&b));
    agree(|d, x, y| d.sgt(x, y).unwrap(), a.signed_gt(&b));
    agree(|d, x, y| d.sge(x, y).unwrap(), a.signed_ge(&b));
}

#[test]
fn negation_and_the_mux_agree_with_their_builders() {
    let d = Design::new();
    let a = d.lit(0b1010, 8).unwrap();
    let b = d.lit(0b0101, 8).unwrap();
    let condition = d.constant(true);

    assert_eq!((!&a).kind(), d.not(&a).kind());
    assert_eq!((!&a).width(), d.not(&a).width());

    let when = a.when(&condition, &b, &a);
    let ite = d.ite(&condition, &b, &a).unwrap();
    assert_eq!(when.kind(), ite.kind());
    assert_eq!(when.width(), ite.width());
}

#[test]
fn operators_do_not_consume_their_operands() {
    // A feedback loop reads one wire many times. If `+` took its operands by
    // value, every expression would force a clone.
    let d = Design::new();
    let acc = d.wire(8).unwrap();
    let one = d.lit(1, 8).unwrap();
    let two = &acc + &one;
    let three = &acc + &one;
    assert_ne!(two.id(), three.id());
    // `acc` is still usable, which is the point.
    assert_eq!(acc.width(), 8);
}

#[test]
fn operators_chain_left_to_right() {
    let d = Design::new();
    let a = d.lit(1, 8).unwrap();
    let b = d.lit(2, 8).unwrap();
    let c = d.lit(3, 8).unwrap();
    let chained = &a + &b + &c;
    assert_eq!(chained.kind(), "Add");
    assert_eq!(chained.width(), 8);
    // The intermediate is real: the outer add reads the inner one.
    let circuit = d.build().unwrap();
    let ferrite_lithic_ir::Node::Add { left, .. } = circuit.get(chained.id()).unwrap() else {
        panic!("expected an Add");
    };
    assert_eq!(circuit.get(*left).unwrap().kind(), "Add");
}

#[test]
fn a_width_error_panics_with_the_builders_own_message() {
    // The concise tier's whole cost is that it panics, so the message is the
    // contract: it must still name both widths and the fix, not just "called
    // `Option::unwrap()` on a `None` value".
    let d = Design::new();
    let wide = d.lit(1, 8).unwrap();
    let narrow = d.lit(1, 4).unwrap();

    let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = &wide + &narrow;
    }))
    .expect_err("a width mismatch should panic");
    let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(
        message.contains('8'),
        "should name the left width: {message}"
    );
    assert!(
        message.contains('4'),
        "should name the right width: {message}"
    );
    assert!(
        message.contains("add"),
        "should name the operation: {message}"
    );
}

#[test]
fn a_panic_points_at_the_caller_not_at_the_operator() {
    use std::sync::{Mutex, OnceLock};

    static LOCATION: OnceLock<Mutex<Option<(String, u32)>>> = OnceLock::new();
    let location = LOCATION.get_or_init(|| Mutex::new(None));

    // The panic *message* does not carry the location, so the hook has to. The
    // default hook already prints it, which is the visible half of this claim; this
    // is the half that fails if someone drops `#[track_caller]`.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(where_) = info.location() {
            *location.lock().expect("location mutex") =
                Some((where_.file().to_string(), where_.line()));
        }
    }));

    let d = Design::new();
    let wide = d.lit(1, 8).unwrap();
    let narrow = d.lit(1, 4).unwrap();

    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = &wide + &narrow;
    }))
    .expect_err("a width mismatch should panic");

    std::panic::set_hook(previous);
    let (file, line) = location
        .lock()
        .expect("location mutex")
        .clone()
        .expect("the panic hook should have recorded a location, which means it fired at all");
    // Which *file* is the claim, and unlike a line number it survives `rustfmt`.
    // Without `#[track_caller]` the reported location is inside `src/ops.rs`.
    assert!(
        file.ends_with("operators.rs"),
        "the panic should point at the caller's file, but it pointed at {file}:{line}"
    );
    assert!(line > 0, "a real location has a line number, got {line}");
}

#[test]
fn an_operator_on_signals_from_two_designs_is_refused_not_merged() {
    let first = Design::new();
    let second = Design::new();
    let a = first.lit(1, 8).unwrap();
    let b = second.lit(1, 8).unwrap();

    // Recovering the design from the left operand means the right one is foreign.
    let err = first.add(&a, &b).unwrap_err();
    assert!(
        matches!(err, ferrite_lithic::Error::ForeignSignal { .. }),
        "expected ForeignSignal, got {err:?}"
    );
    // Nothing was pushed into either design.
    assert_eq!(first.node_count(), 1);
    assert_eq!(second.node_count(), 1);
}

#[test]
fn signals_from_two_designs_are_never_equal() {
    // Otherwise `assert_eq!(a, b)` on two one-bit constants from different designs
    // would pass or fail on arena identity rather than on anything meaningful.
    let first = Design::new();
    let second = Design::new();
    let a = first.lit(1, 1).unwrap();
    let b = second.lit(1, 1).unwrap();
    assert_eq!(a.id(), b.id(), "both are node 0");
    assert_ne!(a, b, "but they are different signals");
}

#[test]
fn a_constant_can_be_adopted_into_another_design() {
    let source = Design::new();
    let target = Design::new();
    let literal = source.lit(0xab, 8).unwrap();
    let adopted = target.adopt(&literal).unwrap();
    assert_eq!(adopted.width(), 8);
    assert_eq!(target.node_count(), 1);
    // Adopting the same design's signal is the identity.
    assert_eq!(source.adopt(&literal).unwrap().id(), literal.id());
}

#[test]
fn a_non_constant_cannot_be_adopted() {
    // There is no single value to copy, and copying one as zero would quietly
    // build the wrong circuit.
    let source = Design::new();
    let target = Design::new();
    let computed = source
        .add(&source.lit(1, 8).unwrap(), &source.lit(2, 8).unwrap())
        .unwrap();
    let err = target.adopt(&computed).unwrap_err();
    assert!(
        matches!(
            err.as_ir(),
            Some(ferrite_lithic_ir::Error::NotAConstant { .. })
        ),
        "expected NotAConstant, got {err:?}"
    );
    assert_eq!(target.node_count(), 0, "a refused adopt pushes nothing");
}

#[test]
fn adopting_from_another_design_yields_a_usable_signal() {
    let source = Design::new();
    let target = Design::new();
    let literal = source.lit(5, 4).unwrap();
    let adopted = target.adopt(&literal).unwrap();
    let doubled = target.add(&adopted, &adopted).unwrap();
    assert_eq!(doubled.kind(), "Add");
    target.build().unwrap();
}
