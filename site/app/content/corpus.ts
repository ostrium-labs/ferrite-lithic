/**
 * Documentation content for the corpus and for what the corpus found.
 *
 * These two pages are the ones that carry the project's actual argument. The corpus page
 * explains the seven tiers and why each design is in them; the findings page is the bug
 * catalogue, which is the part worth reading twice.
 */

import type { Page } from "./types";

export const CORPUS: Page = {
  slug: "corpus",
  title: "The corpus",
  crate: "ferrite-lithic-corpus",
  tier: "—",
  summary:
    "Twenty-one real algorithms written the way hardware would want them, each checked three ways.",
  lede: [
    "Every other crate in the workspace is a tool, and a tool gets tested with inputs chosen by whoever wrote it — so its tests drift toward what the tool was built to do. A corpus entry is a real algorithm written the way hardware would want it, checked three ways: against the original crate as the golden model, through the step testbench, and against Verilator by compiling the emitted Verilog and comparing cycle by cycle.",
    "The golden model is never a second implementation by the same hand. Where the original crate is unavailable the entry says so on its own docs and states what stands in for it, because a self-consistent round trip proves the design is self-consistent and nothing else.",
  ],
  steps: [
    {
      title: "Tier 1 — LFSRs and checksums",
      detail:
        "The cheapest designs with a register and a bit stream, and the ones that find toolchain bugs first. `crc3` is three flops and two conditional XORs consuming one byte per cycle; `crc32` is 32 of them. `ghash` is the most ASIC-shaped thing in the corpus: a CPU does it with carryless multiply over limbs, and the hardware is 128 cycles of a register and two conditional XORs.",
      figure: { value: "4", unit: "entries", caption: "crc3, crc32, lfsr, ghash — and crc32fast's 8 kB lookup is the cost hardware does not pay" },
    },
    {
      title: "Tier 2 — block ciphers",
      detail:
        "Two opposite cases on purpose. `sha256` is 64 unrolled rounds with no ROM and a perfectly serial dependency chain, which is exactly the case SIMD cannot help. `chacha20` is the counterexample to \"ROM-heavy ciphers are the ASIC ones\": no tables at all, just 32-bit adders and rotators, one block per cycle for pure ALU. `aes` is the opposite again — the S-box ROM *is* the design.",
      figure: { value: "200", unit: "duplicate arms", caption: "AES's S-box as a case_ tree, where real silicon would infer one block RAM" },
    },
    {
      title: "Tier 3 — codecs",
      detail:
        "`hex` and `base64` are LUTs, and they are in the corpus because they are the tier that validates the toolchain cheaply: a nibble or sextet table is the smallest honest test of whether a ROM lowers correctly and whether the cosimulator compares the right columns.",
    },
    {
      title: "Tier 4 — automata",
      detail:
        "One state register, one transition table, one byte per edge. This tier contains the project's most useful negative result: `memchr` is a real design and it still loses to the CPU by a measured 4843×, because single-byte search is what SIMD was built for. It is in the corpus anyway, as the baseline the other two are measured against.",
      figure: { value: "4843", unit: "× slower", caption: "memchr: 2540 ns/byte against 0.52 ns/byte. Measured, not assumed." },
    },
    {
      title: "Tier 5 — data infrastructure",
      detail:
        "The decode kernels of a columnar engine, and the probabilistic sketches. `bitpack`, `rle`, `roaring`, `hamming`, `sorting`, `sketch`. The sorting network is measured for depth against the formula rather than against its own layer count, which is the check that catches a network that is one layer short and still sorts most inputs.",
    },
    {
      title: "Tier 6 — packet",
      detail:
        "`packet` is a 64-byte header window in and every field out, combinationally. It reports three verdicts rather than one — `valid` is structural, `csum_ok` is integrity, `malformed` is `ipv4 && !valid` — because a switch that wants to drop bad checksums and a switch that only cares whether a header parses want different things, and folding the checksum into `valid` would cost the first one its choice.",
    },
    {
      title: "Tier 7 — entropy coding",
      detail:
        "`huffman`, `fse` and the `deflate` bit layer. This tier arrived in one commit and had never been run: fourteen of its tests failed. Four were the design's and four were the tests' own, plus a mistyped constant in the sketches. Finishing it is the subject of the findings page.",
      figure: { value: "14", unit: "failures", caption: "in the one commit that added the tier — none of which were in the tools" },
    },
    {
      title: "What each design is checked against",
      detail:
        "Three checks, and they are not interchangeable. The differential against the original crate catches a design that computes the wrong function. The step testbench catches a design whose handshake or timing is wrong. The Verilator equivalence catches an emitter bug that the simulator happens to agree with. A design needs all three, and the ones with no golden crate say so on their own docs.",
    },
  ],
  gotchas: [
    {
      title: "No design here beats a CPU at being a CPU",
      body: "The corpus is not an argument that hardware beats software at software's job. It is an argument that the toolchain is correct, and the evidence is that a 60-line shift-and-XOR design found four bugs that 471 tests of the tools had not.",
    },
    {
      title: "Every table is a multiplexer tree",
      body: "There is no initialised-memory node in the IR, so a 512-entry Huffman decode table is 512 arms of `case_`. Correct, and not what anyone would ship. `Design::rom` documents the cost instead of hiding it.",
    },
  ],
  notes: [
    {
      kind: "measured",
      title: "4843× is a measurement",
      body: "2540 ns/byte for the automaton against 0.52 ns/byte for the crate. The honest benchmark is in the corpus precisely because it is a loss.",
    },
    {
      kind: "decision",
      title: "The golden model is never the same author",
      body: "Where a real crate exists it is used. Where none does — FSE has no golden crate in this workspace at all — the entry states the ceiling of its own evidence rather than implying more.",
    },
  ],
  tests: { total: 305, breakdown: "unit tests over table construction, plus fifteen integration targets, each with a Verilator equivalence check" },
};

export const FINDINGS: Page = {
  slug: "findings",
  title: "What the corpus found",
  crate: "ferrite-lithic-corpus",
  tier: "—",
  summary:
    "Nine defects, and one shape: a constant that looks right, builds a graph of exactly the right width, and means the opposite of what it says.",
  lede: [
    "This is the page worth reading twice. The corpus exists to find bugs in the tools, and it has found nine. They are listed here in full because the pattern is the actual finding: almost none of them are arithmetic mistakes, and almost all of them are things that compile, build a graph of the right shape, and produce a plausible answer.",
    "The first four came from `crc3` — a three-bit LFSR, one byte per cycle, the smallest design in the corpus with a register and a bit stream. They were found by 471 tests of the tools not having found them.",
  ],
  steps: [
    {
      title: "sll and srl were swapped, for the entire life of the project",
      detail:
        "The two shift directions were exactly backwards from A1 through A2. Every crate agreed with every other crate about it, because they were all wrong the same way and each was the other's golden model. Only a design with an external reference — an algorithm that has a published answer — could break the tie.",
      code: `// What the corpus found, and what 471 tests of the tools did not.
design.sll(&value, 1)?;   // had been shifting right
design.srl(&value, 1)?;   // had been shifting left`,
    },
    {
      title: "The FNV-1a offset basis was one digit wrong",
      detail:
        "`0xcbf2_9ce4_8423_2325` where the published constant is `0xcbf2_9ce4_8422_2325`. The multiplier was correct. The failure was caught only by the published check vectors, because the test's own first assertion used the same wrong constant and therefore agreed with itself.",
      code: `// A constant copied into both the code and its own test is not checked twice.
assert_eq!(fnv1a(b"", 0), 0xcbf2_9ce4_8423_2325);   // passes: same wrong constant
assert_eq!(fnv1a(b"a", 0), 0xaf63_dc4c_8601_ec8c);   // fails: the published vector`,
    },
    {
      title: "The IP rebase sliced from the wrong end",
      detail:
        "`byte` treats index zero as the most significant byte, so dropping the Ethernet header has to keep the window's low bits. The code sliced from `offset * 8`, which reads as though index zero were the least significant. Every IP field read as the byte eight earlier, the checksum never verified, and `valid` was stuck low.",
      code: `// Reads plausibly, means the opposite:
// let window = &frame[header..];          // wrong end
let window = &frame[frame.len() - 64..];  // what "byte" means`,
    },
    {
      title: "FSE gave its rarest symbol the most expensive transition",
      detail:
        "A normalised count of `-1` means \"rarer than one symbol in the whole table\". The state arithmetic that is correct for every positive count gave it `nbBits = tableLog` — a transition spanning the entire table, on a symbol that occurs once in `tableSize` of them. It gets `nbBits = 0` and `newState = 0`.",
      code: `// The one case the general arithmetic does not describe.
let (nb_bits, new_state) = if low_probability[symbol] {
    (0, 0)
} else {
    let nb_bits = table_log - highbit32(number);
    (nb_bits, (number << nb_bits) - table_size)
};`,
    },
    {
      title: "The FSE initial state was assembled backwards",
      detail:
        "The state register shifted left and inserted at the bottom, so the bit that arrives first — the state's *low* bit — ended up as its most significant. A reversed state is a state that exists, decodes plausibly, and is wrong from the first symbol on. The design now shifts right and inserts at the top.",
    },
    {
      title: "The bits that build the state were left in the decode buffer",
      detail:
        "So the first transition read the initial state back as its own `low` bits. The first symbol was right and every symbol after it was wrong, which is exactly the shape that survives a spot check.",
    },
    {
      title: "A test that passed for the wrong reason",
      detail:
        "`read_stored_block` compared DEFLATE's sixteen-bit complement as a sixty-four-bit one, so `len != !nlen` was true for every stored block ever written, including correct ones. And the test asserting that a *broken* complement is refused was passing because of the bug rather than despite it.",
      code: `// Wrong: a 64-bit complement no 16-bit length can equal.
if len as u64 != !nlen as u64 { return None; }

// Right: the complement is over the sixteen-bit field.
if len as u32 != (!nlen as u32 & 0xffff) { return None; }`,
    },
    {
      title: "A harness that stopped one cycle early",
      detail:
        "The DEFLATE bit-stream driver exited on the cycle the last byte was *loaded*, so that byte's eight bits never shifted out. Every round trip came up six to eight bits short and looked like a bit-order defect — the most expensive kind of wrong, because it sends you looking in the format instead of the harness.",
    },
    {
      title: "A claim that measurement contradicted",
      detail:
        "The `huffman` module documented \"one symbol per cycle, which is the point\". Measured: a decode spends `code_len` bits and the host supplies one bit per cycle, so the rate is one symbol per code length — seven, for DEFLATE's shortest fixed code. It is one per cycle only for the one-bit single-symbol code. `fse` *can* do it, because an FSE transition may cost zero bits, and that difference is the whole distinction between the two decoders.",
      figure: { value: "7", unit: "cycles per symbol", caption: "the real rate for DEFLATE's shortest fixed code — the docs said one" },
    },
  ],
  gotchas: [
    {
      title: "The recurring shape",
      body: "A constant that looks right, builds a graph of exactly the right width, and means the opposite of what it says. `sll`/`srl`, the FNV basis, the IP rebase, the FSE bit order, the 64-bit complement — five of the nine are this, and not one of them would be caught by a test that asserted the shape instead of the value.",
    },
    {
      title: "Self-consistency proves nothing",
      body: "Every round trip in this project that passes without an external reference proves only that the design agrees with the thing it was compared against. Four of the failures above were found by exactly the checks that were *not* self-consistent: a published vector, the RFC's own code assignment, a hand-derived table.",
    },
  ],
  notes: [
    {
      kind: "measured",
      title: "810 tests, and the corpus still found the bugs",
      body: "The tool crates' own tests are not bad; they were aimed at the wrong things. The change that found the last nine defects was not more tests. It was writing a design that has an answer somewhere outside this repository.",
    },
    {
      kind: "decision",
      title: "Write the finding down",
      body: "Every one of these is in `DETAILS.md` in the repository, with the mechanism rather than just the diff. A bug fixed without the shape recorded is a bug that gets reintroduced by the next person who makes the same reasonable assumption.",
    },
  ],
  tests: { total: 0, breakdown: "this page documents findings rather than a crate's test suite" },
};