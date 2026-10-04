//! The two ways a waveform disagrees with what it was asked to record.

use std::fmt;

use ferrite_lithic_bits::Bits;

/// A waveform that cannot be built as asked.
///
/// Every variant here is a case where recording the value anyway would put a
/// *plausible* thing in the file: a zero for a signal nobody had, a truncated
/// width, a name two signals share. Refusing is cheaper than debugging a
/// waveform that is confidently wrong.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Two signals were declared with one name.
    DuplicateName {
        /// The name both were declared with.
        name: String,
    },
    /// A port was declared with no bits.
    ZeroWidth {
        /// The port.
        port: String,
    },
    /// A port handle from a different waveform was used.
    UnknownPort {
        /// The port's name, which is the closest thing to an identity it has.
        port: String,
    },
    /// A name was looked up that this waveform does not declare.
    UnknownPortName {
        /// The name that was asked for.
        name: String,
        /// Every name this waveform declares.
        known: Vec<String>,
    },
    /// A value was not its port's width.
    Width {
        /// The port.
        port: String,
        /// The width the port declares.
        expected: u32,
        /// The width the value has.
        got: u32,
    },
    /// A lookup by name had no value for a declared signal.
    UnknownSignal {
        /// The signal.
        name: String,
    },
    /// A cycle was read that was never recorded.
    UnknownCycle {
        /// The cycle index.
        cycle: u64,
        /// How many cycles are recorded.
        recorded: usize,
    },
    /// A port list's names and widths disagree in length.
    ShapeMismatch {
        /// How many names it produced.
        names: usize,
        /// How many widths it declares.
        widths: usize,
    },
    /// Writing the file failed.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName { name } => write!(
                f,
                "two signals are declared as `{name}`. A viewer shows one name \
                 once, so the second value would overwrite the first with nothing \
                 to say which was meant"
            ),
            Self::ZeroWidth { port } => write!(
                f,
                "signal `{port}` is declared with zero bits. There is no value to \
                 record and no Verilog declaration to match, so the width is at \
                 least one by construction"
            ),
            Self::UnknownPort { port } => write!(
                f,
                "`{port}` is not a signal of this waveform, or is declared with a \
                 different width. Port handles are only valid for the waveform that \
                 declared them; re-declare it there rather than reusing a handle"
            ),
            Self::UnknownPortName { name, known } => {
                write!(
                    f,
                    "no signal named `{name}`. This waveform has: {}.",
                    known.join(", ")
                )
            }
            Self::Width {
                port,
                expected,
                got,
            } => write!(
                f,
                "signal `{port}` is {expected} bit(s) wide and the value is {got}. A \
                 waveform that records a value of the wrong width is a design that \
                 disagrees with itself, so this is refused rather than truncated"
            ),
            Self::UnknownSignal { name } => write!(
                f,
                "no value for the declared signal `{name}`. Recording a zero \
                 instead would put a plausible lie in the file, and a waveform that \
                 silently invents values is worse than one that refuses"
            ),
            Self::UnknownCycle { cycle, recorded } => write!(
                f,
                "cycle {cycle} was never recorded; {recorded} cycle(s) are. Reading \
                 an unrecorded cycle as zero would make 'nothing ran' look like \
                 'everything was zero'"
            ),
            Self::ShapeMismatch { names, widths } => write!(
                f,
                "the port list produced {names} name(s) for {widths} width(s). A \
                 shape whose names and widths disagree cannot be declared, because \
                 every signal needs both"
            ),
            Self::Io(inner) => write!(f, "{inner}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
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

/// One cycle of one signal that is not what was expected.
///
/// The first mismatch and nothing else, on purpose: a simulation that disagrees
/// with itself does not need a diff, it needs the cycle it first went wrong.
/// One cycle of one signal that is not what was expected.
///
/// The first mismatch and nothing else, on purpose: a simulation that disagrees
/// with itself does not need a diff, it needs the cycle it first went wrong.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mismatch {
    /// The signal.
    pub port: String,
    /// The cycle index.
    pub cycle: u64,
    /// What it should have been.
    pub expected: Bits,
    /// What it was, or [`None`] if that cycle was never recorded.
    pub got: Option<Bits>,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.got {
            Some(got) => write!(
                f,
                "cycle {} of `{}` is {} but should be {}",
                self.cycle,
                self.port,
                crate::data::render_value(got, ferrite_lithic::WaveFormat::Binary),
                crate::data::render_value(&self.expected, ferrite_lithic::WaveFormat::Binary)
            ),
            None => write!(
                f,
                "cycle {} of `{}` was never recorded, and should have been {}",
                self.cycle,
                self.port,
                crate::data::render_value(&self.expected, ferrite_lithic::WaveFormat::Binary)
            ),
        }
    }
}

impl std::error::Error for Mismatch {}
