//! Fixed-width bitvectors for the Ferrite Lithic hardware DSL.
//!
//! This is the foundation layer: an unsigned-free, strictly two-state,
//! runtime-width bitvector with the arithmetic a hardware datapath actually needs.
//! Everything above it — the signal graph, the simulator, the Verilog emitter —
//! is expressed over this type.
//!
//! # Design, and where it came from
//!
//! Every claim about Hardcaml's behaviour was read from source with `file:line`
//! citations rather than taken from its manual, and several of them contradict
//! what the original plan assumed. See
//! [`docs/design-notes.md`](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/design-notes.md)
//! for the full audit.
//!
//! # What is inherited deliberately
//!
//! - **Widths are runtime `u32`, not const generics**
//!   ([ADR-0003](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0003-runtime-widths-not-const-generics.md)).
//!   A hardware DSL is mostly loops and generators, which const generics make
//!   painful, so the error messages have to carry the compensation a compiler
//!   would have. They do: every width mismatch names both widths and the explicit
//!   widening operation that fixes it.
//! - **`add` and `sub` truncate; `mul` widens to `wa + wb` and cannot overflow.**
//!   The dangerous operation is the one that silently loses bits, and widening
//!   `mul` is how the type stops multiplication being the accident.
//! - **Shifts are structural.** There is no shift primitive; `sll`, `srl` and
//!   `sra` desugar to [`select`](Bits::select), [`cat`] and a constant, exactly as
//!   upstream. Adding a shift node would change the emitted Verilog away from the
//!   golden fixtures.
//! - **`Ord` is by width first, then unsigned value**, because a `Bits`
//!   collection sorted with `sort` groups by width and that is observable.
//! - **Division does not exist upstream and is added here**
//!   ([ADR-0009](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0009-add-integer-division-and-remainder.md)):
//!   `udiv`, `sdiv`, `urem`, `srem`. They infer large dividers and are not cheap.
//!
//! # What is deliberately different
//!
//! - **Words are sized exactly.** Hardcaml rounds every width up to a full 64-bit
//!   word, so a 1-bit vector occupies a whole word; we store
//!   `ceil(width / 64)` words and nothing more.
//! - **The zero-upper-bits invariant is enforced by a private constructor**
//!   rather than by a `mask` call at every use site, and every operation
//!   debug-asserts it. Every downstream operation silently depends on it.
//! - **Errors are `Result`, not exceptions**, because with runtime widths the
//!   checks genuinely are runtime.
//! - **Ordered comparisons are implemented directly**, not via Hardcaml's
//!   flip-the-sign-bit-into-the-LSB trick, which needs a width-1 special case
//!   that direct dispatch does not.
//!
//! # The invariant
//!
//! **Unused bits above the width in the top word are always zero.** This is the
//! one sharp edge inherited from upstream, and it is why every method begins with a
//! debug assertion. It holds in release too, enforced by a single masking step in
//! the private constructor every value funnels through.
//!
//! # Examples
//!
//! Truncating addition and widening multiplication, side by side:
//!
//! ```
//! use ferrite_lithic_bits::Bits;
//!
//! let a = Bits::constant(0xff, 8)?;
//! let b = Bits::constant(0x01, 8)?;
//!
//! // `add` gives back exactly the 8 bits asked for, and drops the carry.
//! assert_eq!(a.add(&b)?.width(), 8);
//! assert_eq!(a.add(&b)?.to_u64()?, 0x00);
//!
//! // `mul` widens, so nothing is lost.
//! let product = a.mul(&b)?;
//! assert_eq!(product.width(), 16);
//! assert_eq!(product.to_u64()?, 0x00ff);
//! # Ok::<(), ferrite_lithic_bits::Error>(())
//! ```
//!
//! Structural shifts, and the saturation at or beyond the width:
//!
//! ```
//! use ferrite_lithic_bits::Bits;
//!
//! let v = Bits::constant(0b1100, 4)?;
//! assert_eq!(v.srl(1)?.to_u64()?, 0b0110);
//! assert_eq!(v.sra(1)?.to_u64()?, 0b1110); // sign filled
//! assert_eq!(v.sra(4)?.to_u64()?, 0b1111); // saturated
//! # Ok::<(), ferrite_lithic_bits::Error>(())
//! ```
//!
//! Width errors explain themselves:
//!
//! ```
//! use ferrite_lithic_bits::Bits;
//!
//! let narrow = Bits::constant(1, 8)?;
//! let wide = Bits::constant(1, 16)?;
//! let err = narrow.add(&wide).unwrap_err();
//!
//! assert!(err.to_string().contains("8 bits"));
//! assert!(err.to_string().contains("16 bits"));
//! assert!(err.to_string().contains("zero_extend"));
//! # Ok::<(), ferrite_lithic_bits::Error>(())
//! ```

#![doc(html_root_url = "https://docs.rs/ferrite-lithic-bits/0.0.1")]

mod arith;
mod bits;
mod convert;
mod error;
mod logic;
mod shift;

pub use bits::{Bits, WORD_BITS, bytes_for_width, mask_for_width, words_for_width};
pub use error::{Error, Op};
pub use shift::cat;
