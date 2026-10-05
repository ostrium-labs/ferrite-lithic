//! Canonical Huffman decode, against the design and against Verilator.
//!
//! # Why this file exists
//!
//! [`huffman`] shipped with unit tests on its *table construction* and nothing at all on
//! its *decoder*: no test ever pushed a bit stream through [`huffman::build`] and read a
//! symbol back out. A canonical-code builder and a canonical-code decoder are separate
//! pieces of work that can each be right while the pair is wrong -- the decode table's
//! index order and the design's peek reversal have to agree, and nothing checked that.
//!
//! # What the evidence is
//!
//! In order of strength:
//!
//! 1. **RFC 1951's fixed literal/length code**, whose assignment is published in the RFC
//!    and therefore external to this repository. Running [`canonical_codes`] over it and
//!    comparing the result against the RFC's own numbers checks the builder against a
//!    source nobody here wrote.
//! 2. **A round trip through an independently written encoder**, which writes each symbol's
//!    canonical code into a DEFLATE-ordered bit stream. It is a second transcription of
//!    the canonical rule, which is the point: it inverts the decoder rather than
//!    reimplementing it.
//! 3. **A round trip through the design**, driven a bit at a time through the step
//!    testbench, so a test says what the decoder does rather than what it is built from.
//! 4. **Verilator equivalence**, so the emitted Verilog retires the same symbols on the
//!    same cycles as the simulator.
//!
//! What none of that is: a decode of a real DEFLATE stream. Extracting the symbols from
//! one means the bit layer, the block header and the length/distance machinery, none of
//! which this module claims to implement -- see its own docs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::huffman::{
    self, CodeWord, FIXED_LITERAL_LENGTH_LENGTHS, TABLE_BITS, Table, canonical_codes,
};
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::{Handle, Testbench};
use proptest::prelude::*;

/// The stream the design sees: bits in order, which for DEFLATE is least significant bit
/// of each byte first.
///
/// A Huffman code is written **most significant bit first**, so the encoder appends a
/// code's bits from its top down. That is the one convention in this module that is easy to
/// get backwards, and getting it backwards produces a design that decodes a stream of the
/// wrong bit order into plausible-looking symbols.
fn reference_encode(codes: &[CodeWord], symbols: &[u16]) -> Vec<bool> {
    let mut bits = Vec::new();
    for symbol in symbols {
        let word = codes
            .iter()
            .find(|word| word.symbol == *symbol)
            .expect("the symbol has a code");
        assert!(word.length > 0, "symbol {symbol} does not appear");
        for position in (0..word.length).rev() {
            bits.push((word.code >> position) & 1 == 1);
        }
    }
    bits
}

/// The decoding direction in plain Rust: peek, look up, consume the code's length.
///
/// The peek is built most significant bit first out of the *stream order*, which is what
/// the design's reversal amounts to. A length of zero means no code claims the index, so
/// the stream is finished.
fn reference_decode(table: &Table, bits: &[bool]) -> Vec<u16> {
    let mut at = 0usize;
    let mut out = Vec::new();
    while at < bits.len() {
        let mut index = 0u16;
        for position in 0..TABLE_BITS {
            let bit = bits.get(at + position as usize).copied().unwrap_or(false);
            index = (index << 1) | u16::from(bit);
        }
        let (symbol, length) = table.entry(index);
        if length == 0 {
            break;
        }
        out.push(symbol);
        at += usize::from(length);
    }
    out
}

/// One run of the decoder.
struct Run {
    symbols: Vec<u16>,
    lengths: Vec<u8>,
    /// The longest run of consecutive cycles with `out_valid` high.
    longest_run: usize,
    /// Whether `busy` was ever high.
    saw_busy: bool,
}

async fn drive(tb: &Handle, bits: &[bool]) {
    tb.set("init", 1).await;
    tb.step().await;
    tb.set("init", 0).await;
    let mut at = 0usize;
    while at < bits.len() {
        tb.set("in_bit", u64::from(bits[at])).await;
        // `value` reads the settled value, which is what the design latches on the edge
        // that follows; presenting a bit while `in_ready` is low would drop it.
        let ready = tb.value("in_ready") == 1;
        tb.set("in_valid", u64::from(ready)).await;
        tb.step().await;
        if ready {
            at += 1;
        }
    }
    tb.set("in_valid", 0).await;
    // Drain until the buffer is empty rather than for a fixed count. The decoder has no
    // `done`: it stops when `held` reaches zero, and how many cycles that takes depends on
    // the code lengths of the symbols still buffered.
    for _ in 0..64 {
        if tb.value("busy") == 0 {
            break;
        }
        tb.step().await;
    }
}

fn collect(tb: &Testbench) -> Run {
    let valid = tb.series("out_valid");
    let symbols = tb.series("symbol");
    let lengths = tb.series("code_len");
    let busy = tb.series("busy");

    let mut run = Run {
        symbols: Vec::new(),
        lengths: Vec::new(),
        longest_run: 0,
        saw_busy: false,
    };
    let mut streak = 0usize;
    for index in 0..valid.len() {
        if busy[index].to_u64().expect("one bit") == 1 {
            run.saw_busy = true;
        }
        if valid[index].to_u64().expect("one bit") == 1 {
            run.symbols
                .push(symbols[index].to_u64().expect("nine bits") as u16);
            run.lengths
                .push(lengths[index].to_u64().expect("four bits") as u8);
            streak += 1;
            run.longest_run = run.longest_run.max(streak);
        } else {
            streak = 0;
        }
    }
    run
}

fn hardware(table: &Table, bits: &[bool]) -> Run {
    let design = Design::new();
    huffman::build(&design, table).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let owned = bits.to_vec();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        drive(&tb, &owned).await;
    })
    .expect("the huffman design drives only ports it declares");
    collect(&tb)
}

/// The fixed literal/length code, which is the deepest code a [`Table`] holds exactly.
fn fixed() -> (Table, Vec<CodeWord>) {
    (
        Table::canonical(&FIXED_LITERAL_LENGTH_LENGTHS).unwrap(),
        canonical_codes(&FIXED_LITERAL_LENGTH_LENGTHS).unwrap(),
    )
}

#[test]
fn the_fixed_codes_are_rfc_1951s_own_numbers() {
    // RFC 1951 section 3.2.6 publishes the assignment, so this is a check against a source
    // outside this repository. The derivation: canonical order by (length, symbol), so the
    // twenty-four seven-bit codes are 0..23, the 152 eight-bit codes continue from
    // (0 + 24) << 1 = 48, and the 112 nine-bit codes from (199 + 1) << 1 = 400.
    let codes = canonical_codes(&FIXED_LITERAL_LENGTH_LENGTHS).unwrap();
    let word = |symbol: usize| codes[symbol];
    assert_eq!(
        word(256),
        CodeWord {
            symbol: 256,
            length: 7,
            code: 0
        }
    );
    assert_eq!(
        word(279),
        CodeWord {
            symbol: 279,
            length: 7,
            code: 23
        }
    );
    assert_eq!(
        word(0),
        CodeWord {
            symbol: 0,
            length: 8,
            code: 48
        }
    );
    assert_eq!(
        word(143),
        CodeWord {
            symbol: 143,
            length: 8,
            code: 191
        }
    );
    assert_eq!(
        word(280),
        CodeWord {
            symbol: 280,
            length: 8,
            code: 192
        }
    );
    assert_eq!(
        word(287),
        CodeWord {
            symbol: 287,
            length: 8,
            code: 199
        }
    );
    assert_eq!(
        word(144),
        CodeWord {
            symbol: 144,
            length: 9,
            code: 400
        }
    );
    assert_eq!(
        word(255),
        CodeWord {
            symbol: 255,
            length: 9,
            code: 511
        }
    );
}

#[test]
fn the_fixed_code_fills_every_index() {
    // A complete code is the precondition for the zero-padding argument in the decoder: a
    // nine-bit peek always lands on a code, so a partly filled buffer either matches
    // something inside its real bits or is refused by the length check.
    let (table, _) = fixed();
    for index in 0..512u32 {
        assert_ne!(
            table.entry(index as u16).1,
            0,
            "index {index} claims no code"
        );
    }
}

#[test]
fn the_design_decodes_a_hand_computed_message() {
    // A code small enough to read out by hand: symbols 0, 1 and 2 with codes "0", "10" and
    // "11". The message is 0, 1, 1, 0, 2, so the stream is 0 10 10 0 11 -- eleven bits.
    let table = Table::canonical(&[1u8, 2, 2]).unwrap();
    let codes = canonical_codes(&[1u8, 2, 2]).unwrap();
    let symbols = [0u16, 1, 1, 0, 2];
    let bits = reference_encode(&codes, &symbols);
    assert_eq!(
        bits,
        vec![false, true, false, true, false, false, true, true],
        "the stream is each code's bits, top down: 0 | 10 | 10 | 0 | 11"
    );

    assert_eq!(reference_decode(&table, &bits), symbols.to_vec());
    let run = hardware(&table, &bits);
    assert_eq!(run.symbols, symbols.to_vec());
    assert_eq!(
        run.lengths,
        vec![1u8, 2, 2, 1, 2],
        "and each symbol reports the code length it cost"
    );
}

#[test]
fn the_design_decodes_deflates_fixed_literal_code() {
    // The real code, on a message that exercises all three depths: an eight-bit literal, a
    // nine-bit literal and the seven-bit end-of-block code.
    let (table, codes) = fixed();
    let symbols = [97u16, 0, 255, 144, 256, 65, 286];
    let bits = reference_encode(&codes, &symbols);
    assert_eq!(reference_decode(&table, &bits), symbols.to_vec());

    let run = hardware(&table, &bits);
    assert_eq!(run.symbols, symbols.to_vec(), "the fixed code decodes");
    assert!(run.saw_busy);
}

#[test]
fn a_single_symbol_code_decodes_every_symbol() {
    // RFC 1951 section 3.2.7's single-code rule, and the one incomplete code the module
    // supports: every entry says the same thing, so the peek cannot fail.
    let table = Table::single(7);
    let bits = vec![true, false, true, true, false];
    let run = hardware(&table, &bits);
    assert_eq!(run.symbols, vec![7u16; 5]);
    assert_eq!(run.lengths, vec![1u8; 5]);
}

#[test]
fn a_truncated_code_is_a_stall_the_host_can_see() {
    // A code longer than the bits delivered: the decoder must not invent a symbol and must
    // not raise an error either -- it holds `busy` high with `out_valid` low, which is the
    // only malformed-stream signal this module has.
    let table = Table::canonical(&[1u8, 2, 2]).unwrap();
    let bits = vec![true]; // half of the "10" that symbol 1 needs
    let run = hardware(&table, &bits);
    assert!(run.symbols.is_empty(), "and it decodes nothing");
    assert!(run.saw_busy, "while the host can see it has not finished");
}

#[test]
fn a_one_bit_code_retires_one_symbol_per_cycle() {
    // The strongest shape claim this design can make, and the only code for which it is
    // true. A decode consumes `code_len` bits and the host supplies one per cycle, so a
    // symbol can be retired on every cycle exactly when `code_len == 1` -- which is
    // [`Table::single`] and nothing else. DEFLATE's shortest fixed code is seven bits, so
    // the real rate is one symbol per code length, and claiming otherwise here would be
    // claiming a property the format does not have.
    let table = Table::single(7);
    let bits = vec![true; 40];
    let run = hardware(&table, &bits);
    assert_eq!(run.symbols, vec![7u16; 40]);
    assert!(
        run.longest_run >= 8,
        "only {} consecutive symbols for a one-bit code, which would mean a stall",
        run.longest_run
    );
}

#[test]
fn a_long_code_costs_exactly_its_length_in_cycles() {
    // The rate a real code gets, measured rather than asserted: eight-bit literals come
    // out one per eight cycles, because the buffer only holds nine bits and a decode
    // spends eight of them. This is the number the module docs quote against the
    // alternatives, so it is the one that has to be true.
    let (table, codes) = fixed();
    let symbols = vec![97u16; 40];
    let bits = reference_encode(&codes, &symbols);
    let run = hardware(&table, &bits);
    assert_eq!(run.symbols, symbols);
    assert!(
        run.longest_run <= 2,
        "{} consecutive symbols for an eight-bit code is faster than the code allows",
        run.longest_run
    );
}

#[test]
fn a_code_too_deep_for_the_table_is_refused() {
    // The honest limit rather than a silent truncation: a ten-bit code cannot be resolved
    // by a nine-bit peek, so it is an error and not a shorter symbol.
    let mut lengths = vec![0u8; 11];
    lengths[0] = 10;
    assert!(
        canonical_codes(&lengths).is_ok(),
        "DEFLATE allows fifteen bits"
    );
    assert_eq!(
        Table::canonical(&lengths).unwrap_err(),
        huffman::CodeError::TooDeep { length: 10 },
        "but this table cannot hold one"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// The round trip, on arbitrary messages over the fixed code.
    ///
    /// Sixteen cases rather than the default hundred: the fixed code's table is 512 arms
    /// of `Design::case_`, so each case builds a graph rather than running a loop, and a
    /// hundred of those is a slow test rather than a thorough one. Coverage of the
    /// *stream* comes from the arbitrary messages; coverage of the *table* comes from the
    /// RFC check above.
    #[test]
    fn the_design_decodes_arbitrary_messages_over_the_fixed_code(
        message in prop::collection::vec(any::<u16>(), 1..40),
    ) {
        let (table, codes) = fixed();
        let symbols: Vec<u16> = message.iter().map(|symbol| symbol % 288).collect();
        let bits = reference_encode(&codes, &symbols);
        prop_assert_eq!(reference_decode(&table, &bits), symbols.clone());
        let run = hardware(&table, &bits);
        prop_assert_eq!(run.symbols, symbols);
    }
}

/// The cosimulation plan for a built design.
fn plan_of(design: &Design, name: &str) -> Plan {
    let clock = design
        .input_ports()
        .iter()
        .find(|signal| signal.name().as_deref() == Some(ferrite_lithic_corpus::CLOCK))
        .expect("the clock is a declared input")
        .id();
    let module = ferrite_lithic_rtl::Module::new(
        name,
        design.build().unwrap(),
        clock,
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    Plan::of(design, &module).unwrap()
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_huffman_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the huffman equivalence did not run. The \
             round trip above does not need it."
        );
        return;
    };
    println!("cosimulating huffman with {}", verilator.display());

    let (table, codes) = fixed();
    let symbols: Vec<u16> = vec![97, 0, 255, 144, 65, 256, 286, 97, 97, 98];
    let bits = reference_encode(&codes, &symbols);

    let design = Design::new();
    huffman::build(&design, &table).unwrap();
    let plan = plan_of(&design, "huffman");

    assert_eq!(
        plan.inputs
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "init", "in_valid", "in_bit"],
        "the clock is a port but not a stimulus column"
    );
    assert!(
        plan.outputs.iter().all(|port| port.width <= 64),
        "no port is wider than the generated driver's limit"
    );

    // One bit every two cycles, because the design's `in_ready` drops when the buffer is
    // full and a one-bit-per-cycle stimulus would overrun it. The stimulus is the same
    // handshake the testbench uses, expressed as fixed-width rows.
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, init: u64, valid: u64, bit: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(init, 1).unwrap(),
            Bits::constant(valid, 1).unwrap(),
            Bits::constant(bit, 1).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0, 0)).unwrap();
    stimulus.push(row(0, 1, 0, 0)).unwrap();
    for bit in bits {
        stimulus.push(row(0, 0, 1, u64::from(bit))).unwrap();
        stimulus.push(row(0, 0, 0, 0)).unwrap();
    }
    for _ in 0..24 {
        stimulus.push(row(0, 0, 0, 0)).unwrap();
    }

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And all three meet.
    assert_eq!(
        hardware(&table, &reference_encode(&codes, &symbols)).symbols,
        symbols
    );
}
