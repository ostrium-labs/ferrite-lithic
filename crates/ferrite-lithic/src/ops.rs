//! The concise tier: infix operators that panic instead of returning an error.
//!
//! See the [crate docs](crate) for why there are two tiers. Every operator here
//! delegates to the checked builder on [`Design`] and unwraps the result, so
//! there is exactly one implementation of each operation and the two tiers
//! cannot drift apart. `tests/operators.rs` asserts that for each operator: both
//! paths build a node of the same kind and width.
//!
//! `#[track_caller]` is on every one, so a panic reports the line in the user's
//! design rather than a line in this file.
//!
//! # Why `&Signal` and not `Signal`
//!
//! The traits are implemented on `&Signal` so that `a + b` does not consume its
//! operands. A feedback loop reads a wire many times, and a design that had to
//! clone a signal to use it twice would push the clone cost into every expression.
//!
//! Comparisons deliberately have no operator. `==` is [`PartialEq`] on
//! `Signal`, and it answers "is this the same node", which is a question about
//! handles. Overloading it for value comparison would make
//! `assert_eq!(a, b)` mean "the same node" in one place and "equal values" in
//! another. The value comparison is [`Signal::equals`].

use std::ops::{Add, BitAnd, BitOr, BitXor, Div, Mul, Not, Rem, Sub};

use crate::design::{Design, Signal};

/// Unwraps a builder result, or panics with the builder's own message.
macro_rules! or_panic {
    ($expr:expr) => {
        match $expr {
            Ok(signal) => signal,
            Err(error) => panic!("{error}"),
        }
    };
}

/// `$design.add($a, $b)` via the design that owns `$a`.
macro_rules! binop {
    ($a:expr, $b:expr, $method:ident) => {{
        let left = $a;
        let right = $b;
        let design = left.design();
        or_panic!(Design::$method(&design, left, right))
    }};
}

/// Generates an operator for both `&Signal op &Signal` and the by-value forms
/// that chaining needs.
///
/// `&a + &b + &c` evaluates left to right, so the intermediate is an owned
/// `Signal` and a reference-only impl would stop the chain. The by-value impls
/// consume their left operand, which is right for a temporary and is exactly why
/// the reference impls exist — a design must be able to read one wire many times
/// without cloning it.
///
/// The trait's method name and the builder's name differ for several operators
/// (`BitAnd::bitand` builds with `Design::and`), so both are passed in.
macro_rules! binary_op {
    ($trait:ident, $trait_method:ident, $builder:ident) => {
        impl $trait for &Signal {
            type Output = Signal;

            #[track_caller]
            fn $trait_method(self, rhs: &Signal) -> Signal {
                let design = self.design();
                or_panic!(Design::$builder(&design, self, rhs))
            }
        }

        impl $trait<Signal> for &Signal {
            type Output = Signal;

            #[track_caller]
            fn $trait_method(self, rhs: Signal) -> Signal {
                let design = self.design();
                or_panic!(Design::$builder(&design, self, &rhs))
            }
        }

        impl $trait<&Signal> for Signal {
            type Output = Signal;

            #[track_caller]
            fn $trait_method(self, rhs: &Signal) -> Signal {
                let design = self.design();
                or_panic!(Design::$builder(&design, &self, rhs))
            }
        }

        impl $trait for Signal {
            type Output = Signal;

            #[track_caller]
            fn $trait_method(self, rhs: Signal) -> Signal {
                let design = self.design();
                or_panic!(Design::$builder(&design, &self, &rhs))
            }
        }
    };
}

binary_op!(Add, add, add);
binary_op!(Sub, sub, sub);
binary_op!(Mul, mul, mul);
// Unsigned: there is no signed `/` or `%` operator, and silently picking one would
// make `-1 / 2` a question about the reader's mood.
binary_op!(Div, div, udiv);
binary_op!(Rem, rem, urem);
binary_op!(BitAnd, bitand, and);
binary_op!(BitOr, bitor, or);
binary_op!(BitXor, bitxor, xor);

impl Not for &Signal {
    type Output = Signal;

    #[track_caller]
    fn not(self) -> Signal {
        Design::not(&self.design(), self)
    }
}

impl Not for Signal {
    type Output = Signal;

    #[track_caller]
    fn not(self) -> Signal {
        Design::not(&self.design(), &self)
    }
}

impl Signal {
    /// Value equality, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn equals(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, eq)
    }

    /// Unsigned `<`, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn unsigned_lt(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, ult)
    }

    /// Unsigned `<=`, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn unsigned_le(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, ule)
    }

    /// Unsigned `>`, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn unsigned_gt(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, ugt)
    }

    /// Unsigned `>=`, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn unsigned_ge(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, uge)
    }

    /// Signed `<`, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn signed_lt(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, slt)
    }

    /// Signed `<=`, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn signed_le(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, sle)
    }

    /// Signed `>`, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn signed_gt(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, sgt)
    }

    /// Signed `>=`, one bit wide.
    #[must_use]
    #[track_caller]
    pub fn signed_ge(&self, rhs: &Signal) -> Signal {
        binop!(self, rhs, sge)
    }

    /// `if condition { then_value } else { otherwise }`.
    ///
    /// A method rather than an operator because Rust has no ternary, and
    /// `condition.then(a).otherwise(b)` would read as a builder chain over a
    /// condition rather than a mux.
    #[track_caller]
    pub fn when(&self, condition: &Signal, then_value: &Signal, otherwise: &Signal) -> Signal {
        let design = self.design();
        or_panic!(Design::ite(&design, condition, then_value, otherwise))
    }
}
