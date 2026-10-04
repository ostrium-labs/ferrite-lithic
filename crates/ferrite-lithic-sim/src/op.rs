//! The compiled instruction list.
//!
//! # An enum, not a closure, and that is the whole point
//!
//! Hardcaml compiles every node to a zero-argument OCaml closure
//! ([`src/cyclesim_compile.ml:577-783`](https://github.com/jane-street/hardcaml)).
//! We compile to an [`Op`] with word addresses and interpret it with a `match`.
//! Same execution model, no `dyn`, and — the reason this exists — **the list is
//! inspectable**, which is what makes a simulator-versus-emitted-RTL differential
//! test possible at all. You cannot compare two simulators by comparing closures;
//! you can compare two lists of these.
//!
//! # Words, not values
//!
//! Every operand is a **base word index** into one flat `Vec<u64>` buffer, not a
//! `Bits`. A signal of width `w` occupies [`words_for_width`] consecutive words,
//! so a word index plus the node's width is a complete description of where a
//! value lives. Widths are carried on the ops that need them rather than looked
//! up, so an op is self-contained and can be read on its own.
//!
//! [`words_for_width`]: ferrite_lithic_bits::words_for_width

use ferrite_lithic_bits::Bits;

/// A base index into the simulator's flat word buffer.
pub type Word = usize;

/// One combinational instruction.
///
/// Only *combinational* work appears here. The three node kinds that own storage
/// or carry no evaluation — `Constant`, `Reg`, `Mem`, and the `Wire`s that alias
/// them — do not produce an op at all:
///
/// - a `Constant` **is** a region of the consts section,
/// - a `Reg`'s output **is** its slot in the regs section,
/// - a `Mem` **is** its region of the mems section.
///
/// Their behaviour lives in [`RegUpdate`](crate::RegUpdate) and
/// [`MemWrite`](crate::MemWrite), which run at the clock edge instead of during
/// a settle pass. That split is why [`Program::ops`](crate::Program::ops) is
/// exactly the set of instructions one `comb()` runs, in an order that needs no
/// dependency check at runtime.
///
/// [`Op::ReadPort`] is here rather than with the memories because a read is
/// combinational: it is an array index, evaluated during `comb()`, holding no
/// state of its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Op {
    /// Bitwise complement.
    Not {
        /// Where the result goes.
        dst: Word,
        /// The operand.
        arg: Word,
        /// The operand's width, which is also the result's.
        width: u32,
    },
    /// Bitwise AND. Operand widths are equal.
    BitAnd {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Bitwise OR. Operand widths are equal.
    BitOr {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Bitwise XOR. Operand widths are equal.
    BitXor {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Truncating add. Operand widths are equal.
    Add {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Truncating subtract. Operand widths are equal.
    Sub {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Full-precision multiply, widening to the sum of the operand widths.
    Mul {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The left operand's width.
        left_width: u32,
        /// The right operand's width. The result is `left_width + right_width`.
        right_width: u32,
    },
    /// Unsigned divide.
    UDiv {
        /// Where the result goes.
        dst: Word,
        /// Dividend.
        left: Word,
        /// Divisor.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Signed divide. Overflow is a runtime error, not a silent wrap.
    SDiv {
        /// Where the result goes.
        dst: Word,
        /// Dividend.
        left: Word,
        /// Divisor.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Unsigned remainder.
    URem {
        /// Where the result goes.
        dst: Word,
        /// Dividend.
        left: Word,
        /// Divisor.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Signed remainder, which cannot overflow.
    SRem {
        /// Where the result goes.
        dst: Word,
        /// Dividend.
        left: Word,
        /// Divisor.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Equality. One bit out.
    Eq {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Unsigned less-than. One bit out.
    Ult {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Unsigned less-or-equal. One bit out.
    Ule {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Unsigned greater-than. One bit out.
    Ugt {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Unsigned greater-or-equal. One bit out.
    Uge {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Signed less-than. One bit out.
    Slt {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Signed less-or-equal. One bit out.
    Sle {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Signed greater-than. One bit out.
    Sgt {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// Signed greater-or-equal. One bit out.
    Sge {
        /// Where the result goes.
        dst: Word,
        /// Left operand.
        left: Word,
        /// Right operand.
        right: Word,
        /// The common operand width.
        width: u32,
    },
    /// A contiguous run of bits. The result is `len` bits at bit zero.
    Select {
        /// Where the result goes.
        dst: Word,
        /// The signal being sliced.
        value: Word,
        /// Index of the lowest bit taken.
        offset: u32,
        /// How many bits taken. This is the result's width.
        len: u32,
    },
    /// Concatenation, high part first.
    Cat {
        /// Where the result goes.
        dst: Word,
        /// The high part.
        high: Word,
        /// The low part.
        low: Word,
        /// The high part's width. The result is `high_width + low_width`.
        high_width: u32,
        /// The low part's width.
        low_width: u32,
    },
    /// One value repeated.
    Replicate {
        /// Where the result goes.
        dst: Word,
        /// The value to repeat.
        value: Word,
        /// How many copies.
        count: u32,
        /// The value's width. The result is `value_width * count`.
        value_width: u32,
    },
    /// Two-way select.
    Ite {
        /// Where the result goes.
        dst: Word,
        /// The one-bit condition.
        condition: Word,
        /// Value when the condition is high.
        then_value: Word,
        /// Value when the condition is low.
        otherwise: Word,
        /// The result's width.
        width: u32,
    },
    /// Multi-way select against literal match values, first match wins.
    ///
    /// The match values are literals rather than words, which is why an arm is a
    /// `Bits` and a destination rather than two word indices. Arm order *is* the
    /// priority order, so this is evaluated front to back and stops at the first
    /// hit; the compiler does not need to know which arms overlap.
    Case {
        /// Where the result goes.
        dst: Word,
        /// The value being matched.
        scrutinee: Word,
        /// The scrutinee's width, which is the width an arm's match values are
        /// compared at. Distinct from the result width below.
        scrutinee_width: u32,
        /// The arms, in priority order: match values and the value they select.
        arms: Vec<(Vec<Bits>, Word)>,
        /// The value when no arm matches.
        default: Word,
        /// The result's width.
        width: u32,
    },
    /// An asynchronous read from a memory's word array.
    ///
    /// The memory's own address is a *base word*, not a region pointer, so an
    /// out-of-range read is bounded here rather than trusted.
    ReadPort {
        /// Where the result goes.
        dst: Word,
        /// Base word of the memory's word array.
        mem: Word,
        /// How many words the memory has.
        depth: u32,
        /// The word address to read.
        address: Word,
        /// The address's width.
        address_width: u32,
        /// The one-bit read enable. Low forces the data output to zero.
        enable: Word,
        /// The memory's word width.
        data_width: u32,
    },
}

impl Op {
    /// Where this op writes its result.
    #[must_use]
    pub fn destination(&self) -> Word {
        match *self {
            Op::Not { dst, .. }
            | Op::BitAnd { dst, .. }
            | Op::BitOr { dst, .. }
            | Op::BitXor { dst, .. }
            | Op::Add { dst, .. }
            | Op::Sub { dst, .. }
            | Op::Mul { dst, .. }
            | Op::UDiv { dst, .. }
            | Op::SDiv { dst, .. }
            | Op::URem { dst, .. }
            | Op::SRem { dst, .. }
            | Op::Eq { dst, .. }
            | Op::Ult { dst, .. }
            | Op::Ule { dst, .. }
            | Op::Ugt { dst, .. }
            | Op::Uge { dst, .. }
            | Op::Slt { dst, .. }
            | Op::Sle { dst, .. }
            | Op::Sgt { dst, .. }
            | Op::Sge { dst, .. }
            | Op::Select { dst, .. }
            | Op::Cat { dst, .. }
            | Op::Replicate { dst, .. }
            | Op::Ite { dst, .. }
            | Op::Case { dst, .. }
            | Op::ReadPort { dst, .. } => dst,
        }
    }

    /// A short name for the op, matching the IR node it came from.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Op::Not { .. } => "Not",
            Op::BitAnd { .. } => "BitAnd",
            Op::BitOr { .. } => "BitOr",
            Op::BitXor { .. } => "BitXor",
            Op::Add { .. } => "Add",
            Op::Sub { .. } => "Sub",
            Op::Mul { .. } => "Mul",
            Op::UDiv { .. } => "UDiv",
            Op::SDiv { .. } => "SDiv",
            Op::URem { .. } => "URem",
            Op::SRem { .. } => "SRem",
            Op::Eq { .. } => "Eq",
            Op::Ult { .. } => "Ult",
            Op::Ule { .. } => "Ule",
            Op::Ugt { .. } => "Ugt",
            Op::Uge { .. } => "Uge",
            Op::Slt { .. } => "Slt",
            Op::Sle { .. } => "Sle",
            Op::Sgt { .. } => "Sgt",
            Op::Sge { .. } => "Sge",
            Op::Select { .. } => "Select",
            Op::Cat { .. } => "Cat",
            Op::Replicate { .. } => "Replicate",
            Op::Ite { .. } => "Ite",
            Op::Case { .. } => "Case",
            Op::ReadPort { .. } => "ReadPort",
        }
    }
}
