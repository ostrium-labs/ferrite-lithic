# Measured graph metrics

Every figure below is derived from the IR by `tests/graph_metrics.rs`, for every design, by
the same code. Regenerate the tables with:

```sh
cargo test -p ferrite-lithic-corpus --test graph_metrics -- --nocapture --ignored
```

## What each column is

| Column | Meaning | What it is not |
|---|---|---|
| `nodes` | `Circuit::len()` — one IR node, one operator | **Not gates.** One `add` is a whole adder. |
| `flops` | Nodes where `Node::is_stateful()`, one per register | **Not bits.** See `state bits`. |
| `state bits` | Summed width of those registers | The real memory cost. 5 flops holding 128 bits is not 5 bits. |
| `depth` | Longest combinational chain, in **operator levels** | **Not gate delays.** Four wide operators is four levels, not thousands of gates. |
| `mem bits` | `depth × width` of every `Mem` node | See the finding below: it is zero everywhere. |

Area, power and frequency are **not derivable from the IR** and are not claimed anywhere in
this project. A number here is a fact about the graph; what a synthesiser builds from that
graph is a different question needing a different tool.

## The table

```
design     variant                 nodes  flops state bits   depth  mem bits
--------------------------------------------------------------------------------
crc3       3-bit CRC                  72      1          3      33         0
crc32      CRC-32                     80      1         32      34         0
ghash      128-bit field              62      6        393       7         0
sha256     64 unrolled rounds       6090      8        256     644         0
chacha20   one block/cycle          3347     16        512     484         0
aes        200-arm S-box            2459      4        128      98         0
hex        4 bits in, 8 out           46      2         10       6         0
base64     6 bits in, 8 out          122      5         19       8         0
memchr     comparator                 31      3         17       6         0
memchr     256-entry ROM             287      3         17       6         0
aho_corasick 19 states                 271      4         17       6         0
dfa        5 states, 64 symbols      377      3         12      11         0
bitpack    expander, w=8             255      1          7       8         0
rle        decoder                    21      2         16       4         0
roaring    bitmap                     71      4         68      13         0
hamming    popcount64               1028      1          8      24         0
hamming    top-k, k=7                 36      4         32       7         0
sorting    bitonic, n=8              106      8         64      13         0
sorting    odd-even merge, n=8        91      8         64      13         0
sketch     64-bit state              154      1         64       5         0
packet     frame in/frame out        443     25        329      59         0
fse        tableLog 5                197     10         52      13         0
huffman    DEFLATE fixed code        626      5         27      21         0
deflate    bit layer only             40      2         12       5         0
```

## Node kinds

The histogram is how the hand-derived estimates in the design notes can be checked. The
design notes say things like "roughly 6 700 muxes" for the Huffman table; the measured
count of `Case` and `Ite` nodes here is far smaller, because the IR shares subexpressions
that a gate-level expansion would duplicate.

```
crc3       3-bit CRC              Select 24, BitXor 16, Constant 10, Cat 8, Ite 8
crc32      CRC-32                 Select 24, BitXor 17, Constant 13, Ite 9, Cat 8
ghash      128-bit field          Wire 17, Constant 11, Ite 11, Reg 6, Select 6
sha256     64 unrolled rounds     Constant 1336, Cat 1248, Select 1248, BitXor 640, Add 600
chacha20   one block/cycle        Cat 992, Select 976, Constant 677, Add 336, BitXor 320
aes        200-arm S-box          BitXor 1066, Constant 555, Select 320, Case 200, Cat 156
hex        4 bits in, 8 out       Constant 22, Wire 8, Select 6, Ite 3, Case 2
base64     6 bits in, 8 out       Constant 77, Wire 12, Select 8, Ite 7, BitOr 5
memchr     comparator             Wire 10, Constant 8, Ite 6, Reg 3, Add 1
memchr     256-entry ROM          Constant 264, Wire 10, Ite 6, Reg 3, Add 1
aho_corasick 19 states              Constant 232, Case 22, Wire 9, Reg 4, Select 2
dfa        5 states, 64 symbols   Constant 337, Wire 8, Case 6, Select 6, Add 5
bitpack    expander, w=8          Select 117, Constant 63, Cat 58, Wire 11, Add 1
rle        decoder                Wire 10, Constant 4, Ite 3, Reg 2, Sub 1
roaring    bitmap                 Constant 17, Ite 10, Wire 10, Select 8, Cat 7
hamming    popcount64             Select 382, Constant 255, Cat 254, Add 127, Wire 7
hamming    top-k, k=7             Ite 12, Wire 10, Ult 6, Constant 4, Reg 4
sorting    bitonic, n=8           Ite 48, Ult 24, Wire 18, Constant 8, Reg 8
sorting    odd-even merge, n=8    Ite 38, Ult 19, Wire 18, Constant 8, Reg 8
sketch     64-bit state           Constant 133, Wire 9, BitAnd 3, BitOr 2, Case 2
packet     frame in/frame out     Select 118, Constant 94, Cat 74, Wire 35, Reg 25
fse        tableLog 5             Constant 82, Wire 24, Ite 22, Select 13, BitAnd 12
huffman    DEFLATE fixed code     Constant 542, Select 20, Cat 16, Wire 16, Ite 8
deflate    bit layer only         Wire 13, Constant 10, Ite 6, Eq 3, Reg 2
```

## How much slower than the software? (×-ratios)

Run in **release** — in a debug build the software side is several times slower, which
would flatter the hardware for a reason that has nothing to do with the hardware:

```sh
cargo test -p ferrite-lithic-corpus --test speed --release -- --nocapture --ignored
```

### The comparison, and why a ratio needs a clock

The two sides are in different units. The **design** is measured in *cycles* — exact, and a
property of the design. The **software** is measured in *nanoseconds* — wall clock, and a
property of this host. Dividing them requires converting cycles to time, which requires a
clock frequency, and there is no way around it. So every ratio below is **at 1 GHz**, where
one cycle is one nanosecond, and the frequency is stated rather than buried.

### The software side is the real crate

For each row the reference is the crate that *defines* the algorithm — `crc32fast`, `crc`,
`hex`, `base64`, `memchr` — already present as a dev-dependency for exactly this purpose.
Not a reimplementation written for the benchmark, which would measure the benchmark
author's skill rather than the design's worth.

And each row **asserts that both sides produce identical output before either is timed**.
Without that, a fast design would win by doing less work. `hex` and `base64` compare their
entire emitted output, not a single sampled character.

### The measured ratios

```
design   software    cyc/unit    design ns       sw ns  verdict
------------------------------------------------------------------------
crc32    crc32fast       1.00         1.00      0.1157  software 8.6x faster
crc3     crc             1.00         1.00      2.2392  HARDWARE 2.24x faster
hex      hex             3.00         3.00      4.1424  HARDWARE 1.38x faster
base64   base64          5.33         5.33      0.3876  software 13.8x faster
memchr   memchr          1.01         1.01      0.2383  software 4.2x faster
```

**Two of the five designs beat the software at 1 GHz.** That is not the result the project
expected to report, and it is why the benchmark exists.

- `crc3` is **2.2× faster** than the `crc` crate. Three flops and one byte per cycle is a
  very small circuit, and a general-purpose CRC library spends more time per byte on width
  bookkeeping than the arithmetic.
- `hex` is **1.4× faster**. Three cycles per byte against a crate that does the same work
  with a table lookup and some branching.
- `memchr` loses by only **4.2×**, against AVX2 doing 32 bytes per instruction.
- `base64` loses by 13.8× and `crc32` by 8.6×.

## Why is the hardware slower? (and when isn't it?)

Because the comparison is rigged, and knowing *how* it is rigged is the whole lesson.

### The hardware is thirty-one nodes. The CPU is billions of transistors.

Look at the measured table again for `memchr`: **31 nodes, 3 flops, 17 bits of state.**
That is the entire circuit. An AVX2 `memchr` is a core with hundreds of millions of
transistors doing 32 bytes of comparison per instruction on a die built on a 5 nm process
with a deep memory hierarchy behind it.

So `memchr` losing by 4.2x is not "hardware is slow". It is *one fingernail-sized circuit
against an entire CPU core*. The ratio is not measuring the quality of the design; it is
measuring how much silicon the design was allowed to have. Which is the honest way to read
every number in the table above.

### The CPU amortises everything; the circuit pays every cycle

Software runs the algorithm once and then applies it to a megabyte in a tight loop. After
the first pass, the code is in instruction cache, the table is in L1, the working set is in
registers, and the branch predictor has learned the pattern. **None of that setup is
charged per byte.**

A hardware design cannot amortise anything, because it has no program to amortise. Its
state *is* the program. Every bit of it costs a flip-flop, and the state has to be written
on every cycle the design processes data. That is why `crc3`'s three flops and
`crc32`'s thirty-two are not incidental — they are the design's entire memory, and they are
why the designs are small.

### The software is allowed to be cleverer than the hardware

This is the deepest reason, and it is worth stating plainly because it inverts the usual
hardware intuition.

**A hardware design must handle every input.** A software function is free to be
*asymmetric*: to process 32 bytes at once, to detect the common case and skip work, to
specialise a loop for a buffer length it can see, to use a branch the hardware cannot
afford to make. Hardware has no equivalent of "and in this case, do the thing that is
usually right" — a mux is a mux, and you pay for it whether or not the select is ever
anything but zero.

Concrete examples in this corpus:

- **The `memchr` crate does 32 bytes per AVX2 instruction.** `memchr`'s design does one
  byte per edge, because one byte per edge is what the algorithm needs and that is all it
  does. Neither is a mistake; one of them had 32x more room.
- **`base64` processes three bytes in four sextet-cycles.** The software processes three
  bytes at a time too, but does it with a table lookup and branch-free arithmetic, and gets
  0.39 ns/byte against the design's 5.33 cycles.
- **`crc32` is a serial accumulator** in this design. The recurrence is linear, so the
  software can — and vectorised CRC implementations do — fold several bytes together and
  break the dependency chain. The design as built cannot, and adding a pipelined variant is
  a different design.

### So why do `crc3` and `hex` *win*?

Because the win comes from doing less, not from going faster.

`crc3` beats the `crc` crate 2.2x. The crate is a general-purpose CRC library: generic over
width, handling arbitrary polynomials, with per-byte machinery for a computation whose real
state is **three bits**. The design is three flops, a table, and one byte per edge, with
none of the generality to pay for. `hex` wins for the same reason against a smaller margin.

The general shape of a result:

> Hardware loses when the CPU has more parallelism available than the design uses, and wins
> when the design's *smallness* is the point.

### The load-bearing assumption: 1 GHz, and `crc32` will not make it

The table is quoted at 1 GHz so the ratio is computable at all. But `crc32`'s measured
**combinational depth is 34 operator levels** — a table-driven CRC is 32 XORs deep. Real
logic at that depth does not close at 1 GHz on a modern process; it would run at a few
hundred megahertz.

So `crc32`'s true figure is **worse** than the 8.6x in the table, by the ratio of its real
clock to 1 GHz. The same applies to `crc3` at 33 levels. This is not a caveat bolted on for
comfort: the depth column from the first table and the ratio column from this one have to be
read together, and a design with a 644-level path like `sha256` could not close at *any*
clock worth having.

### What hardware is actually for

None of this is an argument against hardware. It is an argument against *general-purpose
hardware on a general-purpose comparison*. A dedicated circuit is worth building when:

- **The CPU cannot do it at all.** Streaming from a sensor with no memory to buffer into, or
  a protocol conversion at a rate the bus must sustain.
- **Latency must be bounded.** A fixed pipeline is a fixed latency. Software has cache
  misses, branch mispredicts, cache-line eviction and scheduler preemption; the circuit has
  none of them and cannot.
- **The CPU is doing something else.** Hardware is parallelism, and it is free parallelism
  in the sense that it does not compete for the core.
- **The algorithm is regular enough that a small circuit covers it.** Which is precisely
  the `crc3` and `hex` case above.

Against a modern CPU on the same data, with both free to use everything available, a small
straightforward circuit usually loses. That is a fact about transistors, not about
toolchains.

### Coverage, and what is excluded

| included | excluded, and why |
|---|---|
| `crc32`, `crc3`, `hex`, `base64`, `memchr` | **`sha256`, `aes`, `chacha20`** — the designs are bare block compressors or single-block transforms, while the crates do padding, chaining and serialisation on top. Timing one against the other compares a compressor with a complete cipher. The row would be a statement about the crate's API, not about this design. |
| | **`ghash`, `roaring`, `bitpacking`, `aho-corasick`** — measurable in principle, but each needs its own driver and its own fair unit of work. Not written yet, so listed as *not measured* rather than estimated. |
| | **The other eleven designs** — no crate defines them, so there is nothing to compare against. A hand-written scalar reference would measure whoever wrote it. |

Every exclusion above is a real gap in the evidence, stated rather than papered over. The
five rows that do exist are honest; a table of twenty-two invented ratios would not be.

### What is still not measured

- **Area and power.** Not derivable from the IR.
- **Frequency.** A ratio is only meaningful relative to a clock, and this project does not
  claim a design will close at any particular one. `sha256`'s 644-level combinational depth
  is the obvious reason it would not.
- **Silicon.** Every figure is about the graph or about this host's CPU.

## Three structural findings

### 1. `mem bits` is zero for every design, and that is the memory gap

There is no `Mem` node anywhere in the corpus. `Design::rom` and `Design::mem` emit
`Case` and `Constant` nodes instead — you can see it directly in the `memchr` rows, where
the 256-entry ROM variant carries **264 `Constant` nodes and zero memory bits** against the
comparator variant's 8.

This is the documented gap, now with a measurement behind it rather than an assertion. A
256-entry table is correct and honest as a multiplexer tree and completely wrong for an FPGA
block RAM.

### 2. `sha256` and `chacha20` have combinational paths hundreds of levels long

`sha256` is 644 operator levels deep and `chacha20` is 484. Both unroll every round
specifically to get **one block per cycle**, and that is the trade: no synthesis flow meets
timing on a 644-level combinational path without pipelining, and neither design is
pipelined. The depth is the cost of the throughput claim, and it is visible here rather than
being discovered in a timing report.

`combinational_depth_is_bounded` pins the worst case at 650 levels so an unrolled design
that grows a round cannot pass unnoticed.

### 3. The wall-clock harness measures the simulator, and must not be quoted as hardware

`tests/memchr.rs` wraps the **software simulator** in a timer and compares its wall clock
against the crate's. That number moves by more than 8× between build profiles on one host,
because it depends on host load, build profile and clock rather than on the design.

What it measures is the simulator: a software interpreter stepping one node per cycle costs
far more wall time than the single clock edge it is modelling. So it is a debugging signal,
not a hardware figure, and `tests/speed.rs` exists to replace it.

The claim that survives, and needs no wall clock at all:

> The design retires **one byte per clock edge by construction**. The crate retires 32 bytes
> per AVX2 instruction.

Converting the design side to time needs a clock, which is why every ratio in this document
is quoted at a stated 1 GHz — and why they must be read together with the
combinational-depth column above. A design 34 levels deep will not close at 1 GHz.

## Reproducing

```sh
# Structural metrics, all designs (no timing, deterministic).
cargo test -p ferrite-lithic-corpus --test graph_metrics -- --nocapture --ignored

# Design cycles per unit vs the reference crate's wall clock, at a stated 1 GHz.
cargo test -p ferrite-lithic-corpus --test speed --release -- --nocapture --ignored
```

Both are excluded from the default suite: the first because it prints rather than asserts,
the second because a timing assertion is a flake generator and the assertion it makes is
deliberately almost empty. CI runs the corpus with `--nocapture` so the printed ratio lands
in the job log, and fails if anything reported `SKIPPED`.
