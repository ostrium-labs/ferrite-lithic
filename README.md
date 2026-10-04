Ferrite Lithic
==============

An embedded hardware DSL, a cycle simulator, and an RTL generator, in Rust.

Hardcaml's model, reimplemented natively. Not a wrapper, not a binding, not a
transpiler from another language: a Rust library you call from Rust.

Status
------

**Phase A2, in progress.** Every A1 crate is implemented and tested, and so are
A2's three: `ferrite-lithic-derive` (port-list shapes), `ferrite-lithic-wave`
(cycle-indexed values and VCD) and `ferrite-lithic-cosim` (Verilator
equivalence). `ferrite-lithic-tb` is the remaining crate.

Two gates depend on tools that are looked for rather than required, and both
**print what they skipped**: `iverilog` for the A1 syntax gate and `verilator`
for the A2 equivalence check. Install them and those tests run; leave them out and
the rest of the suite is unchanged.

The design notes that settle the simulator's execution model, the Verilog
emitter's contract and the port-list rule are in `docs/design-notes.md`, and
`DETAILS.md` is the single place everything known lives.

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

```text
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

471 tests across the eight crates: 96 in `ferrite-lithic-bits`, 116 in
`ferrite-lithic-ir`, 55 in the front end, 48 in the simulator, 49 in the Verilog
emitter, 39 in `ferrite-lithic-derive`, 41 in `ferrite-lithic-wave` and 27 in
`ferrite-lithic-cosim`. The bits crate
runs them in four layers: unit tests, doctests, property tests against a
`num-bigint` oracle, an exhaustive enumeration of every value and operand pair up
to width 6, and an explicit width 1..40 sweep for `mul` covering the
"operands must span two words" assumption inherited from upstream. The IR crate
runs the cycle claims from the research as named tests, plus property tests that
check every width against an independent recomputation and every topological
order against its own definition.

### `ferrite-lithic`

The front end: the same graph, written as arithmetic.

```rust
use ferrite_lithic::Design;

# fn main() -> Result<(), ferrite_lithic::Error> {
let d = Design::new();

let clk = d.input("clk", 1)?;
let acc = d.wire(8)?;

// The nesting that the IR's `&mut self` constructors forbid.
let next = d.add(&acc, &d.lit(1, 8)?)?;
let next = d.reg(&next, &clk, &d.constant(false), &d.constant(false))?;
d.drive(&acc, &next)?;
# Ok(())
# }
```

**There are two tiers, because `a + b` cannot return an error.** Every builder
takes `&self` and returns `Result`; the infix operators build the same nodes but
panic, since `impl Add` has one output type. `tests/operators.rs` compares the two
paths for every operator so they cannot drift.

```rust
use ferrite_lithic::Design;

# fn main() -> Result<(), ferrite_lithic::Error> {
let d = Design::new();
let a = d.lit(0b1010_1010, 8)?;
let b = d.lit(0x0f, 8)?;

let checked = d.add(&a, &b)?;   // Result, with a message naming both widths
let concise = &a + &b;          // panics, with the same message
assert_eq!(checked.kind(), concise.kind());
# Ok(())
# }
```

The arena is `Rc<RefCell<Circuit>>` behind `&self` builders, which is what makes
nesting possible. The `RefCell` is quarantined to the builder: a `Signal` is a
node id and a cached width with no reference *into* the arena, so the graph stays
cheap to traverse, and every borrow is taken and released inside one method body
that cannot re-enter.

### `ferrite-lithic-sim`

The cycle simulator: compile a graph to word addresses, then step it.

```rust
use ferrite_lithic::Design;
use ferrite_lithic_sim::Sim;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let d = Design::new();

let clk = d.input("clk", 1)?;
let acc = d.wire(8)?;
let next = d.add(&acc, &d.lit(1, 8)?)?;
let next = d.reg(&next, &clk, &d.constant(false), &d.constant(false))?;
d.drive(&acc, &next)?;
d.output("acc", 8, &acc)?;

let mut sim = Sim::new(&d)?;
let (before, after) = sim.step()?;
assert_eq!(after.get("acc").unwrap().to_u64().unwrap(), 1);
println!("{before} | {after}");
# Ok(())
# }
```

**A cycle is three phases, and two of them settle the net.** `before_clock_edge`
drives the clock high and snapshots, `at_clock_edge` commits registers and
memories, `after_clock_edge` drives the clock low and snapshots again. Both
snapshots are first-class values: `before` is what the outputs were, `after` is
what the edge made them, which is how you watch a register change on the edge it
changes.

**Registers are written twice on purpose.** Every register's next value goes into
a shadow section, and then the whole section is copied over the live one. Writing
in place would make each register depend on the order registers are visited in, and
a chain of registers would shift one cycle *per register* rather than once per
design. `tests/cycles.rs` has a two-register chain that fails if that is wrong.

**Operations are an inspectable `enum`, not closures.** Hardcaml compiles each
node to an OCaml closure; we compile to an `Op` carrying word addresses and
interpret it with a `match`. Same execution model, no `dyn`, and the list can be
read — which is what makes comparing two simulators, or this one against emitted
Verilog, possible at all.

**Each operation has two implementations, on purpose.** A hand-written `u64` path
for values that fit in one word, and a fallback through `Bits` for the rest. Nearly
every signal is one word wide and a `Bits` operation allocates; but writing
add-with-carry and schoolbook multiply again here to save that one allocation would
mean two implementations of the same semantics. The property test over widths
1..=200 checks both paths against `Bits`, so the fast path is not trusted for being
fast.

**Three things are refused rather than approximated**, because a wrong answer that
looks right is worse than no answer: a submodule `Instance` with no model, a design
with two clocks, and division by zero. Emitted Verilog answers `x` for the last
one and a two-state simulator has no `x` to give, so it is reported instead.

### `ferrite-lithic-rtl`

The Verilog emitter: one module per circuit, and names that do not move.

```rust
use ferrite_lithic::Design;
use ferrite_lithic_rtl::Module;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let d = Design::new();
let clk = d.input("clk", 1)?;
let d_in = d.input("d", 8)?;

let acc = d.wire(8)?;
let next = d.add(&acc, &d_in)?;
let next = d.reg(&next, &clk, &d.constant(false), &d.constant(false))?;
d.drive(&acc, &next)?;
d.output("acc", 8, &acc)?;

let module = Module::new(
    "accumulator",
    d.build()?,
    clk.id(),
    d.input_ports().iter().map(|s| s.id()).collect(),
    d.output_ports().iter().map(|s| s.id()).collect(),
);
print!("{}", module.emit()?);
# Ok(())
# }
```

**The same program emits the same bytes, structurally.** Hardcaml has to defend
that with a `normalize_uids` pass that rewrites process-global uids into DFS
pre-order — off by default, with its own test commented out as "brittle to changes
in the test environment". Node ids here *are* arena indices and arena indices are
construction order, so walking the graph in id order is already canonical and
there is no pass to go wrong.

**Names are allocated in three states: ports, then named signals, then unnamed
ones in graph order.** Deliberately not depth-first, so names land in roughly the
order the logic does, and user names always win over derived ones. An unnamed
signal is `signal_<node_kind>`, collisions append `_<n>`, matching is
case-insensitive, Verilog keywords get a trailing `_`, and anything that is not a
legal identifier is rewritten rather than refused. `tests/naming.rs` asserts every
one of those against the emitted text.

**One wire per node, so no expression is ever nested.** An eight-bit add is
`assign signal_Add = a + b;` and not `assign x = a + b + c;`, which means each line
of the emitted Verilog is one line of the design, and the emitter needs no
precedence table at all. Literals are the exception and stay inline: a constant
*is* its value, and naming it would put an identifier in the output that nothing
reads.

**`clear` holds and `reset` clears**, emitted as `q <= q;` and `q <= 8'h00;`. The
reference emits `else if (clear) q <= clear_to;` and our IR has no `clear_to` field
— the front end's truth table and the simulator both say a cleared register keeps
its value, so a zero here would have been a register that disagrees with its own
simulator. A control input tied low is dropped instead: a test that can never fire
is noise.

**Six things are refused rather than approximated**, because emitting text that
means a different circuit is worse than not emitting: a literal clock, a second
clock domain, a module with state and no clock, a whole memory used as a value, a
port listed in both directions, and a port with no name.

### `ferrite-lithic-derive`

A module's interface, as a plain struct.

```rust
use ferrite_lithic::{inputs, outputs, Design, Signal};
use ferrite_lithic_derive::PortList;

#[derive(PortList)]
struct FifoInputs {
    #[clock] clock: Signal,
    clear: Signal,
    #[bits(32)] d: Signal,
}

#[derive(PortList)]
struct FifoOutputs {
    #[bits(32)] q: Signal,
    #[exists] ready: Option<Signal>,
}

let design = Design::new();
let ports = inputs::<FifoInputs>(&design).unwrap();
assert_eq!(FifoInputs::PORT_WIDTHS, [1, 1, 32]);
```

**Direction is not inferred, and must not be.** Hardcaml's ppx does not infer it
either: the user supplies two interface types, one for inputs and one for outputs,
and the circuit builder sits between them. A convention on field names would be
wrong on exactly the designs where it matters — `d` is an input on a FIFO and an
output on a load unit — so the derive produces a *shape* (names, widths, and a
field walk) and `inputs`/`outputs`/`wires`/`assign` materialise it into a design.

| Attribute | On | Meaning |
|---|---|---|
| `#[bits(N)]` | field | Port width. Defaults to `1`. |
| `#[length(N)]` | field | A collection of `N` ports. Required on a `Vec<Signal>`. |
| `#[rtlname("x")]` | field | Verilog port name. Defaults to the field name. |
| `#[exists]` | field | The port may be absent. Needs `Option<Signal>`. |
| `#[clock]` | field | The module clock. At most one. |
| `#[wave_format("hex")]` | field | `binary`, `hex` or `decimal`. Defaults to `binary`. |
| `#[rtlprefix("p_")]` | struct | Rewrites the Verilog-facing names. |
| `#[rtlsuffix("_q")]` | struct | As above. |
| `#[rtlmangle]` | struct | Legalise the names for Verilog. |

Two shapes rather than one is not a workaround. One design cannot hold two nodes
with one name, so a module whose inputs and outputs are both called `d` cannot be
declared from a single list — and it could not be declared in Verilog either.

### `ferrite-lithic-wave`

Cycle-indexed values, and the file they render to.

```rust
use ferrite_lithic_bits::Bits;
use ferrite_lithic_wave::WaveData;

let mut wave = WaveData::new("shifter");
let data = wave.register("data", 8).unwrap();
for (cycle, value) in [0x01u64, 0x02].into_iter().enumerate() {
    wave.set(cycle as u64, &data, &Bits::constant(value, 8).unwrap()).unwrap();
}
let expected: Vec<Bits> = [1u64, 2]
    .into_iter()
    .map(|value| Bits::constant(value, 8).unwrap())
    .collect();
wave.assert_series(&data, &expected).unwrap();
```

Hardcaml's golden waveforms are ASCII renders from `hardcaml_waveterm`, a
proprietary external library, so the bytes are not reproducible and we do not try.
What *is* reproducible is the per-cycle data, and that is the first-class value
here: assertions read `value`/`series`, and `Vcd` is a rendering of that data
rather than the source of it. A test that asserted on a rendered waveform would
be asserting on the renderer.

No `$date` in the file, and positional identifiers rather than names derived from
signals: two runs of one design produce the same bytes, and two ports whose names
legalise alike can both be dumped.

### `ferrite-lithic-cosim`

The emitted Verilog, checked against the simulator, cycle by cycle.

```rust
use ferrite_lithic_bits::Bits;
use ferrite_lithic_cosim::{Plan, Port, Stimulus, compare};

let plan = Plan {
    top: "passthrough".to_string(),
    clock: Some("clk".to_string()),
    inputs: vec![Port { name: "d".to_string(), width: 8 }],
    outputs: vec![Port { name: "q".to_string(), width: 8 }],
};

let mut stimulus = Stimulus::new();
let value = Bits::constant(0xa5, 8).unwrap();
stimulus.push(vec![value.clone()]).unwrap();

// `run_with` needs a built `Harness`, which costs seconds of Verilator build
// time; `run` builds one. `compare` needs neither and is pure.
let simulator = vec![vec![value]];
let report = compare(&plan.output_names(), &simulator, &simulator).unwrap();
assert!(report.is_equivalent());
```

Verilator arrives here, as ADR-0010 deferred it. Hardcaml's harness lives in the
separate `janestreet/hardcaml_verilator` repository, so this is net-new work. The
crate is arranged so that **only** the Verilator build and run need the toolchain:
the stimulus encoding, the generated C++ driver, the comparison and its reporting
are ordinary Rust that the suite exercises unconditionally. The equivalence tests
skip — loudly, on stdout — when `verilator` is absent.

One cycle is: clock low, inputs applied, clock high, evaluate, read. The read is
*after* the rising edge, which is `Sim::step`'s second snapshot; reading before the
edge would put the two backends a register update apart on every registered port.
Registers start at zero in both, which is a property of the two tools and is said
in the generated driver rather than assumed quietly.

`generator` is Hardcaml's `test/lib/generator.ml` ported: weighted op choices,
widths from `[1; 2; 3; 64; 100]`, feedback registers, multiport memories, and a
seed that makes a design a pure function of its seed. Its two carries-over are both
checks on the *generator*: a design is a pure function of its seed, and more than
two thirds of a fixed-seed corpus must produce a changing output — a generator that
produced constants would make the differential pass by never disagreeing.

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
| `ferrite-lithic` | Front-end DSL: `Signal` handle with operator overloads and builder functions | A1 **done** |
| `ferrite-lithic-sim` | Cycle simulator over a flat, topologically sorted op list | A1 **done** |
| `ferrite-lithic-rtl` | Verilog emitter with stable naming | A1 **done** |
| `ferrite-lithic-derive` | `#[derive(PortList)]`: a module interface's names and widths from a struct | A2 **done** |
| `ferrite-lithic-wave` | Cycle-indexed values, asserted on directly, plus a VCD rendering | A2 **done** |
| `ferrite-lithic-cosim` | Verilator equivalence checking against `ferrite-lithic-sim` | A2 **done** |
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

```text
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
