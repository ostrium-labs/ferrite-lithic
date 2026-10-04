//! Turning a graph into word addresses and a flat instruction list.

use core::ops::Range;

use ferrite_lithic_bits::words_for_width;
use ferrite_lithic_ir::{Circuit, Deps, Node, NodeId};

use crate::error::Error;
use crate::op::{Op, Word};

/// A named boundary signal: an input port, an output port, or the clock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Port {
    /// The port's name, as it will appear in emitted Verilog.
    pub name: String,
    /// Its width in bits.
    pub width: u32,
    /// Base word of its storage.
    pub word: Word,
}

/// A register's behaviour at the clock edge.
///
/// Separate from [`Op`] because a register is not evaluated during a settle pass:
/// its *output* is its slot in the regs section, so there is nothing to compute.
/// What it needs is a rule for what to write into the shadow copy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegUpdate {
    /// The data input.
    pub data: Word,
    /// The clock. One bit.
    pub clock: Word,
    /// Synchronous reset. One bit; high clears the register to zero.
    pub reset: Word,
    /// Synchronous clear. One bit; high holds the register at its current value.
    pub clear: Word,
    /// The register's slot in the regs section. The *shadow* copy is the same
    /// offset into `regs_next`, which is what makes the blit a contiguous copy.
    pub current: Word,
    /// The register's width.
    pub width: u32,
}

/// One memory write port's behaviour at the clock edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemWrite {
    /// Base word of the memory's word array in the mems section.
    pub mem: Word,
    /// How many words the memory has.
    pub depth: u32,
    /// The memory's word width.
    pub data_width: u32,
    /// The data to write.
    pub data: Word,
    /// The address to write to. An address at or past `depth` is dropped.
    pub address: Word,
    /// The address's width.
    pub address_width: u32,
    /// The write enable. One bit.
    pub enable: Word,
}

/// Where each kind of storage lives in the flat buffer.
///
/// The order is comb, regs, regs_next, mems, consts, inherited from
/// [`src/cyclesim_compile.ml:322-419`](https://github.com/jane-street/hardcaml).
/// The layout itself is not load-bearing — any partition would run — but it makes
/// a buffer dump readable and keeps us diffable against the reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sections {
    /// Combinational results, and the storage of undriven input-port wires.
    pub comb: Range<usize>,
    /// Register outputs.
    pub regs: Range<usize>,
    /// The shadow copy registers are written to before the blit.
    pub regs_next: Range<usize>,
    /// Memory word arrays.
    pub mems: Range<usize>,
    /// Literal values.
    pub consts: Range<usize>,
}

/// A compiled circuit: word addresses, instructions, and the port list.
///
/// Compiling once and stepping many times is the point. Nothing here depends on
/// the values being simulated, so a [`Program`] is immutable and cheap to clone,
/// and one program can back several [`Sim`](crate::Sim)s.
///
/// # Public, because it is meant to be looked at
///
/// The op list, the sections and the port list are all readable. That is not
/// debug-only surface: comparing a compiled program against another
/// implementation's compiled program is how a differential test works, and a
/// simulator you cannot inspect is very hard to debug when it disagrees with the
/// RTL it is standing in for.
#[derive(Clone, Debug)]
pub struct Program {
    /// The graph this was compiled from.
    ///
    /// Kept so a compiled program can still answer questions about the circuit:
    /// what kind a node is, what it is called, how wide it is. Those are exactly
    /// the questions a differential test asks when two implementations disagree,
    /// and re-deriving them from word addresses alone would mean guessing.
    circuit: Circuit,
    ops: Vec<Op>,
    reg_updates: Vec<RegUpdate>,
    mem_updates: Vec<MemWrite>,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
    clock: Option<Port>,
    node_words: Vec<Option<Word>>,
    sections: Sections,
}

impl Program {
    /// Compiles a circuit into a steppable program.
    ///
    /// `inputs` and `outputs` are the module's port lists, in declaration order.
    /// They are taken as node ids rather than discovered by name, because "which
    /// wires are ports" is a property of the module boundary and not something
    /// the graph can work out for itself.
    ///
    /// # Every node is compiled, including dead ones
    ///
    /// Reachability pruning is deliberately not applied. A dead node still costs
    /// one op per settle pass, which is cheap, but it means an unsupported node
    /// kind is refused even when nothing reads its result. That is the better
    /// failure: a design containing an unmodelled instance *does* contain
    /// something this simulator cannot run, and silently ignoring it would let a
    /// test pass against a circuit the user did not write.
    ///
    /// # Errors
    ///
    /// [`Error::Ir`] if the circuit fails its own checks or contains a
    /// combinational loop; [`Error::NoInstanceModel`] for an instance;
    /// [`Error::MultipleClocks`] if registers are clocked by more than one signal.
    pub fn compile(
        circuit: &Circuit,
        inputs: &[NodeId],
        outputs: &[NodeId],
    ) -> Result<Self, Error> {
        // Validate before reading anything. `check_with_inputs` is what makes an
        // input port legal, so it has to run first or every port looks like an
        // undriven-wire bug.
        circuit.check_with_inputs(inputs)?;
        let order = circuit.topological_order(Deps::SimulationScheduling)?;
        let mut alloc = Allocator::new(circuit.len());

        // Pass one: classify every node and count the words each section needs.
        // Under `SimulationScheduling` a register and a memory are terminal, so
        // they sort before the nodes that read them, which is what lets a read
        // port rely on its memory already having a base word.
        for id in &order {
            let width = circuit.width_of(*id);
            match circuit.get(*id)? {
                // An undriven wire is an input port and needs storage of its own;
                // a driven one is an alias, resolved in pass three.
                Node::Wire { driver, .. } => {
                    if driver.get().is_none() {
                        alloc.reserve_value(*id, Section::Comb, width);
                    }
                }
                Node::Constant { value } => {
                    alloc.reserve_value(*id, Section::Const, value.width());
                }
                Node::Reg { .. } => alloc.reserve_value(*id, Section::Reg, width),
                Node::Mem {
                    depth, data_width, ..
                } => {
                    let words = Allocator::memory_size(*depth, *data_width)?;
                    alloc.reserve(*id, Section::Mem, words);
                }
                _ => alloc.reserve_value(*id, Section::Comb, width),
            }
        }

        // Pass two: turn those sizes into absolute addresses.
        let sections = alloc.place_all();

        // Pass three: chase every driven wire to its first non-wire driver, so no
        // op ever names a wire. A wire driven by a wire is legal and resolves
        // transitively, memoised through the same table.
        for id in &order {
            if let Node::Wire { driver, .. } = circuit.get(*id)?
                && let Some(driver) = driver.get()
            {
                alloc.alias_wire(*id, driver, circuit)?;
            }
        }

        let mut builder = OpBuilder::new(circuit, &alloc);

        // Pass four: every operand address is now known, so the ops can be built.
        // Ordering follows the topological order, and the four node kinds that
        // contribute no instruction contribute nothing.
        for id in &order {
            if let Some(op) = builder.op_for(circuit.get(*id)?, *id)? {
                builder.ops.push(op);
            }
        }

        // Registers and memories are terminal under the simulation relation, so
        // their operands may not have been placed when the walk above reached
        // them. They are collected here instead, once every address exists.
        // Order within each list does not matter: an edge commit reads only state
        // and the data input, never another register's freshly written value.
        for id in &order {
            match circuit.get(*id)? {
                Node::Reg {
                    data,
                    clock,
                    reset,
                    clear,
                } => {
                    builder.note_clock(*clock, circuit)?;
                    builder.reg_updates.push(RegUpdate {
                        data: builder.word(*data),
                        clock: builder.word(*clock),
                        reset: builder.word(*reset),
                        clear: builder.word(*clear),
                        current: builder.word(*id),
                        width: builder.width(*id),
                    });
                }
                Node::Mem { .. } => builder.collect_mem_writes(*id, circuit)?,
                _ => {}
            }
        }

        let clock = builder.clock.map(|id| Port {
            name: port_name(circuit, id),
            width: 1,
            word: builder.word(id),
        });
        // Ports before the vectors are moved out, since resolving a port needs the
        // builder's address table.
        let inputs = finish_ports(circuit, &builder, inputs)?;
        let outputs = finish_ports(circuit, &builder, outputs)?;

        Ok(Program {
            circuit: circuit.clone(),
            ops: builder.ops,
            reg_updates: builder.reg_updates,
            mem_updates: builder.mem_updates,
            inputs,
            outputs,
            clock,
            node_words: alloc.resolved,
            sections,
        })
    }

    /// The circuit this was compiled from.
    #[must_use]
    pub fn circuit(&self) -> &Circuit {
        &self.circuit
    }

    /// The combinational instructions, in the order one settle pass runs them.
    #[must_use]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// The register update rules applied at a clock edge.
    #[must_use]
    pub fn reg_updates(&self) -> &[RegUpdate] {
        &self.reg_updates
    }

    /// The memory write port rules applied at a clock edge.
    #[must_use]
    pub fn mem_updates(&self) -> &[MemWrite] {
        &self.mem_updates
    }

    /// The input ports, in declaration order.
    #[must_use]
    pub fn inputs(&self) -> &[Port] {
        &self.inputs
    }

    /// The output ports, in declaration order.
    #[must_use]
    pub fn outputs(&self) -> &[Port] {
        &self.outputs
    }

    /// The single clock this design is clocked by, if it has any register.
    #[must_use]
    pub fn clock(&self) -> Option<&Port> {
        self.clock.as_ref()
    }

    /// Where each kind of storage lives.
    #[must_use]
    pub fn sections(&self) -> &Sections {
        &self.sections
    }

    /// Total words in the flat buffer.
    #[must_use]
    pub fn buffer_words(&self) -> usize {
        self.sections.consts.end
    }

    /// The base word of a node's storage, following wires to their driver.
    #[must_use]
    pub fn word_of(&self, id: NodeId) -> Option<Word> {
        self.node_words.get(id.index()).copied().flatten()
    }

    /// The base word and width of any named signal, ports or not.
    ///
    /// This is the "what is this signal right now" lookup, and it is why the
    /// circuit is kept: names live on the graph, not on the op list.
    #[must_use]
    pub fn named_signal(&self, name: &str) -> Option<(NodeId, Word, u32)> {
        self.circuit.names().find_map(|(id, n)| {
            if n != name {
                return None;
            }
            Some((id, self.word_of(id)?, self.circuit.width_of(id)))
        })
    }

    /// Looks up an input or output port by name.
    #[must_use]
    pub fn port(&self, name: &str) -> Option<&Port> {
        self.inputs
            .iter()
            .chain(self.outputs.iter())
            .chain(self.clock.iter())
            .find(|p| p.name == name)
    }

    /// The input port called `name`, with the ports that do exist if there is none.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchInput`].
    pub fn input_port(&self, name: &str) -> Result<&Port, Error> {
        self.inputs
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| Error::NoSuchInput {
                name: name.to_string(),
                known: self.inputs.iter().map(|p| p.name.clone()).collect(),
            })
    }

    /// The output port called `name`.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchOutput`].
    pub fn output_port(&self, name: &str) -> Result<&Port, Error> {
        self.outputs
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| Error::NoSuchOutput {
                name: name.to_string(),
                known: self.outputs.iter().map(|p| p.name.clone()).collect(),
            })
    }
}

/// A node's port name, falling back to its index so a message is never empty.
fn port_name(circuit: &Circuit, id: NodeId) -> String {
    circuit
        .name(id)
        .map_or_else(|| format!("node {}", id.index()), ToString::to_string)
}

/// Turns a port list into [`Port`]s with resolved addresses.
fn finish_ports(
    circuit: &Circuit,
    builder: &OpBuilder<'_>,
    ports: &[NodeId],
) -> Result<Vec<Port>, Error> {
    ports
        .iter()
        .map(|id| {
            Ok(Port {
                name: port_name(circuit, *id),
                width: builder.width(*id),
                word: builder.word(*id),
            })
        })
        .collect()
}

/// Where a node's storage goes, decided in two passes.
///
/// # Why counting comes first
///
/// The sections are laid out in a fixed order — comb, regs, regs_next, mems,
/// consts — so a node's absolute word depends on the size of every section before
/// it. Placing as we walk therefore does not work: a memory visited early does not
/// yet know how wide the combinational logic in front of it turned out to be.
///
/// So the first walk only *counts* words per section, and the second assigns
/// absolute bases. Nodes are placed within a section in arena order, which is
/// already canonical, so a circuit always compiles to the same addresses.
///
/// Getting this wrong is not subtle in the output but very subtle in the code: a
/// register placed at word 0 collides with the first combinational signal, and the
/// only symptom is a design that mysteriously does not count.
struct Allocator {
    /// Per node: the section it belongs to and how many words it needs.
    placement: Vec<Option<Placement>>,
    resolved: Vec<Option<Word>>,
    comb_words: usize,
    reg_words: usize,
    mem_words: usize,
    const_words: usize,
}

/// A node's section and size, recorded before any address exists.
#[derive(Clone, Copy)]
struct Placement {
    section: Section,
    words: usize,
}

/// Which section a placement belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Section {
    Comb,
    Reg,
    Mem,
    Const,
}

impl Allocator {
    fn new(nodes: usize) -> Self {
        Allocator {
            placement: vec![None; nodes],
            resolved: vec![None; nodes],
            comb_words: 0,
            reg_words: 0,
            mem_words: 0,
            const_words: 0,
        }
    }

    /// Records that `id` needs `words` words of `section`.
    fn reserve(&mut self, id: NodeId, section: Section, words: usize) {
        match section {
            Section::Comb => self.comb_words += words,
            Section::Reg => self.reg_words += words,
            Section::Mem => self.mem_words += words,
            Section::Const => self.const_words += words,
        }
        self.placement[id.index()] = Some(Placement { section, words });
    }

    /// Reserves a value of `width` bits.
    fn reserve_value(&mut self, id: NodeId, section: Section, width: u32) {
        self.reserve(id, section, words_for_width(width));
    }

    /// Words a memory needs, refusing a size the buffer cannot address.
    fn memory_size(depth: u32, data_width: u32) -> Result<usize, Error> {
        let words = u64::from(depth) * words_for_width(data_width) as u64;
        usize::try_from(words).map_err(|_| Error::BufferTooLarge {
            what: format!("a memory {depth} words deep"),
            width: data_width,
        })
    }

    /// Turns the recorded sizes into absolute addresses, in arena order.
    fn place_all(&mut self) -> Sections {
        let sections = Sections {
            comb: 0..self.comb_words,
            regs: self.comb_words..self.comb_words + self.reg_words,
            regs_next: self.comb_words + self.reg_words..self.comb_words + 2 * self.reg_words,
            mems: self.comb_words + 2 * self.reg_words
                ..self.comb_words + 2 * self.reg_words + self.mem_words,
            consts: self.comb_words + 2 * self.reg_words + self.mem_words
                ..self.comb_words + 2 * self.reg_words + self.mem_words + self.const_words,
        };
        let mut cursors = [
            sections.comb.start,
            sections.regs.start,
            sections.mems.start,
            sections.consts.start,
        ];
        for (index, placement) in self.placement.iter().enumerate() {
            let Some(placement) = placement else {
                continue;
            };
            let slot = match placement.section {
                Section::Comb => 0,
                Section::Reg => 1,
                Section::Mem => 2,
                Section::Const => 3,
            };
            let base = cursors[slot];
            cursors[slot] += placement.words;
            self.resolved[index] = Some(base);
        }
        sections
    }

    /// Points a wire at its driver's storage, following a chain of wires.
    ///
    /// Runs after [`Allocator::place_all`], so every non-wire node already has an
    /// address and the walk terminates at the first step in the common case.
    fn alias_wire(&mut self, wire: NodeId, driver: NodeId, circuit: &Circuit) -> Result<(), Error> {
        let mut cursor = driver;
        // Bounded by the node count: each step either lands on an already-placed
        // node or consumes one more node of the chain.
        for _ in 0..=circuit.len() {
            if let Some(word) = self.resolved[cursor.index()] {
                self.resolved[wire.index()] = Some(word);
                return Ok(());
            }
            match circuit.get(cursor) {
                // An unplaced node that is not a wire, or a wire with no driver,
                // cannot happen once the circuit has been checked. Either would
                // mean the address table is wrong, which is a bug here rather than
                // a user error, so it is named rather than papered over.
                Ok(Node::Wire { driver, .. }) => match driver.get() {
                    Some(next) => cursor = next,
                    None => return Err(Error::Unplaced { node: cursor }),
                },
                _ => return Err(Error::Unplaced { node: cursor }),
            }
        }
        Err(Error::WireChainTooLong { node: wire })
    }
}

/// Builds the op list once every address is known.
struct OpBuilder<'a> {
    circuit: &'a Circuit,
    alloc: &'a Allocator,
    ops: Vec<Op>,
    reg_updates: Vec<RegUpdate>,
    mem_updates: Vec<MemWrite>,
    /// The first clock any register used. A second one is refused, so this stays
    /// `Option` and is checked exactly once per register.
    clock: Option<NodeId>,
}

impl<'a> OpBuilder<'a> {
    fn new(circuit: &'a Circuit, alloc: &'a Allocator) -> Self {
        OpBuilder {
            circuit,
            alloc,
            ops: Vec::new(),
            reg_updates: Vec::new(),
            mem_updates: Vec::new(),
            clock: None,
        }
    }

    /// Base word of a node's storage, following wires to their driver.
    ///
    /// # Panics
    ///
    /// If the node was never placed. That cannot happen after the two address
    /// passes, and a program that read the wrong memory would be worse than a
    /// panic here.
    fn word(&self, id: NodeId) -> Word {
        self.alloc
            .resolved
            .get(id.index())
            .copied()
            .flatten()
            .unwrap_or_else(|| panic!("node {} was never given a word address", id.index()))
    }

    fn width(&self, id: NodeId) -> u32 {
        self.circuit.width_of(id)
    }

    /// Records a register's clock, refusing a design with more than one.
    fn note_clock(&mut self, clock: NodeId, circuit: &Circuit) -> Result<(), Error> {
        match self.clock {
            None => {
                self.clock = Some(clock);
                Ok(())
            }
            Some(existing) if existing == clock => Ok(()),
            Some(existing) => Err(Error::MultipleClocks {
                first: port_name(circuit, existing),
                second: port_name(circuit, clock),
            }),
        }
    }

    fn op_for(&mut self, node: &Node, id: NodeId) -> Result<Option<Op>, Error> {
        let dst = self.word(id);
        let op = match node {
            // A constant is a region of the consts section, a register is its slot
            // in the regs section, a memory is its region of the mems section, and
            // a wire is an alias. None of them is computed, so none is an op.
            Node::Constant { .. } | Node::Wire { .. } | Node::Reg { .. } | Node::Mem { .. } => {
                return Ok(None);
            }
            Node::Not { arg } => Op::Not {
                dst,
                arg: self.word(*arg),
                width: self.width(id),
            },
            Node::Select { value, offset, len } => Op::Select {
                dst,
                value: self.word(*value),
                offset: *offset,
                len: *len,
            },
            Node::BitAnd { left, right } => Op::BitAnd {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            Node::BitOr { left, right } => Op::BitOr {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            Node::BitXor { left, right } => Op::BitXor {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            Node::Add { left, right } => Op::Add {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            Node::Sub { left, right } => Op::Sub {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            Node::Mul { left, right } => Op::Mul {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                left_width: self.width(*left),
                right_width: self.width(*right),
            },
            Node::UDiv { left, right } => Op::UDiv {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            Node::SDiv { left, right } => Op::SDiv {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            Node::URem { left, right } => Op::URem {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            Node::SRem { left, right } => Op::SRem {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(id),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Eq { left, right } => Op::Eq {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Ult { left, right } => Op::Ult {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Ule { left, right } => Op::Ule {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Ugt { left, right } => Op::Ugt {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Uge { left, right } => Op::Uge {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Slt { left, right } => Op::Slt {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Sle { left, right } => Op::Sle {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Sgt { left, right } => Op::Sgt {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            // The *operand* width, not `self.width(id)`: a comparison is one bit
            // wide, and the interpreter has to know how wide the values it is
            // about to reinterpret as signed are.
            Node::Sge { left, right } => Op::Sge {
                dst,
                left: self.word(*left),
                right: self.word(*right),
                width: self.width(*left),
            },
            Node::Cat { high, low } => Op::Cat {
                dst,
                high: self.word(*high),
                low: self.word(*low),
                high_width: self.width(*high),
                low_width: self.width(*low),
            },
            Node::Replicate { value, count } => Op::Replicate {
                dst,
                value: self.word(*value),
                count: *count,
                value_width: self.width(*value),
            },
            Node::Ite {
                condition,
                then_value,
                otherwise,
            } => Op::Ite {
                dst,
                condition: self.word(*condition),
                then_value: self.word(*then_value),
                otherwise: self.word(*otherwise),
                width: self.width(id),
            },
            Node::Case {
                scrutinee,
                arms,
                default,
                ..
            } => Op::Case {
                dst,
                scrutinee: self.word(*scrutinee),
                scrutinee_width: self.width(*scrutinee),
                arms: arms
                    .iter()
                    .map(|arm| (arm.matches.clone(), self.word(arm.value)))
                    .collect(),
                default: self.word(*default),
                width: self.width(id),
            },
            Node::ReadPort {
                mem,
                address,
                enable,
                data_width,
            } => {
                let (_, depth) = self.circuit.memory_shape(*mem)?;
                Op::ReadPort {
                    dst,
                    mem: self.word(*mem),
                    depth,
                    address: self.word(*address),
                    address_width: self.width(*address),
                    enable: self.word(*enable),
                    data_width: *data_width,
                }
            }
            Node::Instance { name, .. } => {
                return Err(Error::NoInstanceModel { name: name.clone() });
            }
        };
        Ok(Some(op))
    }

    /// Records a memory's write ports.
    fn collect_mem_writes(&mut self, id: NodeId, circuit: &Circuit) -> Result<(), Error> {
        let Node::Mem {
            depth,
            data_width,
            write_ports,
        } = circuit.get(id)?
        else {
            return Ok(());
        };
        let (depth, data_width) = (*depth, *data_width);
        let mem = self.word(id);
        for port in write_ports {
            self.mem_updates.push(MemWrite {
                mem,
                depth,
                data_width,
                data: self.word(port.data),
                address: self.word(port.address),
                address_width: self.width(port.address),
                enable: self.word(port.enable),
            });
        }
        Ok(())
    }
}
