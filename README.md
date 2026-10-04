Ferrite Lithic
==============

An embedded hardware DSL, a cycle simulator, and an RTL generator, in Rust.

Hardcaml's model, reimplemented natively. Not a wrapper, not a binding, not a
transpiler from another language: a Rust library you call from Rust.

Status
------

**Phase A1, in progress.** `ferrite-lithic-bits` is implemented and tested; the
rest of the crate split below is the intended shape. The design notes that settle
the IR, the simulator's execution model, and the Verilog emitter's contract are in
`docs/design-notes.md`, and `DETAILS.md` is the single place everything known
lives.

### `ferrite-lithic-bits`

Fixed-width bitvectors over `u64` words, with runtime widths.

```rust
use ferrite_lithic_bits::Bits;

let a = Bits::constant(0xff, 8)?;
let b = Bits::constant(0x01, 8)?;

// `add` gives back exactly the 8 bits asked for, and drops the carry.
assert_eq!(a.add(&b)?.to_u64()?, 0x00);

// `mul` widens, so nothing is lost.
let product = a.mul(&b)?;
assert_eq!(product.width(), 16);
assert_eq!(product.to_u64()?, 0x00ff);
# Ok::<(), ferrite_lithic_bits::Error>(())
```

Widths are runtime values, so a width mismatch is a runtime error rather than a
compile error. The error message therefore names both operand widths and the
explicit operation that fixes it:

```
width mismatch in `add`: the left operand is 8 bits and the right operand is
16 bits. `add` and `sub` require equal widths and return the left operand's
width, discarding whatever falls off the top. Resize both operands first with
`Bits::zero_extend` or `Bits::sign_extend`, or narrow the wider one with
`Bits::truncate`.
```

Deliberate asymmetries, inherited from Hardcaml and kept:

| Operation | Result width | Why |
|---|---|---|
| `add`, `sub`, `neg` | the left operand's | truncation is explicit, never silent-by-accident |
| `mul` | `wa + wb`, cannot overflow | the dangerous operation should not be the truncating one |
| `sll`, `srl`, `sra` | unchanged | structural: `select` + `cat` + a constant, no shift primitive |
| `udiv`, `sdiv`, `urem`, `srem` | the left operand's | absent from Hardcaml entirely; infer large dividers |

Division by zero is refused rather than wrapped, and signed division overflow
(`-2^(w-1) / -1`) is refused rather than returned as `-2^(w-1)`.

95 tests, in four layers: unit tests, doctests, property tests against a
`num-bigint` oracle, an exhaustive enumeration of every value and operand pair up
to width 6, and an explicit width 1..40 sweep for `mul` covering the
"operands must span two words" assumption inherited from upstream.

Crate layout
------------

| Crate | Purpose | Phase |
|---|---|---|
| `ferrite-lithic-bits` | Fixed-width bitvectors over `u64` words, runtime widths | A1 **done** |
| `ferrite-lithic-ir` | Signal graph: arena of nodes, `NodeId` indices, wires resolved after construction so cycles are legal | A1 |
| `ferrite-lithic` | Front-end DSL: `Signal` handle with operator overloads and builder functions | A1 |
| `ferrite-lithic-sim` | Cycle simulator over a flat, topologically sorted op list | A1 |
| `ferrite-lithic-rtl` | Verilog emitter with stable naming | A1 |
| `ferrite-lithic-derive` | `#[derive]` for port-list structs, `map`/`iter`/`of_signal` | A2 |
| `ferrite-lithic-wave` | VCD waveform writer | A2 |
| `ferrite-lithic-cosim` | Verilator equivalence checking against `ferrite-lithic-sim` | A2 |
| `ferrite-lithic-tb` | Coroutine/async-style step testbenches | A3 |

Design decisions
----------------

1. **Runtime widths, not const generics.** Const-generic widths give
   compile-time checks but make graph construction in loops and generators
   painful. We keep runtime checks and invest in the error messages instead.
2. **Embedded DSL, not a new language.** The Hardcaml model. The
   new-language route is already covered by Spade and others.
3. **Own IR first.** Emitting FIRRTL or CIRCT could reuse their optimisers and
   Verilog backends but adds a heavy dependency. Revisit at the end of Phase A2
   with real designs in hand.
4. **Port the design, write tests from behaviour.** Hardcaml is MIT (Jane
   Street), so this is permitted and attributed in `NOTICE`. We take the
   architecture and the semantics, not the code.
5. **Division is added, and it is not cheap.** Hardcaml has no integer division
   or remainder at any layer. A real datapath needs `/` for constant-reciprocal
   tricks, so `udiv`, `sdiv`, `urem` and `srem` are genuine primitives here and
   documented as inferring large dividers.

Building
--------

```
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
```

The toolchain is pinned to an exact channel in `rust-toolchain.toml`; CI also
checks that the declared minimum, 1.97, still builds.

Scope
-----

Non-goals, deliberately: writing a new HDL language, and compiling arbitrary
Rust programs to hardware. The bridge to Strata covers a dataflow subset with
static shapes, not general program synthesis.

Relationship to Strata
----------------------

The FPGA path in [`ferrite-strata`](../ferrite-strata) is implemented as a
Strata backend: graph in, bitstream out. That backend is the first real test of
Strata's closed-vendor-plugin idea, because Lithic behaves like a vendor with
its own architecture and its own compiler.

Licence
-------

Apache-2.0. See `LICENSE` and `NOTICE`.
