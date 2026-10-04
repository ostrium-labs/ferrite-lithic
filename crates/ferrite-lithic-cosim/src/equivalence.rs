//! The comparison itself, and the divergence it reports.
//!
//! Everything here is pure, which is why it lives in its own module: this is the
//! code with the arithmetic in it, and it has to be checkable on a machine with no
//! Verilator installed.
//!
//! Every function here is testable without Verilator installed, which is the point
//! of putting them in their own module: the harness is net-new work, and the only
//! way to be confident in the parts that run is to keep them separate from the
//! one part that needs a vendor toolchain.

use ferrite_lithic_bits::Bits;

use crate::error::{Error, Mismatch, Report};

/// Which backend a value came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// `ferrite-lithic-sim`.
    Simulator,
    /// The emitted Verilog, under Verilator.
    Verilator,
}

impl core::fmt::Display for Source {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Simulator => "ferrite-lithic-sim",
            Self::Verilator => "verilator",
        })
    }
}

/// One cycle of one output port the two backends disagree about.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Divergence {
    /// The cycle index.
    pub cycle: u64,
    /// The output port's name.
    pub port: String,
    /// What the simulator said.
    pub simulator: Bits,
    /// What the Verilog said.
    pub verilator: Bits,
}

impl core::fmt::Display for Divergence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "cycle {} of `{}`: {} says {}, {} says {}",
            self.cycle,
            self.port,
            Source::Simulator,
            self.simulator.to_binary_digits(),
            Source::Verilator,
            self.verilator.to_binary_digits(),
        )
    }
}

impl Divergence {
    /// The two values as a mismatch, for a caller that wants one shape of failure.
    #[must_use]
    pub fn as_mismatch(&self) -> Mismatch {
        Mismatch {
            cycle: self.cycle,
            port: self.port.clone(),
            expected: self.simulator.clone(),
            got: self.verilator.clone(),
        }
    }
}

/// Compares two backends' per-cycle output values.
///
/// # Errors
///
/// [`Error::Arity`] if the two runs produced a different
/// number of cycles, which means one of them stopped early and comparing the
/// common prefix would report agreement on a run that never finished.
pub fn compare(
    port_names: &[String],
    simulator: &[Vec<Bits>],
    verilator: &[Vec<Bits>],
) -> Result<Report, Error> {
    if simulator.len() != verilator.len() {
        return Err(Error::Arity {
            cycle: simulator.len().min(verilator.len()) as u64,
            expected: simulator.len(),
            got: verilator.len(),
        });
    }
    let mut divergences = Vec::new();
    for (cycle, (left, right)) in simulator.iter().zip(verilator).enumerate() {
        if left.len() != right.len() {
            return Err(Error::Arity {
                cycle: cycle as u64,
                expected: left.len(),
                got: right.len(),
            });
        }
        for (index, (mine, theirs)) in left.iter().zip(right).enumerate() {
            if mine != theirs {
                divergences.push(Divergence {
                    cycle: cycle as u64,
                    port: port_names
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("output {index}")),
                    simulator: mine.clone(),
                    verilator: theirs.clone(),
                });
            }
        }
    }
    Ok(Report {
        cycles: simulator.len(),
        divergences,
    })
}
