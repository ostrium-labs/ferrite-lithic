# Plan: protocol emulator ASIC for the Jane Street competition

Target, hardware, and the firmware that will run on it. Written against the
competition rules verbatim and against the actual Loams source, not against what
the arithmetic *should* be.

Status: draft, 2026-10-04. Deadline **18 January 2027**.

## 1. Target

From the Jane Street blog post, *"Can you design a chip? Announcing the protocol
emulator ASIC competition"* (10 Sep 2026), verified against the source:

| Item | Value |
|---|---|
| Process | IHP **130nm CMOS5L** (`ihp-sg13cmos5l`) via Tiny Tapeout |
| Flow | `ttihp-verilog-template`, branch **`cmos5l`**, RTL to GDS via LibreLane |
| Tiles | `info.yaml` = **6x4** = 24 tiles, ~0.7 mm², ~1K logic cells/tile |
| Register file | ~320 DFF/tile |
| Instruction memory | **SRAM preferred over flip-flops**; ~3.7–8.8 Kbit/tile → **~11–26 KB** |
| Submission | **18 Jan 2027**. Shuttle target March 2027; chips after fabrication |
| Open source | Required. Build in public explicitly allowed |
| Contact | `asic-competition@janestreet.com` |

The brief: a *"general-purpose protocol emulator"* — *"a tiny CPU with an
instruction set designed for reading pins, writing pins, counting cycles, and
hitting timing precisely enough that you can implement a real protocol in
firmware rather than in fixed logic."* Baseline UART, SPI, I2C; stretch low-speed
USB and 10 Mbit Ethernet; also JTAG, SWD, PS/2, CAN.

Two rules from the post shape everything below:

> "The goal isn't to put a UART block, an SPI block, and an I2C block on one die
> and call it done. Your chip should be **reprogrammable enough to support new
> protocols after fabrication**."

> "We're particularly interested in projects with **unique functionality**, as
> well as those that demonstrate **novel approaches to design and verification
> methodologies**… including **formal methods, random constrained tests,
> AI-assisted verification**."

**sky130 is not the submission target.** It stays as a fast local iteration
target — same 130nm class, different standard-cell library and DRC/LVS decks, so
cross-porting is real work. CMOS5L is what gets submitted.

## 2. Why this entry can win

Prior art on this exact node, all submitted to Tiny Tapeout IHP 26b already:
`tt_um_fabien_pio` (AstraPIO), `tt_um_catalinlazar_nanopio` (nanoPIO),
`tt_um_mini_kraken`, `tt_um_jet_seq8b` (SEQ8). **A PIO state machine is a solved
problem here.** Building one cannot place.

Nobody on that list is running a production event-streaming protocol stack. That
is the entry: implement Loams' protocol translation from firmware on a chip that
has no protocol logic in silicon at all.

Reference for judging style: the Advent of FPGA challenge (Nov 2025 – Feb 2026,
213 submissions, judged by the same two people) rewarded *"detailed write-ups
that others can reference"* and *"trickier solves that took the solution further
in some dimension."* 151 of 213 submissions used Hardcaml. **A Rust-authored
DSL is a legitimate differentiator, not a liability.**

## 3. Architecture

RISC-V core plus a custom pin/timing ISA. Firmware holds every protocol.

**Core: PicoRV32 (RV32I).** At roughly 750 LUTs it is free inside 24K cells, it
is already verified, and the core is not where novelty lives. Writing our own CPU
would spend schedule on the least differentiated part of the design. Licence to
confirm before committing.

**Custom instructions.** Each maps directly to a phrase in the brief:

| Instruction | Purpose | Brief's words |
|---|---|---|
| `PIN_OUT rd, pin` / `PIN_DIR` | drive and tri-state a pin | "writing pins" |
| `PIN_SAMPLE rd, pin, delay` | sample after N cycles | oversampling, without which UART and I2C are impossible |
| `PIN_EDGE` + oversample config | edge detect, programmable rate | I2C ack, clock stretching |
| `CYCLE` | read the cycle counter | "counting cycles" |
| `WAIT_CYCLES` | delay | "hitting timing precisely" |
| `SIMD` | 8x8 int8 MAC / dot product | see §5 |

**Firmware image in SRAM**, per the post's own advice. This is the item most
likely to block us: `ferrite-lithic` has no blackbox-instantiation story yet, so
SRAM macro instantiation is week-1 work.

## 4. What runs on it

Ladder in the order the post recommends — *"Start by getting a UART transmitter
out of a pin. Then make it programmable."*

1. **UART** transmit from firmware, then receive, then a framed link.
2. **SPI** master, then **I2C** master with open-drain ack handling.
3. **Protocol translation from firmware.** The contract already exists in the
   repo: `crates/loams-stream-grpc/proto/loams/stream/v1/stream.proto`, whose
   own comment reads *"Native stream ingest, used by protocol adapters through
   Dapr invocation."*

   ```protobuf
   service StreamService { rpc Produce(...); rpc ProduceCloudEvents(...); }
   ```

   It accepts CloudEvents as protobuf *or* as
   `application/cloudevents-batch+json`, the latter annotated as *"a pass-through
   for senders that already hold JSON, such as Dapr."*

   **The JSON batch path is the firmware target.** Plain JSON over a framed
   serial link avoids both protobuf varints and HTTP/2 HPACK, which is what makes
   it fit 11–26 KB of program memory. gRPC/HTTP2 is the stretch path once the
   basics hold. EventFabric is a further target: it is CloudEvents-native, so the
   same batched-JSON codec is reused.
4. **Stretch:** 10BASE-T Ethernet, which is what would let a real TCP/IP stack
   carry real Dapr traffic. JTAG and SWD are cheap wins — pure pin protocols that
   prove the ISA generalises beyond the three named baselines.

## 5. The Loams core, honestly

The ambition was to put Loams' core on silicon. The source does not support it,
and the plan says so rather than pretending otherwise.

An audit of all ~293k lines found **the entire arithmetic core is roughly 200
lines across six functions.** Specifically:

- **No ANN algorithm is owned.** `loams-collection/src/index_build.rs` configures
  `ivf_pq` / `ivf_rq` / `with_ivf_hnsw_sq_params` and hands the work to
  `lance-index`. `loams-hnsw/src/qdrant.rs` configures HNSW/SQ/PQ/BQ and hands it
  to `qdrant-edge`. No graph traversal, no PQ codebook distance, no k-means, no
  RaBitQ.
- **No numeric aggregation kernels.** `loams-query/src/exec/aggs.rs` rewrites
  requests and delegates to Tantivy collectors. Every aggregate is external.
- **No text arithmetic.** Porter stemming allocates a `Vec<char>` per word;
  tokenisation walks Unicode property tables; the rest is posting-list traversal.
- **No vector quantisation.** Only configuration; PQ/SQ/BQ are in the pinned
  dependencies.

The one genuine kernel is `crates/loams-query/src/vector.rs:20-44` — dot, cosine,
Euclid and Manhattan with **f64 accumulation in strict index order**, rounded to
f32. `batch_rows` presents it as a row-major dense GEMV with a broadcast query
vector, which is the textbook systolic-array input.

**And it is deliberately shaped against that.** Decision **D79**, owner-approved
(`docs/design/13-decision-log.md`):

> "the exact vector kernel scores every vector result (f64 accumulation in index
> order, rounded to f32) … SIMD kernels differ in summation order between
> libraries"

Consequences, all binding:

1. **No tree or pairwise reduction inside a row.** The add chain must stay
   sequential in index order. That forbids wide intra-row systolic GEMV and
   forces **row-parallelism** instead: many candidate rows in flight, each with
   its own sequential f64 accumulator, sharing a broadcast query register.
   Still a good ASIC shape — just not the most efficient one.
2. **f64 is non-negotiable** today. f16/i8 are deferred to M2 under D94. f64 MACs
   cost 4–8x the area of f32.
3. **Bit-exactness is a gate, not a goal.** Any accelerator must be certified
   against `score()` on random inputs including subnormals, intermediate
   infinities and `-0.0`. Note `loams-hnsw/src/flat.rs` adds `+ 0.0` to normalise
   `-0.0`, so the sign of zero **is** observable in tie-breaking.
4. **Changing summation order is a ruling-level API change**, breaking the
   "results identical with hot tier on/off" and "BM25 scores bit-identical"
   gates. It is not an optimisation decision.

**At the dimension Loams actually uses, hardware loses.** The ANN corpus is
`dim 16`. There the dot product is ~16 multiply-adds while the `PrimaryKey` parse
(`ann.rs:203`) and the `BinaryHeap` sift in `TopK::push` cost tens of
nanoseconds. At `dim 768` and up it flips. **There is no committed benchmark** —
`bench/results/` contains only safekeeper durability runs, no search or ANN
profile — so the production dimension distribution is unknown, and that is the
single largest uncertainty in this analysis.

**Therefore, for this submission:**

- The `SIMD` instruction ships **narrow and parameterised**, and its first job is
  **proving bit-exactness** against `vector.rs:20-44`. That is a genuine and
  cheap verification result: a hardware instruction certified bit-for-bit against
  a production Rust function that has an owner-approved ruling about summation
  order. It also forces us to confront D79 early rather than discover it late.
- **A software kernel lands first.** Write the bit-exact multi-row rescore
  kernel, benchmark it against `score()` at dim 16/128/768/1536 with realistic
  key widths, and only then argue about silicon. The audit's own verdict was
  "do SIMD first, and measure." A missing benchmark is a measurement problem, not
  a hardware problem.
- **Claim nothing for graph, text search or analytical acceleration.** There is
  nothing to accelerate, and overclaiming here is how an entry loses to a
  smaller honest one.
- **The honest answer to "where does Loams silicon go" is D92**, not this chip:
  sampled continuous recall, re-running 1% of vector queries exactly in the
  background under a CPU budget, plus a `POST /recall` endpoint pushing 25–100
  sampled vectors through the exact kernel. Batch, throughput-oriented,
  latency-insensitive, f64 — the ideal accelerator shape, and it does not exist
  in the code yet. That is a follow-on chip.

**Loams-side blockers** for any future integration, recorded now:

- `Cargo.toml:205` sets `unsafe_code = "forbid"` workspace-wide. A DMA/MMIO ring
  buffer driver cannot be written in that workspace without a lint carve-out.
- `lance = "=12.0.0"` and `qdrant-edge = "=0.8.0"` are exact-pinned, with a
  documented lockstep policy (D51). The two libraries owning the interesting
  vector math are immovable without a fork.

**One software bug found on the way**, worth fixing regardless of hardware:
`loams-qdrant/src/scoring.rs:434-442` calls `key(i)` and `key(remaining[best])`
*inside* the `argmax` comparison, recomputing a dimension-length dot product on
every element, inside an already `O(limit · n · dim)` loop. An accidental ~2x
recompute. Fixing it is worth more than any accelerator.

## 6. Verification — the differentiator

The post singles verification out as where the field is heading. We have
something no other entrant does: **a production conformance suite to validate
against.** `loams-meta-conformance` is 4,488 lines in the repo today.

- **Differential testing.** The same protocol codec compiled native and on-chip,
  run against identical conformance vectors. A mismatch is a hardware bug with a
  reproducible witness, not a hex dump to be eyeballed.
- **Constrained-random protocol fuzzing.** Mutate CloudEvents batches, Iggy and
  Fluss frames; assert the chip and the host codec agree, and that neither
  panics nor hangs.
- **Formal.** Safety properties on the pin/timing ISA, where the interesting
  claims are safety — "never miss an ack edge", "never drive a pin while it is
  still an input" — not liveness.
- **Bit-exactness certification** of the `SIMD` instruction against
  `vector.rs:20-44`, including subnormals, infinities and `-0.0`. See §5.
- **AI-assisted verification**, explicitly welcomed: generating conformance
  vectors from the `.proto` files. Legitimate here precisely because each
  generated vector is checkable against a production implementation rather than
  taken on trust.

## 7. Timeline, 15 weeks to 18 January

| Weeks | Deliverable | Gate |
|---|---|---|
| 1–2 | `ferrite-lithic` → CMOS5L → GDS, end to end | **De-risk first.** SRAM macro instantiation working; our DSL synthesises and places |
| 3–5 | CPU, pin/timing ISA, firmware loader | UART out of a pin |
| 6–8 | SPI, I2C from firmware | Baseline protocols complete |
| 9–12 | CloudEvents batched-JSON path, event-store framing, `SIMD` + bit-exactness | First protocol translation running |
| 13–15 | Conformance, formal, constrained-random, write-up | Submit |

Week 1–2 is front-loaded deliberately. The post warns that *"a design that looks
small enough after synthesis can still be difficult to route or too slow"*, and a
DSL that cannot hit a real PDK is worthless however good the CPU is.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Our DSL cannot instantiate CMOS5L SRAM macros | Week-1 spike. Hard blocker, found early |
| Area. 24 tiles is tight; 8x4 (+30%) is only "under consideration" | Design to 24. Keep `SIMD` width and CPU size parameterised so 8x4 is a one-line change |
| No published CMOS5L I/O spec | Email `asic-competition@janestreet.com`. The 26-pin / 66 MHz figures in circulation are **sky130** pads and do not transfer |
| A PIO core is already solved on this node | The entry is the protocol translation, not the state machine |
| Protocol scope overruns | JSON batch path only. Protobuf and HTTP/2 are stretch, and Iggy/Fluss framing is a later rung |
| Loams arithmetic is not where we hoped | §5. Do not overclaim; SIMD and measure before silicon |
| `unsafe_code = "forbid"` blocks Loams-side drivers | Known; needs a carve-out decision before any integration work |

## 9. Parked

- **sky130 as submission target.** Retained as a local iteration target only.
- **Open 3nm PDK.** Research only; no fabrication path is expected to exist.
- **MIMX 3.1 flash / Strata flash backend.** No such product verifies. If the
  i.MX RT reading was meant, there is still no Rust-native stack for xSPI-octal
  LUT sequences, on-die SECDED ECC, or the FlexSPI FCB — a clean Strata backend
  when wanted. One trap to carry: on the Infineon parts, programming the same
  16-byte half-page twice after an erase silently disables ECC.
- **Graph, text search, analytical acceleration.** Nothing to accelerate.

## 10. Open questions for Jane Street

1. Confirm the CMOS5L pad set and I/O budget — the published figures are for
   sky130.
2. Confirm Rust-generated Verilog is acceptable submission output.
3. Whether the 8x4 tile size is likely to be available.
4. Whether a submission may depend on an external SRAM macro, and whether
   pre-verified SRAM collateral is supplied.