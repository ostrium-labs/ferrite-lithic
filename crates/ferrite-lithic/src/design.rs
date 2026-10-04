//! [`Design`], [`Signal`], and the error type the front end reports.

use std::cell::{Ref, RefCell};
use std::fmt;
use std::rc::Rc;

use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{CaseArm, Circuit, Deps, Error as IrError, NodeId, Op as IrOp, Relation};

/// Anything that can go wrong while building a circuit.
///
/// This wraps the two errors underneath rather than inventing a third hierarchy:
/// a bad literal is [`ferrite_lithic_bits::Error`], and everything about graph
/// shape is [`ferrite_lithic_ir::Error`]. Both already name the operation, both
/// widths, and the fix, so the wrappers only add context.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A literal could not be built.
    Bits(ferrite_lithic_bits::Error),
    /// The graph operation was rejected.
    Ir(IrError),
    /// A signal from one design was used in another without being adopted.
    ForeignSignal {
        /// The arena the signal actually belongs to.
        belongs_to: u64,
        /// The arena it was passed into.
        used_in: u64,
    },
}

impl Error {
    /// The IR-level error, if this is one.
    #[must_use]
    pub const fn as_ir(&self) -> Option<&IrError> {
        match self {
            Self::Ir(inner) => Some(inner),
            _ => None,
        }
    }

    /// The bits-level error, if this is one.
    #[must_use]
    pub const fn as_bits(&self) -> Option<&ferrite_lithic_bits::Error> {
        match self {
            Self::Bits(inner) => Some(inner),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bits(inner) => write!(f, "{inner}"),
            Self::Ir(inner) => write!(f, "{inner}"),
            Self::ForeignSignal {
                belongs_to,
                used_in,
            } => write!(
                f,
                "this signal belongs to design {belongs_to}, but it was used in \
                 design {used_in}. A signal is a node id plus a handle to its own \
                 arena, so passing one to a different design would push new nodes \
                 into the wrong arena while reporting the old design's ids. Call \
                 `Design::adopt` on the foreign signal to copy it in as a literal \
                 of its current value, or build the signal in the design that will \
                 use it."
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<ferrite_lithic_bits::Error> for Error {
    fn from(inner: ferrite_lithic_bits::Error) -> Self {
        Self::Bits(inner)
    }
}

impl From<IrError> for Error {
    fn from(inner: IrError) -> Self {
        Self::Ir(inner)
    }
}

/// A handle to one node in one [`Design`]'s arena.
///
/// Cloning is cheap and does not copy the node. A signal carries its width,
/// which is what lets the operators report a mismatch without reaching back into
/// the arena.
///
/// A signal belongs to the design that produced it. Passing one to another
/// design is refused by [`Design::adopt`] rather than silently accepted.
#[derive(Clone)]
pub struct Signal {
    id: NodeId,
    width: u32,
    arena: Rc<Arena>,
}

impl Signal {
    fn new(id: NodeId, width: u32, arena: Rc<Arena>) -> Self {
        Self { id, width, arena }
    }

    /// The underlying node id, for handing back to the IR.
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// The signal's width in bits.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The node's kind name, for error messages and debugging.
    ///
    /// On the signal rather than only on [`Design`] so a node can be inspected
    /// without the design that made it.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        self.arena
            .circuit
            .borrow()
            .get(self.id)
            .map_or("Unknown", ferrite_lithic_ir::Node::kind)
    }

    /// Whether this node holds state: a register, a memory, or an instance.
    #[must_use]
    pub fn is_stateful(&self) -> bool {
        self.arena
            .circuit
            .borrow()
            .get(self.id)
            .is_ok_and(ferrite_lithic_ir::Node::is_stateful)
    }

    /// This signal's name, if it has one.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.arena
            .circuit
            .borrow()
            .name(self.id)
            .map(str::to_string)
    }

    /// A cheap handle onto the design that owns this signal.
    pub(crate) fn design(&self) -> Design {
        Design {
            arena: Rc::clone(&self.arena),
        }
    }

    fn belongs_to(&self, arena: &Rc<Arena>) -> bool {
        Rc::ptr_eq(&self.arena, arena)
    }
}

impl fmt::Debug for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signal")
            .field("id", &self.id)
            .field("width", &self.width)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Signal {
    /// Two signals are equal when they are the same node in the same arena.
    ///
    /// Comparing only node ids would make a signal from one design equal to a
    /// signal from another, which is precisely the mistake this crate refuses.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Rc::ptr_eq(&self.arena, &other.arena)
    }
}

impl Eq for Signal {}

/// Builds a circuit.
///
/// Every method takes `&self`, which is what lets a call be nested inside
/// another. See the [crate docs](crate) for why that needs interior mutability
/// and why the borrow cannot overlap.
#[derive(Debug, Clone)]
pub struct Design {
    arena: Rc<Arena>,
}

/// The state a [`Design`] and its [`Signal`]s share.
///
/// Split out so that a [`Signal`] can hold the same `Rc` the design does and
/// recover a usable [`Design`] view from it. Without this the operators could not
/// work: an operator is handed only its operands, so it needs a way back to the
/// arena they live in.
#[derive(Debug)]
struct Arena {
    circuit: RefCell<Circuit>,
    /// Wires supplied from outside the module. An input port is an undriven wire
    /// by construction, so `check` has to be told which wires are ports rather
    /// than inferring it from a name or a width.
    inputs: RefCell<Vec<NodeId>>,
    /// Wires driven from inside and presented as module outputs, in declaration
    /// order. See [`Design::output`].
    outputs: RefCell<Vec<NodeId>>,
    /// Identity, so a mixed-design error can name two designs rather than print
    /// addresses.
    tag: u64,
}

impl Default for Design {
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    static NEXT_TAG: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

impl Design {
    /// A design with no nodes.
    #[must_use]
    pub fn new() -> Self {
        let tag = NEXT_TAG.with(|next| {
            let tag = next.get();
            next.set(tag.wrapping_add(1));
            tag
        });
        Self {
            arena: Rc::new(Arena {
                circuit: RefCell::new(Circuit::new()),
                inputs: RefCell::new(Vec::new()),
                outputs: RefCell::new(Vec::new()),
                tag,
            }),
        }
    }

    /// A design wrapping an existing circuit.
    #[must_use]
    pub fn from_circuit(circuit: Circuit) -> Self {
        let design = Self::new();
        *design.arena.circuit.borrow_mut() = circuit;
        design
    }

    /// This design's identity, used in cross-design error messages.
    #[must_use]
    pub fn tag(&self) -> u64 {
        self.arena.tag
    }

    /// Borrow the arena for reading.
    pub fn circuit(&self) -> Ref<'_, Circuit> {
        self.arena.circuit.borrow()
    }

    /// How many nodes have been pushed.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.arena.circuit.borrow().len()
    }

    /// Run the IR's completeness checks: dangling operands, width invariants, and
    /// wires that are read but never driven.
    pub fn check(&self) -> Result<(), Error> {
        let inputs = self.arena.inputs.borrow().clone();
        self.arena.circuit.borrow().check_with_inputs(&inputs)?;
        Ok(())
    }

    /// Whether `signal` is a node of *this* design.
    ///
    /// Backends need this, because a [`NodeId`](ferrite_lithic_ir::NodeId) on its
    /// own is only meaningful within one arena: signal 3 of another design is not
    /// signal 3 of this one, and a lookup that skipped this check would silently
    /// read the wrong node. The operators get it for free from
    /// [`PartialEq`]; a consumer holding only a node id does not.
    #[must_use]
    pub fn owns(&self, signal: &Signal) -> bool {
        signal.belongs_to(&self.arena)
    }

    /// The wires declared as input ports, in declaration order.
    #[must_use]
    pub fn input_ports(&self) -> Vec<Signal> {
        self.arena
            .inputs
            .borrow()
            .iter()
            .map(|id| self.wrap(*id))
            .collect()
    }

    /// The wires declared as output ports, in declaration order.
    #[must_use]
    pub fn output_ports(&self) -> Vec<Signal> {
        self.arena
            .outputs
            .borrow()
            .iter()
            .map(|id| self.wrap(*id))
            .collect()
    }

    /// Check the design and hand back the circuit.
    ///
    /// The circuit is a snapshot: later pushes into this design do not change it.
    pub fn build(&self) -> Result<Circuit, Error> {
        self.check()?;
        Ok((*self.arena.circuit.borrow()).clone())
    }

    /// Copy a foreign signal into this design as a constant of its current value.
    ///
    /// Only meaningful for a signal that already has a value — a constant, or a
    /// node this design can read. A wire or an operation from another design has
    /// no single value to copy, so those are refused rather than silently
    /// producing a constant of zero.
    pub fn adopt(&self, foreign: &Signal) -> Result<Signal, Error> {
        if foreign.belongs_to(&self.arena) {
            return Ok(foreign.clone());
        }
        let node = foreign.arena.circuit.borrow().get(foreign.id)?.clone();
        match node {
            ferrite_lithic_ir::Node::Constant { value } => Ok(self.bits(value)),
            other => Err(Error::Ir(IrError::NotAConstant {
                target: foreign.id,
                kind: other.kind(),
            })),
        }
    }

    fn wrap(&self, id: NodeId) -> Signal {
        let width = self.arena.circuit.borrow().width_of(id);
        Signal::new(id, width, Rc::clone(&self.arena))
    }

    fn push(&self, id: NodeId) -> Signal {
        self.wrap(id)
    }

    fn same_design(&self, signals: &[&Signal]) -> Result<(), Error> {
        for s in signals {
            if !s.belongs_to(&self.arena) {
                return Err(Error::ForeignSignal {
                    belongs_to: s.arena.tag,
                    used_in: self.arena.tag,
                });
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ leaves

    /// A constant of the given value and width.
    ///
    /// `value` is truncated to `width` bits, so a width above 64 cannot hold the
    /// whole value. That is stated rather than hidden because it is the one place
    /// a caller can lose information without an error.
    pub fn lit(&self, value: u64, width: u32) -> Result<Signal, Error> {
        Ok(self.bits(Bits::constant(value, width)?))
    }

    /// A constant from an already-built [`Bits`], which carries its own width.
    pub fn bits(&self, value: Bits) -> Signal {
        let id = self.arena.circuit.borrow_mut().constant(value);
        self.push(id)
    }

    /// A constant of `width` zero bits.
    pub fn zeros(&self, width: u32) -> Result<Signal, Error> {
        Ok(self.bits(Bits::zeros(width)?))
    }

    /// A constant of `width` one bits.
    pub fn ones(&self, width: u32) -> Result<Signal, Error> {
        Ok(self.bits(Bits::ones(width)?))
    }

    /// The one-bit constant `false`, which is what an unused reset or enable wants.
    #[must_use]
    pub fn constant(&self, value: bool) -> Signal {
        self.bits(Bits::constant(u64::from(value), 1).expect("a 1-bit constant is always valid"))
    }

    /// An undriven wire of the given width.
    pub fn wire(&self, width: u32) -> Result<Signal, Error> {
        let id = self.arena.circuit.borrow_mut().wire(width)?;
        Ok(self.push(id))
    }

    /// An undriven, named wire: a module input port.
    ///
    /// Naming at construction is deliberate. A port that is created unnamed and
    /// named later is how a design ends up with two anonymous inputs and a
    /// generated port order nobody chose.
    pub fn input(&self, name: &str, width: u32) -> Result<Signal, Error> {
        let id = self.arena.circuit.borrow_mut().wire(width)?;
        self.arena.circuit.borrow_mut().set_name(id, name)?;
        self.arena.inputs.borrow_mut().push(id);
        Ok(self.push(id))
    }

    /// An output port: a named wire driven by `driver`.
    ///
    /// Recorded in declaration order, symmetrically with [`Design::input`]. A
    /// backend needs the port list to know what a cycle's result *is*: without
    /// it a simulator can only dump every named node, and the answer to "what did
    /// this module produce" becomes a guess about which names matter.
    pub fn output(&self, name: &str, width: u32, driver: &Signal) -> Result<Signal, Error> {
        let id = self.arena.circuit.borrow_mut().wire(width)?;
        self.drive(&self.push(id), driver)?;
        self.arena.circuit.borrow_mut().set_name(id, name)?;
        self.arena.outputs.borrow_mut().push(id);
        Ok(self.push(id))
    }

    // ------------------------------------------------------------ drivers

    /// Attach `driver` to `wire`, closing a loop or filling an input.
    pub fn drive(&self, wire: &Signal, driver: &Signal) -> Result<(), Error> {
        self.same_design(&[wire, driver])?;
        self.arena.circuit.borrow_mut().drive(wire.id, driver.id)?;
        Ok(())
    }

    /// Detach whatever drives `wire`, returning the old driver.
    pub fn undrive(&self, wire: &Signal) -> Result<Option<Signal>, Error> {
        self.same_design(&[wire])?;
        let old = self.arena.circuit.borrow_mut().undrive(wire.id)?;
        Ok(old.map(|id| self.push(id)))
    }

    /// What drives `wire`, if anything.
    pub fn driver_of(&self, wire: &Signal) -> Result<Option<Signal>, Error> {
        self.same_design(&[wire])?;
        let id = self.arena.circuit.borrow().driver_of(wire.id)?;
        Ok(id.map(|id| self.push(id)))
    }

    // ------------------------------------------------------------ names

    /// Give a node a name.
    pub fn set_name(&self, signal: &Signal, name: &str) -> Result<(), Error> {
        self.same_design(&[signal])?;
        self.arena.circuit.borrow_mut().set_name(signal.id, name)?;
        Ok(())
    }

    /// Remove a node's name.
    pub fn clear_name(&self, signal: &Signal) -> Result<(), Error> {
        self.same_design(&[signal])?;
        self.arena.circuit.borrow_mut().clear_name(signal.id)?;
        Ok(())
    }

    /// A node's name, if it has one.
    #[must_use]
    pub fn name(&self, signal: &Signal) -> Option<String> {
        if !signal.belongs_to(&self.arena) {
            return None;
        }
        self.arena
            .circuit
            .borrow()
            .name(signal.id)
            .map(str::to_string)
    }

    // ------------------------------------------------------------ combinational

    /// Bitwise complement.
    #[must_use]
    pub fn not(&self, arg: &Signal) -> Signal {
        let id = self.arena.circuit.borrow_mut().not(arg.id);
        self.push(id)
    }

    /// Bitwise AND. Requires equal widths.
    pub fn and(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().bit_and(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Bitwise OR. Requires equal widths.
    pub fn or(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().bit_or(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Bitwise exclusive OR. Requires equal widths.
    pub fn xor(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().bit_xor(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Addition, truncating to the operands' common width.
    pub fn add(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().add(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Subtraction, truncating to the operands' common width.
    pub fn sub(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().sub(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Multiplication, widening to the sum of the operand widths.
    pub fn mul(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().mul(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Unsigned division. Infers a large divider; see the crate notes.
    pub fn udiv(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().udiv(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Signed division. Infers a large divider; see the crate notes.
    pub fn sdiv(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().sdiv(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Unsigned remainder. Infers a large divider; see the crate notes.
    pub fn urem(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().urem(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Signed remainder. Infers a large divider; see the crate notes.
    pub fn srem(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().srem(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Equality. One bit wide.
    pub fn eq(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().eq(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Unsigned `<`. One bit wide.
    pub fn ult(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().ult(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Unsigned `<=`. One bit wide.
    pub fn ule(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().ule(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Unsigned `>`. One bit wide.
    pub fn ugt(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().ugt(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Unsigned `>=`. One bit wide.
    pub fn uge(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().uge(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Signed `<`. One bit wide.
    pub fn slt(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().slt(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Signed `<=`. One bit wide.
    pub fn sle(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().sle(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Signed `>`. One bit wide.
    pub fn sgt(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().sgt(left.id, right.id)?;
        Ok(self.push(id))
    }

    /// Signed `>=`. One bit wide.
    pub fn sge(&self, left: &Signal, right: &Signal) -> Result<Signal, Error> {
        self.same_design(&[left, right])?;
        let id = self.arena.circuit.borrow_mut().sge(left.id, right.id)?;
        Ok(self.push(id))
    }

    // ------------------------------------------------------------ selection

    /// A run of `len` bits starting at `offset`, counting from the least
    /// significant bit.
    pub fn slice(&self, value: &Signal, offset: u32, len: u32) -> Result<Signal, Error> {
        self.same_design(&[value])?;
        let id = self
            .arena
            .circuit
            .borrow_mut()
            .select(value.id, offset, len)?;
        Ok(self.push(id))
    }

    /// Concatenate, most significant first, so `concat(&[a, b])` puts `a` above
    /// `b`.
    ///
    /// # Errors
    ///
    /// If fewer than one signal is given, or the widths sum to zero.
    pub fn concat(&self, parts: &[Signal]) -> Result<Signal, Error> {
        let refs: Vec<&Signal> = parts.iter().collect();
        self.same_design(&refs)?;
        let ids: Vec<NodeId> = parts.iter().map(Signal::id).collect();
        let Some((first, rest)) = ids.split_first() else {
            return Err(Error::Ir(IrError::ZeroWidth { op: IrOp::Cat }));
        };
        // `Circuit::cat` is binary, so fold right: accumulate and put each next
        // part *below* what is already built. Folding left would put the first
        // part at the bottom, the opposite of the stated order.
        let mut accumulated = *first;
        for next in rest {
            accumulated = self.arena.circuit.borrow_mut().cat(accumulated, *next)?;
        }
        Ok(self.push(accumulated))
    }

    /// Repeat a signal `count` times, most significant copy first.
    pub fn replicate(&self, value: &Signal, count: u32) -> Result<Signal, Error> {
        self.same_design(&[value])?;
        let id = self.arena.circuit.borrow_mut().replicate(value.id, count)?;
        Ok(self.push(id))
    }

    /// Widen with zero bits above.
    pub fn zero_extend(&self, value: &Signal, new_width: u32) -> Result<Signal, Error> {
        self.resize(value, new_width, false)
    }

    /// Widen by replicating the sign bit.
    pub fn sign_extend(&self, value: &Signal, new_width: u32) -> Result<Signal, Error> {
        self.resize(value, new_width, true)
    }

    /// Narrow by discarding the high bits.
    pub fn truncate(&self, value: &Signal, new_width: u32) -> Result<Signal, Error> {
        self.same_design(&[value])?;
        if new_width == 0 {
            return Err(Error::Ir(IrError::ZeroWidth { op: IrOp::Select }));
        }
        if new_width > value.width {
            return Err(Error::Ir(IrError::SelectOutOfRange {
                op: IrOp::Select,
                value_width: value.width,
                offset: 0,
                len: new_width,
            }));
        }
        self.slice(value, 0, new_width)
    }

    /// Widen or narrow, choosing the fill from `signed`.
    fn resize(&self, value: &Signal, new_width: u32, signed: bool) -> Result<Signal, Error> {
        self.same_design(&[value])?;
        if new_width == 0 {
            return Err(Error::Ir(IrError::ZeroWidth { op: IrOp::Select }));
        }
        if new_width == value.width {
            return Ok(value.clone());
        }
        if new_width < value.width {
            return self.slice(value, 0, new_width);
        }
        // Widening. Both extensions keep the low `width` bits in place, so the
        // interesting half is what goes above them, and `cat` takes high first.
        let low = self.slice(value, 0, value.width)?;
        let fill = if signed {
            // The sign bit is bit `width - 1`; replicate it upwards.
            let msb = self.slice(value, value.width - 1, 1)?;
            self.replicate(&msb, new_width - value.width)?
        } else {
            self.zeros(new_width - value.width)?
        };
        self.concat(&[fill, low])
    }

    /// Shift left by a constant amount. Structural: select plus `cat`.
    ///
    /// Shifting by at least the width yields all zeros. A shift of zero is the
    /// whole value and builds no filler, since there is no zero-width `cat` part
    /// to build and a zero-width value does not exist.
    ///
    /// The moved bits end up in the *high* positions, so the surviving low bits are
    /// `concat`'s first argument and the zero fill is its second.
    pub fn sll(&self, value: &Signal, by: u32) -> Result<Signal, Error> {
        self.same_design(&[value])?;
        if by == 0 {
            return self.slice(value, 0, value.width);
        }
        if by >= value.width {
            return self.zeros(value.width);
        }
        let high = self.slice(value, 0, value.width - by)?;
        let low = self.zeros(by)?;
        self.concat(&[high, low])
    }

    /// Shift right by a constant amount, filling with zeros. Structural.
    ///
    /// The moved bits end up in the *low* positions, so the zero fill is `concat`'s
    /// first argument and the surviving high bits are its second.
    pub fn srl(&self, value: &Signal, by: u32) -> Result<Signal, Error> {
        self.same_design(&[value])?;
        if by == 0 {
            return self.slice(value, 0, value.width);
        }
        if by >= value.width {
            return self.zeros(value.width);
        }
        let high = self.zeros(by)?;
        let low = self.slice(value, by, value.width - by)?;
        self.concat(&[high, low])
    }

    /// Shift right by a constant amount, filling with the sign bit. Structural.
    pub fn sra(&self, value: &Signal, by: u32) -> Result<Signal, Error> {
        self.same_design(&[value])?;
        if by == 0 {
            return self.slice(value, 0, value.width);
        }
        if by >= value.width {
            // Sign-fill the whole width, which is what an arithmetic shift by
            // more than the width does.
            let msb = self.slice(value, value.width - 1, 1)?;
            return self.replicate(&msb, value.width);
        }
        let msb = self.slice(value, value.width - 1, 1)?;
        let high = self.replicate(&msb, by)?;
        let low = self.slice(value, by, value.width - by)?;
        self.concat(&[high, low])
    }

    // ------------------------------------------------------------ structural

    /// A two-way mux. `condition` must be one bit.
    pub fn ite(
        &self,
        condition: &Signal,
        then_value: &Signal,
        otherwise: &Signal,
    ) -> Result<Signal, Error> {
        self.same_design(&[condition, then_value, otherwise])?;
        let id = self
            .arena
            .circuit
            .borrow_mut()
            .ite(condition.id, then_value.id, otherwise.id)?;
        Ok(self.push(id))
    }

    /// A `case` over a literal scrutinee. Arms are tried in order; `default` is
    /// used when none matches.
    pub fn case_(
        &self,
        scrutinee: &Signal,
        arms: &[(u64, Signal)],
        default: &Signal,
    ) -> Result<Signal, Error> {
        let mut all: Vec<&Signal> = vec![scrutinee, default];
        all.extend(arms.iter().map(|(_, s)| s));
        self.same_design(&all)?;
        let mut case_arms = Vec::with_capacity(arms.len());
        for (value, result) in arms {
            let literal = Bits::constant(*value, scrutinee.width)?;
            case_arms.push(CaseArm::new(literal, result.id));
        }
        let id = self
            .arena
            .circuit
            .borrow_mut()
            .case_(scrutinee.id, case_arms, default.id)?;
        Ok(self.push(id))
    }

    /// A register.
    ///
    /// The register's output *is* the returned signal: it stores no output value
    /// of its own, which is what makes "is this stateful?" a constructor match.
    /// `reset` and `clear` must be one bit.
    /// A register. Its output is the returned signal; `data` is its input.
    ///
    /// Both control inputs are one bit wide and both act *synchronously*, at the
    /// edge:
    ///
    /// | clock | reset | clear | effect |
    /// |---|---|---|---|
    /// | 0 | – | – | holds |
    /// | 1 | 1 | – | becomes zero |
    /// | 1 | 0 | 1 | holds |
    /// | 1 | 0 | 0 | becomes `data` |
    ///
    /// **`clear` gates the update and `reset` overrides it.** That ordering is the
    /// one worth stating out loud, because the two are easy to swap and a swapped
    /// pair produces a register that never updates — which looks exactly like a
    /// simulator that is not advancing. To *hold* a register, assert `clear`; to
    /// *reset* it, assert `reset`.
    ///
    /// Registers start at zero; a non-zero starting value is a simulation-time
    /// choice rather than part of the circuit.
    pub fn reg(
        &self,
        data: &Signal,
        clock: &Signal,
        reset: &Signal,
        clear: &Signal,
    ) -> Result<Signal, Error> {
        self.same_design(&[data, clock, reset, clear])?;
        let id = self
            .arena
            .circuit
            .borrow_mut()
            .reg(data.id, clock.id, reset.id, clear.id)?;
        Ok(self.push(id))
    }

    /// A memory. Reads and writes go through separate port calls.
    pub fn mem(&self, data_width: u32, depth: u32) -> Result<Signal, Error> {
        let id = self.arena.circuit.borrow_mut().mem(data_width, depth)?;
        Ok(self.push(id))
    }

    /// A memory's data width and depth.
    pub fn memory_shape(&self, mem: &Signal) -> Result<(u32, u32), Error> {
        self.same_design(&[mem])?;
        Ok(self.arena.circuit.borrow().memory_shape(mem.id)?)
    }

    /// A memory's write ports, in construction order.
    pub fn write_ports_of(&self, mem: &Signal) -> Result<Vec<MemoryWritePort>, Error> {
        self.same_design(&[mem])?;
        let ports = self.arena.circuit.borrow().write_ports_of(mem.id)?.to_vec();
        Ok(ports
            .into_iter()
            .map(|port| MemoryWritePort {
                data: self.push(port.data),
                address: self.push(port.address),
                enable: self.push(port.enable),
            })
            .collect())
    }

    /// Attach a write port to a memory. `enable` must be one bit.
    pub fn write_port(
        &self,
        mem: &Signal,
        data: &Signal,
        address: &Signal,
        enable: &Signal,
    ) -> Result<(), Error> {
        self.same_design(&[mem, data, address, enable])?;
        self.arena
            .circuit
            .borrow_mut()
            .write_port(mem.id, data.id, address.id, enable.id)?;
        Ok(())
    }

    /// A read port. The read is asynchronous by construction; a synchronous read
    /// is a register fed from here.
    pub fn read_port(
        &self,
        mem: &Signal,
        address: &Signal,
        enable: &Signal,
    ) -> Result<Signal, Error> {
        self.same_design(&[mem, address, enable])?;
        let id = self
            .arena
            .circuit
            .borrow_mut()
            .read_port(mem.id, address.id, enable.id)?;
        Ok(self.push(id))
    }

    /// A submodule instance with a single output.
    ///
    /// One output only, because a multi-signal submodule needs a bundle type and
    /// that design is not settled. This is an A1 limitation, not a position.
    pub fn instance(
        &self,
        name: &str,
        params: &[(&str, Bits)],
        inputs: &[Signal],
        output_width: u32,
    ) -> Result<Signal, Error> {
        let mut all: Vec<&Signal> = inputs.iter().collect();
        all.retain(|s| !s.belongs_to(&self.arena));
        if !all.is_empty() {
            return Err(Error::ForeignSignal {
                belongs_to: all[0].arena.tag,
                used_in: self.arena.tag,
            });
        }
        let params = params
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect::<Vec<_>>();
        let ids = inputs.iter().map(Signal::id).collect::<Vec<_>>();
        let id = self
            .arena
            .circuit
            .borrow_mut()
            .instance(name, params, ids, output_width)?;
        Ok(self.push(id))
    }

    // ------------------------------------------------------------ queries

    /// The signals in evaluation order, under the given dependency relation.
    pub fn topological_order(&self, relation: Deps) -> Result<Vec<Signal>, Error> {
        let ids = self.arena.circuit.borrow().topological_order(relation)?;
        Ok(ids.into_iter().map(|id| self.push(id)).collect())
    }

    /// The signals reachable from `roots`, under the given relation.
    pub fn reachable(&self, roots: &[Signal], relation: Deps) -> Result<Vec<Signal>, Error> {
        self.same_design(&roots.iter().collect::<Vec<_>>())?;
        let ids = roots.iter().map(Signal::id).collect::<Vec<_>>();
        let out = self.arena.circuit.borrow().reachable(&ids, relation)?;
        Ok(out.into_iter().map(|id| self.push(id)).collect())
    }

    /// Every signal in the design, in construction order.
    pub fn signals(&self) -> Vec<Signal> {
        self.arena
            .circuit
            .borrow()
            .node_ids()
            .map(|id| self.wrap(id))
            .collect()
    }

    /// The node's kind name, for error messages and debugging.
    #[must_use]
    pub fn kind_of(&self, signal: &Signal) -> Option<&'static str> {
        if !signal.belongs_to(&self.arena) {
            return None;
        }
        self.arena
            .circuit
            .borrow()
            .get(signal.id)
            .ok()
            .map(|n| n.kind())
    }

    /// Whether a node holds state: a register, a memory, or an instance.
    #[must_use]
    pub fn is_stateful(&self, signal: &Signal) -> Option<bool> {
        if !signal.belongs_to(&self.arena) {
            return None;
        }
        self.arena
            .circuit
            .borrow()
            .get(signal.id)
            .ok()
            .map(|n| n.is_stateful())
    }

    /// The width the IR records for a node, which is a cross-check on the width
    /// cached in the handle.
    #[must_use]
    pub fn ir_width_of(&self, signal: &Signal) -> Option<u32> {
        if !signal.belongs_to(&self.arena) {
            return None;
        }
        Some(self.arena.circuit.borrow().width_of(signal.id))
    }

    /// The name of the relation, for diagnostics.
    #[must_use]
    pub fn relation_name(relation: Deps) -> &'static str {
        relation.name()
    }

    /// The IR error's operation name, for tests and messages.
    #[must_use]
    pub fn op_name(op: IrOp) -> &'static str {
        op.name()
    }

    /// The IR error's relation name, for tests and messages.
    #[must_use]
    pub fn ir_relation_name(relation: Relation) -> &'static str {
        relation.name()
    }
}

/// One of a memory's write ports, as signals.
#[derive(Debug, Clone)]
pub struct MemoryWritePort {
    /// The data to write.
    pub data: Signal,
    /// The address to write to.
    pub address: Signal,
    /// The write enable, one bit.
    pub enable: Signal,
}
