//! What a testbench can fail on.

use std::fmt;

/// Everything a testbench can refuse.
#[derive(Debug)]
pub enum Error {
    /// The simulator rejected the design.
    Sim(Box<ferrite_lithic_sim::Error>),
    /// A port name is not one of the design's ports.
    UnknownPort {
        /// The name that was used.
        name: String,
        /// Every input port the design declares.
        inputs: Vec<String>,
        /// Every output port the design declares.
        outputs: Vec<String>,
    },
    /// A value does not fit its port.
    PortWidth {
        /// The port's name.
        port: String,
        /// The width the port declares.
        expected: u32,
        /// The width of the value that was offered.
        got: u32,
    },
    /// A value was too wide for a `u64`.
    TooWide {
        /// The port's name.
        port: String,
        /// The width the port declares.
        width: u32,
    },
    /// A testbench future asked to suspend.
    ///
    /// See the crate docs: a cycle boundary here is *work*, not a yield, so no
    /// future this crate provides ever returns `Pending`. A foreign future that
    /// does is a future waiting for something that will never happen, and the
    /// testbench says so instead of hanging.
    Suspended,
    /// The design has no clock, so there is no cycle to step.
    NoClock,
    /// Waveform recording refused something.
    Wave(Box<ferrite_lithic_wave::Error>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sim(inner) => write!(f, "{inner}"),
            Self::UnknownPort {
                name,
                inputs,
                outputs,
            } => {
                write!(f, "no port named `{name}`")?;
                if !inputs.is_empty() {
                    write!(f, ". Inputs: {}", inputs.join(", "))?;
                }
                if !outputs.is_empty() {
                    write!(f, ". Outputs: {}", outputs.join(", "))?;
                }
                Ok(())
            }
            Self::PortWidth {
                port,
                expected,
                got,
            } => write!(
                f,
                "port `{port}` is {expected} bits wide but the value is {got}. \
                 Resize it, or use `drive_bits` with a value of the right width."
            ),
            Self::TooWide { port, width } => write!(
                f,
                "port `{port}` is {width} bits wide, which does not fit in a `u64`. \
                 Use `drive_bits`."
            ),
            Self::Suspended => write!(
                f,
                "a testbench future returned `Pending`. Every future this crate \
                 provides does its work on the first poll and then completes, \
                 because advancing a cycle is work rather than a suspension. A \
                 future that waits for a wake cannot be driven by a testbench \
                 that never yields."
            ),
            Self::NoClock => write!(
                f,
                "the design has no clock, so `step` has no edge to apply. Settle a \
                 combinational design with `settle` instead."
            ),
            Self::Wave(inner) => write!(f, "{inner}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ferrite_lithic_sim::Error> for Error {
    fn from(inner: ferrite_lithic_sim::Error) -> Self {
        Self::Sim(Box::new(inner))
    }
}

impl From<ferrite_lithic_wave::Error> for Error {
    fn from(inner: ferrite_lithic_wave::Error) -> Self {
        Self::Wave(Box::new(inner))
    }
}
