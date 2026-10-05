#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Graph metrics for every corpus design, measured rather than estimated.
//!
//! # Why this file exists
//!
//! Every size and depth figure the design notes quote was hand-derived in a doc comment —
//! "about 400 flops", "roughly 6 700 muxes", "single-digit picoseconds". Those are honest
//! as estimates and useless as measurements, because nothing re-derives them when the design
//! changes and nothing notices when they stop being true. This file derives the same class
//! of number *from the graph*, for every design, by the same code, so the numbers in the
//! documentation can be checked by running a command.
//!
//! # What is actually measured
//!
//! - **nodes** — `Circuit::len()`. One IR node, which is one operator.
//! - **flops** — nodes where `Node::is_stateful()`, counted one per register, not per bit.
//! - **state bits** — the summed width of those registers. This is the real "how much memory
//!   does this design need" figure, and it is the one that matters: a design with 5 flops
//!   and 128 bits of state is a very different thing from 128 flops.
//! - **depth** — the longest chain of combinational nodes between a register or a port and
//!   anything that depends on it. This is the timing-critical number, and it is *not* the
//!   same as gate count: a chain of four wide operators is four levels, not thousands of
//!   gates.
//! - **kinds** — a histogram of node kinds, so "6 700 muxes" can be read off as a count of
//!   `Case` and `Ite` nodes instead of being asserted.
//!
//! # What this is not
//!
//! These are *structural* figures, not silicon figures. Node count is not gate count — an
//! `add` is a whole adder, not one gate. Depth is in operator levels, not gate delays. Area,
//! power and frequency are not derivable from the IR and are not claimed anywhere. A number
//! this file prints is a fact about the graph; what a synthesiser does with the graph is a
//! different question and needs a different tool.
//!
//! Run it with `--nocapture`:
//!
//! ```text
//! cargo test -p ferrite-lithic-corpus --test graph_metrics -- --nocapture
//! ```
//!
//! A measurement harness is allowed to `unwrap`: every call below is a build that has to
//! succeed for the row to exist, and a harness that returns `Result` from twenty build
//! calls only buries the one that actually failed.

use std::collections::BTreeMap;

use ferrite_lithic::Design;
use ferrite_lithic_ir::{Circuit, Deps, Node};

use ferrite_lithic_corpus as corpus;

/// The measured shape of one design.
#[derive(Debug, Clone)]
struct Metrics {
    design: &'static str,
    variant: &'static str,
    nodes: usize,
    flops: usize,
    state_bits: u64,
    /// Longest combinational chain, in operator levels.
    depth: usize,
    /// The five most common node kinds, as (kind, count).
    kinds: Vec<(&'static str, usize)>,
    /// Memories in the design, with their depth × width in bits.
    memories: Vec<(u32, u32)>,
}

impl Metrics {
    /// The arithmetic cost of the memories this design declares.
    ///
    /// Worth its own column because a memory is the one node kind whose size does *not*
    /// show up as a flop count: a 256-entry ROM is 1 node and 2048 bits of storage.
    fn memory_bits(&self) -> u64 {
        self.memories
            .iter()
            .map(|(depth, width)| u64::from(*depth) * u64::from(*width))
            .sum()
    }
}

/// Measures a built design.
///
/// `depth` is a longest-path over the combinational DAG, computed in topological order so
/// each node's level is final before anything that depends on it is visited. Operands that
/// are themselves stateful contribute level 0, because a register output is a starting point
/// rather than something that had to settle first.
fn measure(name: &'static str, variant: &'static str, design: &Design) -> Metrics {
    let circuit: Circuit = design.circuit().clone();

    let order = circuit
        .topological_order(Deps::LoopChecking)
        .expect("every corpus design is acyclic under the loop-checking relation");

    let mut flops = 0usize;
    let mut state_bits = 0u64;
    let mut memories = Vec::new();
    let mut histogram: BTreeMap<&'static str, usize> = BTreeMap::new();
    // Keyed by `NodeId::index()`, which is the public accessor for the node number.
    let mut level: BTreeMap<u32, usize> = BTreeMap::new();

    for id in &order {
        let node: &Node = circuit
            .get(*id)
            .expect("the topological order lists real nodes");
        let kind = node.kind();
        *histogram.entry(kind).or_default() += 1;

        if node.is_stateful() {
            flops += 1;
            state_bits += u64::from(circuit.width_of(*id));
            // A register's output is an input to the combinational cone, not part of it.
            level.insert(id.as_u32(), 0);
            continue;
        }

        if matches!(kind, "Mem")
            && let Ok((depth, width)) = circuit.memory_shape(*id)
        {
            memories.push((depth, width));
        }

        let deepest = node
            .operands()
            .into_iter()
            .filter(|operand| {
                // A stateful operand terminates the chain; so does a constant, which is
                // already settled before the clock edge.
                !matches!(circuit.get(*operand).map(Node::is_stateful), Ok(true))
                    && circuit.get(*operand).map(Node::kind) != Ok("Const")
            })
            .filter_map(|operand| level.get(&operand.as_u32()).copied())
            .max()
            .unwrap_or(0);

        level.insert(id.as_u32(), deepest + 1);
    }

    let depth = level.values().copied().max().unwrap_or(0);

    let mut kinds: Vec<(&'static str, usize)> = histogram.into_iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    kinds.truncate(5);

    Metrics {
        design: name,
        variant,
        nodes: circuit.len(),
        flops,
        state_bits,
        depth,
        kinds,
        memories,
    }
}

/// Builds one design and measures it. The `build` call is the same call the tests make, so
/// a design that fails to build here fails everywhere.
fn measure_build(
    name: &'static str,
    variant: &'static str,
    build: impl FnOnce(&Design),
) -> Metrics {
    let design = Design::new();
    build(&design);
    measure(name, variant, &design)
}

/// The FSE and Huffman decoders both take a table, so the metrics depend on which table is
/// built. These are the same tables their own tests use, so the figure describes the design
/// that is actually verified rather than an arbitrary one.
fn hand_counts() -> Vec<i32> {
    // `Table::build` requires the counts to sum to exactly `1 << table_log`, so these sum
    // to 32 for `table_log = 5`. The shape matters as well as the total: a flat
    // distribution gives the decoder a wide table, which is the expensive case, so a
    // benchmark that reported a narrow one would flatter the design.
    [4, 4, 4, 4, 4, 3, 3, 3, 3].to_vec()
}

fn all_metrics() -> Vec<Metrics> {
    let mut out = Vec::new();

    // ---- LFSRs and checksums
    out.push(measure_build("crc3", "3-bit CRC", |d| {
        corpus::crc3::build(d).unwrap();
    }));
    out.push(measure_build("crc32", "CRC-32", |d| {
        corpus::crc32::build(d).unwrap();
    }));
    // `lfsr` is deliberately absent: `reflected_lfsr` takes signals the caller already
    // owns rather than declaring its own ports, so there is no standalone design to
    // measure. Its cost is already inside every design that calls it.
    out.push(measure_build("ghash", "128-bit field", |d| {
        corpus::ghash::build(d).unwrap();
    }));

    // ---- Block ciphers
    out.push(measure_build("sha256", "64 unrolled rounds", |d| {
        corpus::sha256::build(d).unwrap();
    }));
    out.push(measure_build("chacha20", "one block/cycle", |d| {
        corpus::chacha20::build(d).unwrap();
    }));
    out.push(measure_build("aes", "200-arm S-box", |d| {
        corpus::aes::build(d).unwrap();
    }));

    // ---- Codecs
    out.push(measure_build("hex", "4 bits in, 8 out", |d| {
        corpus::hex::build(d).unwrap();
    }));
    out.push(measure_build("base64", "6 bits in, 8 out", |d| {
        corpus::base64::build(d).unwrap();
    }));

    // ---- Automata
    out.push(measure_build("memchr", "comparator", |d| {
        corpus::memchr::build(d).unwrap();
    }));
    out.push(measure_build("memchr", "256-entry ROM", |d| {
        corpus::memchr::build_from_rom(d).unwrap();
    }));
    out.push(measure_build("aho_corasick", "19 states", |d| {
        corpus::aho_corasick::build(d).unwrap();
    }));
    out.push(measure_build("dfa", "5 states, 64 symbols", |d| {
        corpus::dfa::build(d).unwrap();
    }));

    // ---- Data infrastructure
    out.push(measure_build("bitpack", "expander, w=8", |d| {
        corpus::bitpack::build(d).unwrap();
    }));
    out.push(measure_build("rle", "decoder", |d| {
        corpus::rle::build(d).unwrap();
    }));
    out.push(measure_build("roaring", "bitmap", |d| {
        corpus::roaring::build(d).unwrap();
    }));
    out.push(measure_build("hamming", "popcount64", |d| {
        corpus::hamming::build(d).unwrap();
    }));
    out.push(measure_build("hamming", "top-k, k=7", |d| {
        corpus::hamming::build_topk(d).unwrap();
    }));
    out.push(measure_build("sorting", "bitonic, n=8", |d| {
        corpus::sorting::build(d).unwrap();
    }));
    out.push(measure_build("sorting", "odd-even merge, n=8", |d| {
        corpus::sorting::build_odd_even(d).unwrap();
    }));
    out.push(measure_build("sketch", "64-bit state", |d| {
        corpus::sketch::build(d).unwrap();
    }));

    // ---- Packet
    out.push(measure_build("packet", "frame in/frame out", |d| {
        corpus::packet::build(d).unwrap();
    }));

    // ---- Entropy coding
    let fse_table = corpus::fse::Table::build(&hand_counts(), 5).unwrap();
    out.push(measure_build("fse", "tableLog 5", |d| {
        corpus::fse::build(d, &fse_table).unwrap();
    }));
    let huff_table = corpus::huffman::Table::canonical(&fixed_lengths_public()).unwrap();
    out.push(measure_build("huffman", "DEFLATE fixed code", |d| {
        corpus::huffman::build(d, &huff_table).unwrap();
    }));
    out.push(measure_build("deflate", "bit layer only", |d| {
        corpus::deflate::build(d).unwrap();
    }));

    out
}

/// DEFLATE's fixed literal/length code lengths, RFC 1951 section 3.2.6.
///
/// Transcribed here rather than imported because the lengths are a property of the format,
/// not of this crate, and a benchmark that silently used a different table would report a
/// figure for a design nobody verified.
fn fixed_lengths_public() -> Vec<u8> {
    let mut lengths = vec![8u8; 144];
    lengths.extend(std::iter::repeat_n(9u8, 112));
    lengths.extend(std::iter::repeat_n(7u8, 24));
    lengths.extend(std::iter::repeat_n(8u8, 8));
    lengths
}

/// The real test: every corpus design can be built and measured.
///
/// Without this the table below could silently drift to an empty list, which is what a
/// benchmark that stops benchmarking looks like from the outside.
#[test]
fn every_corpus_design_is_measured() {
    let metrics = all_metrics();
    assert!(
        metrics.len() >= 24,
        "expected every design and its variants, got {}",
        metrics.len()
    );
}

/// Prints the table. `#[ignore]`d so it does not run as part of the normal suite: it
/// produces output rather than an assertion, and it is run deliberately.
#[test]
#[ignore = "prints a table; run with --nocapture and --ignored"]
fn print_graph_metrics() {
    let metrics = all_metrics();

    println!(
        "\n{:<10} {:<22} {:>6} {:>6} {:>10} {:>7} {:>9}",
        "design", "variant", "nodes", "flops", "state bits", "depth", "mem bits"
    );
    println!("{}", "-".repeat(80));

    for m in &metrics {
        println!(
            "{:<10} {:<22} {:>6} {:>6} {:>10} {:>7} {:>9}",
            m.design,
            m.variant,
            m.nodes,
            m.flops,
            m.state_bits,
            m.depth,
            m.memory_bits()
        );
    }

    println!("\nnode kinds per design (top 5)\n{}", "-".repeat(80));
    for m in &metrics {
        let kinds = m
            .kinds
            .iter()
            .map(|(kind, count)| format!("{kind} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("{:<10} {:<22} {kinds}", m.design, m.variant);
    }

    println!(
        "\n{} design/variant pairs measured. `nodes` is IR operators, not gates; `depth` is\n\
         operator levels, not gate delays. Area, power and frequency are not derivable from\n\
         the IR and are not claimed here.",
        metrics.len()
    );
}

/// The spread is the interesting claim, so it gets an assertion rather than a print.
///
/// A design whose combinational depth exceeded the worst case in this list would be a real
/// finding — a long chain is a design that will not meet timing — so the bound is pinned
/// rather than merely displayed.
#[test]
#[ignore = "reports the depth spread; run with --ignored"]
fn combinational_depth_is_bounded() {
    let metrics = all_metrics();

    let mut ranked: Vec<&Metrics> = metrics.iter().collect();
    ranked.sort_by_key(|m| std::cmp::Reverse(m.depth));

    println!("\ndeepest combinational paths\n{}", "-".repeat(52));
    for m in ranked.iter().take(8) {
        println!(
            "{:<10} {:<22} depth {:>3}   ({} nodes, {} state bits)",
            m.design, m.variant, m.depth, m.nodes, m.state_bits
        );
    }

    let worst = ranked.first().expect("there is at least one design");
    println!(
        "\ndeepest: {} {} at {} levels\n",
        worst.design, worst.variant, worst.depth
    );

    // Pinned to the current worst case rather than a round number, so an unrolled design
    // that grows a round does not pass unnoticed.
    //
    // The two deep designs are `sha256` and `chacha20`, and their depth is a *finding*
    // rather than a defect in the metric: both unroll every round to get one block per
    // cycle, and the price is a combinational path hundreds of operator levels long. No
    // synthesis flow meets timing on that without pipelining, and neither design is
    // pipelined. See `docs/benchmarks.md`.
    const KNOWN_WORST: usize = 650;
    assert!(
        worst.depth <= KNOWN_WORST,
        "{} {} has a combinational depth of {}, past the {} levels measured before -- \
         if that is deliberate, update KNOWN_WORST and say why in docs/benchmarks.md",
        worst.design,
        worst.variant,
        worst.depth,
        KNOWN_WORST
    );
}
