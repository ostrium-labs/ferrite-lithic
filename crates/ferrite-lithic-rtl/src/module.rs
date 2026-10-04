//! What to emit: a named circuit with a port list and a clock.

use ferrite_lithic_ir::{Circuit, NodeId};

use crate::error::Error;
use crate::naming::legalise;

/// Which way a port points.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    /// Driven from outside the module.
    Input,
    /// Read from outside the module.
    Output,
}

impl core::fmt::Display for Direction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Input => "input",
            Self::Output => "output",
        })
    }
}

/// How a module's ports are named in its port list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PortNames {
    /// The port's identifier is the name on its wire.
    ///
    /// This is the default, and it is what you want for a leaf module: the names in
    /// the port list are the names in the design.
    #[default]
    Wire,
    /// Inputs are `i0`, `i1`, … and outputs are `o0`, `o1`, …, in port order.
    ///
    /// # Why this exists
    ///
    /// [`Node::Instance`](ferrite_lithic_ir::Node::Instance) records a
    /// submodule's *name*, its parameters and its inputs' *order* — but not the
    /// names of its ports, because the graph has nowhere to put them. An
    /// instantiation site therefore cannot name the ports it connects to, and a
    /// submodule emitted with [`PortNames::Wire`] cannot be bound by one.
    ///
    /// A positional scheme closes that loop with no guesswork: an instantiation
    /// site connects `.i0(..)`, `.i1(..)`, `.o0(..)` in order, and a submodule
    /// declared with these names is bindable. Positional names are *reserved*
    /// before any user name is allocated, so a signal called `i0` is renamed
    /// rather than allowed to shadow a port and break the binding.
    Positional,
}

/// A named circuit, its ports, and its clock.
///
/// # Why the port list is explicit
///
/// A module's input port is an *undriven wire* — something outside supplies it.
/// So "which nodes are ports" cannot be recovered from the graph after the fact
/// without a rule, and the rule Hardcaml uses is the port list its `derive` macro
/// generates (`src/signal_graph.ml:59-62`). We take the list explicitly for the
/// same reason [`Circuit::check_with_inputs`](ferrite_lithic_ir::Circuit::check_with_inputs)
/// does: a guessed port set is a guessed module interface, and a testbench written
/// against the guess is a testbench for the wrong circuit.
///
/// # Why the clock is a field
///
/// A memory write port carries no clock ([`WritePort`](ferrite_lithic_ir::WritePort)
/// is data, address and enable), and the IR's one-clock-domain rule is enforced
/// nowhere in the types. Naming the clock on the module turns that from a runtime
/// surprise into a fact every emitter can rely on: the memory's write ports clock
/// off the module clock, and a register whose clock is some *other* signal is
/// [`Error::ClockMismatch`] rather than a second domain quietly folded into the
/// first.
#[derive(Clone, Debug)]
pub struct Module {
    name: String,
    circuit: Circuit,
    clock: Option<NodeId>,
    inputs: Vec<NodeId>,
    outputs: Vec<NodeId>,
    ports: PortNames,
}

/// The port counts that make two same-named modules the same definition.
///
/// Two modules with one name and one signature are collapsed to a single emitted
/// definition. See [`emit_library`](crate::emit_library) for what that does and
/// does not prove.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Signature {
    /// How many inputs.
    pub inputs: usize,
    /// How many outputs.
    pub outputs: usize,
    /// The input widths, in order.
    pub input_widths: Vec<u32>,
    /// The output widths, in order.
    pub output_widths: Vec<u32>,
    /// How ports are named.
    pub ports: PortNames,
}

impl Module {
    /// A module with a clock.
    ///
    /// The clock is what every register and every memory write port in this module
    /// is clocked by; A1 has no second domain.
    ///
    /// `name` is legalised into an identifier the way every signal name is, so
    /// `Module::new("signed", ..)` emits `module signed_`. A module name is the one
    /// name that cannot be *mangled* — there is no pool to take a free name from,
    /// and a submodule must stay findable under the name its instantiation site
    /// spells — but it can still be rewritten into something a parser accepts.
    /// [`name`](Module::name) reports what was emitted.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        circuit: Circuit,
        clock: NodeId,
        inputs: Vec<NodeId>,
        outputs: Vec<NodeId>,
    ) -> Self {
        Self {
            name: legalise(&name.into()),
            circuit,
            clock: Some(clock),
            inputs,
            outputs,
            ports: PortNames::Wire,
        }
    }

    /// A module with no clock, for a design with no state.
    ///
    /// A register or a memory in a clockless module is [`Error::NoClock`]. Use this
    /// for a purely combinational block so that the absence of a clock is a
    /// decision rather than an omission.
    ///
    /// `name` is legalised, as in [`Module::new`].
    #[must_use]
    pub fn combinational(
        name: impl Into<String>,
        circuit: Circuit,
        inputs: Vec<NodeId>,
        outputs: Vec<NodeId>,
    ) -> Self {
        Self {
            name: legalise(&name.into()),
            circuit,
            clock: None,
            inputs,
            outputs,
            ports: PortNames::Wire,
        }
    }

    /// Choose how this module's ports are named.
    #[must_use]
    pub fn with_port_names(mut self, ports: PortNames) -> Self {
        self.ports = ports;
        self
    }

    /// The module's name, as it appears in `module <name> (`.
    ///
    /// This is the legalised name, which is not always the name that went in: a name
    /// that is a Verilog keyword, or that cannot be spelled in Verilog at all, has
    /// already been rewritten. The instantiation of a submodule legalises the name
    /// it is given the same way, so a definition and the site that names it always
    /// agree.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The circuit to emit.
    #[must_use]
    pub const fn circuit(&self) -> &Circuit {
        &self.circuit
    }

    /// The module's clock, if it has one.
    #[must_use]
    pub const fn clock(&self) -> Option<NodeId> {
        self.clock
    }

    /// The input ports, in port-list order.
    #[must_use]
    pub fn inputs(&self) -> &[NodeId] {
        &self.inputs
    }

    /// The output ports, in port-list order.
    #[must_use]
    pub fn outputs(&self) -> &[NodeId] {
        &self.outputs
    }

    /// How this module's ports are named.
    #[must_use]
    pub const fn port_names(&self) -> PortNames {
        self.ports
    }

    /// This module's port counts and widths, for comparing two definitions.
    ///
    /// # Errors
    ///
    /// [`Error::Ir`], if a port id this circuit did not mint. It cannot be measured,
    /// and guessing a width for it would put a wrong number in a Verilog range.
    pub fn signature(&self) -> Result<Signature, Error> {
        let widths = |ids: &[NodeId]| -> Result<Vec<u32>, Error> {
            ids.iter()
                .map(|id| self.circuit.get(*id).map(|_| self.circuit.width_of(*id)))
                .collect::<Result<Vec<u32>, ferrite_lithic_ir::Error>>()
                .map_err(Error::Ir)
        };
        Ok(Signature {
            inputs: self.inputs.len(),
            outputs: self.outputs.len(),
            input_widths: widths(&self.inputs)?,
            output_widths: widths(&self.outputs)?,
            ports: self.ports,
        })
    }

    /// Emit this module: one `module` … `endmodule`.
    ///
    /// # Errors
    ///
    /// Whatever emitting the module reports, which is
    /// [`Error::Ir`] for a graph that fails its own checks and the rest for a
    /// module description Verilog cannot express.
    pub fn emit(&self) -> Result<String, Error> {
        crate::emit::emit_module(self)
    }
}
