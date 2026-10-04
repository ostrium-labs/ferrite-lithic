//! Why a module could not be emitted.

use core::fmt;

use ferrite_lithic_bits::Error as BitsError;
use ferrite_lithic_ir::{Error as IrError, NodeId};

use crate::module::Direction;

/// An emission failure.
///
/// # One type, and why it is not split
///
/// The emitter's failures are all the same kind of thing: the graph is legal, and
/// something about *describing* it as Verilog has no answer. A register clocked by
/// a literal, a wire driven by a whole memory, an input port with no name — none of
/// these is a bug in the circuit, and all of them are bugs in the request to emit
/// it. Splitting "the graph is wrong" from "the module description is wrong" would
/// double the type for no gain in what a caller can do about it, which is fix the
/// description.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// The circuit failed one of the IR's own checks.
    Ir(IrError),
    /// The module has no name.
    ///
    /// A module name is the one identifier that cannot be derived, so there is
    /// nothing to fall back to. Pass one to [`Module::new`](crate::Module::new).
    EmptyModuleName,
    /// A port wire has no name.
    ///
    /// Under [`PortNames::Wire`](crate::PortNames::Wire) a port's identifier *is*
    /// its name, so an unnamed port cannot be emitted. Name it when it is declared
    /// — `Design::input` and `Design::output` both require a name for exactly this
    /// reason.
    UnnamedPort {
        /// Which list the port came from.
        direction: Direction,
        /// The port's node.
        id: NodeId,
    },
    /// A node's name is the empty string.
    EmptySignalName {
        /// The node carrying the empty name.
        id: NodeId,
    },
    /// The same node is listed as both an input and an output port.
    ///
    /// That is an `inout` in Verilog, and this emitter does not model tri-state
    /// ([the design notes](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/design-notes.md)
    /// record it as deliberately not ported). Listing it twice is refused rather
    /// than emitted as a port with two directions.
    PortIsBothDirections {
        /// The node listed twice.
        id: NodeId,
    },
    /// A register's clock is a literal.
    ///
    /// `@(posedge 1'b0)` is not a thing that can ever fire, and `@(posedge 1'b1)`
    /// is not a clock. There is no honest text for a literal edge, so a literal
    /// clock is refused instead of guessed at.
    ConstantClock {
        /// The register with the literal clock.
        reg: NodeId,
    },
    /// A register is clocked by a different signal than the module's clock.
    ///
    /// A1 is a single clock domain. Two clocks need per-domain edge scheduling, and
    /// emitting the second one into the first's `always` block would produce a
    /// circuit that is not the one described.
    ClockMismatch {
        /// The register with the wrong clock.
        reg: NodeId,
        /// The clock it actually uses.
        found: NodeId,
        /// The module's declared clock.
        expected: NodeId,
    },
    /// A module has state but no clock.
    ///
    /// Built with [`Module::combinational`](crate::Module::combinational), which
    /// takes no clock. A register or a memory needs one to be written at all.
    NoClock {
        /// What needed the clock.
        kind: &'static str,
        /// The node that needed it.
        id: NodeId,
    },
    /// A node is used where a single value is expected, and it is not one.
    ///
    /// Three things reach it. A memory is an *array*, so it has no value expression
    /// and the fix is a read port. A hand-built `Select` can have a length of zero —
    /// `Circuit::select` refuses one, but `add_node` does not check — and
    /// `a[0 +: 0]` is not Verilog. A hand-built `Replicate` can have a count of zero,
    /// which is a zero-width result and the declaration would be a lie.
    ///
    /// The variant names the offending kind rather than only handling memories,
    /// because the emitter's job is to know which node kinds are values, and this is
    /// where that answer is written down.
    NotAValue {
        /// The node used as a value.
        id: NodeId,
        /// What kind of node it is.
        kind: &'static str,
    },
    /// A literal could not be built at the width the graph asked for.
    ///
    /// The only literal this emitter builds is a register's reset value at the
    /// register's own width, and a width of zero cannot happen in a checked graph.
    /// The variant exists because `Bits::zeros` is fallible and a `?` needs
    /// somewhere to land; it is unreachable in practice.
    Bits(BitsError),
    /// An output port has no driver.
    UndrivenPort {
        /// The output port.
        id: NodeId,
    },
    /// Two modules in one library share a name but disagree about their ports.
    ///
    /// Modules with the same name and the same port signature are collapsed to one
    /// definition, which is what "one module per instance, deduplicated by name"
    /// means. Two same-named modules with *different* ports cannot both be
    /// defined, so this is an error.
    ModuleSignatureConflict {
        /// The shared module name.
        name: String,
        /// How many ports the first definition had, as `inputs, outputs`.
        first: (usize, usize),
        /// How many the second had.
        second: (usize, usize),
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ir(e) => write!(f, "the circuit failed its own checks: {e}"),
            Self::EmptyModuleName => {
                write!(f, "the module has no name; pass one to Module::new")
            }
            Self::UnnamedPort { direction, id } => write!(
                f,
                "{id} is an {direction} port with no name, and a port's identifier is its \
                 name; name it where it is declared"
            ),
            Self::EmptySignalName { id } => {
                write!(
                    f,
                    "{id} has an empty name; clear it instead of setting it to \"\""
                )
            }
            Self::PortIsBothDirections { id } => write!(
                f,
                "{id} is listed as both an input and an output port, which is an inout; \
                 list it once"
            ),
            Self::ConstantClock { reg } => write!(
                f,
                "register {reg} is clocked by a literal, which has no edge; drive it from a \
                 clock signal"
            ),
            Self::ClockMismatch {
                reg,
                found,
                expected,
            } => write!(
                f,
                "register {reg} is clocked by {found}, but the module's clock is {expected}; \
                 A1 emits one clock domain, so make them the same signal"
            ),
            Self::NoClock { kind, id } => write!(
                f,
                "{kind} {id} needs a clock, but this module was built without one; use \
                 Module::new"
            ),
            Self::NotAValue { id, kind } => write!(
                f,
                "{id} has no single-value expression: it is a {kind} node. A memory must be \
                 read through a read port, and a node with no width has nothing to emit"
            ),
            Self::Bits(e) => write!(f, "a literal could not be built: {e}"),
            Self::UndrivenPort { id } => {
                write!(f, "output port {id} has no driver; drive it")
            }
            Self::ModuleSignatureConflict {
                name,
                first,
                second,
            } => write!(
                f,
                "module `{name}` is defined twice with different ports: first {} in, {} out, \
                 then {} in, {} out; rename one",
                first.0, first.1, second.0, second.1
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Ir(e) => Some(e),
            Self::Bits(e) => Some(e),
            _ => None,
        }
    }
}

impl From<IrError> for Error {
    fn from(e: IrError) -> Self {
        Self::Ir(e)
    }
}

impl From<BitsError> for Error {
    fn from(e: BitsError) -> Self {
        Self::Bits(e)
    }
}
