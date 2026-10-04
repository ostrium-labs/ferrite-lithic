//! The front-end DSL: build a circuit by writing arithmetic.
//!
//! This crate is a thin, ergonomic layer over [`ferrite_lithic_ir`]. It adds no
//! node kinds and no semantics; every operation here pushes exactly one IR node.
//! What it adds is a way to *write* a circuit, and three decisions that make that
//! possible.
//!
//! ```
//! use ferrite_lithic::{Design, Error};
//!
//! # fn main() -> Result<(), Error> {
//! let d = Design::new();
//!
//! // An 8-bit accumulator that adds one every clock. Written as arithmetic,
//! // with no node ids, no widths spelled out, and no arena in sight.
//! let clk = d.input("clk", 1)?;
//! let acc = d.wire(8)?;
//! let next = d.add(&acc, &d.lit(1, 8)?)?;
//! let next = d.reg(&next, &clk, &d.constant(false), &d.constant(true))?;
//! d.drive(&acc, &next)?;
//!
//! // The feedback loop through a register is legal, so the graph checks clean
//! // and sorts. The exact node count is not the point, so it is not asserted.
//! let circuit = d.build()?;
//! use ferrite_lithic_ir::Deps;
//! circuit.topological_order(Deps::LoopChecking)?;
//! # Ok(())
//! # }
//! ```
//!
//! # Two tiers, because `a + b` cannot return an error
//!
//! Every builder on [`Design`] returns [`Result`], so a width mistake is a
//! recoverable error carrying a message that names both widths and the fix.
//! Infix operators cannot do that — `impl Add` has one output type — so they are
//! a second, concise tier that panics instead. Each operator delegates to the
//! builder, so there is one implementation of each operation and the tiers cannot
//! drift apart; they build separate nodes, like any two calls would.
//!
//! ```
//! use ferrite_lithic::Design;
//!
//! let d = Design::new();
//! let a = d.lit(0xff, 8).unwrap();
//! let b = d.lit(1, 8).unwrap();
//!
//! let checked = d.add(&a, &b).unwrap();    // checked: Result<Signal, Error>
//! let concise = &a + &b;                   // concise: panics on a width error
//!
//! // Two calls, two nodes — the tiers differ in how they report failure, not in
//! // what they build.
//! assert_eq!(checked.kind(), concise.kind());
//! assert_eq!(checked.width(), concise.width());
//! ```
//!
//! The concise tier is not the default because this project treats errors as
//! load-bearing: a panic in the middle of an elaboration is a crash with no
//! design context, whereas [`Error`] names the operation, both widths, and the
//! explicit change that fixes it. A panic here does at least carry the builder's
//! own message and `#[track_caller]`, so it points at the line that got it wrong.
//! Use the operators where the widths are already known to line up — arithmetic
//! on signals of the same width — and the builders everywhere else.
//!
//! # Why the arena is behind a `RefCell`
//!
//! [`ferrite_lithic_ir::Circuit`] appends to a `Vec`, so its constructors take
//! `&mut self`. That makes nesting impossible:
//!
//! ```compile_fail
//! # use ferrite_lithic::Design;
//! # let d = Design::new();
//! # let a = d.lit(1, 8).unwrap();
//! d.add(a, d.lit(1, 8).unwrap()); // two `&mut` borrows of `d` in one expression
//! ```
//!
//! The IR crate rejects `Rc<RefCell<..>>` for good reason — it makes the graph
//! expensive to traverse and reintroduces runtime borrow checking. So this is the
//! resolution, and it is deliberately narrow:
//!
//! - Only the *builder* is interior-mutable, never a node. A [`Signal`] is a
//!   `NodeId` plus a cached width; it holds no reference *into* the arena, so the
//!   graph stays as cheap to walk as the IR intended.
//! - The `RefCell` borrow is taken and released inside a single method body
//!   that calls no user code and re-enters no builder, so two borrows can never
//!   overlap. A `RefCell` panic here would need a re-entrant call, and there is
//!   no path to one.
//! - Signals are `Clone`, not `Copy`, because each one keeps the arena alive.
//!   That is the cost: a `Signal` from one [`Design`] can be passed to another,
//!   and the second will silently push into its own arena while reporting the
//!   first's node id. [`Design::adopt`] converts a foreign signal into a local
//!   one, and it is the only supported way to move a signal between designs.
//!
//! # Widths are runtime values, so literals are checked
//!
//! [`Design::lit`] takes `value` and `width` separately, and a width beyond 64
//! truncates the value. That is stated rather than hidden because it is the one
//! place a caller can lose information without an error.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod design;
mod ops;

pub use design::{Design, Error, Signal};
