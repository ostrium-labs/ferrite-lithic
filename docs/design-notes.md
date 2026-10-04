# Ferrite Lithic: design notes

Phase 0 output. Settles the IR, the simulator's execution model, and the Verilog
emitter's contract, so that Phase A1 can be built without re-litigating them.

Every claim about Hardcaml's behaviour below was read from the source at
revision `ff4fe60` (`v0.18~preview`) in `/home/keshav/Ostrium/hardcaml`, with
`file:line` citations, rather than taken from the manual.

## Settled

| Question | Decision |
|---|---|
| Repository | `ostrium-labs/ferrite-lithic` |
| Crate names | `ferrite-lithic*` — bare `lithic` is permanently taken on crates.io and collides with the card-issuing fintech at lithic.com |
| Licence | Apache-2.0 |
| Hardcaml licence | MIT (Jane Street) — port the design, write tests from behaviour, attribute in `NOTICE` |
| Widths | Runtime, not const generics (plan decision 1) |
| Language model | Embedded Rust DSL, not a new language (plan decision 2) |
| IR | Own arena, not FIRRTL/CIRCT (plan decision 3) |
| HDL output | Verilog first, stable naming so diffs are readable |

## 1. `ferrite-lithic-bits`: fixed-width bitvectors

Hardcaml's representation is a single mutable `Bytes.t` with the width stored in
word 0 and data starting at byte offset 8 (`kernel/bits0.mli:1-12`,
`kernel/bits0.ml:22-25`). Two invariants matter:

- **Even a 1-bit vector occupies a full 64-bit word** (`kernel/bits0.mli:8-11`).
- **Unused upper bits of the last word must be zero.** This is a precondition of
  essentially every operation, enforced by a `mask` applied after each
  arithmetic op (`kernel/bits0.ml:113-122`).

There is no S/Z/M distinction anywhere: `Signal` and `Bits` are strictly
two-state, and there is no X in the value domain. A four-state `Logic.Std_logic`
exists in the kernel (`kernel/logic_intf.ml:3-30`) but only feeds parameter
values, never the synthesizable graph.

**Our representation.** `struct Bits { width: u32, words: Vec<u64> }`, sized
`words_of_width(w) = (w + 63) / 64`. We do **not** copy the "1 bit occupies a
full word" waste. We *do* keep the zero-upper-bits invariant, enforce it with a
single `mask` in a private constructor, and make it debug-asserted in every op,
because every downstream op silently depends on it.

### Semantics we inherit deliberately

- **Add and subtract truncate** (modular two's complement). Result width equals
  the left operand's width; Hardcaml asserts equal input widths
  (`kernel/comb.ml:542-545`) and masks the result. There is no automatic
  widening — the caller must reach for the unsigned/signed `Typed_math` module,
  which resizes both operands to `max(wa, wb) + 1` (`kernel/comb.ml:1690-1700`).
  This "you asked for n bits, you got n bits" rule is the single most common
  source of hardware bugs, so the error message must name both widths and say
  that widening is available.
- **Multiply is full precision and cannot overflow**: the result is `wa + wb`
  bits wide (`kernel/bits.ml:605-609`), and at signal level
  `op2 Mulu (width a + width b)` (`kernel/signal.ml:210`). Truncation is always
  explicit, via `select`. This asymmetry is deliberate and we keep it: the
  dangerous operation is the one that silently loses bits.
- **Comparison requires equal widths** except for multiply, which widens.
- **`Bits` ordering is by width first, then unsigned value**
  (`kernel/bits0.ml:46-61`) — an ordering on values of *different* widths, which
  we reproduce because it is observable.

### Signed operations are synthesised, not primitive

Signed comparison flips the sign bit into the LSB position and then does an
unsigned compare (`kernel/comb.ml:728-746`), with width 1 special-cased. Signed
multiply corrects the unsigned result. We implement these directly rather than
via that trick, but we keep the same observable results.

### Shifts are structural

All three shifts desugar into `select` + `cat` + a constant, with **no shift
primitive in the `Signal` graph, the simulator, or the RTL backends**
(`kernel/comb.ml:913-947`). The shift amount is a compile-time OCaml int
(`~by:`), not a signal. A *variable* shift is `log_shift`, a chain of
two-input muxes (`kernel/comb.ml:973-982`).

We keep shifts structural. Adding a shift node would make the simulator faster
but would change the emitted Verilog away from what the golden fixtures show,
and shifters are a thing we would rather let the synthesiser decide than bake
in. `sra` sign-fills with the msb; `srl` zero-fills; both saturate for
`shift >= width`.

### Gaps we will fill

- **Integer division and remainder do not exist in Hardcaml at all.** Not in
  `Comb.S` (`kernel/comb_intf.ml:304-813`), not in the signal op enum
  (`kernel/signal__type_intf.ml:121-133`), not in the simulator op list
  (`src/cyclesim_ops.ml`), not in the RTL binop enum (`src/rtl_ast.mli:53-63`).
  The only `/` and `%` are IEEE float, and those are simulation-only opaque ops
  that cannot be synthesised (`src/cyclesim_float_ops.ml:106-107`). A real
  datapath needs `/` for constant reciprocal tricks, so we add `udiv`, `sdiv`,
  `urem`, `srem` as genuine primitives, and document that they infer large
  dividers rather than being cheap.

## 2. `ferrite-lithic-ir`: the signal graph and how cycles stay legal

This is the mechanism most worth understanding, and Hardcaml's version is
*not* what the plan assumed.

**There is no arena and no node ids at construction time.** A node is a
heap-allocated OCaml variant containing its already-constructed children
(`kernel/signal__type_intf.ml:372-406`). Identity is a `uid` from a
**process-global mutable counter** (`kernel/signal__type.ml:966`).

**Cycles are made legal by exactly one mechanism: the `Wire` node**, which is
the only node with a mutable field (`kernel/signal__type_intf.ml:221-229`):

```ocaml
type 'signal t = { info : Info.t; mutable driver : 'signal option }
```

Every other constructor is built bottom-up over finished children
(`kernel/signal.ml:197-237`), so there is no other possible back edge. The
workflow is: allocate an undriven wire, build logic that reads it, then close the
loop with `<--` (`kernel/signal.ml:269-278`). Assignment carries three checks:
already driven, width mismatch, and assigning to a non-wire
(`kernel/signal.ml:242-267`).

**Why this forces an arena in Rust.** A plain immutable Rust tree cannot
express this, and `Rc<RefCell<..>>` for every node would make the graph
untraversable cheaply. We use `Vec<Node>` plus `NodeId`, with the wire's driver
held as `Cell<Option<NodeId>>` — interior mutability with no runtime borrow
checking, since we never hand out references into the arena. This matches the
plan's stated shape, and the research confirms the arena is not merely
convenient but effectively required.

**Deferred resolution.** The concrete algorithm to copy is
`Signal_graph.rewrite` (`src/signal_graph.ml:105-204`): create *unattached
replacement wires* and register the uid mapping first, then recursively rewrite
each driver, then re-attach. This is how separately-built sub-circuits are
stitched together, and we want the same three-phase shape.

**Three dependency relations, not one.** Hardcaml parameterises traversal over a
functor and instantiates it three ways
(`src/signal_graph.ml:258-325`), and the difference is semantically visible:

| Relation | Reg | Multiport_mem | Inst | Case match constants |
|---|---|---|---|---|
| `Deps_for_loop_checking` | terminal | terminal | terminal | followed |
| `Deps_for_simulation_scheduling` | terminal | terminal | followed | followed |
| `Deps_without_case_matches` | followed | followed | followed | dropped |

The consequences are specific and worth testing: **a combinational loop through
a register is legal** (`test/lib/test_combinational_loop.ml:152-182`); **a loop
through a memory read port is not** (`:243-268`); **a loop into a memory write
port is** (`:270-313`). We implement this as an enum parameter on the traversal
rather than a trait object, so the three cases stay monomorphised and fast.

**Registers store no output value.** `Reg` holds its input `d`; the register's
output *is* the node (`kernel/signal__type_intf.ml:243-296`). Distinguishing
stateful from combinational is therefore purely by node constructor, which is
the cheapest possible test and we keep it.

**Clock domains are out of scope for A1.** Hardcaml has three overlapping
mechanisms — a plain `clock` signal on the register, a phantom-typed
`Clocked_signal` that checks domains on every operation
(`kernel/clocked_signal.ml:36-195`), and the `Always` API layered on top. Only
the first is needed to start, and the phantom-typed approach relies on
locally-abstract types that have no clean Rust equivalent. A1 is
single-clock-domain; the `Cell`-based design leaves room to add a domain field
later without reworking the arena.

**Memories.** `Multiport_mem` holds write ports; read ports are *separate*
`Mem_read_port` nodes (`kernel/signal__type_intf.ml:298-319`). Reads are
asynchronous by construction, and a synchronous read is the user composing a
register with a read port. We keep this, because it makes the "what is stateful"
question answerable from the node kind alone.

## 3. `ferrite-lithic-sim`: the cycle simulator

**Compilation.** Topologically sort, assign every signal a **word address** into
one flat buffer, then compile each node into a closure over those addresses
(`src/cyclesim_compile.ml:479-497`). The layout is a "section" scheme
(`src/cyclesim_compile.ml:322-419`): comb at offset 0, then `regs`, then
`regs_next`, then `mems`, then `consts`.

**Wires cost nothing at runtime.** At build time each wire is chased to its
first non-wire driver and *aliased* to that driver's node
(`src/cyclesim_compile.ml:312-319, 397-401`). We keep this; it is free and it
means the hot loop never sees a wire.

**Two-phase register update, and it is not optional.** `reg_update` writes into
the `regs_next` shadow section, then a `blit` commits `regs_next` → `regs`
(`src/cyclesim_compile.ml:1016-1034`). The reasons are: every register's next
value must be computed from one consistent pre-edge state, otherwise feedback
through a register's own `q` produces order-dependent results; multi-domain
designs need per-domain commits; and reset must be applied to *both* copies or a
register that is currently disabled loses its reset value
(`src/cyclesim_compile.ml:1074-1079`). We keep the shadow section and the blit.

**No event queue, no delta cycles, no delay modelling.** One `comb()` call is
one full settle pass in a precomputed topological order. A topological sort is a
hard precondition, not an optimisation — `Cyclesim_compile.allocate` raises
`"Combinational loop"` (`src/cyclesim_compile.ml:486-488`). We add no
relaxation fallback, because a combinational loop in emitted Verilog is an
infinite loop in the vendor toolchain anyway, and failing loudly is the only
useful behaviour.

**The step is three phases, with two output snapshots per cycle**
(`src/cyclesim.ml:32-46`):

1. `before_clock_edge` — advance the counter, drive clock ports, copy in inputs,
   `comb()`, snapshot outputs as *before*.
2. `at_clock_edge` — `reg_update`, `mem_update`, blit `regs_next` → `regs`.
3. `after_clock_edge` — `comb_last_layer()`, snapshot outputs as *after`.

Both snapshots are first-class (`src/cyclesim0_intf.ml:58`) and the distinction
is genuinely useful: it is how you observe a register on the same edge it
changes. `comb_last_layer` re-evaluates only the nodes on paths from a register
or memory read port to an output, instead of the whole net
(`src/cyclesim_compile.ml:875-884`). We implement the full `comb()` in both
phases in A1 and add `last_layer` only once a profile shows it matters.

**Registers and memories start at zero** — the arena is zero-filled
(`src/cyclesim_compile.ml:492`) — and `initialize_to` is then applied to *both*
register copies. Hardcaml also has a pseudorandom initialiser
(`src/cyclesim_compile.ml:1120-1167`), which we adopt for the random-circuit
differential test.

**Multi-clock alignment.** Hardcaml doubles its internal clock frequency when any
clock has an odd period, so edges can land mid-period
(`src/cyclesim_compile.ml:44-58`). Genuinely subtle, and not needed for A1.

### Deliberate divergence: an op enum, not closures

Hardcaml compiles each node to a zero-argument OCaml closure
(`src/cyclesim_compile.ml:577-783`). In Rust we compile to an `enum Op` with
word-address fields, interpreted by a `match`. Same execution model, no `dyn`,
and — the real reason — **the op list is inspectable**, which is what makes a
simulator-versus-emitted-RTL differential test possible.

## 4. `ferrite-lithic-rtl`: Verilog emission

**Structure.** One module per circuit instance, deduplicated by name
(`src/rtl.ml:75-167`). Not one file per signal. Emission order is fixed
(`src/rtl_verilog_of_ast.ml:391-399`): port list, declarations, statements,
alias assigns, output assigns, `endmodule`.

**Naming is stable given deterministic construction order, and this is a
requirement, not a lucky accident.** The name map is built in three states
(`src/circuit.ml:83-104, 164-211`): ports first, then explicitly named signals,
then unnamed ones in graph order — deliberately *not* depth-first, so that names
land in roughly the order they appear in the RTL, and so that user names win
over derived ones. Unnamed signals get `signal_<node_kind>`
(`src/rtl_name.ml:182-210`); collisions append `_<n>` and retry
(`src/mangler.ml:32-39`); matching is case-insensitive; Verilog reserved words
and illegal identifiers are escaped (`src/rtl_name.ml:19-30`).

Our position is stronger than Hardcaml's here, and it is worth being explicit
about why. Hardcaml's naming depends on a **process-global mutable uid counter**,
so it is only reproducible if the same program runs the same way; there is a
`normalize_uids` pass to rewrite uids into DFS pre-order, and **it is off by
default** and its own test is commented out as "brittle to changes in the test
environment" (`test/lib/test_uid_normalization.ml:56-59`). With an arena, node
ids *are* construction order by definition, so snapshot stability is structural
rather than something we have to defend with a pass. We get byte-identical
Verilog for identical input programs for free.

**One `always` block per register**, never grouped by clock domain
(`src/rtl_ast.ml:329-390`). The body is a fixed nesting —
`if (reset == level) q <= reset_to; else if (clear) q <= clear_to; else if
(enable) q <= d; else q <= d;` — with the sensitivity list being the clock edge
plus the reset edge. Clock conditions are erased from the body and become the
sensitivity list (`src/rtl_verilog_of_ast.ml:153-157`). Nonblocking `<=` in
clocked blocks, blocking `=` in combinational logic.

One quirk to be aware of rather than copy blindly: a **mux wider than two inputs,
and any `cases` node, is emitted as `reg` plus `always @* ... case ...`**
(`src/rtl_ast.ml:501-510`). That is correct Verilog, but it means "is this
signal a register?" is not answerable from the emitted text.

**Instantiation packs all outputs into one concatenated vector**, which is then
sliced per output port (`docs/conversion_to_rtl.md:249-260`). It looks odd and
it is efficient; we keep it, because it makes the instance ABI uniform.

Memories become `reg [31:0] name [0:depth-1]`, one `always` per write port, one
`initial` per initialised address, and `assign q = mem[addr];` for reads
(`src/rtl_ast.ml:392-434`). The declared memory *type* is populated but ignored
by the Verilog writer (`src/rtl_verilog_of_ast.ml:123-129`).

**FSMs.** Built through `Always.State_machine` with `Binary`, `Gray` or `Onehot`
encoding (`kernel/always.ml:414-493`); the default attribute is
`fsm_encoding="one_hot"`, chosen empirically per the comment at
`kernel/always.ml:396-412`. Onehot adds `reset_to`/`clear_to` so the FSM powers
up in state 0.

**Not ported from Hardcaml's gaps, on purpose:** tri-state (it lives in a
separate, unintegrated legacy API, `src/structural_intf.ml:19-46`, and is not
implemented by the Verilog writer); vendor primitives (external project); design
constraints and pin locations (absent entirely); and `Architecture`
(`Small`/`Balanced`/`Fast`), which is re-exported but has **no consumer that
changes codegen** — it is a documentation marker, so we skip it rather than
invent semantics for it.

## 5. `ferrite-lithic-derive`: what the macro must and must not do

**Direction is not inferred, and must not be.** Hardcaml's ppx does not infer it
either; the user supplies two separate interface types, one for inputs and one
for outputs, and `create_fn : Signal.t I.t -> Signal.t O.t` sits between them
(`test/lib/test_fifo.ml:8-31`, `src/circuit.ml:521-522`). Direction is
materialised by the circuit builder, not the macro: each declared input becomes
a named *undriven wire* (which is exactly the criterion for graph input
discovery, `src/signal_graph.ml:59-62`), and each output becomes a new named
wire assigned from the expression (`src/circuit.ml:644-698`).

So our derive macro generates a **shape**, not a direction:

```rust
#[derive(PortList)]
struct Fifo { clock: (), clear: (), wr: (), d: u32, rd: (), q: u32, full: () }
```

producing `PORT_NAMES`, `widths()`, `map`, `iter`, `map2`, `to_list` — and the
user instantiates it twice, once as the input list and once as the output list.
Attempting to infer direction from field names or types would be a new, worse
mechanism than Hardcaml's.

Also note that **`of_signal` is not generated by the ppx**; it comes from a
runtime functor (`kernel/interface_intf.ml:466`). In Rust that is a `PortList`
trait with a blanket impl and free functions `wires`/`inputs`/`outputs`/
`assign`, which is the natural shape.

Attributes to support: `#[bits(N)]` (default 1), `#[length(N)]` required for
collections, `#[rtlname]`, `#[rtlprefix]`, `#[rtlsuffix]`, `#[rtlmangle]`,
`#[exists]` for optional ports, `#[wave_format]`
(`ppx/src/label_attribute.ml:19-31`).

## 6. Testing

The plan's strategy holds up, with one substitution and one addition.

**Keep:** property tests on `Bits` against a big-integer reference; snapshot
tests on emitted Verilog; cosim equivalence over random stimulus.

**Substitute.** Hardcaml's golden waveform expectations are ASCII renders
produced by `hardcaml_waveterm`, an **external proprietary Jane Street library**
(`README.md:112-114`). We cannot reproduce those byte-for-byte and should not
try. What we can use instead is the per-cycle data itself, which is a
first-class value independent of rendering (`src/wave_data_in_cycles.ml`).
Assertions compare cycle-indexed `Bits`, not pretty pictures.

**No generated Verilog is checked in.** The only HDL in the tree is
hand-written fixtures under `test/rtl/` (`naming.v`, `test_parameters_verilog.v`,
and VHDL equivalents) used as simulator inputs. The golden Verilog corpus lives
in ~40 inline `[%expect]` blocks across `test/rtl_gen/` and `test/lib/`. Those
are extractable as byte-level fixtures for `ferrite-lithic-rtl`, with
`test/rtl_gen/test_registers.ml` and `test_memories.ml` the highest-value first
targets, but they must be lifted out of OCaml source rather than copied from a
directory.

**No cycle-accurate stimulus corpus is checked in either** — stimuli are inline
OCaml per test. Extracting them means reading the tests. We accept writing fresh
stimulus for the Rust port.

**Cheap syntax gate: Icarus, not Verilator.** Every Hardcaml RTL test emits
Verilog, shells out to `iverilog -t null -g2012` to prove it parses, and
snapshots the text (`test/rtl_gen/testing.ml:18-32`). We adopt exactly this in
A1. It needs no Verilator install and catches malformed output immediately.
Verilator arrives in A2 with `ferrite-lithic-cosim`, which is **net-new work** —
the in-repo harness does not exist, it lives in the separate
`janestreet/hardcaml_verilator` repository. The natural first cosim targets are
the register and memory designs, since those already have both a definition and
expected output.

**The highest-value test in the suite is the random-circuit differential.**
`test/lib/generator.ml` is a recursive random-design generator — weighted op
choices, widths drawn from `[1;2;3;64;100]` capped at 200, feedback registers
via `reg_fb`, multiport memories, and matching random input vectors. It depends
only on `Signal`/`Circuit`/Quickcheck, so it lifts almost directly. Hardcaml
already uses it as a *differential* test between dedup and non-dedup graphs
(`test/lib/test_dedup.ml:191-197`) and as a generator-quality guard: a fixed
seed `0x3eadbeef` over 1000 circuits, requiring more than two thirds to produce
non-constant outputs. We reuse both, and add the comparison that matters for us —
`ferrite-lithic-sim` against emitted Verilog under Verilator.

## 7. Deliberate divergences from Hardcaml

| Area | Hardcaml | Ours | Why |
|---|---|---|---|
| Node identity | global mutable uid counter | arena index | thread safety, and determinism by construction |
| Compiled sim ops | zero-arg closures | `enum Op` interpreted | no `dyn`, and inspectable for differential testing |
| Add/sub/mul impls | three kept in sync: C stub, OCaml, `Bits_packed` | one | they exist only because OCaml allocation is expensive |
| Division / remainder | absent | added | a real datapath needs it |
| Error handling | `raise_s` with `Caller_id` location | `Result<T, Error>` with source location | runtime widths mean checks are runtime, not compile time |
| Waveform goldens | tied to proprietary waveterm | cycle-indexed `Bits` | we cannot reproduce their renderer |
| `normalize_uids` | exists, off by default, test disabled | unnecessary | arena order is already canonical |
| Multi-clock domains | three mechanisms, incl. phantom types | single domain in A1 | locally-abstract types have no clean Rust form |

Two OCaml-isms to avoid on sight: the global uid counter (thread a context
handle through every constructor instead), and late name attachment via mutable
metadata (`kernel/signal__type.ml:1078-1081`) — Hardcaml needs a lazily
computed name map *because* names can be attached after a node is consumed.
Our build phase should attach names before freezing the arena, which removes an
entire category of bug.

One inherited sharp edge to guard explicitly: Hardcaml's multiply path assumes
operands span at least two words (`kernel/bits.ml:449`, and `smul_core` reads
`unsafe_get32 u (m - 1)`) with no assertion to that effect, covered only
empirically by a width 1..40 sweep (`test/lib/test_bits.ml:285`). Our `Vec<u64>`
implementation has no such assumption; the property test must cover small widths
explicitly rather than trusting the general case.

## Open questions

1. **Does `comb_last_layer` earn its complexity?** Deferred until a profile of a
   real design (FIFO or FIR) says so.
2. **Four-state simulation.** Strictly two-state, like Hardcaml. X-propagation
   would help catch uninitialised reads but doubles the value width and the
   emitted Verilog. Default two-state; possibly a debug-only four-state mode.
3. **Elaborated generics.** Hardcaml has no VHDL-style entity reuse — parameter
   values reach only instantiations (`src/instantiation.mli:19-23`). For the
   MAC-array generator library this matters, and the answer is probably
   parameterisation in *Rust* (generic functions and const generics inside the
   generator crate) rather than an in-IR generic mechanism.
4. **Verilog or SystemVerilog.** Hardcaml has a `two_state` flag switching
   `wire`/`reg` for `logic`/`bit` (`src/rtl_config.ml:3-9`). Worth revisiting
   once real vendor flows are targeted, since Vivado and Yosys differ.
