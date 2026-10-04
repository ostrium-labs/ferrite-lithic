//! Why a circuit could not be compiled or stepped.

use core::fmt;

use ferrite_lithic_bits::Error as BitsError;
use ferrite_lithic_ir::Error as IrError;

/// A compile or simulation failure.
///
/// # Compile errors and step errors are the same type on purpose
///
/// A cycle simulator has one unavoidable runtime error class — signed division
/// overflow — and it is data-dependent, so it cannot be a compile error without
/// giving up on data-dependent division. Keeping one error type means a caller
/// handling simulation cannot forget the step half, and `Display` says which kind
/// it was.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// The circuit failed one of the IR's own checks.
    Ir(IrError),
    /// The flat buffer could not be sized.
    Bits(BitsError),
    /// The design failed its own checks before it could be compiled.
    Design(ferrite_lithic::Error),
    /// A signal from another design was used where one from this design was needed.
    ///
    /// The node ids are arena-local, so accepting a foreign signal would read a
    /// plausible-looking but entirely unrelated node. That is the worst kind of
    /// wrong answer: it would produce a simulation of a circuit the user never
    /// wrote.
    ForeignSignal {
        /// The signal's name, if it has one.
        name: String,
    },
    /// A named input port does not exist.
    ///
    /// The known names are listed: a typo in a testbench's port name is otherwise
    /// indistinguishable from a design with no inputs at all.
    NoSuchInput {
        /// The name that was asked for.
        name: String,
        /// The input ports that do exist, in declaration order.
        known: Vec<String>,
    },
    /// A named output port does not exist.
    NoSuchOutput {
        /// The name that was asked for.
        name: String,
        /// The output ports that do exist, in declaration order.
        known: Vec<String>,
    },
    /// A value was written to a port of the wrong width.
    PortWidth {
        /// The port's name.
        name: String,
        /// The port's declared width.
        expected: u32,
        /// The width of the value supplied.
        got: u32,
    },
    /// A submodule instance has no simulation model.
    ///
    /// Instances are combinational black boxes to the graph and opaque storage to
    /// a simulator, so modelling one means writing a per-instance model with its
    /// own state. That is net-new work, and until it exists the honest answer is
    /// to refuse rather than to treat an instance as a constant zero — which would
    /// simulate a *different circuit* and quietly pass tests.
    NoInstanceModel {
        /// The submodule name, as it will appear in emitted Verilog.
        name: String,
    },
    /// The design uses more than one clock signal.
    ///
    /// A1 has a single clock domain. Two clocks would need per-domain edge
    /// scheduling, and simulating them as one is worse than refusing: the results
    /// would depend on the order registers happened to be visited in.
    MultipleClocks {
        /// One clock's name.
        first: String,
        /// The other clock's name.
        second: String,
    },
    /// A design with no register has no clock to drive.
    ///
    /// A register-free design has nothing that changes between edges, so the
    /// three-phase step has no edge to apply. This is legal to simulate as long as
    /// the caller only ever settles combinations, which is what
    /// [`Sim::comb`](crate::Sim::comb) is for.
    NoClock,
    /// The buffer would need more words than can be addressed.
    ///
    /// Only reachable through a memory whose `depth * words_per_word` overflows,
    /// which takes an absurd `depth` — but silently wrapping would hand back a
    /// buffer that is far too small, so it is checked.
    BufferTooLarge {
        /// What was being sized.
        what: String,
        /// The width of the value that made it too large.
        width: u32,
    },
    /// Unsigned division or remainder by zero.
    ///
    /// Reported rather than answered, even though the answer would be zero. A
    /// divide-by-zero in a design is a bug in the design, and a two-state
    /// simulator has no way to make it visible the way Verilog's `x` would — so
    /// it has to be an error here or it is invisible everywhere. See the crate
    /// docs for why this is a divergence from the emitted Verilog.
    DivisionByZero {
        /// Which operation asked.
        op: &'static str,
    },
    /// Signed division that overflows: the most negative value over `-1`.
    ///
    /// The graph is legal — the operands have widths, and nothing about the
    /// *graph* is wrong — so this can only be caught by running it, which is why
    /// stepping a design is fallible at all.
    SignedDivisionOverflow {
        /// The operand width, in bits.
        width: u32,
    },
    /// A node never received a word address, which is a bug in this crate.
    ///
    /// Kept as a real error rather than a panic so a failing differential test can
    /// report it instead of unwinding through a property-test harness.
    Unplaced {
        /// The node that has no address.
        node: ferrite_lithic_ir::NodeId,
    },
    /// A chain of wires driven by wires was longer than the circuit has nodes.
    WireChainTooLong {
        /// The wire the walk started from.
        node: ferrite_lithic_ir::NodeId,
    },
    /// An initial value was requested for something that is not a register.
    InitializeTarget {
        /// The node's kind, e.g. `Mem`.
        kind: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Ir(e) => write!(f, "{e}"),
            Error::Bits(e) => write!(f, "{e}"),
            Error::Design(e) => write!(f, "{e}"),
            Error::ForeignSignal { name } => write!(
                f,
                "`{name}` belongs to a different design; a node id only means something \
                 within one arena, so using it here would read an unrelated node"
            ),
            Error::NoSuchInput { name, known } => {
                write!(f, "no input port named `{name}`")?;
                write_ports(f, known)
            }
            Error::NoSuchOutput { name, known } => {
                write!(f, "no output port named `{name}`")?;
                write_ports(f, known)
            }
            Error::PortWidth {
                name,
                expected,
                got,
            } => write!(
                f,
                "input port `{name}` is {expected} bits wide but the value is {got} bits; \
                 resize the value with `Bits::zero_extend`, `Bits::sign_extend` or \
                 `Bits::truncate` to match the port"
            ),
            Error::NoInstanceModel { name } => write!(
                f,
                "submodule `{name}` has no simulation model; ferrite-lithic-sim does not \
                 model instances yet, and treating one as a constant zero would simulate \
                 a different circuit"
            ),
            Error::MultipleClocks { first, second } => write!(
                f,
                "this design has more than one clock, `{first}` and `{second}`; Phase A1 \
                 simulates a single clock domain, and simulating two as one would make \
                 results depend on the order registers are visited in"
            ),
            Error::NoClock => write!(
                f,
                "this design has no register, so it has no clock to drive at a clock edge; \
                 a register-free design can still be settled with `comb`"
            ),
            Error::BufferTooLarge { what, width } => write!(
                f,
                "{what} with a width of {width} bits needs more words than the buffer can \
                 address; halve the width or the depth"
            ),
            Error::DivisionByZero { op } => write!(
                f,
                "{op} by zero; a two-state simulator has no `x` to produce here, so this \
                 is reported rather than silently answered with zero. Emitted Verilog \
                 yields `x` for the same operation, which is a cosim difference to handle \
                 there."
            ),
            Error::SignedDivisionOverflow { width } => write!(
                f,
                "signed division overflows: the most negative {width}-bit value over -1 has \
                 no representable result"
            ),
            Error::Unplaced { node } => write!(
                f,
                "internal error: node {} was never given a word address, so the \
                 simulator cannot evaluate it",
                node.index()
            ),
            Error::WireChainTooLong { node } => write!(
                f,
                "internal error: the chain of wires driven by wires starting at node \
                 {} is longer than the circuit has nodes",
                node.index()
            ),
            Error::InitializeTarget { kind } => write!(
                f,
                "an initial value can only be given to a register, but this signal is a {kind}; \
                 memories start zero-filled, which is the only initial state they have"
            ),
        }
    }
}

impl core::error::Error for Error {}

impl From<ferrite_lithic::Error> for Error {
    fn from(e: ferrite_lithic::Error) -> Self {
        Error::Design(e)
    }
}

impl From<IrError> for Error {
    fn from(e: IrError) -> Self {
        Error::Ir(e)
    }
}

impl From<BitsError> for Error {
    fn from(e: BitsError) -> Self {
        Error::Bits(e)
    }
}

/// Appends a port list to a "no such port" message, or says there are none.
fn write_ports(f: &mut fmt::Formatter<'_>, known: &[String]) -> fmt::Result {
    if known.is_empty() {
        write!(f, "; the design declares no ports of that direction")
    } else {
        write!(f, "; it declares {}", known.join(", "))
    }
}
