//! What a cosimulation can fail at, and what a disagreement looks like.

use core::fmt;

use ferrite_lithic_bits::Bits;

/// Something in the cosimulator could not be done.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A stimulus cycle had the wrong number of values.
    Arity {
        /// The cycle index.
        cycle: u64,
        /// How many values the port list wants.
        expected: usize,
        /// How many were supplied.
        got: usize,
    },
    /// A value was not its port's width.
    Width {
        /// The cycle index.
        cycle: u64,
        /// The input port's position in the port list.
        port: usize,
        /// The width the port declares.
        expected: u32,
        /// The width the value has.
        got: u32,
    },
    /// A word in the harness's output was not hex.
    HexWord {
        /// The word, as it appeared.
        word: String,
    },
    /// The design could not be emitted.
    Emit(Box<ferrite_lithic_rtl::Error>),
    /// The front end rejected the design or a builder's arguments.
    Design(Box<ferrite_lithic::Error>),
    /// The simulator rejected the design, a port name or a value.
    Sim(Box<ferrite_lithic_sim::Error>),
    /// Verilator was not found.
    NoVerilator,
    /// The plan names a clock the design does not declare as an input.
    ClockNotDeclared {
        /// The clock's name.
        clock: String,
        /// Every declared input name.
        known: Vec<String>,
    },
    /// A process failed.
    Process {
        /// What was run, for the message.
        what: String,
        /// Its exit status, if it ran at all.
        status: Option<i32>,
        /// Whatever it wrote to stderr.
        stderr: String,
    },
    /// A file operation failed.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arity {
                cycle,
                expected,
                got,
            } => write!(
                f,
                "cycle {cycle} has {got} value(s) where the port list wants {expected}. \
                 Both backends are driven from one stimulus list, so a cycle that \
                 is a different shape makes 'cycle {cycle}' mean two things and \
                 every comparison after it meaningless"
            ),
            Self::Width {
                cycle,
                port,
                expected,
                got,
            } => write!(
                f,
                "cycle {cycle}, input port {port}: the value is {got} bit(s) where \
                 the port declares {expected}. A value of the wrong width is \
                 refused rather than truncated, because truncation would look like \
                 a design bug three stages later"
            ),
            Self::HexWord { word } => write!(
                f,
                "`{word}` is not a hex word. The harness prints one fixed-width hex \
                 word per output per cycle; anything else means the harness was not \
                 the one this crate generated"
            ),
            Self::Emit(inner) => write!(f, "{inner}"),
            Self::Design(inner) => write!(f, "{inner}"),
            Self::Sim(inner) => write!(f, "{inner}"),
            Self::ClockNotDeclared { clock, known } => write!(
                f,
                "the module's clock is `{clock}`, which is not a declared input \
                 port. Declared inputs: {}.",
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            ),
            Self::NoVerilator => write!(
                f,
                "verilator was not found on PATH. Cosimulation needs it; without \
                 it, run the checks that do not: the simulator has its own tests, \
                 and `ferrite-lithic-rtl`'s Icarus gate proves the emitted Verilog \
                 elaborates"
            ),
            Self::Process {
                what,
                status,
                stderr,
            } => write!(
                f,
                "{what} failed ({status:?}){}",
                if stderr.trim().is_empty() {
                    String::new()
                } else {
                    format!(":\n{}", stderr.trim())
                }
            ),
            Self::Io(inner) => write!(f, "{inner}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Design(inner) => Some(inner),
            Self::Emit(inner) => Some(inner),
            Self::Sim(inner) => Some(inner),
            Self::Io(inner) => Some(inner),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(inner: std::io::Error) -> Self {
        Self::Io(inner)
    }
}

impl From<ferrite_lithic_rtl::Error> for Error {
    fn from(inner: ferrite_lithic_rtl::Error) -> Self {
        Self::Emit(Box::new(inner))
    }
}

impl From<ferrite_lithic::Error> for Error {
    fn from(inner: ferrite_lithic::Error) -> Self {
        Self::Design(Box::new(inner))
    }
}

impl From<ferrite_lithic_sim::Error> for Error {
    fn from(inner: ferrite_lithic_sim::Error) -> Self {
        Self::Sim(Box::new(inner))
    }
}

/// One cycle of one signal that is not what was expected.
///
/// The same shape as `ferrite-lithic_wave::Mismatch`, deliberately: a test that
/// asserts on a waveform and a test that cosimulates should report a failure the
/// same way, because the person reading either is the same person.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mismatch {
    /// The cycle index.
    pub cycle: u64,
    /// The signal or port name.
    pub port: String,
    /// What was expected.
    pub expected: Bits,
    /// What was observed.
    pub got: Bits,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cycle {} of `{}` is {} but should be {}",
            self.cycle,
            self.port,
            self.got.to_binary_digits(),
            self.expected.to_binary_digits()
        )
    }
}

impl std::error::Error for Mismatch {}

/// What a comparison found.
///
/// A count of agreements is worth carrying, because "compared 400 cycles and
/// agreed on all of them" is the result this crate exists to produce, and a bare
/// `Ok(())` does not say it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Report {
    /// How many cycles were compared.
    pub cycles: usize,
    /// Every cycle and port that disagreed, in cycle order.
    pub divergences: Vec<crate::equivalence::Divergence>,
}

impl Report {
    /// Whether the two backends agreed on everything.
    #[must_use]
    pub fn is_equivalent(&self) -> bool {
        self.divergences.is_empty()
    }

    /// The first disagreement, which is the one to read.
    #[must_use]
    pub fn first(&self) -> Option<&crate::equivalence::Divergence> {
        self.divergences.first()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_equivalent() {
            return write!(f, "equivalent over {} cycle(s)", self.cycles);
        }
        write!(
            f,
            "{} divergence(s) over {} cycle(s); first: {}",
            self.divergences.len(),
            self.cycles,
            self.divergences[0]
        )
    }
}
