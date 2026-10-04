//! The arena: [`Circuit`] owns the nodes, mints ids, and enforces the three
//! assignment checks.

use std::cell::Cell;
use std::collections::BTreeMap;

use ferrite_lithic_bits::Bits;

use crate::deps::Deps;
use crate::error::{Error, Op, Relation};
use crate::id::NodeId;
use crate::node::{CaseArm, Node, WritePort, address_width_for_depth};

/// A signal graph.
///
/// # Why an arena
///
/// Hardcaml has no arena and no node ids at construction time: a node is a
/// heap-allocated OCaml variant containing its already-constructed children
/// (`kernel/signal__type_intf.ml:372-406`), and identity is a `uid` from a
/// **process-global mutable counter** (`kernel/signal__type.ml:966`).
///
/// That will not translate. A plain immutable Rust tree cannot express a feedback
/// loop at all, and `Rc<RefCell<..>>` for every node would make the graph
/// expensive to traverse. So nodes live in a `Vec`, ids are indices, and the one
/// node that has to be mutable after construction — the wire — holds its driver in
/// a [`Cell`]. Interior mutability with no runtime borrow checking, because no
/// reference into the arena is ever handed out.
///
/// # Cycles
///
/// The [`Node::Wire`] is the *only* node with a mutable field, and every other
/// variant is built bottom-up over finished children
/// (`kernel/signal.ml:197-237`), so there is no other possible back edge. The
/// workflow is the same as upstream's: allocate an undriven wire with
/// [`wire`](Self::wire), build logic that reads it, then close the loop with
/// [`drive`](Self::drive).
///
/// # The three assignment checks
///
/// [`drive`](Self::drive) performs the same three checks Hardcaml performs and for
/// the same reasons (`kernel/signal.ml:242-267`): the target must be a wire, it
/// must not already be driven, and the driver's width must match. All three are
/// graph-correctness checks, not ergonomics — the first two because a node that
/// silently ignored the assignment would make the graph disagree with the program,
/// the third because a wire and its driver disagreeing in width would put a
/// truncation nobody wrote into the hardware.
#[derive(Clone, Debug, Default)]
pub struct Circuit {
    nodes: Vec<Node>,
    names: BTreeMap<NodeId, String>,
}

impl Circuit {
    /// An empty circuit.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            names: BTreeMap::new(),
        }
    }

    /// How many nodes the circuit holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the circuit holds no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Every node id, in construction order.
    ///
    /// Construction order *is* the canonical order, which is what makes emitted
    /// Verilog reproducible without a normalisation pass.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.nodes.len()).map(NodeId::from_index)
    }

    /// Borrow a node.
    ///
    /// # Errors
    ///
    /// If `id` was not minted by this circuit.
    pub fn get(&self, id: NodeId) -> Result<&Node, Error> {
        self.nodes.get(id.index()).ok_or(Error::UnknownNode {
            id,
            len: self.nodes.len(),
        })
    }

    /// A node's width in bits.
    ///
    /// # Panics
    ///
    /// If `id` was not minted by this circuit. A `NodeId` is a bare index and
    /// there is nothing cheap to validate, so this is a programming error rather
    /// than a runtime condition; the DSL layer never lets a signal handle outlive
    /// its circuit. [`Circuit::get`] is the checked version.
    #[must_use]
    pub fn width_of(&self, id: NodeId) -> u32 {
        let node = self
            .nodes
            .get(id.index())
            .unwrap_or_else(|| panic!("{id} does not belong to this circuit"));
        node.width(self)
    }

    /// The node's name, if one was attached.
    #[must_use]
    pub fn name(&self, id: NodeId) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }

    /// Every named node, in name order.
    pub fn names(&self) -> impl Iterator<Item = (NodeId, &str)> + '_ {
        self.names.iter().map(|(id, name)| (*id, name.as_str()))
    }

    // ---------------------------------------------------------------- literals

    /// A literal, whose width comes from the value.
    #[must_use]
    pub fn constant(&mut self, value: Bits) -> NodeId {
        self.push(Node::Constant { value })
    }

    /// An undriven wire of the given width.
    ///
    /// # Errors
    ///
    /// If `width` is zero. There is no zero-width value domain anywhere in this
    /// stack.
    pub fn wire(&mut self, width: u32) -> Result<NodeId, Error> {
        if width == 0 {
            return Err(Error::ZeroWidth { op: Op::Wire });
        }
        Ok(self.push(Node::Wire {
            width,
            driver: Cell::new(None),
        }))
    }

    /// Close a feedback loop by attaching `driver` to `wire`.
    ///
    /// This is the only way a back edge can be created, which is what makes the
    /// `Wire` the only mutable node.
    ///
    /// # Errors
    ///
    /// - [`Error::NotAWire`] if `wire` is not a wire node.
    /// - [`Error::AlreadyDriven`] if `wire` already has a driver.
    /// - [`Error::WidthMismatch`] if `driver` is not exactly `wire`'s width.
    pub fn drive(&mut self, wire: NodeId, driver: NodeId) -> Result<(), Error> {
        let (width, existing) = match self.get(wire)? {
            Node::Wire { width, driver } => (*width, driver.get()),
            other => {
                return Err(Error::NotAWire {
                    target: wire,
                    kind: other.kind(),
                });
            }
        };
        if let Some(existing) = existing {
            return Err(Error::AlreadyDriven {
                wire,
                width,
                existing,
                attempted: driver,
            });
        }
        if self.width_of(driver) != width {
            return Err(Error::WidthMismatch {
                op: Op::Drive,
                lhs: width,
                rhs: self.width_of(driver),
            });
        }
        let Node::Wire { driver: cell, .. } = &mut self.nodes[wire.index()] else {
            unreachable!("checked by the match above")
        };
        cell.set(Some(driver));
        Ok(())
    }

    /// Detach a wire's driver, returning it.
    ///
    /// Exists because [`drive`](Self::drive) refuses to overwrite, so a program
    /// that legitimately wants to swap a driver has to say so explicitly rather
    /// than getting the last write silently.
    ///
    /// # Errors
    ///
    /// If `wire` is not a wire node.
    pub fn undrive(&mut self, wire: NodeId) -> Result<Option<NodeId>, Error> {
        let Node::Wire { driver, .. } = self.get(wire)? else {
            return Err(Error::NotAWire {
                target: wire,
                kind: self.get(wire)?.kind(),
            });
        };
        let previous = driver.replace(None);
        Ok(previous)
    }

    /// A wire's driver, if it has one.
    ///
    /// # Errors
    ///
    /// [`Error::NotAWire`] if `wire` is not a wire node. A `None` result means the
    /// wire is undriven, which is legal until someone reads it — see
    /// [`check`](Self::check).
    pub fn driver_of(&self, wire: NodeId) -> Result<Option<NodeId>, Error> {
        let Node::Wire { driver, .. } = self.get(wire)? else {
            return Err(Error::NotAWire {
                target: wire,
                kind: self.get(wire)?.kind(),
            });
        };
        Ok(driver.get())
    }

    /// A memory's word width and depth, if it is a memory.
    ///
    /// # Errors
    ///
    /// If `mem` is not a memory node.
    pub fn memory_shape(&self, mem: NodeId) -> Result<(u32, u32), Error> {
        match self.get(mem)? {
            Node::Mem {
                data_width, depth, ..
            } => Ok((*data_width, *depth)),
            other => Err(Error::NotAMemory {
                target: mem,
                kind: other.kind(),
            }),
        }
    }

    /// A memory's write ports, in construction order.
    ///
    /// # Errors
    ///
    /// If `mem` is not a memory node.
    pub fn write_ports_of(&self, mem: NodeId) -> Result<&[WritePort], Error> {
        match self.get(mem)? {
            Node::Mem { write_ports, .. } => Ok(write_ports),
            other => Err(Error::NotAMemory {
                target: mem,
                kind: other.kind(),
            }),
        }
    }

    // ------------------------------------------------------------- combinational

    /// Bitwise complement.
    #[must_use]
    pub fn not(&mut self, arg: NodeId) -> NodeId {
        self.push(Node::Not { arg })
    }

    /// A run of `len` bits starting at `offset`, counting from the least
    /// significant bit. The result is `len` bits wide.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroWidth`] if `len` is zero, and [`Error::SelectOutOfRange`] if
    /// the window runs past the end of `value`.
    pub fn select(&mut self, value: NodeId, offset: u32, len: u32) -> Result<NodeId, Error> {
        if len == 0 {
            return Err(Error::ZeroWidth { op: Op::Select });
        }
        let value_width = self.width_of(value);
        if u64::from(offset) + u64::from(len) > u64::from(value_width) {
            return Err(Error::SelectOutOfRange {
                op: Op::Select,
                value_width,
                offset,
                len,
            });
        }
        Ok(self.push(Node::Select { value, offset, len }))
    }

    /// Bitwise AND. Requires equal widths.
    pub fn bit_and(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::BitAnd, left, right)?;
        Ok(self.push(Node::BitAnd { left, right }))
    }

    /// Bitwise OR. Requires equal widths.
    pub fn bit_or(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::BitOr, left, right)?;
        Ok(self.push(Node::BitOr { left, right }))
    }

    /// Bitwise XOR. Requires equal widths.
    pub fn bit_xor(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::BitXor, left, right)?;
        Ok(self.push(Node::BitXor { left, right }))
    }

    /// Truncating add. Requires equal widths; the result is the left width.
    pub fn add(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Add, left, right)?;
        Ok(self.push(Node::Add { left, right }))
    }

    /// Truncating subtract. Requires equal widths; the result is the left width.
    pub fn sub(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Sub, left, right)?;
        Ok(self.push(Node::Sub { left, right }))
    }

    /// Full-precision multiply. Widens to the sum of the operand widths and cannot
    /// overflow, so the operand widths may differ.
    pub fn mul(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        Self::checked_sum(Op::Mul, self.width_of(left), self.width_of(right))?;
        Ok(self.push(Node::Mul { left, right }))
    }

    /// Unsigned divide. Requires equal widths; the result is the left width.
    pub fn udiv(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::UDiv, left, right)?;
        Ok(self.push(Node::UDiv { left, right }))
    }

    /// Signed divide. Requires equal widths; the result is the left width.
    ///
    /// Division by zero and `-2^(w-1) / -1` are legal *graphs* and are refused at
    /// simulation time, because refusing them here would mean a design that can
    /// never be built, even if the offending values are unreachable.
    pub fn sdiv(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::SDiv, left, right)?;
        Ok(self.push(Node::SDiv { left, right }))
    }

    /// Unsigned remainder. Requires equal widths; the result is the left width.
    pub fn urem(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::URem, left, right)?;
        Ok(self.push(Node::URem { left, right }))
    }

    /// Signed remainder. Requires equal widths; the result is the left width.
    pub fn srem(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::SRem, left, right)?;
        Ok(self.push(Node::SRem { left, right }))
    }

    /// Equality. One bit out.
    pub fn eq(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Eq, left, right)?;
        Ok(self.push(Node::Eq { left, right }))
    }

    /// Unsigned less-than. One bit out.
    pub fn ult(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Ult, left, right)?;
        Ok(self.push(Node::Ult { left, right }))
    }

    /// Unsigned less-or-equal. One bit out.
    pub fn ule(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Ule, left, right)?;
        Ok(self.push(Node::Ule { left, right }))
    }

    /// Unsigned greater-than. One bit out.
    pub fn ugt(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Ugt, left, right)?;
        Ok(self.push(Node::Ugt { left, right }))
    }

    /// Unsigned greater-or-equal. One bit out.
    pub fn uge(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Uge, left, right)?;
        Ok(self.push(Node::Uge { left, right }))
    }

    /// Signed less-than. One bit out.
    pub fn slt(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Slt, left, right)?;
        Ok(self.push(Node::Slt { left, right }))
    }

    /// Signed less-or-equal. One bit out.
    pub fn sle(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Sle, left, right)?;
        Ok(self.push(Node::Sle { left, right }))
    }

    /// Signed greater-than. One bit out.
    pub fn sgt(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Sgt, left, right)?;
        Ok(self.push(Node::Sgt { left, right }))
    }

    /// Signed greater-or-equal. One bit out.
    pub fn sge(&mut self, left: NodeId, right: NodeId) -> Result<NodeId, Error> {
        self.require_equal(Op::Sge, left, right)?;
        Ok(self.push(Node::Sge { left, right }))
    }

    /// Concatenate, `high` above `low`. Widens to the sum of the widths.
    pub fn cat(&mut self, high: NodeId, low: NodeId) -> Result<NodeId, Error> {
        Self::checked_sum(Op::Cat, self.width_of(high), self.width_of(low))?;
        Ok(self.push(Node::Cat { high, low }))
    }

    /// Repeat `value` `count` times. Widens to `width * count`.
    ///
    /// A `count` of zero is a zero-width result and is refused, consistent with
    /// everywhere else in the stack.
    pub fn replicate(&mut self, value: NodeId, count: u32) -> Result<NodeId, Error> {
        let width = self.width_of(value);
        let result = width.checked_mul(count).ok_or(Error::WidthOverflow {
            op: Op::Replicate,
            bits: u64::from(width) * u64::from(count),
        })?;
        if result == 0 {
            return Err(Error::ZeroWidth { op: Op::Replicate });
        }
        Ok(self.push(Node::Replicate { value, count }))
    }

    /// Two-way select. The condition is one bit; the two values must match.
    pub fn ite(
        &mut self,
        condition: NodeId,
        then_value: NodeId,
        otherwise: NodeId,
    ) -> Result<NodeId, Error> {
        self.require_one_bit(Op::Ite, "condition", condition)?;
        self.require_equal(Op::Ite, then_value, otherwise)?;
        Ok(self.push(Node::Ite {
            condition,
            then_value,
            otherwise,
        }))
    }

    /// Multi-way select against literal match values, in priority order.
    ///
    /// Every arm's `matches` must have the scrutinee's width, and every arm value
    /// and the default must share one width. A case *selects* between values; it
    /// never converts them.
    ///
    /// # Errors
    ///
    /// - [`Error::CaseNoArms`] if `arms` is empty.
    /// - [`Error::WidthMismatch`] if a match value or an arm's width disagrees.
    pub fn case_(
        &mut self,
        scrutinee: NodeId,
        arms: Vec<CaseArm>,
        default: NodeId,
    ) -> Result<NodeId, Error> {
        if arms.is_empty() {
            return Err(Error::CaseNoArms { op: Op::Case });
        }
        let scrutinee_width = self.width_of(scrutinee);
        for arm in &arms {
            for candidate in &arm.matches {
                if candidate.width() != scrutinee_width {
                    return Err(Error::WidthMismatch {
                        op: Op::Case,
                        lhs: scrutinee_width,
                        rhs: candidate.width(),
                    });
                }
            }
        }
        let width = self.width_of(arms[0].value);
        for (index, arm) in arms.iter().enumerate() {
            let arm_width = self.width_of(arm.value);
            if arm_width != width {
                return Err(Error::CaseArmWidthMismatch {
                    arm: index,
                    lhs: width,
                    rhs: arm_width,
                });
            }
        }
        let default_width = self.width_of(default);
        if default_width != width {
            return Err(Error::CaseArmWidthMismatch {
                arm: arms.len(),
                lhs: width,
                rhs: default_width,
            });
        }
        Ok(self.push(Node::Case {
            scrutinee,
            arms,
            default,
            width,
        }))
    }

    // ------------------------------------------------------------------ stateful

    /// A register. Its output is the node itself; `data` is its input.
    ///
    /// Inherited from Hardcaml deliberately (`kernel/signal__type_intf.ml:243-296`):
    /// a register stores no output value, so "is this stateful?" is answered by
    /// matching on the constructor.
    ///
    /// `clock`, `reset` and `clear` are one bit by construction, not by
    /// convention — there is no such thing as an 8-bit clock.
    pub fn reg(
        &mut self,
        data: NodeId,
        clock: NodeId,
        reset: NodeId,
        clear: NodeId,
    ) -> Result<NodeId, Error> {
        self.require_one_bit(Op::Reg, "clock", clock)?;
        self.require_one_bit(Op::Reg, "reset", reset)?;
        self.require_one_bit(Op::Reg, "clear", clear)?;
        Ok(self.push(Node::Reg {
            data,
            clock,
            reset,
            clear,
        }))
    }

    /// A memory of `depth` words, each `data_width` bits wide.
    ///
    /// Starts as all zeros, inherited from Hardcaml: nothing needs an initial value
    /// because nothing reads a memory before the first write in a well-formed
    /// design, and making the reset value explicit would put a reset port on every
    /// memory for no benefit.
    ///
    /// # Errors
    ///
    /// If either width or depth is zero.
    pub fn mem(&mut self, data_width: u32, depth: u32) -> Result<NodeId, Error> {
        if data_width == 0 {
            return Err(Error::ZeroWidth { op: Op::Mem });
        }
        if depth == 0 {
            return Err(Error::ZeroWidth { op: Op::Mem });
        }
        Ok(self.push(Node::Mem {
            data_width,
            depth,
            write_ports: Vec::new(),
        }))
    }

    /// Add a write port to a memory.
    ///
    /// Ports are added after construction rather than passed in, because a memory
    /// with no write ports is a legitimate read-only constant array and forcing
    /// callers to invent one would be noise.
    ///
    /// # Errors
    ///
    /// - If `mem` is not a memory.
    /// - [`Error::WidthMismatch`] if `data` is not exactly the memory's word width.
    /// - If `enable` is not one bit.
    /// - [`Error::MemoryAddressTooNarrow`] if `address` cannot reach every word.
    pub fn write_port(
        &mut self,
        mem: NodeId,
        data: NodeId,
        address: NodeId,
        enable: NodeId,
    ) -> Result<(), Error> {
        self.require_one_bit(Op::Mem, "write enable", enable)?;
        let Node::Mem {
            data_width, depth, ..
        } = *self.get(mem)?
        else {
            return Err(Error::NotAMemory {
                target: mem,
                kind: self.get(mem)?.kind(),
            });
        };
        if self.width_of(data) != data_width {
            return Err(Error::WidthMismatch {
                op: Op::Mem,
                lhs: data_width,
                rhs: self.width_of(data),
            });
        }
        let needed = address_width_for_depth(depth);
        let address_width = self.width_of(address);
        if address_width < needed {
            return Err(Error::MemoryAddressTooNarrow {
                mem,
                depth,
                address_width: needed,
            });
        }
        let Node::Mem { write_ports, .. } = &mut self.nodes[mem.index()] else {
            unreachable!("checked by the match above")
        };
        write_ports.push(WritePort {
            data,
            address,
            enable,
        });
        Ok(())
    }

    /// An asynchronous read port on `mem`.
    ///
    /// Combinational, and deliberately a separate node: a read port holds no state
    /// of its own, which is what keeps "what is stateful" answerable from the node
    /// constructor alone. A synchronous read is the caller composing a
    /// [`reg`](Self::reg) with this.
    ///
    /// # Errors
    ///
    /// - If `mem` is not a memory.
    /// - If `enable` is not one bit.
    /// - [`Error::MemoryAddressTooNarrow`] if `address` cannot reach every word.
    pub fn read_port(
        &mut self,
        mem: NodeId,
        address: NodeId,
        enable: NodeId,
    ) -> Result<NodeId, Error> {
        self.require_one_bit(Op::ReadPort, "read enable", enable)?;
        let Node::Mem {
            data_width, depth, ..
        } = self.get(mem)?
        else {
            return Err(Error::NotAMemory {
                target: mem,
                kind: self.get(mem)?.kind(),
            });
        };
        let (data_width, depth) = (*data_width, *depth);
        let needed = address_width_for_depth(depth);
        let address_width = self.width_of(address);
        if address_width < needed {
            return Err(Error::MemoryAddressTooNarrow {
                mem,
                depth,
                address_width: needed,
            });
        }
        Ok(self.push(Node::ReadPort {
            mem,
            address,
            enable,
            data_width,
        }))
    }

    /// An instance of a named submodule, producing one output of `output_width`.
    ///
    /// See [`Node::Instance`] for why one output, not a bundle.
    ///
    /// # Errors
    ///
    /// If `output_width` is zero, or the name is empty.
    pub fn instance(
        &mut self,
        name: impl Into<String>,
        params: Vec<(String, Bits)>,
        inputs: Vec<NodeId>,
        output_width: u32,
    ) -> Result<NodeId, Error> {
        let name = name.into();
        if output_width == 0 {
            return Err(Error::ZeroWidth { op: Op::Instance });
        }
        if name.is_empty() {
            return Err(Error::EmptyInstanceName { op: Op::Instance });
        }
        Ok(self.push(Node::Instance {
            name,
            params,
            inputs,
            output_width,
        }))
    }

    // -------------------------------------------------------------------- naming

    /// Attach a name to a node.
    ///
    /// Names appear verbatim in emitted Verilog, so they are attached during
    /// construction — before the graph is frozen — rather than lazily afterwards.
    /// That is a deliberate divergence: Hardcaml attaches names late via mutable
    /// metadata, which is why it needs a lazily computed name map and a
    /// `normalize_uids` pass to make output reproducible.
    ///
    /// # Errors
    ///
    /// [`Error::DuplicateName`] if another node already holds the name.
    pub fn set_name(&mut self, id: NodeId, name: impl Into<String>) -> Result<(), Error> {
        let name = name.into();
        self.get(id)?;
        if let Some((&first, _)) = self.names.iter().find(|(_, existing)| *existing == &name) {
            return Err(Error::DuplicateName {
                name,
                first,
                second: id,
            });
        }
        self.names.insert(id, name);
        Ok(())
    }

    /// Remove a node's name, if it has one.
    ///
    /// # Errors
    ///
    /// [`Error::NameMissing`] if the node has no name to remove.
    pub fn clear_name(&mut self, id: NodeId) -> Result<(), Error> {
        self.get(id)?;
        match self.names.remove(&id) {
            Some(_) => Ok(()),
            None => Err(Error::NameMissing { node: id }),
        }
    }

    // ------------------------------------------------------------- dependencies

    /// The nodes `id` depends on, under `relation`.
    ///
    /// A [`Node::Wire`]'s dependency is its driver, which is the edge that makes
    /// a feedback loop visible to cycle detection at all. A node the relation
    /// treats as terminal has no dependencies here: the traversal stops.
    ///
    /// # Errors
    ///
    /// If `id` was not minted by this circuit.
    pub fn deps_in(&self, id: NodeId, relation: Deps) -> Result<Vec<NodeId>, Error> {
        let node = self.get(id)?;
        let mut out = Vec::new();
        match node {
            // A wire depends on its driver, and that is the only mutable edge in
            // the graph.
            Node::Wire { driver, .. } => {
                if let Some(driver) = driver.get() {
                    out.push(driver);
                }
            }
            _ if !relation.is_followed(node) => {}
            _ => node.push_operands(&mut out),
        }
        // Dependencies are in operand order, which is fixed per node kind, so the
        // result is deterministic. Downstream scheduling relies on that.
        Ok(out)
    }

    /// Every node reachable from `roots`, including the roots, in id order.
    ///
    /// # Errors
    ///
    /// Propagates any dangling operand.
    pub fn reachable(&self, roots: &[NodeId], relation: Deps) -> Result<Vec<NodeId>, Error> {
        let mut seen = vec![false; self.nodes.len()];
        let mut stack: Vec<NodeId> = roots.to_vec();
        while let Some(id) = stack.pop() {
            if seen[id.index()] {
                continue;
            }
            seen[id.index()] = true;
            for dep in self.deps_in(id, relation)? {
                if dep.index() >= self.nodes.len() {
                    return Err(Error::UnknownNode {
                        id: dep,
                        len: self.nodes.len(),
                    });
                }
                stack.push(dep);
            }
        }
        Ok((0..self.nodes.len())
            .filter(|&i| seen[i])
            .map(NodeId::from_index)
            .collect())
    }

    /// A topological order of every node, under `relation`.
    ///
    /// Uses Kahn's algorithm, so the result is a *valid* order rather than a
    /// canonical one; ties are broken by id, which makes the order deterministic
    /// for a fixed graph even though it is not the only valid order. Arena
    /// construction order is used as the tie-break precisely so that emitted
    /// Verilog is byte-identical between runs.
    ///
    /// # Errors
    ///
    /// [`Error::CombinationalLoop`] if the graph has a cycle under `relation`,
    /// with the cycle itself in the message.
    pub fn topological_order(&self, relation: Deps) -> Result<Vec<NodeId>, Error> {
        let len = self.nodes.len();
        let mut dependents: Vec<Vec<NodeId>> = vec![Vec::new(); len];
        let mut in_degree = vec![0usize; len];

        for id in self.node_ids() {
            for dep in self.deps_in(id, relation)? {
                if dep.index() >= len {
                    return Err(Error::UnknownNode { id: dep, len });
                }
                dependents[dep.index()].push(id);
                in_degree[id.index()] += 1;
            }
        }

        // A min-heap keyed on id would be the textbook choice; a sorted ready list
        // is used instead because it is allocation-light and, more importantly,
        // because popping the lowest id is what makes the output order canonical.
        let mut ready: Vec<NodeId> = (0..len)
            .filter(|&i| in_degree[i] == 0)
            .map(NodeId::from_index)
            .collect();
        ready.sort_unstable();

        let mut order = Vec::with_capacity(len);
        let mut emitted = 0usize;
        while let Some(&next) = ready.first() {
            ready.remove(0);
            order.push(next);
            emitted += 1;
            for &dependent in &dependents[next.index()] {
                in_degree[dependent.index()] -= 1;
                if in_degree[dependent.index()] == 0 {
                    let position = ready.partition_point(|id| *id < dependent);
                    ready.insert(position, dependent);
                }
            }
        }

        if emitted == len {
            return Ok(order);
        }
        Err(Error::CombinationalLoop {
            relation: relation_name(relation),
            cycle: self.find_cycle(relation)?,
        })
    }

    /// Find one cycle in the graph, for an error message.
    fn find_cycle(&self, relation: Deps) -> Result<Vec<NodeId>, Error> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Unvisited,
            OnPath,
            Done,
        }
        let len = self.nodes.len();
        let mut mark = vec![Mark::Unvisited; len];
        let mut path: Vec<NodeId> = Vec::new();

        for start in self.node_ids() {
            if mark[start.index()] != Mark::Unvisited {
                continue;
            }
            // Iterative DFS with an explicit stack of (node, next dependency
            // index), because a hardware graph can be deep enough to matter and
            // recursion here would be a stack-overflow source in a build tool.
            let mut stack: Vec<(NodeId, usize)> = vec![(start, 0)];
            mark[start.index()] = Mark::OnPath;
            path.push(start);
            while let Some(&mut (node, ref mut cursor)) = stack.last_mut() {
                let deps = self.deps_in(node, relation)?;
                if *cursor < deps.len() {
                    let dep = deps[*cursor];
                    *cursor += 1;
                    match mark[dep.index()] {
                        Mark::Unvisited => {
                            mark[dep.index()] = Mark::OnPath;
                            path.push(dep);
                            stack.push((dep, 0));
                        }
                        Mark::OnPath => {
                            // Found it. Report the loop starting where the
                            // repeated node first appears, and close it, so the
                            // message reads as a path rather than as a suffix.
                            let start_at =
                                path.iter().position(|&id| id == dep).expect("on the path");
                            let mut cycle = path[start_at..].to_vec();
                            cycle.push(dep);
                            return Ok(cycle);
                        }
                        Mark::Done => {}
                    }
                } else {
                    mark[node.index()] = Mark::Done;
                    path.pop();
                    stack.pop();
                }
            }
        }
        // A cycle must exist if the topological sort failed, so this is a bug
        // rather than a user error.
        Err(Error::CombinationalLoop {
            relation: relation_name(relation),
            cycle: Vec::new(),
        })
    }

    // ------------------------------------------------------------------- rewrite

    /// Stitch a sub-circuit's outputs into the parent graph.
    ///
    /// This copies `Signal_graph.rewrite` (`src/signal_graph.ml:105-204`) in its
    /// three phases, and the phase split is the point rather than an
    /// implementation detail:
    ///
    /// 1. **Validate and register.** Every entry maps an *original wire* to an
    ///    *unattached replacement wire* of the same width. Both are checked before
    ///    anything moves, so a malformed mapping cannot leave the graph half
    ///    rewired.
    /// 2. **Rewrite drivers.** Every operand in the graph is remapped. Note that
    ///    operands are rewritten but wire *drivers* are not — that is phase three,
    ///    and doing both at once would make the phases indistinguishable.
    /// 3. **Re-attach.** Each replacement wire is given the original's driver,
    ///    itself remapped through the same mapping, so a chain of replacements
    ///    resolves in one pass.
    ///
    /// After this the original wires are unreferenced. They *keep* their drivers
    /// rather than being undriven, which is what upstream does and which keeps the
    /// substitution reversible if a caller wants the original driver back. They
    /// become dead nodes that a future dead-code pass would collect; until one
    /// exists the emitter will emit them as unused logic, which synthesisers drop.
    ///
    /// # Errors
    ///
    /// - [`Error::NotAWire`] if an original is not a wire.
    /// - [`Error::ReplacementNotAWire`], [`Error::ReplacementDriven`] or
    ///   [`Error::ReplacementWidthMismatch`] for a malformed replacement.
    /// - [`Error::UnknownNode`] for an id that is not from this circuit.
    pub fn apply_replacements(
        &mut self,
        replacements: &BTreeMap<NodeId, NodeId>,
    ) -> Result<(), Error> {
        // Phase 1: validate the whole mapping before mutating anything.
        for (&original, &replacement) in replacements {
            let original_node = self.get(original)?;
            if !matches!(original_node, Node::Wire { .. }) {
                return Err(Error::NotAWire {
                    target: original,
                    kind: original_node.kind(),
                });
            }
            let original_width = self.width_of(original);
            match self.get(replacement)? {
                Node::Wire { width, driver } => {
                    if let Some(driver) = driver.get() {
                        return Err(Error::ReplacementDriven {
                            original,
                            replacement,
                            driver,
                        });
                    }
                    if *width != original_width {
                        return Err(Error::ReplacementWidthMismatch {
                            original,
                            replacement,
                            lhs: original_width,
                            rhs: *width,
                        });
                    }
                }
                other => {
                    return Err(Error::ReplacementNotAWire {
                        original,
                        replacement,
                        kind: other.kind(),
                    });
                }
            }
        }

        // Phase 2: rewrite every operand. Drivers are deliberately left alone.
        let lookup = |id: NodeId| replacements.get(&id).copied().unwrap_or(id);
        for node in &mut self.nodes {
            node.map_operands(lookup);
        }

        // Phase 3: re-attach. Reading the drivers first avoids borrowing two
        // entries of `self.nodes` at once.
        let mut attachments: Vec<(NodeId, NodeId)> = Vec::new();
        for (&original, &replacement) in replacements {
            let driver = match self.get(original)? {
                Node::Wire { driver, .. } => driver.get(),
                _ => None,
            };
            if let Some(driver) = driver {
                attachments.push((replacement, lookup(driver)));
            }
        }
        for (wire, driver) in attachments {
            let Node::Wire { driver: cell, .. } = &mut self.nodes[wire.index()] else {
                unreachable!("validated as a wire in phase 1")
            };
            cell.set(Some(driver));
        }
        Ok(())
    }

    // --------------------------------------------------------------------- check

    /// Check the whole graph: no dangling operands, no undriven wires, and every
    /// stored width agrees with the width its operands imply.
    ///
    /// An *input port* is an undriven wire by construction, so this reports one.
    /// Use [`Circuit::check_with_inputs`] when the circuit has ports.
    ///
    /// The width check is not redundant bookkeeping. Widths are *stored* on the
    /// node kinds that cannot derive them from their operands — `Case`, `ReadPort`,
    /// `Mem`, `Instance`, `Wire` — so that [`width_of`](Self::width_of) is a single
    /// match with no arena walk. Every constructor derives the width and stores it,
    /// which means the two can only disagree if a node was built by hand through
    /// [`add_node`](Self::add_node). This is where that is caught.
    ///
    /// # Errors
    ///
    /// The first problem found, in node id order.
    pub fn check(&self) -> Result<(), Error> {
        self.check_with_inputs(&[])
    }

    /// [`Circuit::check`], but treating `inputs` as driven from outside.
    ///
    /// A module's input port is an undriven wire by construction — something
    /// outside the module supplies it — so `check` needs to be told which wires
    /// are ports rather than inferring it from a name or a width. Hardcaml gets
    /// this from the port lists its `derive` macro generates; we take the list
    /// explicitly here and let the front end collect it.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownNode`] if an id in `inputs` is not in this circuit, and
    /// otherwise whatever [`Circuit::check`] reports.
    pub fn check_with_inputs(&self, inputs: &[NodeId]) -> Result<(), Error> {
        let mut externally_driven = vec![false; self.nodes.len()];
        for id in inputs {
            if id.index() >= self.nodes.len() {
                return Err(Error::UnknownNode {
                    id: *id,
                    len: self.nodes.len(),
                });
            }
            // Naming it a port does not excuse it being the wrong kind of node.
            if !matches!(self.get(*id)?, Node::Wire { .. }) {
                return Err(Error::NotAWire {
                    target: *id,
                    kind: self.get(*id)?.kind(),
                });
            }
            externally_driven[id.index()] = true;
        }
        for id in self.node_ids() {
            let node = self.get(id)?;
            for operand in node.operands() {
                if operand.index() >= self.nodes.len() {
                    return Err(Error::UnknownNode {
                        id: operand,
                        len: self.nodes.len(),
                    });
                }
            }
            // Stored-versus-derived widths.
            let derived = match node {
                Node::Case { arms, width, .. } => {
                    let first = self.width_of(arms[0].value);
                    (first, *width)
                }
                Node::ReadPort {
                    mem, data_width, ..
                } => (self.width_of(*mem), *data_width),
                // A select stores its own `len`, so the invariant that matters is
                // that the window fits the value it slices. `add_node` does no
                // checking, so a hand-built node can disagree.
                Node::Select {
                    value, offset, len, ..
                } => {
                    let value_width = self.width_of(*value);
                    if u64::from(*offset) + u64::from(*len) <= u64::from(value_width) {
                        continue;
                    }
                    (value_width, *len)
                }
                _ => continue,
            };
            if derived.0 != derived.1 {
                return Err(Error::WidthInvariant {
                    node: id,
                    stored: derived.1,
                    derived: derived.0,
                });
            }
        }
        // Undriven wires, but only ones somebody actually reads: an undriven wire
        // that nothing depends on is a leftover, not a bug, and reporting it would
        // make `check` fail on a work-in-progress graph.
        let mut referenced = vec![false; self.nodes.len()];
        for id in self.node_ids() {
            for operand in self.get(id)?.operands() {
                referenced[operand.index()] = true;
            }
            // A wire's driver is an edge but not an operand, so `operands` misses
            // it and `check` has to count it explicitly.
            let driver = match self.get(id)? {
                Node::Wire { driver, .. } => driver.get(),
                _ => None,
            };
            if let Some(driver) = driver {
                referenced[driver.index()] = true;
            }
        }
        for id in self.node_ids() {
            // A wire that nothing reads is a leftover, not a bug; reporting it
            // would make `check` fail on a work-in-progress graph.
            if !referenced[id.index()] {
                continue;
            }
            if externally_driven[id.index()] {
                continue;
            }
            if let Node::Wire { width, driver } = self.get(id)?
                && driver.get().is_none()
            {
                return Err(Error::UndrivenWire {
                    wire: id,
                    width: *width,
                });
            }
        }
        Ok(())
    }

    // ----------------------------------------------------------------- internals

    /// Append a node and mint its id.
    fn push(&mut self, node: Node) -> NodeId {
        let id = NodeId::from_index(self.nodes.len());
        self.nodes.push(node);
        id
    }

    /// Append a hand-built node.
    ///
    /// An escape hatch for node kinds this crate does not model yet, at the cost
    /// of the constructor-time width validation. [`check`](Self::check) is how you
    /// find out whether the node you built is self-consistent.
    ///
    /// # Errors
    ///
    /// If the node's kind is unknown to the width machinery, which cannot happen
    /// today; the signature is `Result` so that adding a kind later does not
    /// silently change this method's contract.
    pub fn add_node(&mut self, node: Node) -> Result<NodeId, Error> {
        Ok(self.push(node))
    }

    fn require_equal(&self, op: Op, left: NodeId, right: NodeId) -> Result<(), Error> {
        let lhs = self.width_of(left);
        let rhs = self.width_of(right);
        if lhs == rhs {
            Ok(())
        } else {
            Err(Error::WidthMismatch { op, lhs, rhs })
        }
    }

    fn require_one_bit(&self, op: Op, operand: &'static str, signal: NodeId) -> Result<(), Error> {
        let width = self.width_of(signal);
        if width == 1 {
            Ok(())
        } else {
            Err(Error::NotOneBit {
                op,
                operand,
                signal,
                width,
            })
        }
    }

    fn checked_sum(op: Op, a: u32, b: u32) -> Result<u32, Error> {
        a.checked_add(b).ok_or(Error::WidthOverflow {
            op,
            bits: u64::from(a) + u64::from(b),
        })
    }
}

fn relation_name(relation: Deps) -> Relation {
    match relation {
        Deps::LoopChecking => Relation::LoopChecking,
        Deps::SimulationScheduling => Relation::SimulationScheduling,
        Deps::WithoutCaseMatches => Relation::WithoutCaseMatches,
    }
}
