/**
 * Documentation content for the nine tool crates.
 *
 * Every code sample below is transcribed from the crates' own doc examples and public
 * signatures rather than written from memory, and every number is a count taken from the
 * repository. The `measured` notes exist to keep that distinction visible: a figure that
 * was measured says so, and a figure that is merely asserted does not get to.
 */

import type { Page } from "./types";

export const TOOLCHAIN: Page[] = [
  {
    slug: "bits",
    title: "Fixed-width bitvectors",
    crate: "ferrite-lithic-bits",
    tier: "A1",
    summary:
      "Runtime-width integers over u64 words, with the width rules that make Verilog agree with you.",
    lede: [
      "This is the bottom of the stack and the only crate with no dependency on anything else in the workspace. It is a fixed-width bitvector: the width is a runtime value, not a type parameter, because the DSL builds graphs whose widths are only known once the design is.",
      "It looks like a `u64` with a width attached and it is not. Every operation here has to answer the same question — what happens when two operands disagree — and the answer has to be the one Verilog gives, because the whole project rests on the emitted Verilog and the simulator agreeing.",
    ],
    steps: [
      {
        title: "Store words, not bits",
        detail:
          "A value is a `Vec<u64>` plus a width in bits. `words_for_width` is a `const fn`, so the word count is computed rather than searched for. Storing words rather than a bit per bit is the difference between a graph traversal that is memory-bound and one that is not.",
        code: `use ferrite_lithic_bits::{Bits, words_for_width, WORD_BITS};

assert_eq!(WORD_BITS, 64);
// The word count is arithmetic, not a loop.
assert_eq!(words_for_width(1), 1);
assert_eq!(words_for_width(65), 2);`,
        figure: { value: "64", unit: "bits per word", caption: "WORD_BITS — one shift, not a bit vector" },
      },
      {
        title: "Construct from the four shapes hardware actually has",
        detail:
          "`zeros` and `ones` are the two reset values, `constant` takes a host integer, and `from_bytes_le` is what a memory port or a byte-wide bus hands you. There is no `From<u64>` on purpose: a conversion that cannot fail is a conversion that will silently truncate, and the width has to be stated.",
        code: `use ferrite_lithic_bits::Bits;

assert_eq!(Bits::zeros(8)?.to_u64()?, 0x00);
assert_eq!(Bits::ones(8)?.to_u64()?, 0xff);
assert_eq!(Bits::constant(0x5a, 8)?.to_u64()?, 0x5a);

// A memory read, little-endian, exactly as the simulator hands it over.
let bytes = [0x01, 0x02];
assert_eq!(Bits::from_bytes_le(16, &bytes)?.to_u64()?, 0x0201);`,
      },
      {
        title: "Extend and truncate explicitly",
        detail:
          "`zero_extend`, `sign_extend` and `truncate` are three separate functions because they are three different pieces of hardware. Collapsing them into one with a flag makes every call site carry a boolean that says nothing at the call site.",
        code: `use ferrite_lithic_bits::Bits;

let eight = Bits::constant(0xff, 8)?;
assert_eq!(eight.zero_extend(16)?.to_u64()?, 0x00ff);

let high = Bits::constant(0xff, 8)?;
assert_eq!(high.sign_extend(16)?.to_u64()?, 0xffff);

assert_eq!(high.truncate(4)?.to_u64()?, 0xf);`,
      },
      {
        title: "Arithmetic that refuses rather than guesses",
        detail:
          "Every arithmetic method returns `Result`. `udiv` by zero is an error, not a zero and not a panic — a division that cannot be built is a division the caller has to decide about, and the alternative is a design that means something the author did not write. Signed division and remainder are genuine primitives here rather than being expressed through unsigned ones, because Verilog's `/` and `%` are signed for signed operands and the distinction changes the gate count enormously.",
        code: `use ferrite_lithic_bits::Bits;

let a = Bits::constant(7, 8)?;
let b = Bits::constant(2, 8)?;
assert_eq!(a.add(&b)?.to_u64()?, 9);
assert_eq!(a.udiv(&b)?.to_u64()?, 3);
assert_eq!(a.urem(&b)?.to_u64()?, 1);

// Refused, not guessed.
let zero = Bits::zeros(8)?;
assert!(a.udiv(&zero).is_err());`,
      },
      {
        title: "Expose the predicates the hardware needs",
        detail:
          "`is_negative`, `is_min_signed` and `magnitude_is_zero` exist because a signed comparison in hardware is not a comparison on the raw word. A design that needs to know whether an adder overflowed needs `is_min_signed` on the result, and there is no way to derive that from `is_negative` alone.",
      },
    ],
    gotchas: [
      {
        title: "A binary operation returns the wider of the two widths",
        body: "The rule throughout is `max(left, right)`, with the narrower operand zero-extended. This is Verilog's rule, and it is not Rust's. A design that assumes Rust's behaviour silently gains width on every comparison and every `and`.",
      },
      {
        title: "Width zero is an error, not an empty value",
        body: "`Bits::zeros(0)` fails rather than producing an empty vector. A zero-width signal is a modelling mistake, and letting it exist means the emitter has to decide what to print for it.",
      },
    ],
    notes: [
      {
        kind: "decision",
        title: "Runtime widths, not const generics",
        body: "A `const` generic width would make every width a distinct type and every arithmetic operator a trait to be implemented. The width is data here, so `Design::add` does not need to know it at compile time and the graph stays one type.",
      },
      {
        kind: "gap",
        title: "No arbitrary-precision arithmetic",
        body: "The host-side representation is `u64` words, so a design wider than the host can hold bits in is not buildable. Nothing in the corpus needs it, and the limit is recorded rather than hidden.",
      },
    ],
    tests: { total: 96, breakdown: "unit, errors, exhaustive small widths, properties, width sweep" },
  },

  {
    slug: "ir",
    title: "The graph",
    crate: "ferrite-lithic-ir",
    tier: "A1",
    summary: "Nodes, widths and dependencies. No behaviour, no types, no opinions about what hardware is.",
    lede: [
      "The IR is a circuit: a flat arena of nodes, each with a width, connected by edges. That is the whole data structure. It has no notion of a clock, no notion of a process, and no notion of what a register is beyond being a node with a feedback edge.",
      "Keeping it this small is the point. Both backends — the cycle simulator and the Verilog emitter — read this and nothing else, so anything they can both express is expressible, and anything one of them needs that the other cannot is a gap you find immediately rather than at integration.",
    ],
    steps: [
      {
        title: "An arena, so node identity is an integer",
        detail:
          "`Circuit` stores nodes in insertion order and refers to them by `NodeId`. Nothing holds a reference into the arena, which means the graph is cheap to traverse, cheap to clone, and impossible to invalidate.",
        code: `use ferrite_lithic_ir::Circuit;

let mut circuit = Circuit::new();
let a = circuit.wire(8)?;
let b = circuit.wire(8)?;
let sum = circuit.add(a, b)?;
assert_eq!(circuit.width_of(sum), 8);
assert_eq!(circuit.len(), 3);`,
      },
      {
        title: "A wire is a node that something drives",
        detail:
          "`wire` allocates, `drive` connects a driver to it, and `driver_of` reads the connection back. A wire with no driver is a legitimate state — it is what a register looks like between elaboration and the clock edge — so the IR reports it as `Ok(None)` rather than as an error.",
        code: `let mut circuit = Circuit::new();
let w = circuit.wire(4)?;
assert_eq!(circuit.driver_of(w)?, None);

let v = circuit.constant(ferrite_lithic_bits::Bits::constant(3, 4)?);
circuit.drive(w, v)?;
assert_eq!(circuit.driver_of(w)?, Some(v));`,
      },
      {
        title: "Pure constructors for every combinational primitive",
        detail:
          "`add`, `mul`, the four divisions, the eight comparisons, `select`, `cat`, `replicate`, `ite` and `case_` all live on `Circuit` directly. Each is infallible in shape and returns `Result` only where a width rule can be violated — `select` past the end of a value, for instance.",
        code: `let mut c = Circuit::new();
let a = c.wire(8)?;
let b = c.wire(8)?;

let wide = c.add(a, b)?;      // max(8, 8) = 8
let cmp  = c.ult(a, b)?;     // 1 bit
let mux  = c.ite(cmp, a, b)?;`,
      },
      {
        title: "Node kinds are an enum, so an emitter cannot forget one",
        detail:
          "`Node` is a `Node` enum with a variant per primitive — `Constant`, `Wire`, `Not`, `Select`, `BitAnd`, `Add`, `UDiv`, `SRem`, the comparisons, and the rest. An exhaustive `match` in each backend means adding a primitive is a compile error in every backend that has not been taught it, rather than a silently dropped node.",
      },
      {
        title: "Names are optional and checked",
        detail:
          "`name` and `names` attach identifiers for waveforms and generated Verilog. They are optional, because a design that has not been named yet is still a valid design and forcing names at IR-construction time would mean naming things before knowing what they are.",
      },
    ],
    gotchas: [
      {
        title: "Two nodes of different widths in one operator",
        body: "Allowed, and resolved as `max` with zero-extension — the Verilog rule. It is the single most common source of a design that is a byte wider than its author intended.",
      },
      {
        title: "`case_` with no default arm",
        body: "`case_` requires a default. An incomplete `case` in generated Verilog is a latch, and a latch inferred silently by a synthesis tool is the hardest class of bug to find from a waveform.",
      },
    ],
    notes: [
      {
        kind: "decision",
        title: "Flat arena, no tree",
        body: "A tree-shaped IR would make construction easy and every consumer awkward. The flat arena makes construction slightly more explicit and traversal trivial, and traversal is what both backends do constantly.",
      },
    ],
    tests: { total: 116, breakdown: "the largest suite in the workspace — the IR is what everything else is built on" },
  },

  {
    slug: "design",
    title: "The front end",
    crate: "ferrite-lithic",
    tier: "A2",
    summary: "Design is an arena behind builders, and a Signal is a node id with a cached width.",
    lede: [
      "The front end is what you actually call. `Design` owns a `Circuit` and hands out `Signal`s; every operator is a method that appends nodes and returns a new `Signal`. There is no separate builder type, no prelude, and no trait to implement.",
      "The interesting decision is that `Design` is `&self` and the arena is a `RefCell`. That is what makes combinators nest — a helper that takes a `&Design` can be called from inside another helper — and it is safe because every borrow is taken and released inside one method body that cannot re-enter.",
    ],
    steps: [
      {
        title: "A Signal is an id and a width, never a reference",
        detail:
          "Because a `Signal` holds no pointer into the arena, the graph stays cheap to walk and there is no borrow lifetime to thread through every signature. This is also why the arena can be handed to a backend that wants to own it.",
        code: `use ferrite_lithic::Design;

let design = Design::new();
let a = design.wire(8)?;
let b = design.lit(0x5a, 8)?;
let sum = design.add(&a, &b)?;`,
      },
      {
        title: "Registers take their own enable",
        detail:
          "`reg(next, clk, rst, hold)` takes the enable as an explicit argument rather than inferring it. A design with no hold is the common case and passing `constant(false)` says so; a design that sometimes holds says that too, and neither is the default.",
        code: `let clk = design.clock();
let rst = design.reset();
let hold = design.constant(false);

let next = design.add(&a, &b)?;
let state = design.reg(&next, &clk, &rst, &hold)?;`,
      },
      {
        title: "Combinators compose because everything is &Design",
        detail:
          "Writing your own operator is a free function over `&Design` returning `Signal`. That is how the corpus stays readable: `crc32` is a few hundred lines because the FSE and Huffman designs can express a bit buffer without fighting the framework.",
        code: `fn majority(design: &Design, a: &Signal, b: &Signal, c: &Signal) -> Result<Signal, BuildError> {
    let ab = design.and(a, b)?;
    let bc = design.and(b, c)?;
    design.or(&ab, &bc)
}`,
      },
      {
        title: "rom is a case statement, and says so",
        detail:
          "`Design::rom(address, &table, width)` builds exactly the graph a human would write with `case_`: one arm per entry, emitting `always @* case`. It is not a memory, because the IR has no initialised-memory node — which is the single highest-value gap in the toolchain, and `rom` documents the cost rather than hiding it.",
        code: `// A nibble lookup table: a case tree, and honest about it.
let table = [0u64, 1, 4, 9, 16, 25, 36, 49];
let square = design.rom(&addr, &table, 8)?;`,
      },
      {
        title: "check before build",
        detail:
          "`Design::check` runs the structural checks — widths, driven wires, combinational loops — and `build` hands the `Circuit` to a backend. Separating them means a test can assert on the diagnostics rather than on a compiler error.",
      },
    ],
    gotchas: [
      {
        title: "The RefCell is quarantined to the builder",
        body: "A `Signal` never borrows the arena, so a `&Design` can be passed to a helper that itself calls operators. If `Signal` held a reference, every combinator signature would grow a lifetime and nesting would stop working.",
      },
      {
        title: "`mem` is zero-filled, everywhere",
        body: "`Design::mem` allocates a memory node and the simulator's initialise-to-value refuses it. Every lookup table in the corpus is therefore a `case_` or a mux tree. Close enough for a nibble LUT, and a lie for a 256-entry S-box.",
      },
    ],
    notes: [
      {
        kind: "measured",
        title: "arena size is the cost metric",
        body: "`Design::node_count` is what the corpus uses to compare designs, and a `case_`-based table is dramatically larger than the equivalent RAM would be. AES's S-box is 200 duplicated 256-entry arms for exactly this reason.",
      },
    ],
    tests: { total: 58, breakdown: "builder, operators, structural — the operator table is checked exhaustively at small widths" },
  },

  {
    slug: "sim",
    title: "The cycle simulator",
    crate: "ferrite-lithic-sim",
    tier: "A2",
    summary: "Combinational within a cycle, sequential at the edge, and the edge is a real boundary.",
    lede: [
      "The simulator evaluates the graph in topological order. Combinational nodes settle first, then registers sample their inputs and take their new values at the clock edge. Everything else — memories, handshakes, the lot — falls out of that one rule.",
      "The rule matters because it is the rule real hardware uses. A design that works in a simulator which re-evaluates everything in arbitrary order is a design whose behaviour you have not actually checked.",
    ],
    steps: [
      {
        title: "Compile once, then settle",
        detail:
          "`Sim::new` compiles the circuit into an evaluation program. `comb` settles the combinational cone; `step` does the whole cycle and returns the snapshots either side of the edge.",
        code: `use ferrite_lithic_sim::Sim;

let mut sim = Sim::new(&design)?;
sim.initialize_to(&state, ferrite_lithic_bits::Bits::zeros(8)?)?;

let (before, after) = sim.step()?;`,
      },
      {
        title: "Three explicit points around the edge",
        detail:
          "`before_clock_edge`, `at_clock_edge` and `after_clock_edge` are separate methods because separate methods are what let a testbench place stimulus on the right side of the edge. The alternative — a `step` that takes an input — is what caused the `Handle::set` bug recorded in `DETAILS.md`: it silently acquired an extra clock edge.",
        code: `sim.before_clock_edge()?;   // sample here
sim.set_input(&d, &value)?;
sim.at_clock_edge()?;          // registers take their new values
sim.after_clock_edge()?;`,
      },
      {
        title: "Registers update last, and visibly",
        detail:
          "`reg_updates` and `mem_updates` expose what the edge actually did. A testbench that wants to assert on the transition rather than on the value needs exactly this, and it is the reason the corpus can check a one-symbol-per-cycle decoder rather than only its final answer.",
        code: `let updates = sim.reg_updates()?;
for update in updates {
    println!("{}: {:?} -> {:?}", update.signal, update.before, update.after);
}`,
      },
      {
        title: "Errors rather than silent zero",
        detail:
          "An undriven wire, a width mismatch and a combinational loop are all errors. A simulator that substitutes zero for a missing driver produces plausible waveforms for a broken design, which is the failure mode this project has the least tolerance for.",
      },
    ],
    gotchas: [
      {
        title: "initialise-to-value refuses memories",
        body: "Not a limitation left unstated: it is a deliberate refusal, because there is no initialised-memory node in the IR to initialise. Adding one is the change that would unlock it.",
      },
      {
        title: "Widths are not coerced",
        body: "A `Bits` handed to `set_input` must match the port's width or the call errors. Silent zero-extension here would be a bug that only appears on one backend.",
      },
    ],
    tests: { total: 52, breakdown: "including 11 doctests, several of which are the README's examples" },
  },

  {
    slug: "rtl",
    title: "The Verilog emitter",
    crate: "ferrite-lithic-rtl",
    tier: "A2",
    summary: "One structural always block for the whole design, and a signature both backends agree on.",
    lede: [
      "The emitter turns the same `Circuit` the simulator reads into structural Verilog. The shape is deliberate: one `always` block, one giant `begin`/`end`, continuous assignments for combinational logic and non-blocking assignments for registers.",
      "That shape is what makes the cosimulation check meaningful. If the two backends were structurally different — one procedural, one concurrent — then a disagreement would not tell you which one is wrong. Emitting something a synthesis tool reads the way it reads any other RTL is the whole point.",
    ],
    steps: [
      {
        title: "Build a module from the circuit and the clock",
        detail:
          "`Module::new` takes the name, the built circuit, the clock node id, and the input and output id lists. The clock is passed separately because it is the one signal that is a port but not a stimulus column.",
        code: `use ferrite_lithic_rtl::Module;

let module = Module::new(
    "top",
    design.build()?,
    clock_id,
    design.input_ports().iter().map(|s| s.id()).collect(),
    design.output_ports().iter().map(|s| s.id()).collect(),
);`,
      },
      {
        title: "One process, in topological order",
        detail:
          "`emit` walks the graph and writes each node once. Combinational nodes become continuous assignments; registers become non-blocking assignments inside the clocked block. Topological order is what makes the generated Verilog readable when a design goes wrong.",
      },
      {
        title: "A signature both backends can be compared on",
        detail:
          "`signature`, `inputs`, `outputs` and `port_identifiers` define the interface the cosimulator checks. It is derived from the design rather than written out, which is what stops a test from asserting that the stimulus matched a hand-copied column list.",
      },
      {
        title: "Names come from PortList, not from string literals",
        detail:
          "`with_port_names` applies the names a `#[derive(PortList)]` produced. A port's name in the generated Verilog is therefore the same identifier the testbench used, which is what makes a failing cosim cycle readable.",
      },
    ],
    gotchas: [
      {
        title: "The port list is the contract",
        body: "Order matters: the cosimulator's stimulus columns are built from `inputs` in order. Reordering the emitter's list without reordering the plan is a mismatch that shows up as every column comparing the wrong signal.",
      },
    ],
    notes: [
      {
        kind: "measured",
        title: "Verilator is looked for, not required",
        body: "`cargo test` works without it and prints what it skipped. CI installs it and then fails if anything skipped — because a green run in which every equivalence test took its skip path is green and meaningless.",
      },
    ],
    tests: { total: 52, breakdown: "emission, signature, and an Icarus-gated check that the generated Verilog compiles" },
  },

  {
    slug: "derive",
    title: "Ports by derive",
    crate: "ferrite-lithic-derive",
    tier: "A3",
    summary: "A port list is a struct, and the derive reads the shape rather than a list of strings.",
    lede: [
      "A design's interface is a struct with one field per port. `#[derive(PortList)]` reads that struct and produces the names, the widths and the clock designation, so the front end can build the ports and the emitter can name them without either side maintaining a parallel list.",
      "The alternative is a macro taking `(\"clk\", 1), (\"rst\", 1), (\"count\", 16)` — and then a width that appears in one place and not the other, which is a bug that compiles.",
    ],
    steps: [
      {
        title: "Declare the interface as a struct",
        detail:
          "Fields are one bit wide unless `#[bits(n)]` says otherwise, and `#[clock]` marks the one port that is a clock. Field names become port names.",
        code: `use ferrite_lithic::Signal;
use ferrite_lithic_derive::PortList;

#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    #[clock]
    pub clk: Signal,
    pub rst: Signal,
    #[bits(16)]
    pub count: Signal,
}`,
      },
      {
        title: "Hand the struct to the front end",
        detail:
          "`inputs::<Inputs>(design)` declares every port as a named wire in one call and returns a struct of the signals to drive. The alternative is a loop over a name list, which cannot produce a struct.",
        code: `let inputs = ferrite_lithic::inputs::<Inputs>(&design)?;
inputs.count;   // a Signal, already wired to the port`,
      },
      {
        title: "RTL names are separate on purpose",
        detail:
          "`PortList::rtl_names()` applies the `#[rtlname(\"...\")]` attributes. Keeping the display name separate from the field name means a Rust field can be renamed without silently renaming a port in generated Verilog that a testbench refers to.",
      },
    ],
    gotchas: [
      {
        title: "A port list cannot be generic",
        body: "The derive rejects generic structs outright. A port list is a fixed interface, and a generic one would mean the width is not known at the point the ports are built.",
      },
      {
        title: "Default width is one bit",
        body: "A field with no `#[bits]` is a single bit. This is the right default — most control signals are one bit — but it means a forgotten attribute produces a one-bit port rather than a compile error.",
      },
    ],
    tests: { total: 39, breakdown: "the macro's own expansion, error cases, and 1 doctest" },
  },

  {
    slug: "wave",
    title: "Waveform data",
    crate: "ferrite-lithic-wave",
    tier: "A3",
    summary: "Cycle-indexed values you assert on directly, plus a VCD rendering for when you want eyes.",
    lede: [
      "This crate exists because a waveform viewer is a debugging tool and an assertion is a test. Both are useful and they answer different questions: the viewer tells you what happened, the assertion tells you whether it should have.",
      "So the data model is a value per cycle per port, and the primary API is `assert_series`. The VCD rendering is a secondary function of the same data, not a separate capture path.",
    ],
    steps: [
      {
        title: "Register the ports, with their widths",
        detail:
          "`register` and `register_shape` attach a port to the dataset; `top` names the scope and `identifiers` gives the names used in the output.",
        code: `use ferrite_lithic_wave::WaveData;

let mut wave = WaveData::new("top");
wave.register(&port, "count")?;`,
      },
      {
        title: "Record a cycle",
        detail:
          "`record` takes the cycle number and a lookup closure, so the data can be gathered from a snapshot without the wave crate knowing what a snapshot is.",
        code: `wave.record(cycle, |signal| {
    Ok(snapshot.value(signal))
})?;`,
      },
      {
        title: "Assert the series, not the last value",
        detail:
          "`assert_series` compares the whole cycle-indexed sequence and reports the first cycle that differs. This is the single most useful function in the crate: a decoder that retires 39 of 40 symbols produces a correct final answer and a failing series.",
        code: `wave.assert_series(&port, &[
    Bits::constant(0, 8)?,
    Bits::constant(1, 8)?,
    Bits::constant(3, 8)?,
])?;`,
      },
      {
        title: "Read the changes when the series is long",
        detail:
          "`series_changes` returns `(cycle, value)` pairs for the cycles where the value moved, which is what makes a 40-cycle run of mostly-zeros readable in a failure message.",
      },
      {
        title: "Write VCD when you want to look at it",
        detail:
          "`write` consumes the dataset into a `Write`, and `render` gives a string. Same data either way, so a waveform can never disagree with the assertion that passed.",
      },
    ],
    gotchas: [
      {
        title: "Series are indexed by cycle, not by position",
        body: "A gap in cycle numbers is a gap, not a shift. A design that stops clocking produces a short series and a failing assertion rather than a silently compressed one.",
      },
    ],
    tests: { total: 42, breakdown: "data, assertion, mismatch reporting, VCD output" },
  },

  {
    slug: "tb",
    title: "Step testbenches",
    crate: "ferrite-lithic-tb",
    tier: "A3",
    summary: "Coroutine testbenches where every await is one clock edge.",
    lede: [
      "The testbench API is built on async functions, and the rule that makes it work is that `.await` means one clock edge. `tb.step().await` advances time; `tb.set(..).await` applies an input and takes no edge.",
      "That rule was learned the hard way. An earlier version of `set` took an edge, which meant every testbench acquired a spurious leading edge — invisible in CRC-3, whose all-zero init is a fixed point, and visible in CRC-32, whose all-ones init is not.",
    ],
    steps: [
      {
        title: "Run a closure against a design",
        detail:
          "`Testbench::new` inspects the design's declared ports and `run` executes the async closure, checking at the end that the testbench drove only ports the design declares.",
        code: `use ferrite_lithic_tb::Testbench;

let tb = Testbench::new(&design)?;
tb.run(|tb| async move {
    tb.pulse(ferrite_lithic_corpus::RESET).await;
    tb.set("count", 4).await;
    tb.step().await;
})?;`,
      },
      {
        title: "set takes no edge, step takes the edge",
        detail:
          "This is the whole discipline. `set` writes a value into the input and returns; `step` advances the clock. Combining them into one call is what made the leading-edge bug invisible in one design and obvious in another.",
        code: `tb.set("in_bit", 1).await;   // no edge
tb.step().await;                // one edge

// Read what the design settled on, before the next edge.
let ready = tb.peek("in_ready");`,
      },
      {
        title: "Drive buses by value, not by bit",
        detail:
          "`set_bits` and `drive_bits` take a `Bits`, so a testbench never has to shift a bit into a position by hand — which is where a wrong bit order would otherwise live.",
        code: `tb.set_bits("in_byte", Bits::constant(0x5a, 8)?).await;
tb.drive_bits("state", &snapshot)?;`,
      },
      {
        title: "Collect the run and assert on it",
        detail:
          "`series` returns the cycle-indexed values the testbench recorded, so a test can assert on a whole run. `waves` hands back the same data in the wave crate's model, and a failure captures both.",
        code: `let out = tb.series("symbol");
assert_eq!(out.first(), Some(&Bits::constant(72, 9)?));`,
      },
    ],
    gotchas: [
      {
        title: "peek reads the settled value",
        body: "`peek` and `value` read what the design presents on the current cycle, which is what the design latches on the edge that follows. Reading after the edge gets the next cycle's answer, and the loss shows up several symbols later.",
      },
      {
        title: "Only declared ports",
        body: "`run` fails if the testbench drives a name the design does not declare. A typo in a port name is an error rather than a stimulus that silently goes nowhere.",
      },
    ],
    notes: [
      {
        kind: "decision",
        title: "async, because a clock is a suspension point",
        body: "A testbench that reads like a list of clock edges in the order they happen is easier to check against a design than one built from nested callbacks. The `.await` is the clock edge and nothing else is.",
      },
    ],
    tests: { total: 17, breakdown: "the smallest crate, and the one whose contract is easiest to get subtly wrong" },
  },

  {
    slug: "cosim",
    title: "Verilator equivalence",
    crate: "ferrite-lithic-cosim",
    tier: "A3",
    summary: "Two backends, one stimulus, compared cycle by cycle, with the first difference reported.",
    lede: [
      "This is the crate that makes the other ones trustworthy. It takes one design, emits Verilog, compiles it with Verilator, runs both it and the simulator against the same stimulus, and compares every output on every cycle.",
      "The comparison is only meaningful because of what a `Plan` fixes: the same columns, the same initial values, the same clock exclusion. Without that, the two backends are being compared as two different experiments rather than as two implementations of one.",
    ],
    steps: [
      {
        title: "Derive the plan from the design",
        detail:
          "`Plan::of` takes the design and the module and works out the stimulus columns, excluding the clock — which is a port but not something the stimulus drives, because the harness drives it.",
        code: `use ferrite_lithic_cosim::Plan;

let plan = Plan::of(&design, &module)?;

// The clock is a port, but not a stimulus column.
assert_eq!(
    plan.inputs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
    ["rst", "init", "count", "in_valid", "in_bit"],
);`,
      },
      {
        title: "Build the stimulus",
        detail:
          "A `Stimulus` is a list of rows, one per cycle, each a vector of column values. The handshake matters: a design whose `in_ready` drops needs a slower stimulus, or the bits that overrun it are lost and both backends agree on the wrong answer.",
        code: `use ferrite_lithic_cosim::Stimulus;

let mut stimulus = Stimulus::new();
stimulus.push(row(1, 0, 0, 0, 0))?;              // reset
stimulus.push(row(0, 1, count, 0, 0))?;           // init

// One bit every two cycles, because in_ready drops when the buffer is full.
for bit in bits {
    stimulus.push(row(0, 0, 0, 1, u64::from(bit)))?;
    stimulus.push(row(0, 0, 0, 0, 0))?;
}`,
      },
      {
        title: "Build the harness",
        detail:
          "`Harness::build` writes `dut.v` and a generated driver into a **fresh subdirectory per build**, then compiles and links them. Sharing a directory between two harnesses means two processes write the same `dut.v` and then each runs the other's stimulus — the dangerous failure being `is_equivalent` answering about a stimulus the caller never passed.",
        code: `use ferrite_lithic_cosim::Harness;

let verilog = ferrite_lithic_cosim::emit(&design, &plan)?;
let harness = Harness::build(&plan, &verilog, &plan.work_dir())?;`,
      },
      {
        title: "Run both and compare",
        detail:
          "`run_with` returns a report. `is_equivalent` is the verdict and `first` is the first cycle and column that disagreed, which is the difference between a usable failure and a useless one.",
        code: `let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness)?;
assert!(
    report.is_equivalent(),
    "the two backends disagree: {report}\\nfirst: {:?}",
    report.first(),
);`,
      },
    ],
    gotchas: [
      {
        title: "Verilator is pinned to two-state, zero-initialised",
        body: "The argument is `--x-initial 0`. Verilator's default is `unique`, which randomises initial state per run to shake out designs that depend on uninitialised state — the right default for Verilator's own tests and the wrong one here, because the simulator zero-fills and a design relying on reset state would then diverge at random rather than every time.",
      },
      {
        title: "A fixed job count, not every core",
        body: "Verilator is invoked with `-j 4`. With `-j 0` on a many-core machine it intermittently aborts at teardown with an internal thread-pool error. The generated C++ is small, so the parallelism that matters is across designs, which `cargo test` already provides.",
      },
      {
        title: "Skipping is reported, not silent",
        body: "With no Verilator installed the test prints `SKIPPED` and passes. CI installs Verilator and fails if anything skipped, so a green CI run means the equivalence actually ran.",
      },
    ],
    tests: { total: 33, breakdown: "plan derivation, stimulus encoding, harness lifecycle, and real Verilator equivalence" },
  },
];