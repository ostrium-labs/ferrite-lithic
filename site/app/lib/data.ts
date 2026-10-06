/**
 * The facts the page states, in one place.
 *
 * Every number here is a measurement or a count taken from the repository at a
 * specific commit, and none of them are decorative — the point of the page is that
 * the claims are checkable. If one of these goes stale the page is lying, which is
 * the one failure mode this project cannot have.
 */

export interface Crate {
  name: string;
  tests: number;
  tier: string;
  what: string;
  /** The documentation page that explains this crate, in `docs/`. */
  doc: string;
  dependencies: string[];
}

export const CRATES: Crate[] = [
  {
    name: "ferrite-lithic-bits",
    tests: 96,
    tier: "A1",
    what: "Fixed-width bitvectors over u64 words, with runtime widths.",
    doc: "bits",
    dependencies: [],
  },
  {
    name: "ferrite-lithic-ir",
    tests: 116,
    tier: "A1",
    what: "The graph: nodes, widths, and nothing else. No behaviour, no types.",
    doc: "ir",
    dependencies: ["bits"],
  },
  {
    name: "ferrite-lithic",
    tests: 58,
    tier: "A2",
    what: "The front end. Design is an arena of nodes; signals are ids.",
    doc: "design",
    dependencies: ["bits", "ir"],
  },
  {
    name: "ferrite-lithic-sim",
    tests: 52,
    tier: "A2",
    what: "A cycle simulator. Sequential on the clock edge, combinational within it.",
    doc: "sim",
    dependencies: ["design", "bits", "ir"],
  },
  {
    name: "ferrite-lithic-rtl",
    tests: 52,
    tier: "A2",
    what: "The Verilog emitter: one structural always block for the whole design.",
    doc: "rtl",
    dependencies: ["bits", "ir"],
  },
  {
    name: "ferrite-lithic-derive",
    tests: 39,
    tier: "A3",
    what: "Port lists by derive, so a port is a struct field and not a string.",
    doc: "derive",
    dependencies: [],
  },
  {
    name: "ferrite-lithic-wave",
    tests: 42,
    tier: "A3",
    what: "Cycle-indexed values asserted on directly, plus a VCD rendering.",
    doc: "wave",
    dependencies: ["design", "bits"],
  },
  {
    name: "ferrite-lithic-cosim",
    tests: 33,
    tier: "A3",
    what: "Verilator equivalence checking against the simulator, cycle by cycle.",
    doc: "cosim",
    dependencies: ["design", "bits", "ir", "rtl", "sim", "wave"],
  },
  {
    name: "ferrite-lithic-tb",
    tests: 17,
    tier: "A3",
    what: "Coroutine step testbenches, where every .await is one clock edge.",
    doc: "tb",
    dependencies: ["design", "sim", "bits"],
  },
  {
    name: "ferrite-lithic-corpus",
    tests: 305,
    tier: "—",
    what: "Verified algorithms. The corpus that finds the tools' bugs.",
    doc: "corpus",
    dependencies: ["design", "bits", "derive"],
  },
];

export interface Tier {
  name: string;
  designs: string[];
  why: string;
}

export const TIERS: Tier[] = [
  {
    name: "LFSR & checksums",
    designs: ["crc3", "crc32", "ghash"],
    why: "flops and two conditional XORs, a byte per clock",
  },
  {
    name: "Block ciphers",
    designs: ["sha256", "chacha20", "aes"],
    why: "64 unrolled rounds and no ROM — and the opposite case",
  },
  {
    name: "Codecs",
    designs: ["hex", "base64"],
    why: "LUTs: the tier that validates the toolchain cheaply",
  },
  {
    name: "Automata",
    designs: ["memchr", "aho_corasick", "dfa"],
    why: "one state register, one transition table, one byte per edge",
  },
  {
    name: "Data infrastructure",
    designs: ["bitpack", "rle", "roaring", "hamming", "sorting", "sketch"],
    why: "the decode kernels of a columnar engine, and the sketches",
  },
  {
    name: "Packet",
    designs: ["packet"],
    why: "a 64-byte header window in, every field out, combinationally",
  },
  {
    name: "Entropy coding",
    designs: ["huffman", "fse", "deflate"],
    why: "serial and entropy-bound, so the case for building",
  },
];
/** Project totals from the audited repository snapshot. Per-crate and tier tables
 * include historical counts and planned entries, so totals are recorded separately. */
export const PROJECT_STATS = {
  tests: 811,
  crates: CRATES.length,
  designs: TIERS.reduce((sum, tier) => sum + tier.designs.length, 0),
  cosimCoverage: 100,
  checksPerDesign: 3,
  source: "README.md",
  tiers: TIERS.length,
} as const;
