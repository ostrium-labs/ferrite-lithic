Ferrite Lithic
==============

An embedded hardware DSL, a cycle simulator, and an RTL generator, in Rust.

Hardcaml's model, reimplemented natively. Not a wrapper, not a binding, not a
transpiler from another language: a Rust library you call from Rust.

Status
------

**Phase A1, in progress.** `ferrite-lithic-bits` and `ferrite-lithic-ir` are
implemented and tested; the rest of the crate split below is the intended shape.
The design notes that settle the simulator's execution model and the Verilog
emitter's contract are in `docs/design-notes.md`, and `DETAILS.md` is the single
place everything known lives.

### `ferrite-lithic-bits`

Fixed-width bitvectors over `u64` words, with runtime widths.

```rust
use ferrite_lithic_bits::Bits;

let a = Bits::constant(0xff, 8).unwrap();
let b = Bits::constant(0x01, 8).unwrap();

// `add` gives back exactly the 8 bits asked for, and drops the carry.
assert_eq!(a.add(&b).unwrap().to_u64().unwrap(), 0x00);

// `mul` widens, so nothing is lost.
let product = a.mul(&b).unwrap();
assert_eq!(product.width(), 16);
assert_eq!(product.to_u64().unwrap(), 0x00ff);
```

Operations take `&self` and return `Result`, so they chain; the `?` is elided
here for readability.

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

107 tests in `ferrite-lithic-ir`, and 95 in `ferrite-lithic-bits`. The bits crate
runs them in four layers: unit tests, doctests, property tests against a
`num-bigint` oracle, an exhaustive enumeration of every value and operand pair up
to width 6, and an explicit width 1..40 sweep for `mul` covering the
"operands must span two words" assumption inherited from upstream. The IR crate
runs the cycle claims from the research as named tests, plus property tests that
check every width against an independent recomputation and every topological
order against its own definition.

### `ferrite-lithic-ir`

The signal graph: an arena of nodes whose identity is the arena index, and the
only place a feedback loop can come from.

```rust
use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{Circuit, Deps};

let mut c = Circuit::new();

// An undriven wire, built first so logic can read it before it has a driver.
// `Wire` is the only mutable node in the graph, which is what makes a loop
// expressible at all in a bottom-up IR.
let acc = c.wire(8).unwrap();
let one = c.constant(Bits::constant(1, 8).unwrap());
let clk = c.constant(Bits::constant(0, 1).unwrap());

let incremented = c.add(acc, one).unwrap();
let next = c.reg(incremented, clk, clk, clk).unwrap(); // a register's output is the node itself
c.drive(acc, next).unwrap(); // close the loop

// Legal: a register is terminal for combinational loop checking, so feedback
// through one is an ordinary accumulator rather than a cycle.
c.topological_order(Deps::LoopChecking).unwrap();
```

Constructors take `&mut self` and return `Result`, so a call cannot be nested
inside another (`c.add(a, c.constant(..))` will not borrow-check); build operands
into locals first. The DSL crate exists to hide that.

Three inherited decisions do the work, and each is a test:

| Decision | Consequence |
|---|---|
| A register stores no output value | "is this stateful?" is one constructor match, not a flag |
| `Wire` is the only mutable node | a feedback loop has exactly one possible back edge |
| Memories hold write ports; read ports are separate nodes | a read port is never a hidden source of state |

**Node ids are arena indices, so naming needs no normalisation pass.** Hardcaml
gives nodes identity from a process-global mutable counter, and needs
`normalize_uids` to make output reproducible — which it ships **off by default,
with its own test commented out as "brittle"**
(`test/lib/test_uid_normalization.ml:56-59`). Here, construction order *is* the
canonical order, so byte-identical Verilog is structural rather than something to
defend with a pass. `tests/ordering.rs` asserts that a bottom-up build is already
in evaluation order, and that two circuits can be built on two threads at once.

**Three dependency relations, not one**, as an enum parameter rather than a trait
object so the cases stay monomorphised (`src/signal_graph.ml:258-325`). The
difference is observable, and the memory pair is the one that is easy to get
backwards:

| Relation | `Reg` | `Mem` | `Instance` |
|---|---|---|---|
| `LoopChecking` | terminal | terminal | terminal |
| `SimulationScheduling` | terminal | terminal | followed |
| `WithoutCaseMatches` | followed | followed | followed |

So a loop **through a register** is legal, a loop **into a memory write port** is
legal, and a loop **through a memory read port** is not. All three hold for the
same reason: read ports are ordinary combinational nodes while the memory itself
is a terminal.

Crate layout
------------

| Crate | Purpose | Phase |
|---|---|---|
| `ferrite-lithic-bits` | Fixed-width bitvectors over `u64` words, runtime widths | A1 **done** |
| `ferrite-lithic-ir` | Signal graph: arena of nodes, `NodeId` indices, wires resolved after construction so cycles are legal | A1 **done** |
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
