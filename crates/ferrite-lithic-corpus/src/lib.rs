//! Designs that exist to prove the toolchain handles something.
//!
//! # What this crate is
//!
//! Every other crate in the workspace is a tool: a DSL, an IR, a simulator, an
//! emitter, a cosimulator. A tool can be tested with inputs chosen by whoever wrote
//! it, which means the tests drift toward what the tool was built to do. This crate
//! is the other direction: **a design is written the way hardware would want it, and
//! the toolchain has to cope.**
//!
//! So each module here is a real algorithm with real constraints -- a serial
//! dependency chain, a fixed state width, a throughput target -- and each one is
//! checked three ways:
//!
//! 1. **Against the original crate.** The software implementation is the golden
//!    model. `crc3` is checked against the `crc` crate's CRC-3/ROTD, not against a
//!    second implementation written by the same hand that wrote the design, because
//!    a self-written golden model can be wrong in exactly the same way twice.
//! 2. **Through the step testbench.** The design is driven a cycle at a time and
//!    read back as cycle-indexed values, so a test says what the design does rather
//!    than what it is built from.
//! 3. **Against Verilator.** The emitted Verilog is compiled and run, and its
//!    outputs are compared with the simulator's for the same stimulus. A design that
//!    simulates correctly and elaborates to something that behaves differently is
//!    the failure this catches, and it is the one no amount of reading catches.
//!
//! # The synthesizable subset these designs stay inside
//!
//! Chosen up front, because a design that needs a heap allocation or a recursive
//! call is not a design that can go in an ASIC, and finding that out after writing
//! it is expensive:
//!
//! - **No allocation in the design.** A [`ferrite_lithic::Design`] is an arena; every
//!   node is
//!   allocated once at build time and nothing is allocated per cycle.
//! - **No pointer chasing.** Nodes are
//!   [`ferrite_lithic::Signal`]s over flat arena indices
//!   into a flat arena, so a graph walk is array indexing.
//! - **Fixed-size state.** Registers and memories have a width fixed at
//!   construction. Nothing is runtime-sized.
//! - **No dynamic dispatch.** The simulator compiles to an enum op and a `match`.
//!
//! Widths are runtime values rather than const generics (ADR-0003), which is a
//! deliberate exception: the *graph* is built in a loop, and a const-generic width
//! would have to be threaded through that loop as a type parameter.

pub mod aes;
pub mod aho_corasick;
pub mod base64;
pub mod bitpack;
pub mod chacha20;
pub mod crc3;
pub mod crc32;
pub mod deflate;
pub mod dfa;
pub mod fse;
pub mod ghash;
pub mod hamming;
pub mod hex;
pub mod huffman;
pub mod lfsr;
pub mod memchr;
pub mod packet;
pub mod rle;
pub mod roaring;
pub mod sha256;
pub mod sketch;
pub mod sorting;

use std::fmt;

/// Why a design could not be built.
///
/// The front end has two error types -- [`ferrite_lithic::Error`] for graph
/// construction and [`ferrite_lithic::PortError`] for the port-list helpers -- and a
/// design builder calls both, so it needs somewhere to put either. A boxed trait
/// object would also work and would say less.
#[derive(Debug)]
pub enum BuildError {
    /// A node could not be built.
    Design(ferrite_lithic::Error),
    /// The port list could not be materialised.
    Ports(ferrite_lithic::PortError),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Design(inner) => write!(f, "{inner}"),
            Self::Ports(inner) => write!(f, "{inner}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<ferrite_lithic::Error> for BuildError {
    fn from(inner: ferrite_lithic::Error) -> Self {
        Self::Design(inner)
    }
}

impl From<ferrite_lithic::PortError> for BuildError {
    fn from(inner: ferrite_lithic::PortError) -> Self {
        Self::Ports(inner)
    }
}

/// The clock port name every design in this crate uses.
///
/// Named rather than positional so the emitted Verilog, the testbench and the
/// cosimulator all agree on it without each of them being told.
pub const CLOCK: &str = "clk";

/// The reset port name every design in this crate uses.
pub const RESET: &str = "rst";
