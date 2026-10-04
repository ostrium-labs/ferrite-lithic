//! Every operation, against [`Bits`], over widths that straddle the fast path.
//!
//! The simulator has two implementations of each operation — a hand-written `u64`
//! path and a fallback through `Bits` — and this file is what keeps them honest
//! with each other. It is the same shape as `tests/structural.rs` in the front end,
//! one level up: there a structural operation was checked against the value its
//! builder claimed; here a compiled op is checked against the arithmetic the rest
//! of the stack agrees on.
//!
//! # Why the widths run to 200
//!
//! A word is 64 bits, so the boundary between the two paths is at width 65, and a
//! range stopping at 64 would only ever test the fast one. 200 puts three full
//! words in play.
//!
//! The small end matters just as much. Hardcaml's multiply path assumes its
//! operands span at least two words, with no assertion to that effect, covered only
//! by an empirical width sweep (`test/lib/test_bits.ml:285`) — so width 1 has to be
//! exercised rather than assumed. Ours has no such assumption, which is exactly why
//! the sweep is worth keeping.
//!
//! # What is *not* checked independently, and why
//!
//! Unsigned comparison above 64 bits compares through `Bits`' own `Ord`, which is
//! what the simulator's fallback path uses too. So for wide values this file checks
//! the interpreter's plumbing, not the semantics of ordering — and the semantics are
//! already covered by the bits crate's own tests. Signed comparison *is* checked
//! independently, against a sign-extended `i64`, and therefore only up to 64 bits,
//! because there is no wider machine integer to extend into.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_sim::{Error, Sim};
use proptest::prelude::*;

/// A width that is sometimes one word and sometimes several.
fn any_width() -> impl Strategy<Value = u32> {
    1u32..=200
}

/// A value of exactly `width` bits, from `bytes_for_width(width)` random bytes.
///
/// Bytes are the right currency: the word count follows from the width, so the
/// value is genuinely arbitrary rather than a `u64` in a wide costume.
fn value_of(width: u32) -> impl Strategy<Value = Bits> {
    let bytes = prop::collection::vec(any::<u8>(), ferrite_lithic_bits::bytes_for_width(width));
    bytes.prop_map(move |b| Bits::from_bytes_le(width, &b).expect("length is bytes_for_width"))
}

/// A width and one value of that width.
fn single() -> impl Strategy<Value = (u32, Bits)> {
    any_width().prop_flat_map(|w| (Just(w), value_of(w)))
}

/// A width and two values of it.
fn pair() -> impl Strategy<Value = (u32, Bits, Bits)> {
    any_width().prop_flat_map(|w| (Just(w), value_of(w), value_of(w)))
}

/// The operations this file covers, as one enum so a single property covers all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Op {
    BitAnd,
    BitOr,
    BitXor,
    Add,
    Sub,
    Mul,
    UDiv,
    SDiv,
    URem,
    SRem,
    Eq,
    Ult,
    Ule,
    UGt,
    UGe,
    Slt,
    Sle,
    Sgt,
    SGe,
}

impl Op {
    const ALL: &'static [Op] = &[
        Op::BitAnd,
        Op::BitOr,
        Op::BitXor,
        Op::Add,
        Op::Sub,
        Op::Mul,
        Op::UDiv,
        Op::SDiv,
        Op::URem,
        Op::SRem,
        Op::Eq,
        Op::Ult,
        Op::Ule,
        Op::UGt,
        Op::UGe,
        Op::Slt,
        Op::Sle,
        Op::Sgt,
        Op::SGe,
    ];

    fn name(self) -> &'static str {
        match self {
            Op::BitAnd => "bit_and",
            Op::BitOr => "bit_or",
            Op::BitXor => "bit_xor",
            Op::Add => "add",
            Op::Sub => "sub",
            Op::Mul => "mul",
            Op::UDiv => "udiv",
            Op::SDiv => "sdiv",
            Op::URem => "urem",
            Op::SRem => "srem",
            Op::Eq => "eq",
            Op::Ult => "ult",
            Op::Ule => "ule",
            Op::UGt => "ugt",
            Op::UGe => "uge",
            Op::Slt => "slt",
            Op::Sle => "sle",
            Op::Sgt => "sgt",
            Op::SGe => "sge",
        }
    }

    /// Builds the operation on two same-width signals.
    fn build(self, d: &Design, left: &Signal, right: &Signal) -> Signal {
        let r = match self {
            Op::BitAnd => d.and(left, right),
            Op::BitOr => d.or(left, right),
            Op::BitXor => d.xor(left, right),
            Op::Add => d.add(left, right),
            Op::Sub => d.sub(left, right),
            Op::Mul => d.mul(left, right),
            Op::UDiv => d.udiv(left, right),
            Op::SDiv => d.sdiv(left, right),
            Op::URem => d.urem(left, right),
            Op::SRem => d.srem(left, right),
            Op::Eq => d.eq(left, right),
            Op::Ult => d.ult(left, right),
            Op::Ule => d.ule(left, right),
            Op::UGt => d.ugt(left, right),
            Op::UGe => d.uge(left, right),
            Op::Slt => d.slt(left, right),
            Op::Sle => d.sle(left, right),
            Op::Sgt => d.sgt(left, right),
            Op::SGe => d.sge(left, right),
        };
        r.expect("both operands have the same width")
    }
}

/// What the reference implementation says should happen.
#[derive(Clone, Debug, PartialEq)]
enum Expected {
    /// A value, of the width the operation produces.
    Value(Bits),
    /// The divisor is zero.
    DivideByZero,
    /// Signed division of the most negative value by -1.
    Overflow,
}

/// Reinterprets a value of at most 64 bits as a two's-complement `i64`.
///
/// Independent of the simulator's own `signed`, which shifts within a `u64`.
fn widened(bits: &Bits) -> i64 {
    debug_assert!(bits.width() <= 64);
    bits.sign_extend(64)
        .expect("width is at most 64")
        .to_u64()
        .expect("64 bits") as i64
}

/// The reference answer, computed from `Bits` alone.
fn reference(op: Op, a: &Bits, b: &Bits) -> Expected {
    let v = |bits: Bits| Expected::Value(bits);
    match op {
        Op::BitAnd => v(a.and(b).expect("equal widths")),
        Op::BitOr => v(a.or(b).expect("equal widths")),
        Op::BitXor => v(a.xor(b).expect("equal widths")),
        Op::Add => v(a.add(b).expect("equal widths")),
        Op::Sub => v(a.sub(b).expect("equal widths")),
        Op::Mul => v(a.mul(b).expect("equal widths")),
        Op::UDiv | Op::URem | Op::SDiv | Op::SRem => {
            if b.is_zero() {
                return Expected::DivideByZero;
            }
            if op == Op::SDiv && a.is_min_signed() && b.is_all_ones() {
                return Expected::Overflow;
            }
            match op {
                Op::UDiv => v(a.udiv(b).expect("nonzero divisor")),
                Op::URem => v(a.urem(b).expect("nonzero divisor")),
                Op::SDiv => v(a.sdiv(b).expect("nonzero divisor")),
                _ => v(a.srem(b).expect("nonzero divisor")),
            }
        }
        Op::Eq => v(Bits::constant(u64::from(a.eq(b).expect("equal widths")), 1).unwrap()),
        Op::Ult | Op::Ule | Op::UGt | Op::UGe => {
            let (less, equal) = match a.cmp(b) {
                core::cmp::Ordering::Less => (true, false),
                core::cmp::Ordering::Equal => (false, true),
                core::cmp::Ordering::Greater => (false, false),
            };
            let bit = match op {
                Op::Ult => less,
                Op::Ule => less || equal,
                Op::UGt => !less && !equal,
                _ => !less,
            };
            v(Bits::constant(u64::from(bit), 1).unwrap())
        }
        Op::Slt | Op::Sle | Op::Sgt | Op::SGe => {
            let bit = if a.width() <= 64 {
                // Independent of the simulator: sign-extend both into an `i64` and
                // compare there.
                let x = widened(a);
                let y = widened(b);
                match op {
                    Op::Slt => x < y,
                    Op::Sle => x <= y,
                    Op::Sgt => x > y,
                    _ => x >= y,
                }
            } else {
                // Wider than a machine word, so there is nothing to sign-extend
                // into. Fall back to the two's-complement ordering rule: within one
                // sign the unsigned order is the signed order, and across signs the
                // negative value is the smaller one. That is the same rule the
                // simulator's fallback implements, so above 64 bits these cases
                // check the interpreter's plumbing -- op wiring, word indexing,
                // masking -- and not the semantics of signed ordering. The
                // semantics are covered against an `i64` by
                // `signed_comparisons_match_a_sign_extended_i64`, which is why that
                // property stops at 64.
                let (an, bn) = (a.is_negative(), b.is_negative());
                let order = a.cmp(b);
                let lt = if an != bn {
                    an
                } else {
                    order == core::cmp::Ordering::Less
                };
                let gt = if an != bn {
                    bn
                } else {
                    order == core::cmp::Ordering::Greater
                };
                match op {
                    Op::Slt => lt,
                    Op::Sle => !gt,
                    Op::Sgt => gt,
                    _ => !lt,
                }
            };
            v(Bits::constant(u64::from(bit), 1).unwrap())
        }
    }
}

/// Runs one operation on two driven inputs and returns what the simulator computed.
fn simulate(op: Op, width: u32, a: &Bits, b: &Bits) -> Result<Bits, Error> {
    let d = Design::new();
    let left = d.input("a", width).unwrap();
    let right = d.input("b", width).unwrap();
    let out = op.build(&d, &left, &right);
    d.output("out", out.width(), &out).unwrap();
    let mut sim = Sim::new(&d).unwrap();
    sim.set_input("a", a.clone()).unwrap();
    sim.set_input("b", b.clone()).unwrap();
    sim.comb()?;
    Ok(sim.peek("out").unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Every binary operation agrees with `Bits`, for one-word and wide values.
    #[test]
    fn binary_operations_match_bits(
        op_idx in 0usize..Op::ALL.len(),
        (width, a, b) in pair(),
    ) {
        let op = Op::ALL[op_idx];
        match (simulate(op, width, &a, &b), reference(op, &a, &b)) {
            (Ok(got), Expected::Value(want)) => {
                prop_assert_eq!(got.width(), want.width(), "op {}", op.name());
                prop_assert_eq!(got.to_binary_digits(), want.to_binary_digits(),
                    "op {} on {} and {} at width {}", op.name(),
                    a.to_binary_digits(), b.to_binary_digits(), width);
            }
            (Err(Error::DivisionByZero { .. }), Expected::DivideByZero) => {}
            (Err(Error::SignedDivisionOverflow { .. }), Expected::Overflow) => {}
            (got, want) => prop_assert!(false,
                "op {} at width {}: simulator said {:?}, reference said {:?}",
                op.name(), width, got.err().map(|e| e.to_string()),
                match want { Expected::Value(v) => Some(v.to_binary_digits()),
                             Expected::DivideByZero => Some("divide by zero".into()),
                             Expected::Overflow => Some("overflow".into()) }),
        }
    }

    /// Signed comparison is only checked where the reference is independent.
    #[test]
    fn signed_comparisons_match_a_sign_extended_i64(
        op_idx in 0usize..4,
        (width, a, b) in (1u32..=64).prop_flat_map(|w| (Just(w), value_of(w), value_of(w))),
    ) {
        let op = [Op::Slt, Op::Sle, Op::Sgt, Op::SGe][op_idx];
        let got = simulate(op, width, &a, &b).unwrap();
        let want = reference(op, &a, &b);
        let Expected::Value(want) = want else { panic!("a comparison cannot fail") };
        prop_assert_eq!(got.to_binary_digits(), want.to_binary_digits(),
            "op {} at width {}", op.name(), width);
    }

    /// Bitwise complement, which takes one operand.
    #[test]
    fn complement_matches_bits((width, a) in single()) {
        let d = Design::new();
        let arg = d.input("a", width).unwrap();
        let out = d.not(&arg);
        d.output("out", width, &out).expect("output port matches the width");
        let mut sim = Sim::new(&d).unwrap();
        sim.set_input("a", a.clone()).unwrap();
        sim.comb().unwrap();
        let got = sim.peek("out").unwrap();
        let want = a.not().unwrap();
        prop_assert_eq!(got.to_binary_digits(), want.to_binary_digits(), "width {}", width);
    }

    /// `select` is the one operation with a *window*, so its bounds are the risk.
    #[test]
    fn select_takes_the_bits_it_claims(
        (width, offset, len, a) in any_width().prop_flat_map(|w| {
            (
                Just(w),
                0u32..w + 1,
                1u32..=w,
                value_of(w),
            )
        }),
    ) {
        prop_assume!(offset + len <= width);
        let d = Design::new();
        let arg = d.input("a", width).unwrap();
        let out = d.slice(&arg, offset, len).unwrap();
        d.output("out", len, &out).unwrap();
        let mut sim = Sim::new(&d).unwrap();
        sim.set_input("a", a.clone()).unwrap();
        sim.comb().unwrap();
        let got = sim.peek("out").unwrap();
        let want = a.select(offset, len).unwrap();
        prop_assert_eq!(got.to_binary_digits(), want.to_binary_digits(),
            "select {} at {} of {}", len, offset, width);
    }

    /// Concatenation, whose result is wider than either operand.
    #[test]
    fn cat_joins_high_to_low(high_width in any_width(), low_width in any_width()) {
        let (high_width, low_width) = (high_width.min(100), low_width.min(100));
        let d = Design::new();
        let high = d.input("high", high_width).unwrap();
        let low = d.input("low", low_width).unwrap();
        let out = d.concat(&[high, low]).unwrap();
        d.output("out", out.width(), &out).unwrap();
        let mut sim = Sim::new(&d).unwrap();
        // Patterns rather than constants: a distinctive value in each half is what
        // makes a swapped concatenation visible.
        let hv = Bits::ones(high_width).unwrap();
        let lv = Bits::zeros(low_width).unwrap();
        sim.set_input("high", hv.clone()).unwrap();
        sim.set_input("low", lv.clone()).unwrap();
        sim.comb().unwrap();
        let got = sim.peek("out").unwrap();
        let want = ferrite_lithic_bits::cat(&[&hv, &lv]).unwrap();
        prop_assert_eq!(got.width(), want.width());
        prop_assert_eq!(got.to_binary_digits(), want.to_binary_digits(),
            "cat {} and {}", high_width, low_width);
    }

    /// `replicate`, whose result width is a multiple of the operand's.
    #[test]
    fn replicate_repeats_the_whole_value(width in 1u32..=32, count in 1u32..=8) {
        let d = Design::new();
        let arg = d.input("a", width).unwrap();
        let out = d.replicate(&arg, count).unwrap();
        d.output("out", out.width(), &out).unwrap();
        let mut sim = Sim::new(&d).unwrap();
        let v = Bits::ones(width).unwrap();
        sim.set_input("a", v.clone()).unwrap();
        sim.comb().unwrap();
        let got = sim.peek("out").unwrap();
        let want = v.replicate(count).unwrap();
        prop_assert_eq!(got.width(), want.width());
        prop_assert_eq!(got.to_binary_digits(), want.to_binary_digits(),
            "replicate {} of {}", count, width);
    }

    /// `ite`, whose arms are chosen by a one-bit condition.
    #[test]
    fn ite_picks_the_matching_arm(condition_bit in any::<bool>(), width in any_width()) {
        let d = Design::new();
        let cond = d.input("c", 1).unwrap();
        let yes = d.input("yes", width).unwrap();
        let no = d.input("no", width).unwrap();
        let out = d.ite(&cond, &yes, &no).unwrap();
        d.output("out", width, &out).unwrap();
        let mut sim = Sim::new(&d).unwrap();
        let y = Bits::ones(width).unwrap();
        let n = Bits::zeros(width).unwrap();
        sim.set_input("c", Bits::constant(u64::from(condition_bit), 1).unwrap()).unwrap();
        sim.set_input("yes", y.clone()).unwrap();
        sim.set_input("no", n.clone()).unwrap();
        sim.comb().unwrap();
        let got = sim.peek("out").unwrap();
        let want = if condition_bit { &y } else { &n };
        prop_assert_eq!(&got, want);
    }

    /// `case`, where arm order is the priority order and the default is the
    /// fallback. The scrutinee is matched against literals.
    #[test]
    fn case_takes_the_first_matching_arm(width in 2u32..=16) {
        // Width 1 is excluded deliberately: at one bit the literals `1` and `3`
        // are the same value, so the "no arm matches" case would silently become a
        // hit and the property would be asserting nothing.
        let d = Design::new();
        let scrutinee = d.input("s", width).unwrap();
        // Arms are matched against *literals*, so the DSL takes the value rather
        // than a signal: the point of a case is a comparison against a constant.
        let low = d.lit(0b1010, width).unwrap();
        let high = d.lit(0b0101, width).unwrap();
        let fallback = d.lit(0b1111, width).unwrap();
        // `2` is listed before `1`, so a hit on both would prove priority order.
        let out = d
            .case_(&scrutinee, &[(2, high), (1, low)], &fallback)
            .unwrap();
        d.output("out", width, &out).unwrap();
        let mut sim = Sim::new(&d).unwrap();
        for (input, arm) in [
            (Bits::constant(1, width).unwrap(), 0b1010u64),
            (Bits::constant(2, width).unwrap(), 0b0101),
            (Bits::constant(3, width).unwrap(), 0b1111),
        ] {
            sim.set_input("s", input.clone()).unwrap();
            sim.comb().unwrap();
            // Compared as a same-width `Bits`: at width 1 the arm values are
            // themselves truncated, so comparing raw `u64`s would fail on the
            // truncation rather than on the select.
            let want = Bits::constant(arm, width).unwrap();
            prop_assert_eq!(&sim.peek("out").unwrap(), &want, "scrutinee {:?}", input);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// A wide value survives a round trip through the buffer unchanged.
    ///
    /// The other properties all compare against `Bits`, so a bug that *loses* a
    /// word could hide behind a matching pair of losses. This one checks the buffer
    /// against the value that went in.
    #[test]
    fn wide_values_survive_the_buffer((width, a) in single()) {
        let d = Design::new();
        let arg = d.input("a", width).unwrap();
        let out = d.not(&d.not(&arg));
        d.output("out", width, &out).unwrap();
        let mut sim = Sim::new(&d).unwrap();
        sim.set_input("a", a.clone()).unwrap();
        sim.comb().unwrap();
        prop_assert_eq!(&sim.peek("out").unwrap(), &a, "width {}", width);
    }
}
