# Ferrite Lithic: details

Everything known about this project in one place. Companion documents:

- `docs/design-notes.md` — the IR, simulator and emitter design, read from Hardcaml's source
- `docs/plans/jane-street-protocol-emulator.md` — the competition plan and timeline
- `docs/adr/` — 17 accepted architecture decision records
- `../ferrite-strata/DETAILS.md` — the companion runtime

Status: **Phase 0 complete**, design settled, no crates published yet.

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
| `ferrite-lithic-bits` | Fixed-width bitvectors over `u64` words, runtime widths | A1 |
| `ferrite-lithic-ir` | Arena of nodes with `NodeId`, wires resolved after construction | A1 |
| `ferrite-lithic` | Front-end DSL: `Signal` with operator overloads and builders | A1 |
| `ferrite-lithic-sim` | Cycle simulator over a flat, topologically sorted op list | A1 |
| `ferrite-lithic-rtl` | Verilog emitter with stable naming | A1 |
| `ferrite-lithic-derive` | `#[derive]` for port-list structs | A2 |
| `ferrite-lithic-wave` | VCD waveform writer | A2 |
| `ferrite-lithic-cosim` | Verilator equivalence against `ferrite-lithic-sim` | A2 |
| `ferrite-lithic-tb` | Coroutine-style step testbenches | A3 |

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