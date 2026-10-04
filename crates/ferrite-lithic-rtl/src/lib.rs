//! The Verilog emitter: one module per circuit, with names that do not move.
//!
//! # What this is for
//!
//! Three backends consume the same signal graph: this one, the cycle simulator, and
//! eventually a cosimulator that checks the two against each other. This is the
//! one whose output is meant to be read — by a synthesiser, and by a person
//! reviewing a diff.
//!
//! ```
//! use ferrite_lithic::Design;
//! use ferrite_lithic_rtl::Module;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let d = Design::new();
//! let clk = d.input("clk", 1)?;
//! let d_in = d.input("d", 8)?;
//!
//! // An accumulator: eight bits of state that add `d` on every edge.
//! let acc = d.wire(8)?;
//! let next = d.add(&acc, &d_in)?;
//! let next = d.reg(&next, &clk, &d.constant(false), &d.constant(true))?;
//! d.drive(&acc, &next)?;
//! d.output("acc", 8, &acc)?;
//!
//! let module = Module::new(
//!     "accumulator",
//!     d.build()?,
//!     clk.id(),
//!     d.input_ports().iter().map(|s| s.id()).collect(),
//!     d.output_ports().iter().map(|s| s.id()).collect(),
//! );
//! print!("{}", module.emit()?);
//! # Ok(())
//! # }
//! ```
//!
//! # Byte-identical output, structurally
//!
//! Emitting the same program twice produces the same bytes. That is a requirement,
//! not a lucky property, because the only way anyone can test an emitter is to
//! diff its output.
//!
//! Hardcaml has to defend this with work: node identity is a `uid` from a
//! **process-global mutable counter**, so reproducibility depends on the program
//! running the same way twice, and the pass that would make it independent —
//! `normalize_uids` — is **off by default**, with its own test commented out as
//! "brittle to changes in the test environment"
//! (`test/lib/test_uid_normalization.ml:56-59`).
//!
//! We do not need the pass. Node ids *are* arena indices, and arena indices are
//! construction order by definition, so walking the graph in id order is already
//! canonical. What is left to decide is which name each node gets, and that is the
//! three-state rule: ports first, then explicitly named signals, then unnamed ones
//! in graph order. Unnamed signals are named for their node kind and collisions
//! append a numeric suffix; see [`tests/naming.rs`](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/crates/ferrite-lithic-rtl/tests/naming.rs)
//! for the whole contract, asserted against the emitted text.
//!
//! # One module per circuit, deduplicated by name
//!
//! [`emit_library`] writes a set of modules into one file, emitting a same-named
//! module once. What "the same module" means here, and the one case this crate
//! cannot detect, is documented on that function rather than glossed over.
//!
//! # What this refuses
//!
//! Verilog has no spelling for some things this graph can express, and emitting
//! something that *looks* like Verilog but means a different circuit is worse than
//! not emitting. Four boundaries are reported as [`Error`]:
//!
//! - **A register clocked by a literal** ([`Error::ConstantClock`]). `@(posedge
//!   1'b0)` can never fire and `@(posedge 1'b1)` is not a clock.
//! - **A second clock domain** ([`Error::ClockMismatch`], [`Error::NoClock`]).
//!   A1 is one domain, named by the module.
//! - **A whole memory used as a value** ([`Error::NotAValue`]). A memory is an
//!   array; read it through a read port.
//! - **A port in both directions** ([`Error::PortIsBothDirections`]). That is an
//!   `inout`, and tri-state is deliberately not ported from the reference.
//!
//! An unnamed port is refused too ([`Error::UnnamedPort`]): a port's identifier *is*
//! its ABI, and there is nothing to derive one from.
//!
//! # Deliberately not ported
//!
//! From the reference's own gaps, on purpose: tri-state (it lives in an
//! unintegrated legacy API and the Verilog writer does not implement it), vendor
//! primitives, design constraints and pin locations (absent entirely), and
//! `Architecture` (`Small`/`Balanced`/`Fast`), which is re-exported but has no
//! consumer that changes codegen — a documentation marker, so there is nothing to
//! implement.
//!
//! # Verification
//!
//! The golden tests in `tests/emit.rs` compare whole modules as text, and
//! `tests/iverilog.rs` runs them through `iverilog` when it is installed. It is not
//! installed everywhere, and that test says so rather than passing silently.

#![doc(html_root_url = "https://docs.rs/ferrite-lithic-rtl/0.0.1")]
#![forbid(unsafe_code)]

mod emit;
mod error;
mod module;
mod naming;

pub use emit::emit_library;
pub use error::Error;
pub use module::{Direction, Module, PortNames, Signature};
pub use naming::mangle_port_name;
