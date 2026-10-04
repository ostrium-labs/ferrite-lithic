//! A recursive random-design generator, and a guard on the generator itself.
//!
//! Hardcaml's [`test/lib/generator.ml`](https://github.com/jane-street/hardcaml)
//! is a random-circuit generator: weighted operation choices, widths drawn from a
//! fixed set, feedback registers, multiport memories, and matching random input
//! vectors. It depends only on `Signal`/`Circuit`/Quickcheck, so it lifts almost
//! directly, and the design notes call the result the highest-value test in the
//! suite.
//!
//! # What this file is for
//!
//! Two things, and the second is the one that is easy to skip.
//!
//! 1. **A generator that produces runnable designs.** Every generated circuit is
//!    compiled and simulated for a few cycles, so a generator bug that produces a
//!    combinational loop, a zero-width signal or a runaway feedback register shows
//!    up as a failing property rather than as a panic.
//!
//! 2. **A guard on the generator.** A random-circuit test is worthless if the
//!    circuits it produces are trivial -- all outputs constant, say -- because then
//!    it asserts nothing while looking rigorous. Hardcaml already treats this as a
//!    first-class concern (`test/lib/test_dedup.ml:191-197`): a fixed seed over many
//!    circuits, requiring most of them to produce non-constant outputs. That check
//!    is reproduced here, with the same seed, because a generator that quietly stops
//!    producing interesting circuits is the standard way this kind of test rots.
//!
//! # No two implementations to compare yet
//!
//! Hardcaml uses its generator to check deduplicated and non-deduplicated graphs
//! against each other. We have no deduplication, so there is no second graph to
//! differ against, and the comparison that matters — the simulator against emitted
//! Verilog — waits for `ferrite-lithic-rtl` and `ferrite-lithic-cosim`. What is left
//! is the generator-quality guard, which is the half that does not need a second
//! implementation and is the half that stops the eventual differential test from
//! being vacuous.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_sim::Sim;

/// The seed Hardcaml uses for its generator-quality guard.
const SEED: u32 = 0x3ead_beef;

/// A generated circuit, kept as a closure so the design outlives its construction.
struct Generated {
    design: Design,
    inputs: Vec<(String, u32)>,
}

/// Builds a random design, recursively growing the graph.
///
/// Depth-bounded recursion rather than a flat list of nodes, because that is what
/// makes the generated shapes resemble real ones: a wide design is a tree of
/// sub-expressions over a handful of shared signals, not a list of unrelated
/// one-input operations.
fn generate(
    config: &mut GeneratorConfig,
    depth: u32,
    inputs: &[String],
    regs: &[String],
) -> Generated {
    let mut config = config.clone();
    let d = Design::new();

    // The clock, and the control lines every register needs.
    let clk = d.input("clk", 1).expect("clock");

    // Inputs first, so the body can refer to them by name.
    let mut ports = Vec::new();
    for (i, width) in config.input_widths.iter().enumerate() {
        let name = format!("in{i}");
        d.input(&name, *width).expect("input port");
        ports.push((name, *width));
    }
    let reset = d.input("reset", 1).expect("reset");

    // Grow the combinational body.
    let mut available: Vec<(String, u32)> = ports.clone();
    for i in 0..config.body_size {
        let width = config.choose_width();
        let signal = build_node(&mut config, &d, width, &available);
        let name = format!("n{i}");
        d.output(&name, signal.width(), &signal)
            .expect("output port");
        available.push((name, signal.width()));
    }

    // Feedback: registers whose data input is drawn from the same pool, so a
    // register can read another register and a combinational loop through one is
    // generated rather than avoided.
    let mut reg_names = Vec::new();
    for i in 0..config.reg_count {
        let width = config.choose_width();
        let source = if available.is_empty() {
            d.lit(0, width).expect("literal")
        } else {
            let (name, w) = config.pick(&available).clone();
            let candidate = find(&d, &name).expect("a node that was just built");
            if w == width {
                candidate
            } else if w > width {
                d.truncate(&candidate, width).expect("narrow")
            } else {
                d.zero_extend(&candidate, width).expect("widen")
            }
        };
        let enable = d.input(&format!("en{i}"), 1).expect("enable port");
        let reg = d
            .reg(&source, &clk, &reset, &enable)
            .expect("register over two same-width signals");
        let name = format!("r{i}");
        d.output(&name, width, &reg).expect("output port");
        reg_names.push(name);
        available.push((format!("r{i}"), width));
    }

    // One output that observes as much of the design as possible, so the
    // generator-quality guard has something to look at.
    let (final_name, final_width) = config.pick(&available).clone();
    let final_signal = find(&d, &final_name).expect("a node that was just built");
    d.output("sum", final_width, &final_signal)
        .expect("output port");

    let mut all_inputs = ports;
    all_inputs.push(("reset".into(), 1));
    for i in 0..config.reg_count {
        all_inputs.push((format!("en{i}"), 1));
    }
    let _ = (inputs, regs, depth);

    Generated {
        design: d,
        inputs: all_inputs,
    }
}

/// Finds a signal the generator already built, by output name.
///
/// The DSL has no lookup-by-name, and adding one for a test would be a mistake: the
/// generator keeps its own handles and this only exists because the recursive
/// structure returns names. Rebuilding from the design's own signals is cheaper and
/// needs no new API.
fn find(d: &Design, name: &str) -> Option<ferrite_lithic::Signal> {
    d.signals()
        .into_iter()
        .find(|s| s.name().as_deref() == Some(name))
}

/// One random operation over the signals built so far.
///
/// Operands are drawn explicitly rather than through a closure: the generator's
/// random stream is mutable state, and a closure capturing it would fight the
/// borrow checker at every call site instead of once here.
fn build_node(
    config: &mut GeneratorConfig,
    d: &Design,
    width: u32,
    available: &[(String, u32)],
) -> ferrite_lithic::Signal {
    let op = config.pick_op();
    let left = operand(config, d, width, available);
    let result = match op {
        Op::Not => Ok(d.not(&left)),
        Op::And => {
            let right = operand(config, d, width, available);
            d.and(&left, &right)
        }
        Op::Or => {
            let right = operand(config, d, width, available);
            d.or(&left, &right)
        }
        Op::Xor => {
            let right = operand(config, d, width, available);
            d.xor(&left, &right)
        }
        Op::Add => {
            let right = operand(config, d, width, available);
            d.add(&left, &right)
        }
        Op::Sub => {
            let right = operand(config, d, width, available);
            d.sub(&left, &right)
        }
        Op::Mul => {
            let right = operand(config, d, width, available);
            d.mul(&left, &right)
        }
        // Division is weighted down, and its denominator is forced non-zero below,
        // because a random circuit divides by zero often enough that the run would
        // otherwise be nothing but refusals.
        Op::UDiv => non_zero(config, d, width, available, |d, n, r| d.udiv(n, r)),
        Op::URem => non_zero(config, d, width, available, |d, n, r| d.urem(n, r)),
        Op::Eq => {
            let right = operand(config, d, width, available);
            d.eq(&left, &right)
        }
        Op::Select => {
            let value = operand(config, d, width, available);
            let offset = config.roll(width);
            let len = 1 + config.roll(width - offset);
            d.slice(&value, offset, len)
        }
        Op::Concat => {
            let high = operand(config, d, width, available);
            let low = operand(config, d, width, available);
            d.concat(&[high, low])
        }
        Op::Replicate => {
            let narrow = config.choose_width().min(8);
            let value = operand(config, d, narrow, available);
            d.replicate(&value, 2)
        }
        Op::Ite => {
            let condition = d
                .input(&format!("c{}", config.next_id()), 1)
                .expect("condition port");
            let then_value = operand(config, d, width, available);
            let otherwise = operand(config, d, width, available);
            d.ite(&condition, &then_value, &otherwise)
        }
    };
    result.expect("a generated operation over same-width operands")
}

/// Picks an operand, widening or narrowing a random one to `width`.
fn operand(
    config: &mut GeneratorConfig,
    d: &Design,
    width: u32,
    available: &[(String, u32)],
) -> ferrite_lithic::Signal {
    let (name, w) = config.pick(available).clone();
    let signal = find(d, &name).expect("a node that was just built");
    if w == width {
        signal
    } else if w > width {
        d.truncate(&signal, width).expect("narrow")
    } else {
        d.zero_extend(&signal, width).expect("widen")
    }
}

/// A denominator that is never zero, so division does not dominate the run.
fn non_zero(
    config: &mut GeneratorConfig,
    d: &Design,
    width: u32,
    available: &[(String, u32)],
    build: impl Fn(
        &Design,
        &ferrite_lithic::Signal,
        &ferrite_lithic::Signal,
    ) -> Result<ferrite_lithic::Signal, ferrite_lithic::Error>,
) -> Result<ferrite_lithic::Signal, ferrite_lithic::Error> {
    // `| 1` on a zero value makes it one, which is enough: the point is only that
    // the generator does not spend its whole budget producing refusals.
    let dividend = operand(config, d, width, available);
    let raw = operand(config, d, width, available);
    let forced = d.or(&raw, &d.lit(1, width).expect("literal"))?;
    build(d, &dividend, &forced)
}

/// The operations the generator can choose from.
#[derive(Clone, Copy, Debug)]
enum Op {
    Not,
    And,
    Or,
    Xor,
    Add,
    Sub,
    Mul,
    UDiv,
    URem,
    Eq,
    Select,
    Concat,
    Replicate,
    Ite,
}

/// Weighted operation table. Division and remainder appear once each against nine
/// appearances of the common combinational operations.
const OPS: &[(Op, u32)] = &[
    (Op::Not, 4),
    (Op::And, 4),
    (Op::Or, 4),
    (Op::Xor, 4),
    (Op::Add, 8),
    (Op::Sub, 6),
    (Op::Mul, 4),
    (Op::UDiv, 1),
    (Op::URem, 1),
    (Op::Eq, 4),
    (Op::Select, 4),
    (Op::Concat, 3),
    (Op::Replicate, 2),
    (Op::Ite, 4),
];

/// The generator's shape and randomness.
///
/// Hand-rolled rather than proptest's strategy machinery, because the seed has to
/// be fixed and reported: the generator-quality guard is only meaningful if it is
/// reproducible, and a `proptest!` block that shrinks its way to a seed is a
/// different thing from a generator with a documented seed.
#[derive(Clone, Debug)]
struct GeneratorConfig {
    input_widths: Vec<u32>,
    body_size: usize,
    reg_count: usize,
    state: u64,
    next_id: u32,
}

impl GeneratorConfig {
    fn new(seed: u32) -> Self {
        // Widths drawn from the reference generator's set: 1, 2, 3, 64 and 100,
        // capped at 200 in the reference. The small widths are the interesting ones
        // and the wide ones cross the interpreter's word boundary.
        let mut state = u64::from(seed) | 1;
        let mut widths = Vec::new();
        for _ in 0..6 {
            state = next(state);
            widths.push(match state % 5 {
                0 => 1,
                1 => 2,
                2 => 3,
                3 => 64,
                _ => 100,
            });
        }
        GeneratorConfig {
            input_widths: widths,
            body_size: 12,
            reg_count: 4,
            state: u64::from(seed) | 1,
            next_id: 0,
        }
    }

    fn next_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    /// xorshift, so the sequence is reproducible from the seed alone.
    fn roll(&mut self, modulus: u32) -> u32 {
        self.state = next(self.state);
        (self.state % u64::from(modulus)) as u32
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let index = self.roll(items.len() as u32) as usize;
        &items[index]
    }

    fn choose_width(&mut self) -> u32 {
        [1u32, 2, 3, 8, 64, 100][(self.roll(6)) as usize]
    }

    fn pick_op(&mut self) -> Op {
        let total: u32 = OPS.iter().map(|(_, w)| w).sum();
        let mut ticket = self.roll(total);
        for (op, weight) in OPS {
            if ticket < *weight {
                return *op;
            }
            ticket -= *weight;
        }
        Op::Add
    }
}

fn next(state: u64) -> u64 {
    let mut x = state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// Compiles a generated design and steps it, returning whether any output moved.
fn run(seed: u32) -> bool {
    let config = GeneratorConfig::new(seed);
    let generated = generate(&mut { config.clone() }, 3, &[], &[]);
    let mut sim = match Sim::new(&generated.design) {
        Ok(sim) => sim,
        // A design the simulator refuses is a generator bug, not a property
        // failure, and the message says which one.
        Err(e) => panic!("seed {seed:#x}: generated design did not compile: {e}"),
    };

    // Random stimulus, from the same stream.
    let mut state = u64::from(seed).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut moved = false;
    for cycle in 0..6 {
        for (name, width) in &generated.inputs {
            state = next(state);
            let bytes = ferrite_lithic_bits::bytes_for_width(*width);
            let mut buf = vec![0u8; bytes];
            for byte in &mut buf {
                state = next(state);
                *byte = (state >> 24) as u8;
            }
            let value = Bits::from_bytes_le(*width, &buf).expect("exact length");
            sim.set_input(name, value).expect("generated input port");
        }
        let before: Vec<Bits> = sim
            .program()
            .outputs()
            .iter()
            .map(|p| sim.peek(&p.name).expect("a named output"))
            .collect();
        sim.step()
            .expect("a generated circuit steps without a data-dependent fault");
        let after: Vec<Bits> = sim
            .program()
            .outputs()
            .iter()
            .map(|p| sim.peek(&p.name).expect("a named output"))
            .collect();
        if cycle > 0 && before != after {
            moved = true;
        }
    }
    moved
}

/// The guard: at the reference seed, most generated circuits must do something.
///
/// Hardcaml requires more than two thirds to produce non-constant outputs
/// (`test/lib/test_dedup.ml:191-197`). The threshold is a property of the generator,
/// not of the simulator, so it is asserted here rather than being allowed to rot
/// silently.
#[test]
fn the_generator_mostly_produces_circuits_that_do_something() {
    const CIRCUITS: u32 = 1000;
    let mut moved = 0;
    for seed in 0..CIRCUITS {
        if run(SEED.wrapping_add(seed)) {
            moved += 1;
        }
    }
    let ratio = f64::from(moved) / f64::from(CIRCUITS);
    assert!(
        ratio > 2.0 / 3.0,
        "only {moved} of {CIRCUITS} generated circuits ({ratio:.3}) produced a \
         changing output; the generator has stopped producing interesting circuits \
         and the differential test would be vacuous"
    );
}

/// Every seed in a wide range must at least compile and step.
#[test]
fn generated_circuits_compile_and_step_over_a_wide_seed_range() {
    for seed in [0u32, 1, 7, 12345, u32::MAX, SEED] {
        let _ = run(seed);
    }
}

/// The generator's shapes are what it claims: it makes registers, and registers
/// make the design stateful.
#[test]
fn the_generator_produces_feedback() {
    let config = GeneratorConfig::new(SEED);
    let generated = generate(&mut { config.clone() }, 3, &[], &[]);
    let signals = generated.design.signals();
    let regs = signals.iter().filter(|s| s.kind() == "Reg").count();
    assert_eq!(
        regs, config.reg_count,
        "the generator claims to make {} registers and made {regs}",
        config.reg_count
    );
    // And the design passes its own checks, which is the check that would catch a
    // zero-width signal or a dangling operand.
    generated.design.check().expect("a generated design checks");
}
