//! The signal graph for the Ferrite Lithic hardware DSL.
//!
//! This crate is the layer between [`ferrite_lithic_bits`] and everything above
//! it. It owns the nodes, hands out their identities, and is the only place a
//! feedback loop can come from.
//!
//! # Design, and where it came from
//!
//! Every claim about Hardcaml's behaviour was read from source with `file:line`
//! citations rather than taken from its manual, and several of them contradict
//! what the original plan assumed. See
//! [`docs/design-notes.md`](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/design-notes.md)
//! for the full audit.
//!
//! # What is inherited deliberately
//!
//! - **An arena, with the arena index as node identity.** Hardcaml has no arena
//!   and no ids at construction time; a node is a heap-allocated variant over
//!   finished children, and its identity is a `uid` from a **process-global
//!   mutable counter** (`kernel/signal__type.ml:966`). Node ids here are indices
//!   into a `Vec`, which makes them deterministic by construction and thread-safe
//!   as a side effect.
//! - **`Wire` is the only node with a mutable field**
//!   (`kernel/signal__type_intf.ml:221-229`). Every other node is built bottom-up
//!   over finished children (`kernel/signal.ml:197-237`), so a wire is the only
//!   possible back edge — and therefore the only way a cycle can exist. That is
//!   the mechanism the whole crate exists to provide.
//! - **A register stores no output value.** `Reg` holds its input `d`, and the
//!   register's output *is* the node (`kernel/signal__type_intf.ml:243-296`), so
//!   "is this stateful?" is one constructor match.
//! - **Memories hold write ports; read ports are separate nodes**
//!   (`kernel/signal__type_intf.ml:298-319`). Reads are asynchronous by
//!   construction, a read port is never a hidden source of state, and a
//!   synchronous read is the user composing a register with a read port.
//! - **The three assignment checks** on closing a loop (`kernel/signal.ml:242-267`):
//!   the target must be a wire, must not already be driven, and the driver must
//!   match its width.
//! - **Three dependency relations, not one** (`src/signal_graph.ml:258-325`), as
//!   an enum parameter rather than a trait object so the cases stay monomorphised.
//!   See [`Deps`] for the table and for why the three memory cases come out the
//!   way they do.
//!
//! # What is deliberately different
//!
//! - **Node ids are arena indices, so naming needs no normalisation pass.**
//!   Hardcaml needs `normalize_uids` to make output reproducible and ships it **off
//!   by default with its own test commented out as "brittle"**
//!   (`test/lib/test_uid_normalization.ml:56-59`). Here, construction order *is*
//!   the canonical order, so identical programs emit identical Verilog as a
//!   structural property. Names are attached during construction rather than
//!   late, which removes the lazily-computed name map as a category of bug.
//! - **No global state at all**, so two circuits can be built concurrently.
//! - **Errors are `Result`, not exceptions**, and every width error names both
//!   widths and the explicit operation that fixes it — the compensation
//!   [ADR-0003](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0003-runtime-widths-not-const-generics.md)
//!   trades const generics for.
//! - **Stored widths are checked, not trusted.** `width_of` is a single match with
//!   no arena walk, so `Case`, `ReadPort`, `Mem`, `Instance` and `Wire` carry
//!   their width. Every constructor derives it first and stores it, and
//!   [`Circuit::check`] re-derives and compares.
//!
//! # Examples
//!
//! A feedback loop, which is the whole reason the arena exists:
//!
//! ```
//! use ferrite_lithic_bits::Bits;
//! use ferrite_lithic_ir::{Circuit, Deps};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut c = Circuit::new();
//!
//! // An undriven 8-bit wire, built first so logic can read it before it has a
//! // driver. This is the only mutable node in the graph.
//! let acc = c.wire(8)?;
//!
//! let one = c.constant(Bits::constant(1, 8)?);
//! let clk = c.constant(Bits::constant(0, 1)?);
//!
//! // A register whose output is the register node itself.
//! let incremented = c.add(acc, one)?;
//! let next = c.reg(incremented, clk, clk, clk)?;
//!
//! // Close the loop. Legal: a register is terminal for combinational loop
//! // checking, so feedback through one is an ordinary accumulator, not a cycle.
//! c.drive(acc, next)?;
//! c.set_name(acc, "acc")?;
//!
//! c.check()?;
//! c.topological_order(Deps::LoopChecking)?;
//! assert_eq!(c.name(acc), Some("acc"));
//! # Ok(())
//! # }
//! ```
//!
//! The same graph *is* a cycle under the relation that follows registers, which is
//! what makes having three relations load-bearing rather than pedantic:
//!
//! ```
//! # use ferrite_lithic_bits::Bits;
//! # use ferrite_lithic_ir::{Circuit, Deps, Error};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let mut c = Circuit::new();
//! # let acc = c.wire(8)?;
//! # let one = c.constant(Bits::constant(1, 8)?);
//! # let clk = c.constant(Bits::constant(0, 1)?);
//! # let incremented = c.add(acc, one)?;
//! # let next = c.reg(incremented, clk, clk, clk)?;
//! # c.drive(acc, next)?;
//! assert!(c.topological_order(Deps::LoopChecking).is_ok());
//! let err = c.topological_order(Deps::WithoutCaseMatches).unwrap_err();
//! assert!(matches!(err, Error::CombinationalLoop { .. }));
//! # Ok(())
//! # }
//! ```

#![doc(html_root_url = "https://docs.rs/ferrite-lithic-ir/0.0.1")]

mod circuit;
mod deps;
mod error;
mod id;
mod node;

pub use circuit::Circuit;
pub use deps::Deps;
pub use error::{Error, Op, Relation};
pub use id::NodeId;
pub use node::{CaseArm, Node, WritePort, address_width_for_depth};
