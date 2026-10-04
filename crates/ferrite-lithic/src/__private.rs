//! Implementation details `#[derive(PortList)]` calls.
//!
//! Not an API. Everything here is reachable through [`PortList`](crate::PortList)
//! and the four
//! functions that turn a shape into a design ([`inputs`](crate::inputs),
//! [`outputs`](crate::outputs), [`wires`](crate::wires),
//! [`assign`](crate::assign)). This module exists so generated code can call the
//! field-checking helpers, and it is at the crate root rather than inside
//! `ports` because the generated code reaches it as `ferrite_lithic::__private`,
//! which has to be a real path.
//!
//! `__private` is the serde convention for exactly this: a macro's helpers,
//! public enough to be called across a crate boundary, named so that seeing them
//! in generated code is not mistaken for something a reader chose to write.

use crate::ports::Error;

pub use crate::ports::{check_width, take_many, take_one};

/// Checks a signal count against what a list needs.
///
/// # Errors
///
/// [`Error::Arity`] if the counts differ.
pub fn check_arity(got: usize, expected: usize) -> Result<(), Error> {
    if got == expected {
        return Ok(());
    }
    Err(Error::Arity { expected, got })
}

/// Checks that only `#[exists]` ports are marked absent.
///
/// # Errors
///
/// [`Error::NotOptional`] naming the first port that is marked absent but
/// cannot be.
pub fn check_optional(
    present: &[bool],
    optional: &[bool],
    names: &'static [&'static str],
) -> Result<(), Error> {
    for (index, here) in present.iter().enumerate() {
        if !here && !optional.get(index).copied().unwrap_or(false) {
            return Err(Error::NotOptional {
                port: names
                    .get(index)
                    .map_or_else(|| format!("port {index}"), ToString::to_string),
            });
        }
    }
    Ok(())
}
