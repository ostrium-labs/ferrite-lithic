//! Errors produced while building and checking a [`Circuit`](crate::Circuit).
//!
//! # Why this type is as verbose as the one in `ferrite-lithic-bits`
//!
//! Widths are runtime values ([ADR-0003](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0003-runtime-widths-not-const-generics.md)),
//! so a width mismatch during graph construction is a *runtime* event rather than
//! a compile error. Graph construction is loops and generators, which is exactly
//! where const generics hurt, so the trade is worth making — but it means the
//! error message has to carry the compensation the compiler would have provided.
//!
//! Two of these errors are inherited verbatim from Hardcaml, which performs the
//! same three checks on assignment and for the same reasons
//! (`kernel/signal.ml:242-267`): a node that is not a wire cannot be assigned, a
//! wire cannot be assigned twice, and a wire's driver must match its width. The
//! third check exists in ours and not upstream's because we separate arena
//! construction from driving.
//!
//! [`Error`] deliberately carries no source location. This crate has no notion of
//! source text; the DSL layer in `ferrite-lithic` attaches a `Location` when it
//! propagates an error out of a user expression, where the location is known.

use core::fmt;

use crate::id::NodeId;

/// The operation that failed.
///
/// Carried in every error so a message can name what was attempted. Without it a
/// width mismatch between an 8-bit and a 16-bit value is ambiguous: the same pair
/// of widths is an error for `add` and perfectly fine for `cat`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Op {
    /// [`Circuit::constant`](crate::Circuit::constant).
    Constant,
    /// [`Circuit::wire`](crate::Circuit::wire).
    Wire,
    /// [`Circuit::drive`](crate::Circuit::drive).
    Drive,
    /// [`Circuit::not`](crate::Circuit::not).
    Not,
    /// [`Circuit::select`](crate::Circuit::select).
    Select,
    /// [`Circuit::bit_and`](crate::Circuit::bit_and).
    BitAnd,
    /// [`Circuit::bit_or`](crate::Circuit::bit_or).
    BitOr,
    /// [`Circuit::bit_xor`](crate::Circuit::bit_xor).
    BitXor,
    /// [`Circuit::add`](crate::Circuit::add).
    Add,
    /// [`Circuit::sub`](crate::Circuit::sub).
    Sub,
    /// [`Circuit::mul`](crate::Circuit::mul).
    Mul,
    /// [`Circuit::udiv`](crate::Circuit::udiv).
    UDiv,
    /// [`Circuit::sdiv`](crate::Circuit::sdiv).
    SDiv,
    /// [`Circuit::urem`](crate::Circuit::urem).
    URem,
    /// [`Circuit::srem`](crate::Circuit::srem).
    SRem,
    /// [`Circuit::eq`](crate::Circuit::eq).
    Eq,
    /// [`Circuit::ult`](crate::Circuit::ult).
    Ult,
    /// [`Circuit::ule`](crate::Circuit::ule).
    Ule,
    /// [`Circuit::ugt`](crate::Circuit::ugt).
    Ugt,
    /// [`Circuit::uge`](crate::Circuit::uge).
    Uge,
    /// [`Circuit::slt`](crate::Circuit::slt).
    Slt,
    /// [`Circuit::sle`](crate::Circuit::sle).
    Sle,
    /// [`Circuit::sgt`](crate::Circuit::sgt).
    Sgt,
    /// [`Circuit::sge`](crate::Circuit::sge).
    Sge,
    /// [`Circuit::cat`](crate::Circuit::cat).
    Cat,
    /// [`Circuit::replicate`](crate::Circuit::replicate).
    Replicate,
    /// [`Circuit::ite`](crate::Circuit::ite).
    Ite,
    /// [`Circuit::case_`](crate::Circuit::case_).
    Case,
    /// [`Circuit::reg`](crate::Circuit::reg).
    Reg,
    /// [`Circuit::mem`](crate::Circuit::mem).
    Mem,
    /// [`Circuit::read_port`](crate::Circuit::read_port).
    ReadPort,
    /// [`Circuit::instance`](crate::Circuit::instance).
    Instance,
    /// [`Circuit::set_name`](crate::Circuit::set_name).
    Name,
    /// [`Circuit::apply_replacements`](crate::Circuit::apply_replacements).
    Replace,
}

impl Op {
    /// A short name for the operation, for diagnostics and tests.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Constant => "constant",
            Self::Wire => "wire",
            Self::Drive => "drive",
            Self::Not => "not",
            Self::Select => "select",
            Self::BitAnd => "bit_and",
            Self::BitOr => "bit_or",
            Self::BitXor => "bit_xor",
            Self::Add => "add",
            Self::Sub => "sub",
            Self::Mul => "mul",
            Self::UDiv => "udiv",
            Self::SDiv => "sdiv",
            Self::URem => "urem",
            Self::SRem => "srem",
            Self::Eq => "eq",
            Self::Ult => "ult",
            Self::Ule => "ule",
            Self::Ugt => "ugt",
            Self::Uge => "uge",
            Self::Slt => "slt",
            Self::Sle => "sle",
            Self::Sgt => "sgt",
            Self::Sge => "sge",
            Self::Cat => "cat",
            Self::Replicate => "replicate",
            Self::Ite => "ite",
            Self::Case => "case_",
            Self::Reg => "reg",
            Self::Mem => "mem",
            Self::ReadPort => "read_port",
            Self::Instance => "instance",
            Self::Name => "name",
            Self::Replace => "replace",
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Op::Constant => "constant",
            Op::Wire => "wire",
            Op::Drive => "drive",
            Op::Not => "not",
            Op::Select => "select",
            Op::BitAnd => "bit_and",
            Op::BitOr => "bit_or",
            Op::BitXor => "bit_xor",
            Op::Add => "add",
            Op::Sub => "sub",
            Op::Mul => "mul",
            Op::UDiv => "udiv",
            Op::SDiv => "sdiv",
            Op::URem => "urem",
            Op::SRem => "srem",
            Op::Eq => "eq",
            Op::Ult => "ult",
            Op::Ule => "ule",
            Op::Ugt => "ugt",
            Op::Uge => "uge",
            Op::Slt => "slt",
            Op::Sle => "sle",
            Op::Sgt => "sgt",
            Op::Sge => "sge",
            Op::Cat => "cat",
            Op::Replicate => "replicate",
            Op::Ite => "ite",
            Op::Case => "case_",
            Op::Reg => "reg",
            Op::Mem => "mem",
            Op::ReadPort => "read_port",
            Op::Instance => "instance",
            Op::Name => "set_name",
            Op::Replace => "apply_replacements",
        };
        f.write_str(name)
    }
}

/// Which dependency relation a traversal was using.
///
/// Named so an error can say *which* notion of "cycle" was violated, because
/// they genuinely differ: a loop through a register is legal for loop checking
/// and is a scheduling loop for simulation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Relation {
    /// [`Deps::LoopChecking`](crate::Deps::LoopChecking).
    LoopChecking,
    /// [`Deps::SimulationScheduling`](crate::Deps::SimulationScheduling).
    SimulationScheduling,
    /// [`Deps::WithoutCaseMatches`](crate::Deps::WithoutCaseMatches).
    WithoutCaseMatches,
}

impl Relation {
    /// A short name for the relation, for diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LoopChecking => "loop-checking",
            Self::SimulationScheduling => "simulation-scheduling",
            Self::WithoutCaseMatches => "without-case-matches",
        }
    }
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Relation::LoopChecking => "combinational loop checking",
            Relation::SimulationScheduling => "simulation scheduling",
            Relation::WithoutCaseMatches => "rewriting without case matches",
        };
        f.write_str(text)
    }
}

/// A graph that could not be built or checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// A `NodeId` did not belong to this circuit.
    ///
    /// Only reachable by holding a `NodeId` across circuits, which the DSL layer
    /// never does. Kept because the cost of catching it is one bounds check and
    /// the failure mode otherwise is a panic with no context.
    UnknownNode {
        /// The offending id.
        id: NodeId,
        /// How many nodes the circuit actually holds.
        len: usize,
    },

    /// Two operands had different widths where equal widths are required.
    WidthMismatch {
        /// The operation that refused the operands.
        op: Op,
        /// Width of the left operand, in bits.
        lhs: u32,
        /// Width of the right operand, in bits.
        rhs: u32,
    },

    /// A width computation overflowed `u32`, so the result cannot be represented.
    WidthOverflow {
        /// The operation whose result width overflowed.
        op: Op,
        /// The sum of operand widths, which exceeded `u32::MAX`.
        bits: u64,
    },

    /// A signal that must be exactly one bit wide was not.
    ///
    /// Conditions, clocks, resets and clears are all 1-bit by construction, not by
    /// convention, because there is no sensible 8-bit clock.
    NotOneBit {
        /// The operation that required one bit.
        op: Op,
        /// Which operand of that operation.
        operand: &'static str,
        /// The offending signal.
        signal: NodeId,
        /// Its actual width, in bits.
        width: u32,
    },

    /// A width of zero was requested.
    ZeroWidth {
        /// The operation that requested it.
        op: Op,
    },

    /// A non-wire node was used as the target of an assignment.
    ///
    /// Hardcaml raises the same error for the same reason
    /// (`kernel/signal.ml:242-267`): `Wire` is the only node with a mutable
    /// driver, so nothing else can legally be re-driven, and silently ignoring
    /// the assignment would produce a graph that does not match the program.
    NotAWire {
        /// The node that was assigned to.
        target: NodeId,
        /// What kind of node it actually is.
        kind: &'static str,
    },

    /// A wire was driven twice.
    ///
    /// Hardcaml raises the same error (`kernel/signal.ml:242-267`). Two drivers on
    /// one wire is a short circuit in hardware, so the second assignment is
    /// refused rather than replacing the first.
    AlreadyDriven {
        /// The wire.
        wire: NodeId,
        /// The width of the wire, in bits.
        width: u32,
        /// The driver that was already attached.
        existing: NodeId,
        /// The driver that was refused.
        attempted: NodeId,
    },

    /// A wire had no driver when the circuit was checked for completeness.
    UndrivenWire {
        /// The wire.
        wire: NodeId,
        /// Its declared width, in bits.
        width: u32,
    },

    /// A slice fell outside the signal it was slicing.
    ///
    /// Distinct from [`Error::WidthMismatch`] because nothing is mismatched: the
    /// window `[offset, offset + len)` simply does not fit, and the message has to
    /// say where it ran off the end.
    SelectOutOfRange {
        /// The operation that requested it.
        op: Op,
        /// The width of the signal being sliced.
        value_width: u32,
        /// The requested low bit.
        offset: u32,
        /// The requested length.
        len: u32,
    },

    /// A signal that is not a constant was adopted as one.
    NotAConstant {
        /// The node that was expected to be a constant.
        target: NodeId,
        /// What kind of node it actually is.
        kind: &'static str,
    },

    /// A memory-only operation was applied to a node that is not a memory.
    ///
    /// Separate from [`Error::NotAWire`] because reporting a non-memory as "not a
    /// wire" names the wrong requirement: the caller wanted a memory, and telling
    /// them to create a wire sends them somewhere else entirely.
    NotAMemory {
        /// The node that was used as a memory.
        target: NodeId,
        /// What kind of node it actually is.
        kind: &'static str,
    },

    /// An instance was built with an empty name.
    ///
    /// The instance does not exist yet when this is detected, so like
    /// [`Error::CaseNoArms`] it carries nothing identifying.
    EmptyInstanceName {
        /// The operation that requested it.
        op: Op,
    },

    /// A `case` was built with no arms.
    CaseNoArms {
        /// The case node's own id is not known here — the case does not exist
        /// yet — so this carries nothing.
        op: Op,
    },

    /// A `case` arm's value width did not match the first arm's.
    CaseArmWidthMismatch {
        /// Index of the offending arm, counting from zero.
        arm: usize,
        /// Width of the first arm's value, in bits.
        lhs: u32,
        /// Width of the offending arm's value, in bits.
        rhs: u32,
    },

    /// The dependency graph contains a cycle under the stated relation.
    ///
    /// The message names the relation because the answer differs by relation: a
    /// loop through a register is a perfectly ordinary feedback path for
    /// combinational loop checking, and an error for simulation scheduling. See
    /// [`Deps`](crate::Deps).
    CombinationalLoop {
        /// The relation the cycle was found under.
        relation: Relation,
        /// The nodes on the cycle, in dependency order, starting and ending at the
        /// same node.
        cycle: Vec<NodeId>,
    },

    /// Two nodes were given the same name.
    DuplicateName {
        /// The name that collided.
        name: String,
        /// The node that already held it.
        first: NodeId,
        /// The node that tried to take it.
        second: NodeId,
    },

    /// A node's recorded name was replaced, or a name was removed from a node
    /// that does not exist.
    NameMissing {
        /// The node whose name was addressed.
        node: NodeId,
    },

    /// A replacement in [`apply_replacements`](crate::Circuit::apply_replacements)
    /// was not a wire.
    ReplacementNotAWire {
        /// The node being replaced.
        original: NodeId,
        /// The node offered as its replacement.
        replacement: NodeId,
        /// What kind of node the replacement actually is.
        kind: &'static str,
    },

    /// A replacement in [`apply_replacements`](crate::Circuit::apply_replacements)
    /// was already driven.
    ///
    /// Phase one of the three-phase rewrite creates *unattached* replacement
    /// wires and registers the mapping before rewriting anything. A replacement
    /// that already has a driver is not an unattached wire, so accepting it would
    /// make the third phase ambiguous about which driver wins.
    ReplacementDriven {
        /// The node being replaced.
        original: NodeId,
        /// The replacement wire.
        replacement: NodeId,
        /// The driver it already had.
        driver: NodeId,
    },

    /// A replacement wire's width did not match the node it replaces.
    ReplacementWidthMismatch {
        /// The node being replaced.
        original: NodeId,
        /// The replacement wire.
        replacement: NodeId,
        /// Width of the original, in bits.
        lhs: u32,
        /// Width of the replacement, in bits.
        rhs: u32,
    },

    /// A memory's address width does not cover its depth.
    MemoryAddressTooNarrow {
        /// The memory node.
        mem: NodeId,
        /// The memory's depth, in words.
        depth: u32,
        /// The address width implied by that depth, in bits.
        address_width: u32,
    },

    /// A read port's declared width did not match its memory's data width.
    ReadPortWidthMismatch {
        /// The read port node.
        port: NodeId,
        /// Width of the memory's data, in bits.
        memory: u32,
        /// Width the read port declared, in bits.
        port_width: u32,
    },

    /// An internal invariant was violated: a node's recorded width disagrees with
    /// the width implied by its operands.
    ///
    /// This is a bug in this crate or in a caller that built a node by hand, not
    /// a user error, which is why it exists as a returned error rather than a bare
    /// assertion. Every node constructor computes the derived width and stores it
    /// so that width queries are a single match, and `check` re-derives and
    /// compares.
    WidthInvariant {
        /// The node whose stored width is wrong.
        node: NodeId,
        /// The width the node recorded.
        stored: u32,
        /// The width its operands imply.
        derived: u32,
    },
}

/// The advice appended to a width mismatch, keyed by operation.
///
/// Kept as a function of the operation rather than baked into each call site so
/// that the compensation text cannot drift between the operations that share it.
const fn widen_advice(op: Op) -> &'static str {
    match op {
        Op::Add | Op::Sub => {
            "`add` and `sub` require equal operand widths and produce the left \
             operand's width, discarding whatever falls off the top. Insert an \
             explicit `replicate`, `cat` or width-changing `ite` so the intent is \
             visible in the graph."
        }
        Op::Mul => {
            "`mul` is the one arithmetic operation that does not require equal \
             widths: it widens to the sum of the operand widths and cannot \
             overflow."
        }
        Op::BitAnd | Op::BitOr | Op::BitXor => {
            "the bitwise operators require equal widths. Pad the narrower operand \
             explicitly with `cat` and a `constant` of zeros, or narrow the wider \
             one."
        }
        Op::UDiv | Op::SDiv | Op::URem | Op::SRem => {
            "division and remainder require equal operand widths and produce the \
             left operand's width. Widen or narrow explicitly first."
        }
        Op::Eq | Op::Ult | Op::Ule | Op::Ugt | Op::Uge | Op::Slt | Op::Sle | Op::Sgt | Op::Sge => {
            "comparisons require equal operand widths. Pad or narrow explicitly \
             first; which one is correct changes the answer, so it has to be \
             written down."
        }
        Op::Cat | Op::Replicate => {
            "`cat` and `replicate` have no width requirement — they widen by \
             construction. The mismatch is in one of their operands."
        }
        Op::Drive => {
            "a wire's driver must have exactly the wire's width. Pad or narrow \
             the driver explicitly rather than letting the graph paper over it."
        }
        Op::Reg => {
            "a register's data input must match the register's own width, which \
             is the width of its first construction. Pad or narrow the data input \
             explicitly."
        }
        _ => {
            "resize the operands explicitly, and make the intent visible in the \
             graph rather than relying on an implicit conversion."
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownNode { id, len } => write!(
                f,
                "{id} does not belong to this circuit, which holds {len} nodes. A \
                 `NodeId` is only meaningful for the circuit that minted it; if \
                 this surfaced from the DSL layer, a signal handle outlived the \
                 circuit it was built in."
            ),
            Error::WidthMismatch { op, lhs, rhs } => write!(
                f,
                "width mismatch in `{op}`: the left operand is {lhs} bits and the \
                 right operand is {rhs} bits. {}",
                widen_advice(*op)
            ),
            Error::WidthOverflow { op, bits } => write!(
                f,
                "`{op}` would produce a {bits}-bit result, which does not fit in \
                 the 32-bit width field. Reduce an operand width first."
            ),
            Error::NotOneBit {
                op,
                operand,
                signal,
                width,
            } => write!(
                f,
                "the {operand} of `{op}` must be exactly 1 bit, but {signal} is \
                 {width} bits wide. A clock, reset, clear or select condition has \
                 no meaning at any other width. Narrow it with an explicit \
                 selection, or index the bit you meant."
            ),
            Error::ZeroWidth { op } => write!(
                f,
                "width must be at least 1 bit, but `{op}` was asked for 0 bits. \
                 There is no zero-width value domain: use 1 bit and let the \
                 surrounding `cat` or selection discard what you do not need."
            ),
            Error::SelectOutOfRange {
                op,
                value_width,
                offset,
                len,
            } => write!(
                f,
                "`{op}` asked for bits {offset}..{} of a {value_width}-bit signal, \
                 which runs past the end. Slice within the width, or ask for fewer \
                 bits.",
                u64::from(*offset) + u64::from(*len)
            ),
            Error::NotAConstant { target, kind } => write!(
                f,
                "cannot adopt {target} as a constant, because it is a {kind} node. \
                 Only a constant has a single value that can be copied into another \
                 design; a wire or an operation does not, and copying one as zero \
                 would quietly build the wrong circuit."
            ),
            Error::NotAMemory { target, kind } => write!(
                f,
                "cannot use {target} as a memory, because it is a {kind} node. A \
                 memory is created by `Circuit::mem`, which fixes its data width and \
                 depth; reads come from separate `Circuit::read_port` nodes and \
                 writes from `Circuit::write_port` calls, so a plain wire or constant \
                 is not a substitute."
            ),
            Error::EmptyInstanceName { op } => write!(
                f,
                "an `{op}` needs a non-empty name, because the name becomes the \
                 instance identifier in the generated Verilog. Give it the name of \
                 the module you are instantiating, e.g. `my_submodule`."
            ),
            Error::NotAWire { target, kind } => write!(
                f,
                "cannot drive {target}, because it is a {kind} node rather than a \
                 wire. `Wire` is the only node with a driver, which is what makes a \
                 feedback loop expressible at all: every other node is built \
                 bottom-up over finished children and has nowhere to put a back \
                 edge. Create a wire with `Circuit::wire`, build the logic that \
                 reads it, then close the loop with `Circuit::drive`."
            ),
            Error::AlreadyDriven {
                wire,
                width,
                existing,
                attempted,
            } => write!(
                f,
                "{wire} is already driven by {existing}, so the {width}-bit driver \
                 {attempted} was refused. Two drivers on one wire is a short \
                 circuit. If you meant to build a multiplexer, use `Circuit::ite` \
                 or `Circuit::case_`; if you meant to replace the driver, detach it \
                 first with `Circuit::undrive`."
            ),
            Error::UndrivenWire { wire, width } => write!(
                f,
                "{wire} is a {width}-bit wire with no driver, so nothing defines \
                 its value. Either drive it, or delete it: an undriven wire is \
                 almost always a forgotten `<--` in a design that was only half \
                 stitched together."
            ),
            Error::CaseNoArms { op } => write!(
                f,
                "`{op}` needs at least one arm. A case with no arms has no value; \
                 if you want a constant, build a `constant` directly."
            ),
            Error::CaseArmWidthMismatch { arm, lhs, rhs } => write!(
                f,
                "case arm {arm} is {rhs} bits wide but arm 0 is {lhs} bits. Every \
                 arm and the default must have the same width, because a case \
                 selects between them rather than converting. Pad or narrow arm \
                 {arm} explicitly."
            ),
            Error::CombinationalLoop { relation, cycle } => {
                write!(f, "cycle in {relation}: ")?;
                for (i, id) in cycle.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" -> ")?;
                    }
                    write!(f, "{id}")?;
                }
                f.write_str(". ")?;
                match relation {
                    Relation::LoopChecking => f.write_str(
                        "A register or memory boundary is supposed to break a \
                         combinational loop, so a cycle here means the dependency \
                         relation was followed through a node that should have \
                         terminated it — or the graph genuinely has a \
                         zero-delay combinational loop, which cannot be given a \
                         Verilog meaning. Break it with a `reg`.",
                    ),
                    Relation::SimulationScheduling => f.write_str(
                        "Simulation evaluates nodes in dependency order, and a \
                         register breaks that order, so a cycle cannot be \
                         scheduled even though it is legal hardware. Only a \
                         genuinely combinational cycle should reach this relation; \
                         if it involves a `reg`, the loop is legal for \
                         combinational loop checking and should not be scheduled \
                         this way.",
                    ),
                    Relation::WithoutCaseMatches => f.write_str(
                        "This relation follows registers, memories and instances, \
                         so a cycle here is a true structural cycle. Break it with \
                         a `reg`.",
                    ),
                }
            }
            Error::DuplicateName {
                name,
                first,
                second,
            } => write!(
                f,
                "the name `{name}` is already used by {first}, so {second} was \
                 refused. Names appear verbatim in emitted Verilog, so two nodes \
                 sharing one would produce a module that does not compile. Rename \
                 one, or leave it unnamed and let the emitter use its node id."
            ),
            Error::NameMissing { node } => write!(
                f,
                "{node} has no name to remove. Naming is optional; only clearing an \
                 existing name is an operation that can fail."
            ),
            Error::ReplacementNotAWire {
                original,
                replacement,
                kind,
            } => write!(
                f,
                "the replacement for {original} is {replacement}, which is a {kind} \
                 node rather than a wire. Rewriting creates unattached replacement \
                 wires first and re-attaches them in a later phase; a replacement \
                 that is not a wire has nowhere to be attached."
            ),
            Error::ReplacementDriven {
                original,
                replacement,
                driver,
            } => write!(
                f,
                "the replacement wire {replacement} for {original} is already driven \
                 by {driver}. Phase one of the rewrite requires unattached wires, \
                 so that the mapping can be registered before anything is rewired; \
                 a driven replacement makes the final re-attachment ambiguous."
            ),
            Error::ReplacementWidthMismatch {
                original,
                replacement,
                lhs,
                rhs,
            } => {
                write!(
                    f,
                    "the replacement wire {replacement} for {original} is {rhs} \
                     bits wide but {original} is {lhs} bits. A replacement stands \
                     in for the original everywhere it was used, so the widths \
                     must match exactly; there is no implicit conversion between \
                     them."
                )
            }
            Error::MemoryAddressTooNarrow {
                mem,
                depth,
                address_width,
            } => write!(
                f,
                "memory {mem} has depth {depth}, which needs {address_width} \
                 address bits, but a read port was given fewer. Widen the address \
                 so every word is reachable, or reduce the depth to what the \
                 address can select."
            ),
            Error::ReadPortWidthMismatch {
                port,
                memory,
                port_width,
            } => write!(
                f,
                "read port {port} is {port_width} bits wide but its memory's data \
                 is {memory} bits. A read port returns whole words: it has no \
                 notion of a narrower field, so narrow the result with an explicit \
                 selection instead of asking the port for less."
            ),
            Error::WidthInvariant {
                node,
                stored,
                derived,
            } => write!(
                f,
                "internal error: {node} recorded width {stored} but its operands \
                 imply {derived}. This is a bug in ferrite-lithic-ir, not in the \
                 circuit being built: every node constructor derives the width \
                 from its operands and stores it so width queries are a single \
                 match, and the check re-derives it. Please report it."
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::{Error, Op, Relation};
    use crate::NodeId;

    #[test]
    fn width_mismatch_names_both_widths_and_a_fix() {
        let err = Error::WidthMismatch {
            op: Op::Add,
            lhs: 8,
            rhs: 16,
        };
        let text = err.to_string();
        assert!(text.contains("8 bits"), "{text}");
        assert!(text.contains("16 bits"), "{text}");
        assert!(text.contains("`add`"), "{text}");
        assert!(text.contains("explicit"), "{text}");
    }

    #[test]
    fn every_widenable_op_has_non_generic_advice() {
        // The generic fallback is allowed, but the ops that have a specific
        // rule must not fall through to it, because the specific text is the
        // whole reason the enum exists.
        for op in [
            Op::Add,
            Op::Sub,
            Op::Mul,
            Op::BitAnd,
            Op::UDiv,
            Op::Eq,
            Op::Cat,
        ] {
            let err = Error::WidthMismatch { op, lhs: 1, rhs: 2 };
            assert!(
                !err.to_string()
                    .contains("resize the operands explicitly, and make"),
                "{op} fell through to the generic advice"
            );
        }
    }

    #[test]
    fn already_driven_names_both_drivers() {
        let err = Error::AlreadyDriven {
            wire: NodeId::from_index(0),
            width: 8,
            existing: NodeId::from_index(1),
            attempted: NodeId::from_index(2),
        };
        let text = err.to_string();
        assert!(text.contains("already driven"), "{text}");
        assert!(text.contains("n1"), "{text}");
        assert!(text.contains("n2"), "{text}");
        assert!(text.contains("short"), "{text}");
    }

    #[test]
    fn not_a_wire_explains_the_wire_mechanism() {
        let err = Error::NotAWire {
            target: NodeId::from_index(3),
            kind: "Reg",
        };
        let text = err.to_string();
        assert!(text.contains("Reg"), "{text}");
        assert!(text.contains("back edge"), "{text}");
    }

    #[test]
    fn loop_message_names_the_relation_and_the_cycle() {
        let err = Error::CombinationalLoop {
            relation: Relation::LoopChecking,
            cycle: vec![
                NodeId::from_index(1),
                NodeId::from_index(2),
                NodeId::from_index(1),
            ],
        };
        let text = err.to_string();
        assert!(text.contains("combinational loop checking"), "{text}");
        assert!(text.contains("n1 -> n2 -> n1"), "{text}");
    }

    #[test]
    fn loop_advice_differs_by_relation() {
        let mk = |relation| {
            Error::CombinationalLoop {
                relation,
                cycle: vec![NodeId::from_index(0)],
            }
            .to_string()
        };
        let loop_checking = mk(Relation::LoopChecking);
        let scheduling = mk(Relation::SimulationScheduling);
        assert!(loop_checking.contains("zero-delay"), "{loop_checking}");
        assert!(scheduling.contains("cannot be scheduled"), "{scheduling}");
        assert_ne!(loop_checking, scheduling);
    }

    #[test]
    fn not_one_bit_names_the_operand() {
        let err = Error::NotOneBit {
            op: Op::Ite,
            operand: "condition",
            signal: NodeId::from_index(4),
            width: 8,
        };
        let text = err.to_string();
        assert!(text.contains("condition"), "{text}");
        assert!(text.contains("8 bits"), "{text}");
    }
}
