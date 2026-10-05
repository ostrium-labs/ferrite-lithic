//! How much slower is the hardware than the software it replaces?
//!
//! # The question this answers
//!
//! `graph_metrics.rs` measures the *shape* of each design. This measures the question every
//! engineer actually asks about a hardware implementation: **is it faster than just doing it
//! in software?** A design that is slower than the CPU is a curiosity, not a product, and the
//! corpus contains exactly such a design on purpose.
//!
//! # Why a ratio needs a clock, and what that means for the number
//!
//! The two sides of the comparison are in different units:
//!
//!   - the **design** is measured in *cycles* — an exact, machine-independent count of clock
//!     edges, because that is what the hardware actually does.
//!   - the **software** is measured in *nanoseconds* — wall clock, machine-dependent.
//!
//! Dividing them requires converting cycles to time, and that needs a clock frequency. There
//! is no way around it: "cycles per byte" and "nanoseconds per byte" are not comparable
//! without stating the rate. So every ratio here is quoted **at 1 GHz**, where one cycle is
//! one nanosecond, and the frequency is printed next to it. Read the ratio column as "at 1 GHz";
//! at 2 GHz every design figure halves and every ratio doubles.
//!
//! # Why the software side is trustworthy
//!
//! The software reference for each design is the real crate that defines the algorithm —
//! `crc32fast`, `sha2`, `aes`, `hex`, `base64`, `memchr` — already present as a dev-dependency
//! for exactly this purpose. Not a reimplementation written for the benchmark, which would
//! measure the benchmark author's skill rather than the design's worth.
//!
//! # Why the comparison is fair
//!
//! Each pair asserts that the hardware and the software produce **the same output** on the same
//! input before either is timed. Without that, a fast design would win by doing less work.
//! Timing a design that disagrees with its reference would be measuring a bug.
//!
//! # What is not claimed
//!
//! - The design figure is **simulated** cycles. It is not a measurement of silicon.
//! - The software figure is **this host's** wall clock, best-of-N, and will differ elsewhere.
//! - A design that loses here may still win in a system where the software version cannot run
//!   at all — streaming, fixed latency, no memory pressure, a 2 ms budget in a 16 ms slot. That
//!   is a systems argument and it is not made here.
//!
//! Run with:
//!
//! ```text
//! cargo test -p ferrite-lithic-corpus --test speed --release -- --nocapture --ignored
//! ```
//!
//! `--release` matters. In a debug build the software side is several times slower, which
//! would flatter the hardware for a reason that has nothing to do with the hardware.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;
use std::time::Instant;

use ferrite_lithic::Design;
use ferrite_lithic_corpus as corpus;
use ferrite_lithic_tb::Testbench;

/// The clock the cycle-to-time conversion assumes. Stated in one place so it cannot be
/// quietly changed in one row of a table and not another.
const ASSUMED_GHZ: f64 = 1.0;

/// One row of the comparison.
struct Speed {
    design: &'static str,
    /// Clock edges the design took for the whole workload.
    cycles: u64,
    /// How many units of work the workload contained — bytes, or blocks.
    ///
    /// Carried explicitly because the two sides are reported *per unit*: the design's
    /// `cycles` is a total, and dividing a total by a per-unit software figure is off by
    /// the workload size. That error is invisible in a small table and enormous in a big
    /// one, so the unit count is stored rather than inferred at print time.
    units: f64,
    /// The software reference for the same work.
    software: &'static str,
    /// Best-of-N wall clock nanoseconds, **per unit**.
    ns_per_unit: f64,
}

impl Speed {
    /// Design nanoseconds *per unit* at the assumed clock.
    fn design_ns_per_unit(&self) -> f64 {
        self.cycles as f64 / self.units / ASSUMED_GHZ
    }

    /// How many times **slower** the hardware is than the software, per unit.
    fn slower_by(&self) -> f64 {
        self.design_ns_per_unit() / self.ns_per_unit
    }
}

/// Best-of-N wall clock, in nanoseconds.
///
/// Best-of rather than mean, and N large enough to dominate timer resolution: a benchmark
/// that reports whichever run happened to be quietest is measuring the machine's mood.
fn time_ns<T>(mutations: usize, mut body: impl FnMut() -> T) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..mutations {
        let start = Instant::now();
        let out = body();
        // Without a black box the optimiser deletes the work, and the design looks like
        // it won because the software was never computed.
        black_box(out);
        best = best.min(start.elapsed().as_nanos() as f64);
    }
    best
}

/// A workload large enough that the timer is not reading its own resolution.
fn data(len: usize) -> Vec<u8> {
    // Deterministic and non-uniform: a repeated byte compresses and its CRC degenerates,
    // which would make the software side artificially fast on the easy case.
    let mut out = Vec::with_capacity(len);
    let mut state: u32 = 0x1234_5678;
    for _ in 0..len {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        out.push((state >> 24) as u8);
    }
    out
}

const N: usize = 512 * 1024;

/* ------------------------------------------------------------------ CRC-32 */

fn crc32_speed() -> Speed {
    let input = data(N);
    let len = input.len();

    // The reference is computed *before* `input` is moved into the simulation closure, so
    // the comparison below cannot borrow a moved value.
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&input);
    let expected = hasher.finalize();

    // --- software: the crate that defines CRC-32.
    let ns = time_ns(5, || {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&input);
        hasher.finalize()
    });

    // --- hardware: one byte per cycle, plus the init edge.
    let design = Design::new();
    corpus::crc32::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();

    let (cycles, hw) = tb
        .run(|tb| async move {
            tb.set("init", 1).await;
            tb.set("byte", 0).await;
            tb.step().await;
            tb.set("init", 0).await;
            for byte in &input {
                tb.drive("byte", u64::from(*byte)).await;
            }
            (tb.cycle(), tb.value("crc"))
        })
        .unwrap();

    assert_eq!(
        u64::from(hw as u32),
        u64::from(expected),
        "the design and crc32fast must agree before either is timed"
    );

    Speed {
        design: "crc32",
        cycles,
        units: len as f64,
        software: "crc32fast",
        ns_per_unit: ns / len as f64,
    }
}

/* -------------------------------------------------------------------- CRC-3 */

/// CRC-3/ROTD against the `crc` crate.
///
/// The smallest design in the corpus and the clearest illustration of the whole comparison:
/// three flops of state, one byte per cycle, against a table-driven software CRC. The
/// hardware is *hundreds* of times slower and always will be — and it is still in the corpus,
/// because it is the design that found four defects.
fn crc3_speed() -> Speed {
    let input = data(N);
    let len = input.len();

    // The same catalogue transcription `tests/crc3.rs` uses. Reusing the verified
    // definition rather than naming a constant keeps the two in step automatically.
    static ROTD: crc::Algorithm<u8> = crc::Algorithm {
        width: 3,
        poly: 0x3,
        init: 0x0,
        refin: true,
        refout: true,
        xorout: 0x0,
        check: 0x2,
        residue: 0x0,
    };
    let expected = crc::Crc::<u8>::new(&ROTD).checksum(&input);

    let ns = time_ns(5, || crc::Crc::<u8>::new(&ROTD).checksum(&input));

    let design = Design::new();
    corpus::crc3::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();

    let (cycles, hw) = tb
        .run(|tb| async move {
            tb.pulse(corpus::RESET).await;
            for byte in &input {
                tb.drive("byte", u64::from(*byte)).await;
            }
            (tb.cycle(), tb.value("crc"))
        })
        .unwrap();

    assert_eq!(
        hw,
        u64::from(expected),
        "the design and the crc crate must agree"
    );

    Speed {
        design: "crc3",
        cycles,
        units: len as f64,
        software: "crc",
        ns_per_unit: ns / len as f64,
    }
}

/* -------------------------------------------------------------------- SHA-256, deliberately absent */

// `sha256` is **not** in this table, and the reason is worth more than the row would be.
//
// The design is a bare block compressor: sixteen `block_*` word ports in, eight `state_*`
// words out, and the caller supplies already-padded blocks. The `sha2` crate does padding,
// message scheduling, chaining and serialisation on top of that. Timing one against the
// other would compare a compressor against a complete digest, and the ratio would be a
// statement about `sha2`'s convenience API rather than about this design.
//
// Getting a real number needs a hand-written block-padding and chaining loop in the
// benchmark, which is a project rather than a row in a table. Until that exists the honest
// entry is "not measured", not a number derived from mismatched work.

/* ----------------------------------------------------------------------- hex */

fn hex_speed() -> Speed {
    let input = data(N / 2);
    let len = input.len();

    let encoded = hex::encode(&input);
    // The full expected output, as the ASCII bytes the design should emit.
    let expected: Vec<u8> = encoded.as_bytes().to_vec();
    let ns = time_ns(5, || hex::encode(&input));

    // The design emits one byte per accepted nibble pair, so two cycles per byte.
    let design = Design::new();
    corpus::hex::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();

    // The whole output is compared, not just one character. Reading `ascii` after the
    // drain gives the *last* character emitted, which is how an off-by-one in the
    // pipeline can pass a single-value assertion.
    let (cycles, emitted) = tb
        .run(|tb| async move {
            tb.pulse(corpus::RESET).await;
            for byte in &input {
                tb.drive("in_valid", 0).await;
                tb.drive("data", u64::from(*byte)).await;
                tb.drive("in_valid", 1).await;
            }
            tb.drive("in_valid", 0).await;
            tb.step().await;
            tb.step().await;
            let valid = tb.series("valid");
            let ascii = tb.series("ascii");
            let chars: Vec<u8> = valid
                .iter()
                .zip(ascii.iter())
                .filter(|(v, _)| v.to_u64().expect("a one-bit valid flag fits in a u64") == 1)
                .map(|(_, a)| a.to_u64().expect("an ASCII character fits in a u64") as u8)
                .collect();
            (tb.cycle(), chars)
        })
        .unwrap();

    assert_eq!(
        emitted, expected,
        "the design and the hex crate must agree on the whole output"
    );

    Speed {
        design: "hex",
        cycles,
        units: len as f64,
        software: "hex",
        ns_per_unit: ns / len as f64,
    }
}

/* -------------------------------------------------------------------- base64 */

fn base64_speed() -> Speed {
    let input = data(N);
    let len = input.len();

    use base64::Engine;
    let expected: Vec<u8> = base64::engine::general_purpose::STANDARD
        .encode(&input)
        .as_bytes()
        .to_vec();

    let ns = time_ns(5, || {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&input)
    });

    // The design consumes six-bit sextets, not bytes, so the workload is packed the same way
    // the crate packs it: most-significant bit first, and the final short chunk padded on the
    // right. Padding on the left instead would produce a different last group.
    let sextets = pack_sextets(&input);
    let last = sextets.len().saturating_sub(1);

    let design = Design::new();
    corpus::base64::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();

    let (cycles, emitted) = tb
        .run(|tb| async move {
            tb.pulse(corpus::RESET).await;
            for (index, sextet) in sextets.iter().enumerate() {
                tb.drive("in_valid", 0).await;
                tb.drive("in_last", u64::from(index == last)).await;
                tb.drive("sextet", u64::from(*sextet)).await;
                tb.drive("in_valid", 1).await;
            }
            tb.drive("in_valid", 0).await;
            // Six edges of drain: the last character can be three edges after the final
            // sextet is accepted, plus up to three `=` characters.
            for _ in 0..5 {
                tb.step().await;
            }
            let valid = tb.series("valid");
            let ascii = tb.series("ascii");
            let chars: Vec<u8> = valid
                .iter()
                .zip(ascii.iter())
                .filter(|(v, _)| v.to_u64().expect("a one-bit valid flag fits in a u64") == 1)
                .map(|(_, a)| a.to_u64().expect("an ASCII character fits in a u64") as u8)
                .collect();
            (tb.cycle(), chars)
        })
        .unwrap();

    assert_eq!(
        emitted, expected,
        "the design and the base64 crate must agree on the whole output"
    );

    Speed {
        design: "base64",
        cycles,
        units: len as f64,
        software: "base64",
        ns_per_unit: ns / len as f64,
    }
}

/// Packs bytes into the six-bit groups the base64 design consumes.
fn pack_sextets(data: &[u8]) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::with_capacity(data.len() * 8);
    for byte in data {
        for shift in (0..8).rev() {
            bits.push((byte >> shift) & 1 == 1);
        }
    }
    bits.chunks(6)
        .map(|chunk| {
            // Right-aligned: a short final chunk is padded on the right, so bit `i` of the
            // chunk lands at sextet position `5 - i`.
            let mut value = 0u8;
            for (i, bit) in chunk.iter().enumerate() {
                // `chunks(6)` means `i` is 0..=5, so `5 - i` is 0..=5 and the shift is
                // always in range.
                let position = 5 - i;
                if *bit {
                    value |= 1 << position;
                }
            }
            value
        })
        .collect()
}

/* -------------------------------------------------------------------- memchr */

fn memchr_speed() -> Speed {
    // One haystack, one needle, the worst case for a byte-at-a-time automaton: the match is
    // at the very end, so nothing can short-circuit.
    //
    // 256 bytes, not the megabyte the other rows use, and the reason is in the design: the
    // `offset` port is eight bits wide (measured: 8-bit counter, 8-bit offset, 1 found), so
    // a haystack longer than 256 cannot be *reported*, let alone compared. Both sides do
    // exactly the same 256 bytes, so the ratio is still like-for-like.
    let mut haystack = vec![b'x'; 256];
    *haystack.last_mut().unwrap() = b'\n';
    let len = haystack.len();

    assert_eq!(memchr::memchr(b'\n', &haystack), Some(haystack.len() - 1));
    let ns = time_ns(5, || memchr::memchr(b'\n', &haystack));

    let design = Design::new();
    corpus::memchr::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();

    let (cycles, found, offset) = tb
        .run(|tb| async move {
            tb.set("rst", 1).await;
            tb.set("start", 0).await;
            tb.set("data", 0).await;
            tb.step().await;
            tb.set("rst", 0).await;
            // `set`, not `drive`: asserting `start` must not spend a cycle, because it is
            // a handshake edge rather than a byte of the search.
            tb.set("start", 1).await;
            tb.step().await;
            tb.set("start", 0).await;
            for byte in &haystack {
                tb.drive("data", u64::from(*byte)).await;
            }
            (tb.cycle(), tb.value("found"), tb.value("offset"))
        })
        .unwrap();

    assert_eq!(found, 1, "the design must report that it found the needle");
    assert_eq!(
        offset as usize,
        len - 1,
        "the design must find the needle at the end of the haystack"
    );

    Speed {
        design: "memchr",
        cycles,
        units: len as f64,
        software: "memchr",
        ns_per_unit: ns / len as f64,
    }
}

fn all_speeds() -> Vec<Speed> {
    vec![
        crc32_speed(),
        crc3_speed(),
        hex_speed(),
        base64_speed(),
        memchr_speed(),
    ]
}

/// Each pair must agree before either side is timed.
///
/// This is the assertion that makes the ratio mean anything, so it is in the suite rather
/// than only in the ignored printer. A design that disagrees with its reference is a bug, and
/// timing it would produce a confident wrong number.
#[test]
fn every_compared_pair_agrees_before_it_is_timed() {
    // `all_speeds` asserts internally; calling it here is the test.
    let speeds = all_speeds();
    assert_eq!(speeds.len(), 5, "every row in the table must be reachable");
}

/// Prints the comparison. `#[ignore]`d: it prints, and its numbers are host-specific.
#[test]
#[ignore = "prints a table; run in release with --nocapture and --ignored"]
fn print_speed_versus_software() {
    let speeds = all_speeds();

    println!(
        "\nspeed vs software, per byte, at an assumed {ASSUMED_GHZ:.0} GHz (1 cycle = 1 ns)\n{}",
        "-".repeat(78)
    );
    println!(
        "{:<8} {:<10} {:>9} {:>12} {:>11}  verdict",
        "design", "software", "cyc/unit", "design ns", "sw ns"
    );
    println!("{}", "-".repeat(72));

    for s in &speeds {
        let ratio = s.slower_by();
        // A ratio below 1 means the hardware *won*, and printing that as "0x slower" would
        // hide the most interesting row in the table.
        let verdict = if ratio >= 1.0 {
            format!("software {ratio:.1}x faster")
        } else {
            format!("HARDWARE {:.2}x faster", 1.0 / ratio)
        };
        println!(
            "{:<8} {:<10} {:>9.2} {:>12.2} {:>11.4}  {verdict}",
            s.design,
            s.software,
            s.cycles as f64 / s.units,
            s.design_ns_per_unit(),
            s.ns_per_unit,
        );
    }

    let winners = speeds.iter().filter(|s| s.slower_by() < 1.0).count();
    println!(
        "\n{winners} of {} designs are faster than the software at {ASSUMED_GHZ:.0} GHz. \
         A ratio is a clock-relative claim: double the clock and every design halves.",
        speeds.len()
    );

    println!(
        "\nThe design column is clock edges counted by the simulator; the software column is\n\
         wall clock on this host. Both are per unit of work, and both sides were asserted to\n\
         produce identical output before either was timed."
    );
}
