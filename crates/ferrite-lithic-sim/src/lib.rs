//! The cycle simulator: compile a circuit to word addresses, then step it.
//!
//! # What this is for
//!
//! Three backends will consume the same graph: a Verilog emitter, this simulator,
//! and eventually a cosimulator that checks the two against each other. This crate
//! is the one you can run without a vendor toolchain, so it is where a circuit's
//! behaviour is established before anyone checks whether the emitted Verilog
//! agrees.
//!
//! # Compile once, step many times
//!
//! [`Program`] turns a [`Circuit`](ferrite_lithic_ir::Circuit) into word addresses
//! and an [`Op`] list. Nothing in a `Program` depends on the values being
//! simulated, so one compiled program can back many [`Sim`]s.
//!
//! ```no_run
//! use ferrite_lithic::Design;
//! use ferrite_lithic_sim::Sim;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let d = Design::new();
//! let clk = d.input("clk", 1)?;
//! let acc = d.wire(8)?;
//! let next = d.add(&acc, &d.lit(1, 8)?)?;
//! let next = d.reg(&next, &clk, &d.constant(false), &d.constant(true))?;
//! d.drive(&acc, &next)?;
//! d.output("acc", 8, &acc)?;
//!
//! let mut sim = Sim::new(&d)?;
//! let (_before, after) = sim.step()?;
//! println!("{after}");
//! # Ok(())
//! # }
//! ```
//!
//! # The three phases, and why there are two snapshots
//!
//! A cycle is [`Sim::before_clock_edge`], [`Sim::at_clock_edge`],
//! [`Sim::after_clock_edge`] ([`src/cyclesim.ml:32-46`](https://github.com/jane-street/hardcaml)):
//!
//! 1. advance the counter, drive the clock high, settle, snapshot outputs as
//!    **before**;
//! 2. apply register and memory writes;
//! 3. drive the clock low, settle, snapshot outputs as **after**.
//!
//! Both snapshots are first-class ([`src/cyclesim0_intf.ml:58`](https://github.com/jane-street/hardcaml)).
//! `before` is what the outputs were before the edge and `after` is what they
//! became on it, which is how you observe a register on the same edge it changes.
//!
//! # Two-phase register update, and it is not optional
//!
//! [`Sim::at_clock_edge`] writes every register's next value into a shadow section
//! and then copies the whole section over the live one. Computing next values in
//! place would make each register's result depend on the order registers are
//! visited in, and feedback through a register's own output would shift by one
//! cycle *per register* instead of once per design.
//!
//! # Wires cost nothing at runtime
//!
//! Each wire is chased to its first non-wire driver at compile time and aliased to
//! it ([`src/cyclesim_compile.ml:312-319`](https://github.com/jane-street/hardcaml)).
//! The instruction list therefore never mentions a wire, and reading a wire's
//! value is reading its driver's.
//!
//! # No event queue, no delta cycles
//!
//! One [`Sim::comb`] is one full settle pass in a precomputed topological order. A
//! combinational loop is a compile error, not something to iterate to a fixed
//! point: it is an infinite loop in the vendor toolchain too, so failing loudly is
//! the only useful behaviour.
//!
//! # Deliberate divergence: an op enum, not closures
//!
//! Hardcaml compiles each node to a zero-argument OCaml closure. We compile to an
//! [`Op`] enum interpreted by a `match`. Same execution model, no `dyn`, and the
//! list is **inspectable** — which is what makes comparing two simulators, or this
//! one against emitted Verilog, possible at all.
//!
//! # Where this is deliberately incomplete
//!
//! Three boundaries are refused rather than approximated, because a wrong answer
//! that looks right is worse than no answer:
//!
//! - **Instances have no model** ([`Error::NoInstanceModel`]). An instance is a
//!   combinational black box to the graph and opaque storage to a simulator.
//!   Treating one as a constant zero would simulate a different circuit.
//! - **One clock domain** ([`Error::MultipleClocks`]). Two clocks need per-domain
//!   edge scheduling; simulating them as one makes results depend on visit order.
//! - **Division by zero is an error** ([`Error::DivisionByZero`]). Emitted Verilog
//!   answers `x`, and a two-state simulator has no `x` to give. Reporting is the
//!   only way the mistake survives to be fixed; see [`Error`].
//!
//! Signed division overflow is also an error ([`Error::SignedDivisionOverflow`]),
//! which is why stepping a design is fallible at all: the graph is perfectly legal,
//! and only running it can reveal that the operands make the operation
//! unrepresentable.

#![doc(html_root_url = "https://docs.rs/ferrite-lithic-sim/0.0.1")]

/// The README's examples, compiled and run as doctests.
///
/// They live here rather than on the front-end crate because this is the only crate
/// whose *dev*-dependencies cover every crate the README mentions, so it is the
/// only one where every example in the file can resolve its imports. A crate needs a
/// target for `include_str!` to hang them off, and an unused private struct is the
/// cheapest one.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct ReadmeExamples;

mod compile;
mod error;
mod eval;
mod op;
mod sim;

pub use compile::{MemWrite, Port, Program, RegUpdate, Sections};
pub use error::Error;
pub use op::{Op, Word};
pub use sim::{Sim, Snapshot};
