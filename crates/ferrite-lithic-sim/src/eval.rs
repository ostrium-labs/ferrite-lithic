//! Evaluating the op list against a flat word buffer.
//!
//! # Two paths, on purpose
//!
//! Every operation has a hand-written `u64` path for values that fit in one word
//! and a fallback that goes through [`Bits`]. That is not a premature
//! optimisation and it is not a rewrite-avoidance dodge — it is the one place both
//! matter at once:
//!
//! - Values of 64 bits or fewer are nearly all of them, and a `Bits` operation
//!   allocates. A settle pass over a 10k-node design that allocated per gate
//!   would be unusable, and "allocate per gate" is the kind of decision you only
//!   notice after building the thing.
//! - Wide values need multi-word arithmetic that is already written, already
//!   property-tested and already agreed on by the rest of the stack. Duplicating
//!   add-with-carry and schoolbook multiply here to avoid one `Vec` allocation
//!   would mean two implementations of the same semantics, which is exactly the
//!   failure mode the design notes warn about when they say Hardcaml keeps three
//!   in sync and we keep one.
//!
//! The property test over widths 1..=200 exercises both paths against `Bits`, so
//! the fast path is not trusted because it is fast.

use ferrite_lithic_bits::{Bits, Error as BitsError, WORD_BITS, mask_for_width, words_for_width};

use crate::compile::{MemWrite, Program};
use crate::error::Error;
use crate::op::{Op, Word};

/// The flat word buffer and the program that indexes it.
#[derive(Clone, Debug)]
pub(crate) struct Machine {
    buffer: Vec<u64>,
    /// Offset from a register's live slot to its shadow slot.
    shadow_offset: Word,
    /// Length of the regs section in words, which is what the blit copies.
    reg_words: usize,
}

impl Machine {
    /// A zero-filled buffer with the constant section already written.
    ///
    /// Registers and memories want zeroes, which is what a fresh `Vec` gives, and
    /// is Hardcaml's rule too
    /// ([`src/cyclesim_compile.ml:492`](https://github.com/jane-street/hardcaml)).
    /// Constants want their own values, so they are written here once: a literal
    /// *is* a region of the consts section, which is why it compiles to no
    /// instruction and why otherwise nothing would ever fill it in.
    pub(crate) fn new(program: &Program) -> Self {
        let regs = &program.sections().regs;
        let mut machine = Machine {
            buffer: vec![0; program.buffer_words()],
            shadow_offset: regs.len(),
            reg_words: regs.len(),
        };
        for id in program.circuit().node_ids() {
            if let ferrite_lithic_ir::Node::Constant { value } = program
                .circuit()
                .get(id)
                .expect("a node id from the same circuit")
                && let Some(word) = program.word_of(id)
            {
                machine.write(word, value);
            }
        }
        machine
    }

    pub(crate) fn buffer(&self) -> &[u64] {
        &self.buffer
    }

    /// Reads a node's value out of the buffer.
    pub(crate) fn read(&self, word: Word, width: u32) -> Bits {
        let n = words_for_width(width);
        // The buffer was sized from the same widths, so this cannot be short; a
        // slice that panics on a compiler bug beats a `Result` threaded through
        // every op for something the type system cannot express.
        Bits::from_word_slice(width, &self.buffer[word..word + n])
            .expect("the buffer was sized from this width")
    }

    /// Writes a value into the buffer, masking the top word.
    pub(crate) fn write(&mut self, word: Word, value: &Bits) {
        let dst = &mut self.buffer[word..word + value.word_count()];
        dst.copy_from_slice(value.words());
        if let Some(top) = dst.last_mut() {
            *top &= mask_for_width(value.width());
        }
    }

    /// Writes a single word, masking to `width`.
    pub(crate) fn write_word(&mut self, word: Word, value: u64, width: u32) {
        self.buffer[word] = value & mask_for_width(width);
    }

    /// Writes zero across `width` bits starting at `word`.
    pub(crate) fn write_zero(&mut self, word: Word, width: u32) {
        let n = words_for_width(width);
        for w in &mut self.buffer[word..word + n] {
            *w = 0;
        }
    }

    /// Copies `width` bits from one word range to another.
    ///
    /// Overlapping ranges are copied forwards, which is correct for the
    /// register blit: the shadow section is a distinct region, so no overlap is
    /// possible there, and a forward copy is the right answer for the general case
    /// anyway.
    pub(crate) fn copy(&mut self, from: Word, to: Word, width: u32) {
        let n = words_for_width(width);
        self.buffer.copy_within(from..from + n, to);
        if let Some(top) = self.buffer.get_mut(to + n - 1) {
            *top &= mask_for_width(width);
        }
    }

    /// One full settle pass: every op, in the order the compiler fixed.
    pub(crate) fn comb(&mut self, program: &Program) -> Result<(), Error> {
        for op in program.ops() {
            self.eval(op)?;
        }
        Ok(())
    }

    /// Evaluates one op.
    pub(crate) fn eval(&mut self, op: &Op) -> Result<(), Error> {
        match *op {
            Op::Not { dst, arg, width } => {
                if one_word(width) {
                    self.write_word(dst, !self.buffer[arg], width);
                } else {
                    let value = self.read(arg, width).not()?;
                    self.write(dst, &value);
                }
            }
            Op::BitAnd {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    self.write_word(dst, self.buffer[left] & self.buffer[right], width);
                } else {
                    let value = self.read(left, width).and(&self.read(right, width))?;
                    self.write(dst, &value);
                }
            }
            Op::BitOr {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    self.write_word(dst, self.buffer[left] | self.buffer[right], width);
                } else {
                    let value = self.read(left, width).or(&self.read(right, width))?;
                    self.write(dst, &value);
                }
            }
            Op::BitXor {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    self.write_word(dst, self.buffer[left] ^ self.buffer[right], width);
                } else {
                    let value = self.read(left, width).xor(&self.read(right, width))?;
                    self.write(dst, &value);
                }
            }
            Op::Add {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    // `wrapping_add` rather than `+`: a truncated add must wrap,
                    // and in release a `+` overflow panic would turn a legal
                    // circuit into a crash.
                    self.write_word(
                        dst,
                        self.buffer[left].wrapping_add(self.buffer[right]),
                        width,
                    );
                } else {
                    let value = self.read(left, width).add(&self.read(right, width))?;
                    self.write(dst, &value);
                }
            }
            Op::Sub {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    self.write_word(
                        dst,
                        self.buffer[left].wrapping_sub(self.buffer[right]),
                        width,
                    );
                } else {
                    let value = self.read(left, width).sub(&self.read(right, width))?;
                    self.write(dst, &value);
                }
            }
            Op::Mul {
                dst,
                left,
                right,
                left_width,
                right_width,
            } => {
                let width = left_width + right_width;
                if words_for_width(width) == 1 {
                    // The result fits in a word, so the low bits of a wrapping
                    // multiply are exactly the answer; masking then discards the
                    // bits above `width`.
                    let value = self.buffer[left].wrapping_mul(self.buffer[right]);
                    self.write_word(dst, value, width);
                } else {
                    let value = self
                        .read(left, left_width)
                        .mul(&self.read(right, right_width))?;
                    self.write(dst, &value);
                }
            }
            Op::UDiv {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    let divisor = self.buffer[right];
                    if divisor == 0 {
                        return Err(Error::DivisionByZero {
                            op: "unsigned division",
                        });
                    }
                    self.write_word(dst, self.buffer[left] / divisor, width);
                } else {
                    let divisor = self.read(right, width);
                    let value = self
                        .read(left, width)
                        .udiv(&divisor)
                        .map_err(|e| division_error(e, "unsigned division", width))?;
                    self.write(dst, &value);
                }
            }
            Op::URem {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    let divisor = self.buffer[right];
                    if divisor == 0 {
                        return Err(Error::DivisionByZero {
                            op: "unsigned remainder",
                        });
                    }
                    self.write_word(dst, self.buffer[left] % divisor, width);
                } else {
                    let divisor = self.read(right, width);
                    let value = self
                        .read(left, width)
                        .urem(&divisor)
                        .map_err(|e| division_error(e, "unsigned remainder", width))?;
                    self.write(dst, &value);
                }
            }
            Op::SDiv {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    let lhs = signed(self.buffer[left], width);
                    let rhs = signed(self.buffer[right], width);
                    let value = signed_div(lhs, rhs, width)?;
                    self.write_word(dst, value as u64, width);
                } else {
                    let divisor = self.read(right, width);
                    let value = self
                        .read(left, width)
                        .sdiv(&divisor)
                        .map_err(|e| division_error(e, "signed division", width))?;
                    self.write(dst, &value);
                }
            }
            Op::SRem {
                dst,
                left,
                right,
                width,
            } => {
                if one_word(width) {
                    let lhs = signed(self.buffer[left], width);
                    let rhs = signed(self.buffer[right], width);
                    if rhs == 0 {
                        return Err(Error::DivisionByZero {
                            op: "signed remainder",
                        });
                    }
                    // `MIN % -1` is the one division-shaped operation that cannot
                    // overflow, so it answers zero rather than erroring.
                    let value = if lhs == signed_min(width) && rhs == -1 {
                        0
                    } else {
                        lhs % rhs
                    };
                    self.write_word(dst, value as u64, width);
                } else {
                    let divisor = self.read(right, width);
                    let value = self
                        .read(left, width)
                        .srem(&divisor)
                        .map_err(|e| division_error(e, "signed remainder", width))?;
                    self.write(dst, &value);
                }
            }
            Op::Eq {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    self.buffer[left] == self.buffer[right]
                } else {
                    self.read(left, width).eq(&self.read(right, width))?
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Ult {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    self.buffer[left] < self.buffer[right]
                } else {
                    self.read(left, width).cmp(&self.read(right, width))
                        == core::cmp::Ordering::Less
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Ule {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    self.buffer[left] <= self.buffer[right]
                } else {
                    self.read(left, width).cmp(&self.read(right, width))
                        != core::cmp::Ordering::Greater
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Ugt {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    self.buffer[left] > self.buffer[right]
                } else {
                    self.read(left, width).cmp(&self.read(right, width))
                        == core::cmp::Ordering::Greater
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Uge {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    self.buffer[left] >= self.buffer[right]
                } else {
                    self.read(left, width).cmp(&self.read(right, width))
                        != core::cmp::Ordering::Less
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Slt {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    signed(self.buffer[left], width) < signed(self.buffer[right], width)
                } else {
                    signed_lt(&self.read(left, width), &self.read(right, width))?
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Sle {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    signed(self.buffer[left], width) <= signed(self.buffer[right], width)
                } else {
                    !signed_lt(&self.read(right, width), &self.read(left, width))?
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Sgt {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    signed(self.buffer[left], width) > signed(self.buffer[right], width)
                } else {
                    signed_lt(&self.read(right, width), &self.read(left, width))?
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Sge {
                dst,
                left,
                right,
                width,
            } => {
                let bit = if one_word(width) {
                    signed(self.buffer[left], width) >= signed(self.buffer[right], width)
                } else {
                    !signed_lt(&self.read(left, width), &self.read(right, width))?
                };
                self.write_word(dst, u64::from(bit), 1);
            }
            Op::Select {
                dst,
                value,
                offset,
                len,
            } => {
                // The window must lie inside the *first* word, not merely be one
                // word wide: a 66-bit value sliced at 47 for 18 bits crosses the
                // boundary, and reading only word zero would drop the top bit.
                if offset + len <= WORD_BITS {
                    self.write_word(
                        dst,
                        (self.buffer[value] >> offset) & mask_for_width(len),
                        len,
                    );
                } else {
                    // No source width is needed: the bits wanted are
                    // `[offset, offset + len)`, so reading a range of exactly that
                    // width from the same base word yields them, whatever the
                    // signal's full width was.
                    let sliced = self.read(value, offset + len).select(offset, len)?;
                    self.write(dst, &sliced);
                }
            }
            Op::Cat {
                dst,
                high,
                low,
                high_width,
                low_width,
            } => {
                let width = high_width + low_width;
                if words_for_width(width) == 1 {
                    let value = (self.buffer[high] << low_width) | self.buffer[low];
                    self.write_word(dst, value, width);
                } else {
                    let parts = [self.read(high, high_width), self.read(low, low_width)];
                    let value = ferrite_lithic_bits::cat(&[&parts[0], &parts[1]])?;
                    self.write(dst, &value);
                }
            }
            Op::Replicate {
                dst,
                value,
                count,
                value_width,
            } => {
                let width = value_width * count;
                if words_for_width(width) == 1 {
                    let mut out = 0u64;
                    let mut shift = 0u32;
                    let one = self.buffer[value] & mask_for_width(value_width);
                    for _ in 0..count {
                        out |= one.wrapping_shl(shift);
                        shift += value_width;
                    }
                    self.write_word(dst, out, width);
                } else {
                    let value = self.read(value, value_width).replicate(count)?;
                    self.write(dst, &value);
                }
            }
            Op::Ite {
                dst,
                condition,
                then_value,
                otherwise,
                width,
            } => {
                let source = if self.buffer[condition] & 1 == 1 {
                    then_value
                } else {
                    otherwise
                };
                if one_word(width) {
                    self.write_word(dst, self.buffer[source], width);
                } else {
                    let value = self.read(source, width);
                    self.write(dst, &value);
                }
            }
            // `ref` on the arms and nowhere else: they are the one field in the
            // whole enum that owns a `Vec`, so every other field binds by value.
            Op::Case {
                dst,
                scrutinee,
                scrutinee_width,
                ref arms,
                default,
                width,
            } => {
                let mut source = default;
                if one_word(scrutinee_width) {
                    let value = self.buffer[scrutinee];
                    for arm in arms.iter() {
                        let hit = arm
                            .0
                            .iter()
                            .any(|m| m.width() == scrutinee_width && m.to_u64() == Ok(value));
                        if hit {
                            source = arm.1;
                            break;
                        }
                    }
                } else {
                    let value = self.read(scrutinee, scrutinee_width);
                    for arm in arms.iter() {
                        if arm.0.contains(&value) {
                            source = arm.1;
                            break;
                        }
                    }
                }
                if one_word(width) {
                    self.write_word(dst, self.buffer[source], width);
                } else {
                    let chosen = self.read(source, width);
                    self.write(dst, &chosen);
                }
            }
            Op::ReadPort {
                dst,
                mem,
                depth,
                address,
                enable,
                data_width,
                ..
            } => {
                if self.buffer[enable] & 1 == 0 {
                    self.write_zero(dst, data_width);
                } else {
                    let addr = self.buffer[address] as u32;
                    if u64::from(addr) < u64::from(depth) {
                        let n = words_for_width(data_width);
                        let base = mem + addr as usize * n;
                        self.copy(base, dst, data_width);
                    } else {
                        // An address past the end reads zero. There is no `x` in a
                        // two-state simulator and no trap in hardware, so zero is
                        // the only answer that is neither a lie nor a failure; the
                        // alternative is an error, which would make every design
                        // with a reachable invalid address unusable.
                        self.write_zero(dst, data_width);
                    }
                }
            }
        }
        Ok(())
    }

    /// Writes every register's next value into the shadow section.
    ///
    /// Reads `current` and writes `regs_next`, so every decision is made against
    /// one consistent pre-edge state. Writing in place would make a chain of
    /// registers shift by one cycle per register, which is the bug the shadow
    /// section exists to prevent.
    pub(crate) fn reg_update(&mut self, program: &Program) -> Result<(), Error> {
        let shadow_offset = self.shadow_offset;
        for reg in program.reg_updates() {
            if self.buffer[reg.clock] & 1 == 0 {
                continue;
            }
            let next = if self.buffer[reg.reset] & 1 == 1 {
                Bits::zeros(reg.width)?
            } else if self.buffer[reg.clear] & 1 == 1 {
                self.read(reg.current, reg.width)
            } else {
                self.read(reg.data, reg.width)
            };
            self.write(reg.current + shadow_offset, &next);
        }
        Ok(())
    }

    /// Commits the shadow section over the live one.
    pub(crate) fn blit(&mut self, program: &Program) {
        let shadow_offset = self.shadow_offset;
        let regs = &program.sections().regs;
        let len = self.reg_words;
        self.buffer.copy_within(
            regs.start + shadow_offset..regs.start + shadow_offset + len,
            regs.start,
        );
    }

    /// Applies every enabled memory write.
    pub(crate) fn mem_update(&mut self, writes: &[MemWrite]) {
        for write in writes {
            if self.buffer[write.enable] & 1 == 0 {
                continue;
            }
            let addr = self.buffer[write.address] as u32;
            if u64::from(addr) >= u64::from(write.depth) {
                continue;
            }
            let n = words_for_width(write.data_width);
            let base = write.mem + addr as usize * n;
            let value = self.read(write.data, write.data_width);
            self.write(base, &value);
        }
    }
}

/// Whether a value of `width` bits occupies a single word.
fn one_word(width: u32) -> bool {
    words_for_width(width) == 1
}

/// Reinterprets a stored word as a signed value of `width` bits.
///
/// The buffer stores every value zero-extended, so `as i64` on a narrow word
/// would give the *unsigned* reading. Sign-extending from the declared width
/// first is what makes `4'd15` compare as `-1`.
fn signed(word: u64, width: u32) -> i64 {
    if width >= WORD_BITS {
        word as i64
    } else {
        let shift = WORD_BITS - width;
        ((word << shift) as i64) >> shift
    }
}

/// The most negative value a `width`-bit two's-complement integer can hold.
///
/// `i64::MIN` is only the answer at width 64. At width 8 it is `-128`, and using
/// the wrong one means `-128 / -1` silently wraps to `-128` instead of being
/// reported -- which is exactly the bug this function exists to prevent.
fn signed_min(width: u32) -> i64 {
    if width >= WORD_BITS {
        i64::MIN
    } else {
        -(1i64 << (width - 1))
    }
}

/// Signed division, with the two failure cases reported rather than answered.
fn signed_div(lhs: i64, rhs: i64, width: u32) -> Result<i64, Error> {
    if rhs == 0 {
        return Err(Error::DivisionByZero {
            op: "signed division",
        });
    }
    if lhs == signed_min(width) && rhs == -1 {
        return Err(Error::SignedDivisionOverflow { width });
    }
    Ok(lhs / rhs)
}

/// Reports a `Bits` division failure as this crate's own, so both paths through
/// the interpreter answer the same way.
///
/// The wide path delegates to `Bits`, which has its own diagnostics for exactly
/// these two cases. Passing them through verbatim would mean a 65-bit design and a
/// 64-bit one reported the same mistake differently.
fn division_error(e: BitsError, op: &'static str, width: u32) -> Error {
    match e {
        BitsError::DivisionByZero { .. } => Error::DivisionByZero { op },
        BitsError::SignedDivisionOverflow { .. } => Error::SignedDivisionOverflow { width },
        other => Error::Bits(other),
    }
}

/// Signed less-than for values too wide for one word.
fn signed_lt(lhs: &Bits, rhs: &Bits) -> Result<bool, Error> {
    match (lhs.is_negative(), rhs.is_negative()) {
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        // Same sign: the unsigned comparison is the signed one, because ordering
        // within the negative half and within the positive half both agree with
        // ordering by the underlying magnitude.
        _ => Ok(lhs < rhs),
    }
}
