//! Errors produced by [`Bits`](crate::Bits) operations.
//!
//! # Why this type is so verbose
//!
//! Widths are runtime values ([ADR-0003](../docs/adr/0003-runtime-widths-not-const-generics.md)),
//! so a width mismatch is a *runtime* event rather than a compile error. In a
//! hardware DSL that trade is worth making — graph construction is loops and
//! generators, and const generics make those painful — but it means the error
//! message has to carry the compensation the compiler would otherwise have
//! provided.
//!
//! Every width mismatch message therefore names **both** operand widths and names
//! the explicit widening operations that would fix it. This is not decoration:
//! silently truncating a result is the single most common source of hardware
//! bugs, and a bare "width mismatch" is not enough to debug it.
//!
//! [`Error`] deliberately carries no source location. This crate has no notion of
//! source text; the DSL layer in `ferrite-lithic` attaches a `Location` when it
//! propagates a bits error out of a user expression, where the location is known.

use core::fmt;

/// The operation that failed.
///
/// Carried in every error so a message can name what was attempted. Without it a
/// width mismatch between an 8-bit and a 16-bit value is ambiguous: the same pair
/// of widths is an error for `add` and perfectly fine for `zero_extend`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Op {
    /// [`Bits::zeros`](crate::Bits::zeros), [`Bits::ones`](crate::Bits::ones),
    /// [`Bits::constant`](crate::Bits::constant) or
    /// [`Bits::from_bytes_le`](crate::Bits::from_bytes_le).
    Construct,
    /// [`Bits::zero_extend`](crate::Bits::zero_extend).
    ZeroExtend,
    /// [`Bits::sign_extend`](crate::Bits::sign_extend).
    SignExtend,
    /// [`Bits::truncate`](crate::Bits::truncate).
    Truncate,
    /// [`Bits::add`](crate::Bits::add).
    Add,
    /// [`Bits::sub`](crate::Bits::sub).
    Sub,
    /// [`Bits::neg`](crate::Bits::neg).
    Neg,
    /// [`Bits::mul`](crate::Bits::mul).
    Mul,
    /// [`Bits::udiv`](crate::Bits::udiv).
    UDiv,
    /// [`Bits::sdiv`](crate::Bits::sdiv).
    SDiv,
    /// [`Bits::urem`](crate::Bits::urem).
    URem,
    /// [`Bits::srem`](crate::Bits::srem).
    SRem,
    /// [`Bits::not`](crate::Bits::not).
    Not,
    /// [`Bits::and`](crate::Bits::and).
    And,
    /// [`Bits::or`](crate::Bits::or).
    Or,
    /// [`Bits::xor`](crate::Bits::xor).
    Xor,
    /// [`Bits::eq`](crate::Bits::eq).
    Eq,
    /// [`Bits::ult`](crate::Bits::ult).
    Ult,
    /// [`Bits::ule`](crate::Bits::ule).
    Ule,
    /// [`Bits::ugt`](crate::Bits::ugt).
    Ugt,
    /// [`Bits::uge`](crate::Bits::uge).
    Uge,
    /// [`Bits::slt`](crate::Bits::slt).
    Slt,
    /// [`Bits::sle`](crate::Bits::sle).
    Sle,
    /// [`Bits::sgt`](crate::Bits::sgt).
    Sgt,
    /// [`Bits::sge`](crate::Bits::sge).
    Sge,
    /// [`Bits::select`](crate::Bits::select).
    Select,
    /// [`cat`](crate::cat) or [`Bits::replicate`](crate::Bits::replicate).
    Cat,
    /// [`Bits::sll`](crate::Bits::sll).
    Sll,
    /// [`Bits::srl`](crate::Bits::srl).
    Srl,
    /// [`Bits::sra`](crate::Bits::sra).
    Sra,
    /// [`Bits::bit`](crate::Bits::bit).
    Bit,
    /// [`Bits::to_u64`](crate::Bits::to_u64).
    ToU64,
}

impl Op {
    /// The operation's name as written in the DSL, for use in messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Op::Construct => "construct",
            Op::ZeroExtend => "zero_extend",
            Op::SignExtend => "sign_extend",
            Op::Truncate => "truncate",
            Op::Add => "add",
            Op::Sub => "sub",
            Op::Neg => "neg",
            Op::Mul => "mul",
            Op::UDiv => "udiv",
            Op::SDiv => "sdiv",
            Op::URem => "urem",
            Op::SRem => "srem",
            Op::Not => "not",
            Op::And => "and",
            Op::Or => "or",
            Op::Xor => "xor",
            Op::Eq => "eq",
            Op::Ult => "ult",
            Op::Ule => "ule",
            Op::Ugt => "ugt",
            Op::Uge => "uge",
            Op::Slt => "slt",
            Op::Sle => "sle",
            Op::Sgt => "sgt",
            Op::Sge => "sge",
            Op::Select => "select",
            Op::Cat => "cat",
            Op::Sll => "sll",
            Op::Srl => "srl",
            Op::Sra => "sra",
            Op::Bit => "bit",
            Op::ToU64 => "to_u64",
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A failed [`Bits`](crate::Bits) operation.
///
/// Every variant names the widths involved, and every variant that has a
/// workaround names it too. The reasoning is in the crate-level documentation
/// under "Why this type is so verbose".
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// A width of zero was requested.
    ///
    /// A zero-width `Bits` has no value domain, so it is refused at
    /// construction rather than propagated.
    ZeroWidth {
        /// The operation that requested it.
        op: Op,
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

    /// A selection ran past the end of the value.
    SelectOutOfRange {
        /// Width of the value being selected from, in bits.
        width: u32,
        /// Requested offset, in bits from the least significant end.
        offset: u32,
        /// Requested length, in bits.
        len: u32,
    },

    /// A selection of zero bits was requested.
    SelectEmpty {
        /// Width of the value being selected from, in bits.
        width: u32,
    },

    /// `cat` was called with no parts.
    CatEmpty,

    /// Division or remainder by zero.
    ///
    /// Hardware has no defined result here, so this is an error rather than a
    /// saturating or wrapping convention.
    DivisionByZero {
        /// The division or remainder operation.
        op: Op,
        /// Width of the operands, in bits.
        width: u32,
    },

    /// Signed division or remainder overflowed.
    ///
    /// This is the `-2^(w-1) / -1` case, whose exact quotient is `2^(w-1)` and so
    /// is one bit wider than the operands. Verilog returns `-2^(w-1)` here; we
    /// refuse instead, because silently returning the wrapped value is exactly
    /// the class of bug this crate exists to prevent.
    SignedDivisionOverflow {
        /// The signed division or remainder operation.
        op: Op,
        /// Width of the operands, in bits.
        width: u32,
    },

    /// A conversion was asked to represent a value wider than its target.
    TooWide {
        /// The conversion operation.
        op: Op,
        /// Width of the value being converted, in bits.
        width: u32,
        /// Width of the target type, in bits.
        limit: u32,
    },

    /// A bit index was past the end of the value.
    BitIndexOutOfRange {
        /// Width of the value, in bits.
        width: u32,
        /// Requested bit index.
        index: u32,
    },

    /// A byte buffer had the wrong length for the requested width.
    BufferLength {
        /// The conversion operation.
        op: Op,
        /// The exact length the width requires, in bytes.
        expected: usize,
        /// The length that was supplied, in bytes.
        got: usize,
    },
}

/// The advice appended to a width mismatch, keyed by operation.
///
/// Kept as a function of the operation rather than baked into each call site so
/// that the compensation text cannot drift between the operations that share it.
const fn widen_advice(op: Op) -> &'static str {
    match op {
        Op::Add | Op::Sub => {
            "`add` and `sub` require equal widths and return the left operand's \
             width, discarding whatever falls off the top. Resize both operands \
             first with `Bits::zero_extend` or `Bits::sign_extend`, or narrow the \
             wider one with `Bits::truncate`."
        }
        Op::Mul => {
            "`mul` is the one arithmetic operation that does not require equal \
             widths: it widens to the sum of the operand widths and cannot \
             overflow."
        }
        Op::And | Op::Or | Op::Xor => {
            "the bitwise operators require equal widths. Resize with \
             `Bits::zero_extend`, `Bits::sign_extend` or `Bits::truncate` first."
        }
        Op::UDiv | Op::SDiv | Op::URem | Op::SRem => {
            "division and remainder require equal widths and return the left \
             operand's width. Resize with `Bits::zero_extend`, `Bits::sign_extend` \
             or `Bits::truncate` first."
        }
        Op::Not => {
            "`not` requires both operands to have the width being negated. \
             Widen the operand with `Bits::zero_extend` or `Bits::sign_extend`."
        }
        Op::Eq | Op::Ult | Op::Ule | Op::Ugt | Op::Uge | Op::Slt | Op::Sle | Op::Sgt | Op::Sge => {
            "comparisons require equal widths. Resize both operands with \
             `Bits::zero_extend`, `Bits::sign_extend` or `Bits::truncate` first; \
             which one is correct changes the answer, so it has to be explicit."
        }
        _ => {
            "resize the operands explicitly with `Bits::zero_extend`, \
             `Bits::sign_extend` or `Bits::truncate`."
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ZeroWidth { op } => write!(
                f,
                "width must be at least 1 bit, but `{op}` was asked for 0 bits. \
                 There is no zero-width value domain: use 1 bit and let `select` \
                 discard the bits you do not need."
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
                 the 32-bit width field. Reduce an operand width before the \
                 operation."
            ),
            Error::SelectOutOfRange { width, offset, len } => write!(
                f,
                "`select` of {len} bits at offset {offset} is out of range for a \
                 value {width} bits wide: {offset} + {len} = {} exceeds {width}. \
                 Narrow the selection, or widen the value with \
                 `Bits::zero_extend` or `Bits::sign_extend` first.",
                *offset as u64 + *len as u64
            ),
            Error::SelectEmpty { width } => write!(
                f,
                "`select` of 0 bits was requested from a value {width} bits wide, \
                 but a zero-width value is not representable. Select at least 1 \
                 bit, or use `Bits::zeros` for a constant."
            ),
            Error::CatEmpty => f.write_str(
                "`cat` was called with no parts. Concatenate at least one value; \
                 use `Bits::zeros` for a constant of the width you want.",
            ),
            Error::DivisionByZero { op, width } => write!(
                f,
                "division by zero in `{op}` on a value {width} bits wide. \
                 Hardware has no defined result, so this is refused rather than \
                 wrapped. Guard the divisor with a predicate, or — if the divisor \
                 is a compile time constant — replace the operation with a shift \
                 and mask."
            ),
            Error::SignedDivisionOverflow { op, width } => write!(
                f,
                "signed overflow in `{op}` on a value {width} bits wide: the most \
                 negative value divided by -1 needs {} bits, not {width}. Widen \
                 both operands by one bit with `Bits::zero_extend` before \
                 dividing, or special-case the input.",
                width + 1
            ),
            Error::TooWide { op, width, limit } => write!(
                f,
                "a value {width} bits wide does not fit in {limit} bits in `{op}`. \
                 Narrow it with `Bits::truncate`, or read the words directly with \
                 `Bits::word`."
            ),
            Error::BitIndexOutOfRange { width, index } => write!(
                f,
                "bit {index} is out of range for a value {width} bits wide; valid \
                 indices are 0 to {}.",
                width - 1
            ),
            Error::BufferLength { op, expected, got } => write!(
                f,
                "`{op}` needs a buffer of exactly {expected} bytes for this width, \
                 but {got} bytes were supplied. The byte count is fully determined \
                 by the width, so a mismatched buffer is a bug rather than \
                 something to pad or truncate."
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    // A failing unit test should panic with the offending values visible, so
    // `unwrap` is the right tool here rather than a lint exemption elsewhere.
    #![allow(clippy::unwrap_used)]
    use super::{Error, Op};

    #[test]
    fn op_names_round_trip_through_display() {
        let all = [
            Op::Construct,
            Op::ZeroExtend,
            Op::SignExtend,
            Op::Truncate,
            Op::Add,
            Op::Sub,
            Op::Neg,
            Op::Mul,
            Op::UDiv,
            Op::SDiv,
            Op::URem,
            Op::SRem,
            Op::Not,
            Op::And,
            Op::Or,
            Op::Xor,
            Op::Eq,
            Op::Ult,
            Op::Ule,
            Op::Ugt,
            Op::Uge,
            Op::Slt,
            Op::Sle,
            Op::Sgt,
            Op::Sge,
            Op::Select,
            Op::Cat,
            Op::Sll,
            Op::Srl,
            Op::Sra,
            Op::Bit,
            Op::ToU64,
        ];
        for op in all {
            assert_eq!(op.to_string(), op.name());
        }
    }

    #[test]
    fn width_mismatch_names_both_widths_and_a_fix() {
        let msg = Error::WidthMismatch {
            op: Op::Add,
            lhs: 8,
            rhs: 16,
        }
        .to_string();
        assert!(msg.contains("add"), "{msg}");
        assert!(msg.contains("8 bits"), "{msg}");
        assert!(msg.contains("16 bits"), "{msg}");
        assert!(msg.contains("zero_extend"), "{msg}");
        assert!(msg.contains("truncate"), "{msg}");
    }

    #[test]
    fn mul_width_mismatch_is_impossible_so_its_advice_says_so() {
        let msg = Error::WidthMismatch {
            op: Op::Mul,
            lhs: 8,
            rhs: 16,
        }
        .to_string();
        assert!(msg.contains("does not require equal widths"), "{msg}");
    }

    #[test]
    fn signed_division_overflow_advice_mentions_widening_by_one_bit() {
        let msg = Error::SignedDivisionOverflow {
            op: Op::SDiv,
            width: 8,
        }
        .to_string();
        assert!(msg.contains('9'), "{msg}");
        assert!(msg.contains("zero_extend"), "{msg}");
    }
}
