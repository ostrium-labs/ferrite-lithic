# Ferrite Lithic: details

Everything known about this project in one place. Companion documents:

- `docs/design-notes.md` — the IR, simulator and emitter design, read from Hardcaml's source
- `docs/plans/jane-street-protocol-emulator.md` — the competition plan and timeline
- `docs/adr/` — 17 accepted architecture decision records
- `../ferrite-strata/DETAILS.md` — the companion runtime

Status: **Phase A1 in progress.** `ferrite-lithic-bits`, `ferrite-lithic-ir`, the
`ferrite-lithic` front end and the `ferrite-lithic-sim` cycle simulator are
implemented, documented and tested (309 tests); the remaining A1 crate, `-rtl`, is
not yet written. Nothing is published to crates.io yet.

## 1. What it is

An embedded hardware DSL, a cycle simulator, and an RTL generator, in Rust.
Hardcaml's model, reimplemented natively — not a wrapper, not a binding, not a
transpiler. A Rust library you call from Rust.

The immediate driver is a taped-out chip: a RISC-V core with a pin and timing ISA,
whose firmware implements production event-streaming protocols.

## 2. Identity

| | |
|---|---|
| Repository | `ostrium-labs/ferrite-lithic`, default branch `dev` |
| Crates | `ferrite-lithic`, `-bits`, `-ir`, `-sim`, `-rtl`, `-derive`, `-wave`, `-cosim`, `-tb` |
| Licence | Apache-2.0, copyright appendix filled |
| Prior art | Hardcaml, MIT, Jane Street, studied at `ff4fe60` (`v0.18~preview`) |

Names are prefixed `ferrite-` because bare `lithic` is permanently taken on
crates.io and collides with the card-issuing fintech at lithic.com. See ADR-0002.

`NOTICE` records Hardcaml as a design reference with **no code copied**, and is
written to be extended if code or vectors are ever derived rather than rewritten.

## 3. Crate layout

| Crate | Purpose | Phase |
|---|---|---|
| `ferrite-lithic-bits` | Fixed-width bitvectors over `u64` words, runtime widths | A1 **done** |
| `ferrite-lithic-ir` | Arena of nodes with `NodeId`, wires resolved after construction | A1 **done** |
| `ferrite-lithic` | Front-end DSL: `Signal` with operator overloads and builders | A1 **done** |
| `ferrite-lithic-sim` | Cycle simulator: word addresses and an interpretable op list | A1 **done** |
| `ferrite-lithic-rtl` | Verilog emitter with stable naming | A1 **done** |
| `ferrite-lithic-derive` | `#[derive]` for port-list structs | A2 |
| `ferrite-lithic-wave` | VCD waveform writer | A2 |
| `ferrite-lithic-cosim` | Verilator equivalence against `ferrite-lithic-sim` | A2 |
| `ferrite-lithic-tb` | Coroutine-style step testbenches | A3 |

### What `ferrite-lithic-bits` actually does

Written to §4 and ADR-0003/0009 above, not to the plan. Representation is
`{ width: u32, words: Vec<u64> }` with `ceil(width / 64)` words, and one invariant:
**bits above the width in the top word are always zero**. Every value funnels
through a single private masking constructor, and every public method debug-asserts
the invariant at its own entry rather than relying on the caller.

Beyond the inherited semantics it adds `udiv`, `sdiv`, `urem` and `srem` (ADR-0009),
and three decisions that upstream does not make:

- **Division by zero is an error, not a wrap or a saturate.** Hardware has no
  defined result, so returning one would be inventing semantics.
- **Signed division overflow is an error.** `-2^(w-1) / -1` needs one more bit than
  the operands have. Verilog returns `-2^(w-1)`; we refuse, because silently
  returning the wrapped value is the exact bug class this crate exists to prevent.
  `srem` cannot overflow, so it still answers zero for that input.
- **Every width error names both widths and names the fix.** That is the whole
  compensation for giving up const generics, and `tests/errors.rs` fails if any
  message is shortened to "width mismatch".

**Four bugs were found by the test layers, all in code that looked right.** Worth
recording because each one was invisible to the layer below it:

| Bug | Symptom | Caught by |
|---|---|---|
| `is_ones` compared each raw word against its *masked* form | Always returned `true`, silently disabling the signed-overflow guard | exhaustive enumeration to width 6 |
| `cmp_signed` reversed two-negative comparisons | Put `-1` below `-2^128` | property test at width 129 |
| `magnitude()` negated *after* zero-extending | Gave `2^(w+1) - v` instead of `2^w - v`, wrong for every negative value except `-1` | exhaustive enumeration |
| `sign_extend` filled whole words, so widening inside one word did nothing | `sign_extend(6)` of a 2-bit `0b11` returned `3`, not `63` | doctest |

The lesson is the same one §5 records about Hardcaml's multiply: **at these
widths, assuming a word boundary is an assumption.** The exhaustive layer exists
because random sampling found none of the first three.

### What `ferrite-lithic-ir` actually does

Written to §4 above rather than to the plan, and the arena turned out to be
forced rather than convenient: Hardcaml has no arena and no ids at construction
time, and a plain immutable Rust tree cannot express a feedback loop at all. So
nodes live in a `Vec`, `NodeId` is an index, and `Wire` is the only node with a
mutable field (`Cell<Option<NodeId>>`) — which is the only possible back edge,
and therefore the only way a cycle can exist.

**The three dependency relations are implemented as written, and the memory cases
reconcile.** The design notes record that a loop through a register is legal
(`test_combinational_loop.ml:152-182`), a loop through a memory read port is not
(`:243-268`), and a loop into a memory write port is (`:270-313`). Those looked
contradictory until the read/write split explains them: a read port is an ordinary
combinational node, so the offending cycle `read port → address logic → read port`
is caught by every relation, while the memory itself is terminal for loop checking
and so the write port's feedback path is not. `tests/cycles.rs` asserts all three
from the graph rather than taking the reading on trust.

**Two real bugs, both found by tests that were written to be awkward on purpose.**

| Bug | Symptom | Caught by |
|---|---|---|
| `Mem` was treated as a leaf in the operand list | No dependency through a memory existed under *any* relation, so `WithoutCaseMatches` silently reported no cycle where one existed, and `check` could not see a dangling write-port signal | `a_loop_into_a_memory_write_port_is_legal`, which asserts the relation differs |
| `check`'s "is this wire read by anything" test was inverted | It reported exactly the wires nobody read and passed on the ones something read from | two opposite tests, which is the only reason it was caught |

Both are the same shape as the bits-crate bugs: a plausible-looking `match` arm
that was quietly wrong. The mitigations that found them were cheap and are now
permanent — assert a relation *differs* rather than only that it succeeds, and
write the positive and negative case of every predicate next to each other.

**Two claims are recorded as unproven rather than assumed.** `Deps`'s
`Case` match-constant row is **inert in A1**: our cases match against `Bits`
literals, so there are no match-constant edges to follow or drop, and
`includes_case_matches` cannot change any result. It is kept because dropping the
dimension now would mean revisiting every call site if match constants ever become
signals. And `Node::Instance` produces a **single output**, which is an A1
limitation rather than a design position — a submodule returning several signals
needs a bundle type, and the bundle design is not settled. Single output is enough
to make the node kind real, and `Instance` is the only kind the loop-checking and
simulation-scheduling relations disagree about, so without it the three-way enum
would have had a redundant variant.

## 4. Design, and where it came from

Every claim about Hardcaml was read from source with `file:line` citations, not
taken from the manual. Several contradict what the plan assumed.

**Bits.** `{ width: u32, words: Vec<u64> }`. Hardcaml uses a mutable `Bytes` with the
width in word 0 and a full 64-bit word even for 1 bit; we size the vector exactly but
keep the zero-upper-bits invariant, enforced by a private `mask` and debug-asserted
in every op. Add and subtract **truncate**; multiply is **full precision and cannot
overflow**, returning `wa + wb` bits. Shifts are structural — `select` plus `cat`
plus a constant, with no shift primitive anywhere, exactly as upstream. We keep that:
adding a shift node would change the emitted Verilog away from the golden fixtures.

**The IR needs an arena, and the research proved it.** Hardcaml has no arena and no
node ids at construction time; a node is an OCaml variant over finished children,
with identity from a **process-global mutable counter**. Cycles are legal through
exactly one mechanism: `Wire` is the only node with a mutable field, so it is the only
possible back edge. A plain immutable Rust tree cannot express this.

We use `Vec<Node>` plus `NodeId`, with the wire's driver in a `Cell<Option<NodeId>>`.
No runtime borrow checking, because we never hand out references into the arena.
Deferred resolution copies Hardcaml's three-phase `Signal_graph.rewrite`: create
unattached replacement wires, register the mapping, rewrite drivers, re-attach.

**Naming stability is structural here, which it is not upstream.** Hardcaml needs a
`normalize_uids` pass to make names reproducible, and that pass is **off by default
with its own test commented out as "brittle"**
(`test/lib/test_uid_normalization.ml:56-59`). With an arena, node ids *are*
construction order, so byte-identical Verilog for identical input programs is
structural rather than something to defend with a pass.

**Three dependency relations, not one**, parameterising traversal
(`src/signal_graph.ml:258-325`), and the difference is observable: a combinational
loop through a register is legal, through a memory read port is not, into a memory
write port is.

**Two-phase register update is not optional** (`src/cyclesim_compile.ml:1016-1034`):
next-state goes to a shadow section and is blitted on commit, because every register
must be evaluated from one consistent pre-edge state or feedback through its own `q`
becomes order-dependent, and reset must land in both copies.

**Three-phase step with two output snapshots per cycle** (`src/cyclesim.ml:32-46`):
before edge (drive clocks, copy in, `comb()`, snapshot), at edge (`reg_update`,
`mem_update`, blit), after edge (`comb_last_layer`, snapshot).

**Registers and memories start at zero.** Registers store no output value; the
register's `q` *is* the node, so "is this stateful?" is answered by node constructor
alone — the cheapest possible test.

**The derive macro cannot infer signal direction, and neither can Hardcaml's.** The
user supplies separate input and output types with `create_fn` between them; the macro
generates a shape and the builder materialises direction. Inferring it from field
names would be a new, worse mechanism.

## 5. Deliberate divergences

| Area | Hardcaml | Ours | Why |
|---|---|---|---|
| Node identity | global mutable uid counter | arena index | thread safety; determinism by construction |
| Simulated ops | zero-arg closures | `enum Op`, interpreted | no `dyn`, and inspectable for differential testing |
| Add/sub/mul impls | three kept in sync: C stub, OCaml, `Bits_packed` | one | they exist only because OCaml allocation is expensive |
| Division, remainder | absent at every layer | added | a real datapath needs it |
| Error handling | `raise_s` with `Caller_id` | `Result<T, Error>` with location | runtime widths mean checks are runtime |
| Waveform goldens | tied to proprietary waveterm | cycle-indexed `Bits` | we cannot reproduce their renderer |
| `normalize_uids` | exists, off by default, test disabled | unnecessary | arena order is already canonical |
| Multi-clock domains | three mechanisms incl. phantom types | single domain in A1 | locally-abstract types have no clean Rust form |
| Register clear | a `clear_to` field, cleared to it | no such field, so clear holds | our `Reg` gates the update, and the simulator implements hold |
| Instance ports | named ports on the instance | positional `.i0` / `.o0` | the IR records input order and has nowhere to keep port names |
| Module dedup | interned definitions, one per name | name plus port signature | `Circuit` has no `PartialEq`, so two graphs cannot be compared here |

Two OCaml-isms to avoid on sight: the global uid counter, and late name attachment
via mutable metadata, which is why Hardcaml needs a lazily computed name map. Our
build phase attaches names before freezing the arena, removing a whole bug category.

One inherited sharp edge to guard: Hardcaml's multiply assumes operands span at least
two words with no assertion to that effect, covered only by a width 1..40 sweep. Our
`Vec<u64>` has no such assumption, so the property test must cover small widths
explicitly rather than trusting the general case.

## 6. The competition chip

**Target, verbatim from the Jane Street rules** (blog post 10 Sep 2026, verified
against the source):

| Item | Value |
|---|---|
| Process | IHP **130nm CMOS5L** (`ihp-sg13cmos5l`) via Tiny Tapeout |
| Flow | `ttihp-verilog-template` branch `cmos5l`, RTL to GDS via LibreLane |
| Tiles | `info.yaml` = **6x4** = 24 tiles, ~0.7 mm², ~1K logic cells/tile |
| Registers | ~320 DFF per tile |
| Instruction memory | **SRAM preferred over flip-flops**, ~11–26 KB |
| Deadline | **18 January 2027**; March 2027 shuttle target |
| Open source | Required; build in public explicitly allowed |
| Contact | `asic-competition@janestreet.com` |

The brief is a *"general-purpose protocol emulator"*: *"a tiny CPU with an
instruction set designed for reading pins, writing pins, counting cycles, and
hitting timing precisely enough that you can implement a real protocol in firmware
rather than in fixed logic."* Baselines UART, SPI, I2C; stretch USB and 10 Mbit
Ethernet; also JTAG, SWD, PS/2, CAN.

**Architecture.** PicoRV32 (RV32I) plus custom instructions: `PIN_OUT`, `PIN_DIR`,
`PIN_SAMPLE` with delay, `PIN_EDGE` with programmable oversample, `CYCLE`,
`WAIT_CYCLES`, and a width-parameterised `SIMD`. At roughly 750 LUTs the core is a
small fraction of 24K cells, and it is not where novelty lives.

**Why this can place.** Tiny Tapeout IHP 26b already contains AstraPIO, nanoPIO,
`tt_um_mini_kraken` and SEQ8. A fourth PIO state machine cannot win, and the
competition explicitly rejects fixed-logic blocks. Nobody is running a production
event-streaming protocol stack on it.

**What runs on it.** UART, then SPI and I2C from firmware, then protocol translation.
The contract already exists in Loams: `loams-stream-grpc`'s `StreamService`, whose
comment already reads *"used by protocol adapters through Dapr invocation"*, including
the CloudEvents `application/cloudevents-batch+json` path annotated as a Dapr
pass-through. That JSON path is the firmware target because it avoids both protobuf
varints and HTTP/2 HPACK, which is what makes it fit 11–26 KB.

**Verification, the differentiator.** A production conformance suite to validate
against (`loams-meta-conformance`, 4,488 lines). Differential testing of the same
codec compiled native and on-chip; constrained-random protocol fuzzing; formal safety
properties on the pin/timing ISA; bit-exactness certification of `SIMD` against
Loams; and AI-assisted vector generation from the `.proto` files, legitimate here
because each generated vector is checkable against a production implementation.

## 7. The Loams core, honestly

The ambition was Loams' core on silicon. The source does not support it.

An audit of all ~293k lines found **the entire arithmetic core is roughly 200 lines
across six functions**, and **Loams owns no ANN algorithm at all** — it configures
IVF-PQ and delegates to `lance-index`, configures HNSW/SQ/PQ/BQ and delegates to
`qdrant-edge`. Aggregations have no numeric kernels and delegate to Tantivy. Text is
allocation- and Unicode-table-bound. Vector quantisation is configuration only, in
exact-pinned dependencies.

The one real kernel is `loams-query/src/vector.rs:20-44`, a row-major dense GEMV with
broadcast query — textbook systolic-array input, and deliberately shaped against it.
Decision **D79**, owner-approved, makes summation order part of the public API
contract: *"f64 accumulation in index order, rounded to f32... SIMD kernels differ in
summaration order between libraries."* So no tree reduction inside a row, forcing
row-parallelism with sequential per-lane f64 accumulators, and f64 MACs cost 4–8x the
area of f32.

**At `dim 16`, the dimension Loams' ANN corpus uses, hardware would make it slower.**
The dot product is ~16 multiply-adds while the `PrimaryKey` parse and the
`BinaryHeap` sift cost tens of nanoseconds. And there is **no committed benchmark** —
`bench/results/` is entirely safekeeper durability.

So: `SIMD` ships narrow and its first deliverable is **bit-exactness certification**,
not a throughput claim. A bit-exact software kernel lands first and is benchmarked
before silicon is justified. The audit's own verdict was "do SIMD first, and measure."

The honest answer to "where does Loams silicon go" is **D92 sampled continuous
recall**: 1% of vector queries re-run exactly in the background under a CPU budget,
plus a `POST /recall` endpoint pushing 25–100 sampled vectors through the exact
kernel. Batch, throughput-oriented, f64, and not yet implemented. A follow-on chip.

**No acceleration is claimed for graph, text search or analytical workloads.** There
is nothing to accelerate, and overclaiming loses to a smaller honest entry.

**Loams-side blockers**, recorded now: `Cargo.toml:205` sets
`unsafe_code = "forbid"` workspace-wide, so a DMA/MMIO ring buffer driver cannot be
written without a lint carve-out; and `lance = "=12.0.0"` / `qdrant-edge = "=0.8.0"`
are exact-pinned under a documented lockstep policy.

**One software bug found en route**, worth fixing regardless of hardware:
`loams-qdrant/src/scoring.rs:434-442` computes `key(i)` and `key(remaining[best])`
inside the `argmax` comparison, recomputing a dimension-length dot product per element
inside an already `O(limit · n · dim)` loop.

## 8. Toolchain and flow

**Local.** `rustc`/`cargo` 1.97.1, `protoc` 29.3, `LIBCLANG_PATH` set for bindgen.

**ASIC.** As of 2026-10, with names checked rather than assumed:

- **OpenLane 2 no longer exists under that name.** Efabless shut down; the project
  moved to FOSSi and was renamed **LibreLane** because the namespaces could not be
  acquired. Canonical repo `librelane/librelane`; current 3.0.x. The old
  `openlane2.readthedocs.io` text saying "Classic flow is in beta" is **stale**.
- LibreLane 3.0 supports both `ihp-sg13cmos5l` (the competition node) and sky130.
- **PDK versioning is `ciel`**, the successor to volare:
  `ciel enable --pdk-family sky130 <open_pdks-commit>`. `open_pdks` builds sky130 and
  gf180mcu; IHP comes from `IHP-GmbH/IHP-Open-PDK` and is identified by commit hash.
- **Google's sky130 repos are archived** (2026-04-18 and 2025-05-13). The live
  forks are `fossi-foundation/skywater-pdk*`, with `fossi-foundation/open-pdks` as
  the builder, tagged weekly to twice-weekly.
- Yosys is **0.63** in OpenROAD-flow-scripts master; OpenROAD is versioned
  `2.0-<commit-count>`; KLayout 0.30.9; LibreLane 3.0 pins KLayout 0.30.9, Magic
  8.3.503, Netgen 1.5.287.
- **nextpnr is irrelevant to ASIC.** There is no nextpnr sky130 backend; the open ASIC
  flows use OpenROAD/OpenDB.
- `open_pdks` installs to `/usr/share/pdk`; set `PDK_ROOT`.

**Memory is PDK-conditional and that is a design constraint, not a detail.** sky130
SRAM macros exist (`sky130_sram_macros`, OpenRAM-generated at 43–146 kbit/mm²
depending on port count), but Tiny Tapeout does not bless them and Magic DRC lacks
the SRAM exception rules. IHP has the best open SRAM set — up to **8,820 bits/tile**
from a blessed, DRC-clean macro. The IR therefore emits a macro instantiation only
when the selected PDK has one, and falls back to a generated latch/DFF array
otherwise. A hard-coded SRAM choice is the single most likely way to make the DSL
non-portable.

**The Tiny Tapeout wrapper is unforgiving.** Exactly 26 pins, a fixed port list, and a
globally unique top module name `tt_um_<user>_<project>`. Our emitter generates that
wrapper automatically; it is the most common submission failure.

## 9. Open PDKs and the 3nm question

**Four credible open 3nm PDKs exist. None is fabricable. All require commercial EDA.**

| Kit | Node | Licence | State |
|---|---|---|---|
| PKP3 (PHIMO, Peking) | 3nm nanosheet GAAFET | BSD-3 | most complete; Liberty, 6T SRAM bitcells, PCells; updated Aug 2026; peer-reviewed in *Science China Information Sciences* |
| GT3 (Georgia Tech) | 3nm GAAFET | BSD-3 | 65 cells, 144nm library height; ships LEF and Liberty so **OpenROAD can place it**; DRC/LVS are Synopsys IC Validator |
| USC-3N-2D | 3nm GAAFET | — | built for benchmarking PnR tools in a "3nm-like" environment |
| FreePDK3 (NC State + Synopsys) | 3nm | custom | stale, 2021 |

**IRDS publishes no design kit at all** — only roadmap predictions, which are the
*input* to the predictive PDKs above.

The smallest node usable in an **all-open-source** flow is **7nm**, via ASAP7
(BSD-3, ORFS-supported) — and no shuttle exists for it, so it can never produce
silicon. The irony worth recording: the smallest PDKs anyone can legally touch are
imec's **2nm and 1.4nm** — and they require an NDA and a European affiliation.

**The only nodes with an open PDK *and* a fabricator are 130nm and 180nm.** sky130
and GF180MCU are open because the foundries partnered to grow ecosystems at mature
nodes; there is no commercial incentive whatsoever at 3nm.

**Therefore: 3nm is a research track with no fabrication path, exactly as scoped.**
The engineering-hours-to-first-clean-GDS comparison is roughly 1–4 weeks for sky130
versus 2–6 months for a 3nm research kit, and only the first ends in metal we can
photograph. If we want an advanced-node smoke test that costs nothing, the useful
one is "does our emitted Verilog still synthesise against a FinFET library" using
ASAP7 — not a 3nm milestone.

## 10. Timeline

The competition sets the schedule; the phases are subordinate to it.

| Weeks to 18 Jan | Deliverable | Gate |
|---|---|---|
| 1–2 | `ferrite-lithic` to CMOS5L to GDS, end to end | **De-risk first.** SRAM instantiation working; DSL synthesises and places |
| 3–5 | CPU, pin/timing ISA, firmware loader | UART out of a pin |
| 6–8 | SPI, I2C from firmware | Baseline protocols complete |
| 9–12 | CloudEvents JSON path, event-store framing, `SIMD` + bit-exactness | First protocol translation running |
| 13–15 | Conformance, formal, constrained-random, write-up | Submit |

Week 1–2 is front-loaded because the competition warns that *"a design that looks
small enough after synthesis can still be difficult to route or too slow"*, and a DSL
that cannot hit a real PDK is worthless however good the CPU is.

## 11. Risks

| Risk | Mitigation |
|---|---|
| DSL cannot instantiate CMOS5L SRAM macros | Week-1 spike. Hard blocker, found early |
| Area. 24 tiles is tight; 8x4 (+30%) is only "under consideration" | Design to 24. Keep `SIMD` width and CPU size parameterised so 8x4 is a one-line change |
| No published CMOS5L I/O spec | Email `asic-competition@janestreet.com`. The 26-pin / 66 MHz figures in circulation are **sky130** pads and do not transfer |
| A PIO core is already solved on this node | The entry is the protocol translation, not the state machine |
| Protocol scope overruns | JSON batch path only. Protobuf and HTTP/2 are stretch |
| Loams arithmetic is not where we hoped | §7. SIMD and measure before silicon |
| PicoRV32 licence unsuitable | Fallback is a second permissively licensed small RISC-V, not our own core |
| `unsafe_code = "forbid"` blocks Loams-side drivers | Known; needs a carve-out decision before any integration work |

## 12. Parked

- **sky130 as submission target.** Retained as a local iteration target only.
- **Open 3nm PDK.** Research only; no fabrication path exists or will.
- **Graph, text search, analytical acceleration.** Nothing to accelerate.
- **MIMX 3.1 flash / Strata flash backend.** No such product verifies.

## 13. Open questions for the competition

1. The CMOS5L pad set and I/O budget — published figures are for sky130.
2. Whether Rust-generated Verilog is acceptable submission output.
3. Whether 8x4 tiles is likely to be available.
4. Whether a submission may depend on an external SRAM macro, and whether
   pre-verified SRAM collateral is supplied.
### What the front end settled: a mutable arena cannot have `a + b`

The plan assumed a `Signal` handle with operator overloads, and the arena from
§2 makes half of that impossible. `Circuit` appends to a `Vec`, so its
constructors take `&mut self`, and this does not compile:

```compile_fail
c.add(acc, c.constant(Bits::constant(1, 8)?))?;   // two &mut borrows
```

That is not a papercut; it is the shape of every expression in the language. A
node cannot be built inside another node's arguments. It surfaced immediately as
a compile error in the IR's own test suite while writing the select tests.

There are three ways out, and only one of them keeps errors load-bearing:

| Approach | Why not |
|---|---|
| `&mut self` builders | Nesting stays impossible, which is the whole problem |
| Operators on a `Copy` handle | `a + b` cannot return `Result`, so a width error would have to vanish or panic silently |
| `&self` builders over a shared arena | **Chosen.** Nesting works, and `Result` survives |

So `Design` holds `Rc<RefCell<Circuit>>` and every builder takes `&self`. The
`RefCell` is quarantined in three ways, and each is checkable rather than a
promise:

- **Only the builder is interior-mutable, never a node.** A `Signal` is a
  `NodeId` plus a cached width; it holds no reference *into* the arena, so the
  graph stays as cheap to walk as the IR intended. The IR crate's rejection of
  `Rc<RefCell<..>>` per node still stands.
- **The borrow cannot overlap.** Every builder takes the borrow inside one method
  body that calls no user code and re-enters no builder, so there is no path to a
  `RefCell` panic. A panic would need re-entrancy.
- **It is paid for once.** Operators are implemented on `&Signal` and recover
  their design from the left operand's arena, which is why `Signal` and `Design`
  share a small `Arena` struct rather than a bare `RefCell<Circuit>`.

The operators are a second, *concise* tier that panics, because `impl Add` has
one output type and a width error has to go somewhere. Both tiers call the same
builder, so there is one implementation and `tests/operators.rs` compares the
node kind and width of both paths for every operator. The panic keeps the
builder's own message and `#[track_caller]`, so it names both widths and points at
the user's line — `tests/operators.rs` asserts the *file* the panic is attributed
to, which is the half of `track_caller` that survives `rustfmt`.

Comparisons deliberately have **no** operator. `==` is `PartialEq` on `Signal`
and answers "is this the same node", a question about handles; overloading it for
value equality would make `assert_eq!(a, b)` mean two different things in two
places. The value comparison is `Signal::equals`.

#### Three more bugs, and the shape they share

All three were in code that looked right, and all three were found by tests
written to be awkward:

| Bug | Symptom | Caught by |
|---|---|---|
| `Circuit::write_port`/`read_port`/`memory_shape`/`write_ports_of` reported `NotAWire` for a non-memory | A wire *is* a legal `drive` target, so the message told the reader to do something that would not help | four tests asserting `NotAMemory`, next to one asserting a bad `drive` still says `NotAWire` |
| `instance("")` reported `ZeroWidth` | "width must be at least 1 bit" for a missing name is a non-sequitur; the name becomes the Verilog identifier | a test asserting the message names the problem |
| `Design::concat` folded two parts at a time without accumulating | `concat` of three or more parts **silently dropped everything after the second** — the width came out 8 instead of 12 | the three-way concat test, after the interpreter in `tests/structural.rs` made the wrong value visible |

The `concat` one is the one worth remembering: its width assertion would have
caught it, and a width assertion alone would only have caught *that* case. The
general rule is the same one §4 and §5 keep arriving at — **assert the value, not
just the shape.** So `tests/structural.rs` contains a small constant-folding
interpreter and compares every structural operation against `Bits` over random
inputs. A shift desugared into `select` plus `cat` plus a constant can have all
the right widths and the wrong bits, and nothing else would notice until `-sim`.

#### What the front end forced back into the IR

Three changes, each because the DSL could not be written without them:

- **`Node::Select`.** §1 requires shifts to stay structural, desugaring into
  `select` plus `cat` plus a constant — and there was no select node. Without it
  there is no slice, no shift, no extension, and no truncation, which is most of
  what a datapath is.
- **`Circuit::check_with_inputs`.** An input port is an undriven wire by
  construction, so `check` rejected the flagship accumulator: `clk` was read and
  undriven. Inferring ports from a name would have been a guess; the front end
  collects the list from `Design::input` and passes it. `check` still rejects an
  undriven wire that is *not* a port, and a test asserts that, because a blanket
  exemption would stop catching real mistakes.
- **`NotAConstant`.** `Design::adopt` copies a signal from another design. Only a
  constant has a single value to copy, so anything else is refused — copying an
  operation as zero would quietly build the wrong circuit.

## What the cycle simulator settled

`ferrite-lithic-sim` compiles a `Circuit` into word addresses and an `Op` list,
then steps it. Three decisions in it are worth writing down, and so are the four
bugs that the tests found, because three of those four were invisible in the code.

### Counting has to come before placing

The flat buffer is partitioned into sections in a fixed order — comb, regs,
regs_next, mems, consts — so a node's absolute word depends on the size of every
section in front of it. Placing as the allocator walks therefore does not work: a
memory visited early does not yet know how wide the combinational logic ahead of
it turned out to be.

The first version did exactly that, and the counters started at zero. Registers and
memories were addressed at word 0, on top of the combinational section. Every
signal still had a plausible address and every test still compiled; the accumulator
simply never moved. The fix is to count words per section first and assign absolute
addresses in a second pass, in arena order so the layout stays canonical.

The lesson is the same one as everywhere else in this file: **a wrong word address
is not a crash, it is a plausible number**, and the only thing that catches it is an
assertion about behaviour rather than about shape.

### Two implementations per operation, deliberately

Every operation has a hand-written `u64` path for values that fit in one word, and
a fallback through `Bits`. That looks like duplication and is the opposite: the fast
path exists because nearly every signal is one word wide and a `Bits` operation
allocates, and the slow path exists because add-with-carry and schoolbook multiply
are already written, already property-tested, and already agreed on by the rest of
the stack. Duplicating them to save an allocation would have put two
implementations of the same semantics in the tree, which is precisely the thing §7
of the design notes refuses to inherit from Hardcaml.

`tests/ops.rs` checks both paths against `Bits` over widths 1..=200. The upper end
matters because the boundary between the paths is at width 65, so a range that
stopped at 64 would only ever exercise the fast one; the lower end matters because
Hardcaml's multiply assumes operands span two words with no assertion, covered only
by an empirical sweep.

### Refusing beats guessing, three times over

`Instance`, a second clock, and division by zero are all errors rather than
answers. The reasoning is the same in each case: the alternative produces a
simulation of a circuit the user did not write, and it does so *quietly*. An
instance treated as a constant zero still passes every test in this file.

Division by zero is the sharpest of the three, because emitted Verilog answers `x`
and a two-state simulator cannot. Refusing keeps the mistake visible; answering zero
would hide it until the RTL disagreed. That disagreement is real and will have to be
handled in `ferrite-lithic-cosim`, which is where it belongs.

### The four bugs

| Bug | Symptom | Caught by |
|---|---|---|
| Section cursors started at zero | Every register and constant sat on top of the combinational section; the accumulator never moved | the first smoke test, which was the whole design |
| The constant section was allocated but never written | `add` read a literal of zero, so `x + 1` was `x` | the same smoke test, printed as a buffer dump |
| The ten comparisons stored their *result* width instead of their operand width | `sle` sign-extended from **1 bit** rather than 2, so `-1 <= -2` was true | the differential property, at width 2 |
| `select`'s fast path checked that the *result* was one word, not that the *window* was inside the first | a 66-bit value sliced at 47 for 18 bits lost its top bit | the same property, at width 66 |

The third is the one that would have survived longest. A comparison is one bit wide,
and the code storing "the width of this node" looked obviously right. Signed
arithmetic needs the width of the values being reinterpreted, which is a different
number, and the two agree for every operation except comparisons.

The fourth is the general shape of this kind of bug: the fast path's guard has to be
about the thing the fast path actually assumes. Here it assumed the *result* fit in
a word, which is true, and read a single word, which is only true when the *window*
is inside the first.

### The generator, and guarding the generator

Hardcaml's random-circuit generator lifts almost directly, and the design notes
call the result the highest-value test in the suite. What lifts here is the
generator-quality guard rather than the differential test, because we have no second
graph to compare against yet: Hardcaml uses its generator to check deduplicated
against non-deduplicated circuits, we have no deduplication, and the comparison that
matters — the simulator against emitted Verilog — waits for `-rtl` and `-cosim`.

The guard is the half that survives. A random-circuit test that generates only
trivial circuits asserts nothing while looking rigorous, so the fixed seed `0x3eadbeef`
is run over 1000 circuits and more than two thirds of them must produce a changing
output. That threshold is a property of the generator, not of the simulator, and it
is the thing that would rot silently.

One narrowing came out of writing it: division is weighted down and its denominator
is forced non-zero. A uniform choice over fourteen operations spends an embarrassing
fraction of a run dividing by zero, which tests the refusal path very well and
nothing else.

## What the Verilog emitter settled

### Determinism is structural, not defended

The reference needs a pass to make its output reproducible. Node identity there is
a `uid` from a process-global mutable counter, so two runs only agree if the program
runs the same way twice, and the fix — `normalize_uids`, rewriting uids into DFS
pre-order — is off by default with its own test commented out as "brittle to changes
in the test environment". An arena makes the whole question disappear: ids *are*
indices and indices are construction order, so walking the graph in id order is
already canonical. What is left is the naming rule, and that is three states —
ports, then named signals, then unnamed ones in graph order, deliberately not
depth-first. Two arenas built the same way emit the same bytes, and that is a test
rather than a hope.

### The module name is legalised, and only `iverilog` could say so

Every other identifier gets rewritten when it cannot be spelled in Verilog, and the
module name was emitted verbatim: `Module::new("signed", ..)` produced a file whose
third line was `module signed (`, which `iverilog` rejects with a syntax error. A
golden could not have found it, because a golden reading `module signed (` looks
exactly like one reading `module signed (`. The name is now legalised where the
module is built, and the instantiation site legalises the submodule name the same
way -- otherwise `module signed_` would be instantiated as `signed`, and the error
would be about a missing module rather than about the name. It is still not
*mangled*: no pool of module names to take a free one from, and a submodule has to
stay findable under the name its instantiation site spells.

Installing `iverilog` then paid for itself twice over, because the first design in
the syntax gate registered the output of an adder whose other input was the register
itself -- a cycle, which in a graph is just a wire nobody drives. The front end said
so, but only because a tool was there to ask.

### One wire per node, and therefore no precedence table

The first version inlined operands, and it needed a precedence table to parenthesise
the result. Then it became obvious that every reachable node has a wire anyway, so
the inlining bought nothing and cost a whole precedence layer plus a class of
parenthesisation bug. Emitting `assign signal_Add = a + b;` instead of
`assign x = a + b + c;` means each line of Verilog is one line of the design and the
emitter has nothing to parenthesise. Constants stay inline, because a constant *is*
its value.

### The register block is where the IR and the reference disagree

The reference emits `else if (clear) q <= clear_to;`. We have no `clear_to` field,
the front end documents clear as gating the update, and the simulator implements
hold — so the emitter writes `q <= q;`. A zero there would have been a register
that disagrees with its own simulator, and nothing in the test suite would have
caught it: the goldens are text, and the cosimulator does not exist yet. That is the
argument for reading the emitter's output line by line against the simulator's
semantics rather than against the reference's shape.

The same pass caught a documentation bug rather than a code one. The front end's
canonical accumulator ties `clear` high, which means it holds forever and never
accumulates — correct per the truth table, wrong for an example that calls itself an
accumulator. The emitted `if (1'h1) q <= q;` made it impossible to miss.

### The instance ABI had to be invented, because the IR has nowhere to put it

`Node::Instance` records a submodule's name, its parameters and its inputs' order.
It does not record the names of the submodule's ports, because the graph has no
field for them. So an instantiation site cannot name what it connects to, and a
submodule emitted with its own design's names cannot be bound by one. Rather than
guess at names, the ABI is positional — `.i0`, `.i1`, …, `.o0` — and
`PortNames::Positional` emits a submodule under exactly those names, reserving them
before any user name so a signal called `i0` is renamed rather than shadowing a
port. The reference's packed-output vector degenerates to the single output we have
and becomes visible again when instances grow bundles.

### Six refusals, and the two the reference never has to make

A literal clock, a second clock domain, a stateful module with no clock, a whole
memory used as a value, a port in both directions, and a port with no name. The last
two are cases the reference never meets because it never has to describe a module
from outside, and the first three are cases where a plausible-looking line of
Verilog would compile and mean a different circuit.

Module dedup is the one place the crate is weaker than the reference, and it is
weaker for a knowable reason: the reference interns definitions in a table and can
compare them, and `Circuit` has no `PartialEq`, so identity here is the name plus
the port signature. Two different circuits with the same name *and* the same ports
collapse to the first, undetected. Giving `Circuit` a `PartialEq` is the fix, and it
belongs to the IR crate rather than here.
