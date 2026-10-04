//! The steppable simulator.

use core::fmt;

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::NodeId;

use crate::compile::{Port, Program};
use crate::error::Error;
use crate::eval::Machine;
use crate::op::Word;

/// One cycle's worth of output values.
///
/// Both snapshots of a cycle are first-class values, inherited from Hardcaml's
/// [`src/cyclesim0_intf.ml:58`](https://github.com/jane-street/hardcaml), and the
/// distinction earns its keep: `before` is what the outputs were *before* the
/// edge, `after` is what they became *on* the edge. That is how you observe a
/// register on the same edge it changes, which is the whole reason a cycle
/// simulator keeps two rather than one.
///
/// The values are [`Bits`], not rendered text. Assertions compare cycle-indexed
/// values; a pretty picture is somebody else's problem, later.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    cycle: u64,
    outputs: Vec<(String, Bits)>,
}

impl Snapshot {
    /// The cycle this snapshot was taken at.
    ///
    /// Counting starts at 1: the counter advances *before* the first clock edge, so
    /// the first `before_clock_edge` is cycle 1. The state before the design has
    /// ever been clocked is [`Sim::initial`] instead, and it reports cycle 0.
    #[must_use]
    pub fn cycle(&self) -> u64 {
        self.cycle
    }

    /// The output values, in port declaration order.
    #[must_use]
    pub fn outputs(&self) -> &[(String, Bits)] {
        &self.outputs
    }

    /// The value of one output port.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Bits> {
        self.outputs.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cycle {}", self.cycle)?;
        for (name, value) in &self.outputs {
            write!(f, " {name}={}", value.to_binary_digits())?;
        }
        Ok(())
    }
}

/// A circuit compiled and ready to step.
///
/// Owns one flat word buffer and the [`Program`] that indexes it. The buffer
/// starts zero-filled, which is what makes registers and memories start at zero
/// with no separate initialisation pass.
#[derive(Clone, Debug)]
pub struct Sim {
    design: Design,
    program: Program,
    machine: Machine,
    cycle: u64,
}

impl Sim {
    /// Compiles a design and returns a simulator sitting at its initial state.
    ///
    /// # Errors
    ///
    /// [`Error::Design`] if the design fails its own checks, or whatever
    /// [`Program::compile`] reports.
    pub fn new(design: &Design) -> Result<Self, Error> {
        let circuit = design.build()?;
        let inputs: Vec<NodeId> = design.input_ports().iter().map(Signal::id).collect();
        let outputs: Vec<NodeId> = design.output_ports().iter().map(Signal::id).collect();
        let program = Program::compile(&circuit, &inputs, &outputs)?;
        Ok(Sim {
            design: design.clone(),
            machine: Machine::new(&program),
            program,
            cycle: 0,
        })
    }

    /// The compiled program. Read-only, and public on purpose.
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// The current cycle number, counting completed clock edges.
    #[must_use]
    pub fn cycle(&self) -> u64 {
        self.cycle
    }

    /// Gives a register a starting value.
    ///
    /// Applied to **both** the live and the shadow copy, and that detail is the
    /// whole point: a register whose clear is asserted never takes its reset
    /// value, so if only the live copy were written the shadow would overwrite it
    /// with zero at the first edge. Hardcaml writes both for the same reason
    /// ([`src/cyclesim_compile.ml:1074-1079`](https://github.com/jane-street/hardcaml)).
    ///
    /// # Errors
    ///
    /// [`Error::InitializeTarget`] if the signal is not a register, or
    /// [`Error::PortWidth`] if the value is the wrong width.
    pub fn initialize_to(&mut self, signal: &Signal, value: Bits) -> Result<(), Error> {
        if !matches!(
            self.program.circuit().get(signal.id()),
            Ok(ferrite_lithic_ir::Node::Reg { .. })
        ) {
            return Err(Error::InitializeTarget {
                kind: signal.kind(),
            });
        }
        let word = self.require_word(signal)?;
        if value.width() != signal.width() {
            return Err(Error::PortWidth {
                name: signal
                    .name()
                    .unwrap_or_else(|| format!("node {}", signal.id().index())),
                expected: signal.width(),
                got: value.width(),
            });
        }
        let shadow = word + self.shadow_offset();
        self.machine.write(word, &value);
        self.machine.write(shadow, &value);
        Ok(())
    }

    /// Sets an input port's value.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchInput`] if there is no such port, [`Error::PortWidth`] if
    /// the value is the wrong width.
    pub fn set_input(&mut self, name: &str, value: Bits) -> Result<(), Error> {
        let port = self.program.input_port(name)?.clone();
        if value.width() != port.width {
            return Err(Error::PortWidth {
                name: port.name,
                expected: port.width,
                got: value.width(),
            });
        }
        self.machine.write(port.word, &value);
        Ok(())
    }

    /// Reads any named signal's current value, ports or not.
    ///
    /// The debugging counterpart to [`Sim::set_input`]: "what is `acc` right now?"
    /// is the question you ask when a simulation disagrees with your arithmetic,
    /// and the answer has to be available for internal wires, not just ports.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchOutput`] if no signal in the circuit carries that name. The
    /// message lists the named signals there are, since the usual cause is a typo
    /// or a signal that was never named.
    pub fn peek(&self, name: &str) -> Result<Bits, Error> {
        match self.program.named_signal(name) {
            Some((_, word, width)) => Ok(self.machine.read(word, width)),
            None => Err(Error::NoSuchOutput {
                name: name.to_string(),
                known: self
                    .program
                    .circuit()
                    .names()
                    .map(|(_, n)| n.to_string())
                    .collect(),
            }),
        }
    }

    /// One full settle pass over the combinational logic.
    ///
    /// The third phase of a step is exactly this, and calling it directly is how a
    /// register-free design is simulated: there is no clock edge to apply, but
    /// there is still a net to evaluate.
    ///
    /// # Errors
    ///
    /// [`Error::DivisionByZero`] or [`Error::SignedDivisionOverflow`] if the
    /// combinational logic divides by zero or overflows a signed division.
    pub fn comb(&mut self) -> Result<(), Error> {
        self.machine.comb(&self.program)
    }

    /// Phase one: advance the counter, drive the clock high, settle, and snapshot.
    ///
    /// # Errors
    ///
    /// As [`Sim::comb`], plus [`Error::NoClock`] if the design has no register.
    pub fn before_clock_edge(&mut self) -> Result<Snapshot, Error> {
        let clock = self.clock()?.clone();
        self.cycle += 1;
        self.machine.write_word(clock.word, 1, 1);
        self.machine.comb(&self.program)?;
        Ok(self.snapshot())
    }

    /// Phase two: commit every register and memory write.
    ///
    /// Registers are written into the shadow section and then blitted, so every
    /// register's next value is computed from one consistent pre-edge state.
    /// Without that, feedback through a register's own output gives results that
    /// depend on the order registers happen to be visited in — and a feedback
    /// chain of registers would shift by one cycle *per register*.
    ///
    /// # Errors
    ///
    /// As [`Sim::comb`], plus [`Error::NoClock`].
    pub fn at_clock_edge(&mut self) -> Result<(), Error> {
        self.clock()?;
        self.machine.reg_update(&self.program)?;
        self.machine.mem_update(self.program.mem_updates());
        self.machine.blit(&self.program);
        Ok(())
    }

    /// Phase three: drive the clock low, settle again, and snapshot.
    ///
    /// # Errors
    ///
    /// As [`Sim::comb`], plus [`Error::NoClock`].
    pub fn after_clock_edge(&mut self) -> Result<Snapshot, Error> {
        let clock = self.clock()?.clone();
        self.machine.write_word(clock.word, 0, 1);
        self.machine.comb(&self.program)?;
        Ok(self.snapshot())
    }

    /// All three phases, returning the before and after snapshots of one cycle.
    ///
    /// # Errors
    ///
    /// As the phases it calls.
    pub fn step(&mut self) -> Result<(Snapshot, Snapshot), Error> {
        let before = self.before_clock_edge()?;
        self.at_clock_edge()?;
        let after = self.after_clock_edge()?;
        Ok((before, after))
    }

    /// The state before any edge has been applied: every register and memory at
    /// zero, every combination settled.
    ///
    /// # Errors
    ///
    /// As [`Sim::comb`].
    pub fn initial(&mut self) -> Result<Snapshot, Error> {
        self.comb()?;
        Ok(self.snapshot())
    }

    /// The simulator's whole buffer, for tests that want to look at the sections.
    #[must_use]
    pub fn buffer(&self) -> &[u64] {
        self.machine.buffer()
    }

    /// The clock this design is driven by.
    ///
    /// # Errors
    ///
    /// [`Error::NoClock`] if the design has no register, because then there is no
    /// edge to drive and a register-free design should be settled with
    /// [`Sim::comb`] instead.
    pub fn clock(&self) -> Result<&Port, Error> {
        self.program.clock().ok_or(Error::NoClock)
    }

    /// The base word of a signal's storage, following wires to their drivers.
    ///
    /// # Errors
    ///
    /// [`Error::ForeignSignal`] if the signal belongs to a different design.
    pub fn word_of(&self, signal: &Signal) -> Result<Word, Error> {
        self.require_word(signal)
    }

    fn require_word(&self, signal: &Signal) -> Result<Word, Error> {
        let name = || {
            signal
                .name()
                .unwrap_or_else(|| format!("node {}", signal.id().index()))
        };
        if !self.design.owns(signal) {
            return Err(Error::ForeignSignal { name: name() });
        }
        self.program
            .word_of(signal.id())
            .ok_or(Error::ForeignSignal { name: name() })
    }

    fn shadow_offset(&self) -> Word {
        self.program.sections().regs.len()
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            cycle: self.cycle,
            outputs: self
                .program
                .outputs()
                .iter()
                .map(|port| (port.name.clone(), self.machine.read(port.word, port.width)))
                .collect(),
        }
    }
}
