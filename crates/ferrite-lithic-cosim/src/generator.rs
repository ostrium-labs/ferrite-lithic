//! A random circuit generator, and the stimulus that exercises it.
//!
//! # Ported, not copied
//!
//! Hardcaml's [`test/lib/generator.ml`](https://github.com/jane-street/hardcaml)
//! is a recursive random-design generator: weighted op choices, widths drawn from
//! `[1;2;3;64;100]`, feedback registers, multiport memories, and a matching random
//! input vector per design. It depends only on `Signal` and `Circuit`, so it
//! lifts. Two uses are carried over verbatim because both are checks on the
//! *generator*, not on a design:
//!
//! - a **generator-quality guard**: over many circuits with a fixed seed, more
//!   than two thirds must produce a non-constant output. A generator that keeps
//!   producing constants would make a differential test pass by never disagreeing.
//! - **feedback**, via registers whose input is a function of their own output,
//!   because a design with no cycles cannot find a cycle-related emitter bug.
//!
//! # The seed is part of the test
//!
//! [`Generator::new`] takes a seed and nothing reads the clock, so a design is a
//! pure function of its seed. That is what lets a failing cosimulation be
//! reported as a seed rather than as "it failed once".

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;

use crate::error::Error;
use crate::stimulus::Stimulus;

/// The widths Hardcaml's generator draws from.
///
/// The odd-looking list is the point: `1`, `2` and `3` are the widths that break
/// implementations which assume whole words, and `100` is wider than the 64-bit
/// fast path. Capped at 200 because a design wider than that stops finding bugs
/// and starts taking minutes.
pub const WIDTHS: [u32; 5] = [1, 2, 3, 64, 100];

/// The largest width the generator will draw.
pub const MAX_WIDTH: u32 = 200;

/// How much of a design to generate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Options {
    /// A width from [`WIDTHS`], or anything up to [`MAX_WIDTH`].
    pub width: u32,
    /// How many input ports.
    pub inputs: usize,
    /// How many operations to chain.
    pub depth: usize,
    /// Whether to close feedback loops through registers.
    pub registers: bool,
    /// How many memories to instantiate. Each gets a write and a read port.
    pub memories: usize,
}

impl Default for Options {
    /// A design with state, a memory, and enough of each to be worth cosimulating.
    fn default() -> Self {
        Self {
            width: 8,
            inputs: 4,
            depth: 12,
            registers: true,
            memories: 1,
        }
    }
}

/// A generated design and the stimulus that drives it.
#[derive(Clone, Debug)]
pub struct Generated {
    /// The design itself.
    pub design: Design,
    /// Random input values, one cycle per entry.
    pub stimulus: Stimulus,
    /// The width the data path is.
    pub width: u32,
}

/// A deterministic random circuit generator.
///
/// SplitMix64 rather than anything from `rand`: no dependency, and a fixed
/// algorithm so a seed printed in a failure message means the same thing in a
/// year. (Hardcaml's generator is built on QCheck, which is not reproducible
/// across versions for the same reason.)
#[derive(Clone, Debug)]
pub struct Generator {
    state: u64,
}

impl Generator {
    /// A generator for a seed.
    ///
    /// The seed `0x3ead_beef` is Hardcaml's, kept so the two generators'
    /// corpus means roughly the same thing.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9e37_79b9_7f4a_7c15),
        }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `0..bound`, or zero when `bound` is zero.
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }

    /// True with probability `numerator / denominator`.
    pub fn chance(&mut self, numerator: u32, denominator: u32) -> bool {
        self.next_u64() % u64::from(denominator) < u64::from(numerator)
    }

    /// A random `width`-bit value.
    pub fn value(&mut self, width: u32) -> Bits {
        // Built from words rather than a single `u64` so a width above 64 gets
        // random *high* bits instead of a zero-extended random low word.
        let words = width.div_ceil(64) as usize;
        let mut text = Vec::with_capacity(words);
        for _ in 0..words {
            text.push(self.next_u64().to_be_bytes().to_vec());
        }
        let bytes: Vec<u8> = text.concat();
        Bits::from_bytes_le(width, &bytes[..(width as usize).div_ceil(8)])
            .expect("the byte length is exactly what the width needs")
    }
}

/// Builds one random design.
///
/// # Errors
///
/// [`Error::Emit`] or [`Error::Sim`] if the front end refuses the graph, which
/// would mean the generator built something illegal — a bug worth failing on
/// rather than skipping.
pub fn build(seed: u64, options: &Options) -> Result<Generated, Error> {
    if options.width == 0 || options.width > MAX_WIDTH {
        return Err(Error::Arity {
            cycle: 0,
            expected: MAX_WIDTH as usize,
            got: options.width as usize,
        });
    }
    let mut rng = Generator::new(seed);
    let design = Design::new();
    let width = options.width;

    let clock = design.input("clk", 1)?;
    // `reset` and `clear` are inputs rather than constants so the stimulus
    // exercises the gating paths, not just the update path.
    let reset = design.input("rst", 1)?;
    let clear = design.input("clr", 1)?;

    let mut narrow: Vec<Signal> = vec![clock.clone(), reset.clone(), clear.clone()];
    // Width-tagged, because a generator whose operands disagreed on width would
    // only ever test the front end's width check. `mul` widens to twice the
    // operand width and a truncation narrows again, so the pool holds several
    // widths at once and every operation picks two of the *same* one.
    let mut pool: Vec<(Signal, u32)> = Vec::new();
    for index in 0..options.inputs {
        let port = design.input(&format!("in{index}"), width)?;
        pool.push((port, width));
    }

    for step in 0..options.depth {
        let (left, left_width) = pick(&mut rng, &pool);
        let (right, _) = pick_same_width(&mut rng, &pool, left_width);
        let made = match step % 7 {
            // A divisor that is never zero, because a random one would make the
            // *simulator* report a division by zero and the test would be about
            // the harness rather than the design.
            3 => design.udiv(
                &left,
                &design.lit(nonzero(&mut rng, left_width), left_width)?,
            )?,
            5 => design.srem(
                &left,
                &design.lit(nonzero(&mut rng, left_width), left_width)?,
            )?,
            _ => match rng.below(8) {
                0 => design.add(&left, &right)?,
                1 => design.sub(&left, &right)?,
                2 => design.mul(&left, &right)?,
                3 => design.xor(&left, &right)?,
                4 => design.and(&left, &right)?,
                5 => design.or(&left, &right)?,
                _ => design.not(&left),
            },
        };
        let made_width = made.width();
        pool.push((made, made_width));
        narrow.push(design.ult(&left, &right)?);
        narrow.push(design.eq(&left, &right)?);
    }

    for _ in 0..options.memories {
        let address_bits = 3;
        let memory = design.mem(width, 8)?;
        let (data, data_width) = pick(&mut rng, &pool);
        // An address is not arithmetic, so truncation is the right reading rather
        // than a loss.
        let source = pick(&mut rng, &pool);
        let address = design
            .truncate(&source.0, address_bits)
            .unwrap_or_else(|_| design.lit(0, address_bits).expect("3 bits is valid"));
        // A memory's data width has to match its write port's, so the data is
        // narrowed or widened to it explicitly rather than hoped for.
        let data = match data_width.cmp(&width) {
            core::cmp::Ordering::Equal => data,
            core::cmp::Ordering::Less => design.zero_extend(&data, width)?,
            core::cmp::Ordering::Greater => design.truncate(&data, width)?,
        };
        let enable = narrow[rng.below(narrow.len())].clone();
        design.write_port(&memory, &data, &address, &enable)?;
        let read = design.read_port(&memory, &address, &enable)?;
        let read_width = read.width();
        pool.push((read, read_width));
    }

    if options.registers {
        // Feedback: the register's own output is in the pool it is fed from, so
        // the graph closes a loop through state. A design with no cycle cannot
        // find a cycle-related bug.
        let (source, _) = pick(&mut rng, &pool);
        let feedback = design.reg(&source, &clock, &reset, &clear)?;
        let feedback_width = feedback.width();
        pool.push((feedback.clone(), feedback_width));
        let mixed = design.xor(&source, &feedback)?;
        let mixed_width = mixed.width();
        pool.push((mixed, mixed_width));
    }

    for index in 0..options.inputs {
        // Declared at the design's width, so the driver has to be exactly that
        // width. A pool signal of another width is extended or truncated rather
        // than skipped, because skipping would quietly shrink the output set and
        // a cosimulation over fewer ports proves less.
        let (driver, driver_width) = pick(&mut rng, &pool);
        let driver = match driver_width.cmp(&width) {
            core::cmp::Ordering::Equal => driver,
            core::cmp::Ordering::Less => design.zero_extend(&driver, width)?,
            core::cmp::Ordering::Greater => design.truncate(&driver, width)?,
        };
        design.output(&format!("out{index}"), width, &driver)?;
    }

    let mut stimulus = Stimulus::new();
    for _ in 0..8 {
        // `rst`, `clr`, then one value per data input, which is the generator's
        // own port order: the controls are declared first and the data inputs
        // after them. `clk` is deliberately *not* here — `Plan::of` excludes the
        // clock from the stimulus columns, because both backends drive it
        // themselves and a column for it is a word the generated driver applies
        // and then overwrites the clock with.
        let mut row = vec![rng.value(1), rng.value(1)];
        row.extend((0..options.inputs).map(|_| rng.value(width)));
        stimulus.push(row)?;
    }

    Ok(Generated {
        design,
        stimulus,
        width,
    })
}

/// A random non-zero value, for a divisor.
///
/// Zero is excluded because `udiv` and `srem` by zero are defined errors in this
/// DSL, and a generator that produces them is testing the error path over and
/// over instead of the datapath.
fn nonzero(rng: &mut Generator, width: u32) -> u64 {
    loop {
        let candidate = rng.next_u64();
        if candidate != 0 {
            // Kept inside the width so the literal does not need truncation, which
            // would hide the value the generator actually chose.
            let mask = if width >= 64 {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            let masked = candidate & mask;
            if masked != 0 {
                return masked;
            }
        }
    }
}

/// One signal out of a width-tagged pool.
fn pick(rng: &mut Generator, pool: &[(Signal, u32)]) -> (Signal, u32) {
    pool[rng.below(pool.len())].clone()
}

/// One signal of a given width, or the one handed in if the pool has no other.
///
/// Falling back rather than failing: a pool of one signal has nothing else to
/// offer, and an operation whose two operands are the same signal is a perfectly
/// good circuit.
fn pick_same_width(rng: &mut Generator, pool: &[(Signal, u32)], width: u32) -> (Signal, u32) {
    let matching: Vec<usize> = pool
        .iter()
        .enumerate()
        .filter(|(_, (_, candidate))| *candidate == width)
        .map(|(index, _)| index)
        .collect();
    if matching.is_empty() {
        return pool[rng.below(pool.len())].clone();
    }
    pool[matching[rng.below(matching.len())]].clone()
}
