//! Work in progress: not part of the crate yet.
//!
//! The entropy-coding tier and BLAKE3 and the FFT are being written. This file
//! exists so that a `pub mod` line for it resolves, which is the whole of its
//! job: while the work was in flight it was declared but absent, and a declared
//! module with no file does not compile, so the workspace was red for reasons
//! that had nothing to do with the code anyone was reading.
//!
//! Nothing here is a design, and no test refers to it. When the real module
//! lands this file is replaced by it.

/// A placeholder: the module is not implemented.
#[must_use]
pub const IMPLEMENTED: bool = false;
