//! Bit-unpacking and run-length/delta decode, checked against the crates that define
//! them.
//!
//! Two different golden models in one binary, because the two modules have different
//! ones and pretending otherwise would be the interesting kind of dishonest:
//!
//! - [`bitpack`] is checked against **`bitpacking`**, the crate Quickwit uses inside
//!   Tantivy and therefore the crate a real columnar reader would decode with. The
//!   comparison is not "our values equal theirs": it is *this design, driven at every
//!   bit offset of a block the crate actually compressed*, extracting one value per
//!   cycle, against the crate's own `decompress` of those bytes. That is the whole
//!   decoder, one value at a time.
//! - [`rle`] has **no golden crate**, for the reason its module docs give, so the
//!   reference is `rle::decode` and `rle::delta_decode` -- hand-written, and pinned
//!   separately by hand-computed corner cases rather than only by the differential.
//!
//! Both are driven through the step testbench, where every `.await` is one clock edge
//! (or, for the clockless standalone extractor, one settle), and [`bitpack`] is also
//! cosimulated against Verilator.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bitpacking::BitPacker;
use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::{bitpack, rle};
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::{Handle, Testbench};
use proptest::prelude::*;

/// How many integers one `bitpacking` block holds.
///
/// `BitPacker1x::BLOCK_LEN`, written out rather than referred to so that a change in the
/// crate's block size shows up as a test failure with a number in it rather than as a
/// panic inside the crate.
const BLOCK: usize = 32;

/// Pads a `bitpacking` compression buffer.
///
/// The crate writes a block in whole 32-bit words, so a `num_bits` that does not divide
/// the block length evenly can touch a few bytes past the block's own size. Padding
/// rather than allocating exactly is what keeps widths 3, 5, 6 and 7 -- the ones that
/// are *not* multiples of four and so the interesting ones -- testable at all.
fn padded(len: usize) -> Vec<u8> {
    vec![0u8; len + 16]
}

/// Compresses a block with `bitpacking`'s own scalar codec.
fn golden_compress(values: &[u32], num_bits: u8) -> Vec<u8> {
    assert_eq!(values.len(), BLOCK, "a block is BLOCK integers");
    let packer = bitpacking::BitPacker1x::new();
    let mut compressed = padded(BLOCK * num_bits as usize / 8);
    let written = packer.compress(values, &mut compressed, num_bits);
    compressed.truncate(written);
    compressed
}

/// Decompresses a block with `bitpacking`'s own scalar codec.
fn golden_decompress(compressed: &[u8], num_bits: u8) -> Vec<u32> {
    let packer = bitpacking::BitPacker1x::new();
    let mut out = vec![0u32; BLOCK];
    packer.decompress(compressed, &mut out, num_bits);
    out
}

/// The 64-bit window `index` of `compressed`.
///
/// Little-endian, because that is how `bitpacking` writes a block: byte zero carries the
/// first bit, so the window holding bit `bit` is `bit / 64` and the offset inside it is
/// `bit % 64`. This is the convention the whole design rests on, and getting it wrong
/// would still produce plausible-looking values for the first element or two -- eight of
/// them, in fact, because one window is exactly eight eight-bit values.
fn window(compressed: &[u8], index: usize) -> u64 {
    let start = index * 8;
    let mut bytes = [0u8; 8];
    let available = compressed.len().saturating_sub(start).min(8);
    bytes[..available].copy_from_slice(&compressed[start..start + available]);
    u64::from_le_bytes(bytes)
}

/// The `width` bits of `compressed` that start at bit `bit`, as a 64-bit word.
///
/// This is the **streaming** form: the value is placed at bit zero of the word rather
/// than at its own offset inside a word. It is what a real decoder does, because a value
/// whose bits straddle a 64-bit window boundary has to come out of a word the hardware
/// has already shifted -- and it is why the width sweep below can compare *every* value
/// at widths that do not divide 64.
fn aligned(compressed: &[u8], bit: usize, width: usize) -> u64 {
    let mut out = 0u64;
    for step in 0..width {
        let target = bit + step;
        let bit_in_word = target % 64;
        let source = window(compressed, target / 64);
        out |= ((source >> bit_in_word) & 1) << step;
    }
    out
}

/// Runs the standalone clockless extractor over a compressed block, one value per cycle.
///
/// A clockless design has no rising edge to take, so the testbench's rule applies and
/// every `.await` is a settle: set both inputs, settle, read. That is exactly the shape
/// of the real decoder -- a value per cycle with no state -- which is why the standalone
/// form is the one used for the width sweep.
///
/// Each value is presented at bit zero of `word` (see [`aligned`]), so every value of the
/// block is comparable whatever its width.
fn extracted(compressed: &[u8], num_bits: u32, count: usize) -> Vec<u64> {
    let design = Design::new();
    bitpack::build_extractor(&design, num_bits).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            let bit = index * num_bits as usize;
            tb.set("word", aligned(compressed, bit, num_bits as usize))
                .await;
            tb.set("off", 0).await;
            tb.step().await;
            out.push(tb.value("value"));
        }
        out
    })
    .expect("the extractor drives only ports it declares")
}

/// Runs the extractor with each value at its own offset in its own 64-bit window.
///
/// Returns `(value, fit)` per value, because the two cases are different and the whole
/// point of this form is that they are: at a width that does not divide 64, the values
/// near the end of a window **straddle** it, and this design cannot produce those from a
/// single window. `fit` is the signal that says so, and the test below pins both halves
/// of the contract rather than quietly dropping the awkward values.
fn extracted_in_place(compressed: &[u8], num_bits: u32, count: usize) -> Vec<(u64, u64)> {
    let design = Design::new();
    bitpack::build_extractor(&design, num_bits).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            let bit = index * num_bits as usize;
            tb.set("word", window(compressed, bit / 64)).await;
            tb.set("off", (bit % 64) as u64).await;
            tb.step().await;
            out.push((tb.value("value"), tb.value("fit")));
        }
        out
    })
    .expect("the extractor drives only ports it declares")
}

/// Applies one set of input values and takes exactly one clock edge.
///
/// The testbench applies **one input port per rising edge**, so a cycle in which several
/// inputs change is `set` for all of them and `drive` for the last. The last one is
/// arbitrary; making it the last of the slice keeps the call sites readable.
async fn clock_with(tb: &Handle, values: &[(&str, u64)]) {
    let (last, rest) = values.split_last().expect("a cycle sets at least one port");
    for (name, value) in rest {
        tb.set(name, *value).await;
    }
    tb.drive(last.0, last.1).await;
}

/// The mask for a `num_bits`-wide value.
fn mask(num_bits: u8) -> u32 {
    if num_bits >= 32 {
        u32::MAX
    } else {
        (1u32 << num_bits) - 1
    }
}

#[test]
fn the_design_extracts_the_same_block_the_bitpacking_crate_compressed() {
    // The headline differential: 32 values, compressed by the crate, read back one at a
    // time by the design. Every `num_bits` in 1..=8, because the widths that divide the
    // block length evenly are the easy ones -- three, five, six and seven are where an
    // off-by-one in the intra-window offset shows up.
    for num_bits in 1..=8u8 {
        let values: Vec<u32> = (0..BLOCK)
            .map(|index| (index as u32).wrapping_mul(2_654_435_761) & mask(num_bits))
            .collect();
        let compressed = golden_compress(&values, num_bits);
        let want = golden_decompress(&compressed, num_bits);
        assert_eq!(want, values, "the crate round-trips at {num_bits} bits");

        let observed = extracted(&compressed, u32::from(num_bits), BLOCK);
        assert_eq!(
            observed,
            want.iter().copied().map(u64::from).collect::<Vec<_>>(),
            "the design against `bitpacking` at {num_bits} bits per value"
        );
    }
}

#[test]
fn a_value_at_its_own_offset_is_extracted_when_it_fits_and_declined_when_it_does_not() {
    // At a width that does not divide 64, some values straddle the window boundary: at
    // five bits, value 12 starts at bit 60 and its last bit is in the *next* window.
    // This design cannot produce that from a single 64-bit word -- it is the window
    // register's job, and `build` exposes `wrap` so the caller can do it -- so the
    // contract is that `fit` falls and the value reads zero rather than that the design
    // silently returns half a value. Both halves are asserted, because a design that
    // returned half a value with `fit` low would be much harder to debug.
    let num_bits = 5u8;
    let values: Vec<u32> = (0..BLOCK)
        .map(|index| ((index as u32) * 7 + 3) & mask(num_bits))
        .collect();
    let compressed = golden_compress(&values, num_bits);
    let want = golden_decompress(&compressed, num_bits);

    assert_eq!(
        (12 * usize::from(num_bits)) % 64,
        60,
        "and this is the case that was meant to straddle"
    );

    let observed = extracted_in_place(&compressed, 5, BLOCK);
    let mut compared = 0usize;
    let mut declined = 0usize;
    for (index, ((value, fit), expected)) in observed.iter().zip(want.iter()).enumerate() {
        let offset = (index * usize::from(num_bits)) % 64;
        if offset + usize::from(num_bits) <= 64 {
            assert_eq!(*fit, 1, "value {index} at offset {offset} fits");
            assert_eq!(
                *value,
                u64::from(*expected),
                "value {index} at offset {offset}"
            );
            compared += 1;
        } else {
            assert_eq!(*fit, 0, "value {index} at offset {offset} does not fit");
            assert_eq!(*value, 0, "and reads zero rather than half a value");
            declined += 1;
        }
    }
    assert!(
        compared > 0 && declined > 0,
        "{compared} compared, {declined} declined"
    );
}

#[test]
fn the_next_offset_and_the_wrap_flag_walk_the_window_in_step() {
    // `next_off` is `(off + 8) mod 64` and `wrap` is `off + 8 >= 64`, which for eight-bit
    // values is `off >= 56`. Both come out of the same 8-bit sum, and the carry that
    // distinguishes them is exactly the bit `Design::add` truncates to its left operand's
    // width -- so this asserts the widening is really there.
    let design = Design::new();
    bitpack::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            let mut rows = Vec::new();
            for off in 0..64u64 {
                tb.set("rst", 0).await;
                tb.set("advance", 0).await;
                tb.set("word", 0).await;
                tb.set("off", off).await;
                tb.step().await;
                rows.push((off, tb.value("next_off"), tb.value("wrap"), tb.value("fit")));
            }
            rows
        })
        .unwrap();
    for (off, next, wrap, fit) in observed {
        assert_eq!(next, (off + 8) % 64, "next offset at off {off}");
        assert_eq!(wrap, u64::from(off + 8 >= 64), "wrap at off {off}");
        assert_eq!(fit, u64::from(off + 8 <= 64), "fit at off {off}");
    }
}

#[test]
fn an_offset_past_the_end_of_the_window_reads_zero_and_says_so() {
    // Eight-bit values stop at `off = 56`: at 57 there is no eight-bit value inside a
    // 64-bit window, so the table's default arm answers zero and `fit` falls. A design
    // that returned garbage instead would make a stream decoder's window-crossing bug
    // invisible.
    let design = Design::new();
    bitpack::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            tb.set("rst", 0).await;
            tb.set("advance", 0).await;
            tb.set("word", u64::MAX).await;
            tb.set("off", 56).await;
            tb.step().await;
            let last = (tb.value("value"), tb.value("fit"), tb.value("wrap"));
            tb.set("off", 57).await;
            tb.step().await;
            (last, (tb.value("value"), tb.value("fit"), tb.value("wrap")))
        })
        .unwrap();
    assert_eq!(
        observed.0,
        (0xff, 1, 1),
        "off 56 is the last eight-bit value"
    );
    assert_eq!(observed.1, (0, 0, 1), "and off 57 is not inside the window");
}

#[test]
fn the_position_register_advances_only_when_told_to_and_resets_to_zero() {
    // The register is the one piece of state in the design, so both of its behaviours
    // matter: it holds without `advance`, it moves with it, and reset is a real input
    // rather than the initial value standing in for one.
    let design = Design::new();
    bitpack::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            clock_with(&tb, &[("rst", 1), ("advance", 0), ("word", 0), ("off", 0)]).await;
            let after_reset = tb.value("pos");
            clock_with(&tb, &[("rst", 0), ("advance", 0), ("word", 0), ("off", 8)]).await;
            let held = tb.value("pos");
            clock_with(&tb, &[("rst", 0), ("advance", 1), ("word", 0), ("off", 16)]).await;
            let moved_to_24 = tb.value("pos");
            clock_with(&tb, &[("rst", 0), ("advance", 1), ("word", 0), ("off", 56)]).await;
            let wrapped = (tb.value("pos"), tb.value("wrap"));
            (after_reset, held, moved_to_24, wrapped)
        })
        .unwrap();
    assert_eq!(
        observed.0, 0,
        "reset puts the position at the window's first bit"
    );
    assert_eq!(observed.1, 0, "and `off` alone does not move it");
    assert_eq!(observed.2, 24, "`advance` takes next_off, not `off`");
    assert_eq!(
        observed.3,
        (0, 1),
        "56 + 8 wraps to bit 0 of the next window"
    );
}

#[test]
fn every_declared_width_extracts_the_crates_block() {
    // The clockless standalone extractor, at every width the port list cannot express.
    // One bit is a single mask per address and sixty-four is a whole word, so the two
    // ends of the table's arithmetic are both covered.
    for num_bits in [1u8, 2, 3, 5, 6, 7, 8] {
        let values: Vec<u32> = (0..BLOCK)
            .map(|index| (index as u32 * 40503) & mask(num_bits))
            .collect();
        let compressed = golden_compress(&values, num_bits);
        assert_eq!(
            extracted(&compressed, u32::from(num_bits), BLOCK),
            values.iter().copied().map(u64::from).collect::<Vec<_>>(),
            "{num_bits} bits per value"
        );
    }
}

proptest! {
    /// The differential, on arbitrary blocks and arbitrary widths.
    ///
    /// The sweeps above cover the values at the boundaries; this covers the interior.
    /// Thirty-two cases is enough for a 32-integer block at eight widths, and the
    /// simulation cost is the only thing bounding it.
    #[test]
    fn the_design_agrees_with_bitpacking_on_arbitrary_blocks(
        num_bits in 1u8..=8,
        raw in prop::array::uniform32(any::<u32>())
    ) {
        let values: Vec<u32> = raw.iter().map(|value| value & mask(num_bits)).collect();
        let compressed = golden_compress(&values, num_bits);
        prop_assert_eq!(golden_decompress(&compressed, num_bits), values.clone());
        prop_assert_eq!(
            extracted(&compressed, u32::from(num_bits), BLOCK),
            values.iter().copied().map(u64::from).collect::<Vec<_>>()
        );
    }
}

#[test]
fn the_run_length_designer_decodes_every_run_once() {
    // The differential against the in-crate reference, cycle by cycle: a run of `n`
    // contributes exactly `n` valid cycles, and the run's first value appears on the
    // load edge rather than the one after it.
    let runs = [
        rle::Run::new(7, 3),
        rle::Run::new(200, 1),
        rle::Run::new(0, 2),
    ];
    let want = rle::decode(&runs);
    let observed = rle_hardware(&runs);
    assert_eq!(
        observed,
        want.iter().copied().map(u64::from).collect::<Vec<_>>(),
        "one value per cycle, and only while `valid`"
    );
    assert_eq!(observed.len(), 6, "3 + 1 + 2 values, and not one more");
}

#[test]
fn a_run_of_255_does_not_wrap_into_another_255_values() {
    // The single most consequential bug in this design: an unconditional decrement makes
    // `remaining` go 0, 255, 254, ... and the run emits 255 more values than it had. The
    // longest representable run is therefore the case that catches it.
    let runs = [rle::Run::new(0xab, 255)];
    assert_eq!(rle_hardware(&runs).len(), 255);
}

#[test]
fn a_zero_length_run_emits_nothing_and_leaves_valid_low() {
    let runs = [rle::Run::new(9, 0), rle::Run::new(1, 1)];
    assert_eq!(rle_hardware(&runs), vec![1]);
}

#[test]
fn the_run_counter_reads_back_what_the_designer_thinks_is_left() {
    // `remaining` is an output so a test can watch the counter rather than only its
    // effect, which is the difference between "the run decoded" and "the run counter
    // counted". A run of three must show 3, 2, 1, 0 and then stop.
    let design = Design::new();
    rle::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            clock_with(
                &tb,
                &[("rst", 1), ("load", 0), ("run_len", 0), ("run_value", 0)],
            )
            .await;
            let after_reset = (tb.value("valid"), tb.value("remaining"));
            clock_with(
                &tb,
                &[("rst", 0), ("load", 1), ("run_len", 3), ("run_value", 7)],
            )
            .await;
            let mut counters = vec![tb.value("remaining")];
            let mut valids = vec![tb.value("valid")];
            for _ in 0..4 {
                clock_with(
                    &tb,
                    &[("rst", 0), ("load", 0), ("run_len", 3), ("run_value", 7)],
                )
                .await;
                counters.push(tb.value("remaining"));
                valids.push(tb.value("valid"));
            }
            (after_reset, counters, valids)
        })
        .unwrap();
    assert_eq!(observed.0, (0, 0), "reset empties the run");
    assert_eq!(observed.1, vec![3, 2, 1, 0, 0], "and it stops at zero");
    assert_eq!(
        observed.2,
        vec![1, 1, 1, 0, 0],
        "three values from a run of three"
    );
}

#[test]
fn a_load_in_the_middle_of_a_run_restarts_it() {
    // On a bus this is every row after the first. A design that could only start a run
    // at reset would decode the first row and then emit the wrong byte forever.
    let design = Design::new();
    rle::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            clock_with(
                &tb,
                &[("rst", 1), ("load", 0), ("run_len", 0), ("run_value", 0)],
            )
            .await;
            clock_with(
                &tb,
                &[("rst", 0), ("load", 1), ("run_len", 10), ("run_value", 1)],
            )
            .await;
            clock_with(
                &tb,
                &[("rst", 0), ("load", 1), ("run_len", 2), ("run_value", 2)],
            )
            .await;
            let reloaded = (tb.value("data"), tb.value("remaining"));
            clock_with(
                &tb,
                &[("rst", 0), ("load", 0), ("run_len", 2), ("run_value", 2)],
            )
            .await;
            (reloaded, tb.value("data"), tb.value("remaining"))
        })
        .unwrap();
    assert_eq!(
        observed.0,
        (2, 2),
        "the new run's value and length replace the old run's"
    );
    assert_eq!(observed.2, 1, "and the counter restarts from it");
    assert_eq!(observed.1, 2, "with the new value held");
}

/// Feeds `runs` through the run-length design and returns one `u64` per valid cycle.
fn rle_hardware(runs: &[rle::Run]) -> Vec<u64> {
    let design = Design::new();
    rle::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        clock_with(
            &tb,
            &[("rst", 1), ("load", 0), ("run_len", 0), ("run_value", 0)],
        )
        .await;
        let mut out = Vec::new();
        for run in runs {
            clock_with(
                &tb,
                &[
                    ("rst", 0),
                    ("load", 1),
                    ("run_len", u64::from(run.length)),
                    ("run_value", u64::from(run.value)),
                ],
            )
            .await;
            // The load edge already armed the first value, so it is read before any
            // further edge rather than after one.
            if tb.value("valid") == 1 {
                out.push(tb.value("data"));
            }
            while tb.value("valid") == 1 {
                clock_with(
                    &tb,
                    &[
                        ("rst", 0),
                        ("load", 0),
                        ("run_len", u64::from(run.length)),
                        ("run_value", u64::from(run.value)),
                    ],
                )
                .await;
                if tb.value("valid") == 1 {
                    out.push(tb.value("data"));
                }
            }
        }
        out
    })
    .expect("the rle design drives only ports it declares")
}

proptest! {
    /// The run-length differential, on arbitrary run lists.
    ///
    /// Lengths are drawn from `0..=6` rather than the full byte so that a short random
    /// list usually contains at least one zero-length run and at least one adjacent run
    /// of equal value -- the two cases whose handling a wrong `valid` gets wrong.
    #[test]
    fn the_run_length_design_agrees_with_the_reference_on_arbitrary_runs(
        runs in prop::collection::vec((any::<u8>(), 0u8..6), 0..10)
    ) {
        let runs: Vec<rle::Run> = runs
            .into_iter()
            .map(|(value, length)| rle::Run::new(value, length))
            .collect();
        prop_assert_eq!(rle_hardware(&runs), rle::decode(&runs).iter().copied().map(u64::from).collect::<Vec<_>>());
    }
}

#[test]
fn the_delta_designer_accumulates_from_the_base() {
    let base = 1_000_000u32;
    let deltas = [3u32, 0, 4, 1, 5, 9, 2, 6];
    let want = rle::delta_decode(base, &deltas);
    assert_eq!(
        want.len(),
        deltas.len() + 1,
        "the base is an output of its own"
    );
    let observed = delta_hardware(base, &deltas);
    assert_eq!(
        observed,
        want.iter().copied().map(u64::from).collect::<Vec<_>>()
    );
}

#[test]
fn the_delta_accumulator_wraps_rather_than_saturating() {
    // The definition the reference gives, and a design that saturated instead would give
    // a plausible ascending-looking answer for a descending stream.
    let observed = delta_hardware(u32::MAX, &[1, 2, 3]);
    assert_eq!(observed, vec![u32::MAX as u64, 0, 2, 5]);
}

#[test]
fn a_delta_load_resets_the_accumulator_mid_stream() {
    let design = Design::new();
    rle::build_delta(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            clock_with(&tb, &[("rst", 1), ("load", 0), ("base", 0), ("delta", 0)]).await;
            let after_reset = (tb.value("value"), tb.value("valid"));
            clock_with(&tb, &[("rst", 0), ("load", 1), ("base", 5), ("delta", 1)]).await;
            let loaded = tb.value("value");
            clock_with(&tb, &[("rst", 0), ("load", 0), ("base", 5), ("delta", 1)]).await;
            let stepped = tb.value("value");
            clock_with(&tb, &[("rst", 0), ("load", 1), ("base", 100), ("delta", 7)]).await;
            (after_reset, loaded, stepped, tb.value("value"))
        })
        .unwrap();
    assert_eq!(
        observed.0,
        (0, 0),
        "reset zeroes the accumulator and drops valid"
    );
    assert_eq!(observed.1, 5, "the load edge is the base");
    assert_eq!(observed.2, 6, "and every other edge adds the delta");
    assert_eq!(observed.3, 100, "and a load in the middle replaces it");
}

proptest! {
    /// The delta differential, on arbitrary bases and deltas.
    #[test]
    fn the_delta_design_agrees_with_the_reference_on_arbitrary_deltas(
        base in any::<u32>(),
        deltas in prop::collection::vec(any::<u32>(), 0..12)
    ) {
        let observed = delta_hardware(base, &deltas);
        let want: Vec<u64> = rle::delta_decode(base, &deltas)
            .iter()
            .copied()
            .map(u64::from)
            .collect();
        prop_assert_eq!(observed, want);
    }
}

/// Feeds `deltas` through the delta design and returns the accumulator per cycle.
fn delta_hardware(base: u32, deltas: &[u32]) -> Vec<u64> {
    let design = Design::new();
    rle::build_delta(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        clock_with(&tb, &[("rst", 1), ("load", 0), ("base", 0), ("delta", 0)]).await;
        clock_with(
            &tb,
            &[
                ("rst", 0),
                ("load", 1),
                ("base", u64::from(base)),
                ("delta", 0),
            ],
        )
        .await;
        let mut out = vec![tb.value("value")];
        for delta in deltas {
            clock_with(
                &tb,
                &[
                    ("rst", 0),
                    ("load", 0),
                    ("base", u64::from(base)),
                    ("delta", u64::from(*delta)),
                ],
            )
            .await;
            out.push(tb.value("value"));
        }
        out
    })
    .expect("the delta design drives only ports it declares")
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_bit_unpacker() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the bitpack equivalence did not run. The \
             differentials against `bitpacking` above do not need it."
        );
        return;
    };
    println!("cosimulating bitpack with {}", verilator.display());

    let design = Design::new();
    let ports = bitpack::build(&design).unwrap();
    let module = ferrite_lithic_rtl::Module::new(
        "bitpack",
        design.build().unwrap(),
        ports.inputs.clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let plan = Plan::of(&design, &module).unwrap();
    assert_eq!(
        plan.inputs
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "word", "off", "advance"],
        "the clock is a port but not a stimulus column"
    );

    // A reset, then a walk up the whole offset range with `advance` held high so the
    // position register moves, which exercises the register as well as the table. The
    // offsets include the last legal one and the first illegal one, and the words are
    // all-ones so a one-bit shift would be visible.
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, word: u64, off: u64, advance: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(word, 64).unwrap(),
            Bits::constant(off, 7).unwrap(),
            Bits::constant(advance, 1).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0)).unwrap();
    for off in 0..64u64 {
        stimulus.push(row(0, u64::MAX, off, 1)).unwrap();
    }
    for off in 0..64u64 {
        stimulus
            .push(row(0, 0x0123_4567_89ab_cdef ^ off, off, 0))
            .unwrap();
    }
    assert_eq!(stimulus.cycles(), 129);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same block through the testbench agrees with the crate, so all three --
    // simulator, Verilator and `bitpacking` -- meet.
    let values: Vec<u32> = (0..BLOCK)
        .map(|index| (index as u32 * 11 + 5) & 0xff)
        .collect();
    let compressed = golden_compress(&values, 8);
    assert_eq!(
        extracted(&compressed, 8, BLOCK),
        golden_decompress(&compressed, 8)
            .iter()
            .copied()
            .map(u64::from)
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_run_length_designer() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the rle equivalence did not run. The \
             differential against `rle::decode` above does not need it."
        );
        return;
    };
    println!("cosimulating rle with {}", verilator.display());

    let design = Design::new();
    let ports = rle::build(&design).unwrap();
    let module = ferrite_lithic_rtl::Module::new(
        "rle",
        design.build().unwrap(),
        ports.inputs.clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let plan = Plan::of(&design, &module).unwrap();
    assert_eq!(
        plan.inputs
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "load", "run_len", "run_value"]
    );

    // The corner cases, in one stimulus: a zero-length run, a run of 255 so the counter
    // has to saturate rather than wrap, a one-byte run, and a reset in the middle.
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, load: u64, len: u64, value: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(load, 1).unwrap(),
            Bits::constant(len, 8).unwrap(),
            Bits::constant(value, 8).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0)).unwrap();
    stimulus.push(row(0, 1, 0, 200)).unwrap();
    stimulus.push(row(0, 1, 4, 7)).unwrap();
    for _ in 0..6 {
        stimulus.push(row(0, 0, 4, 7)).unwrap();
    }
    stimulus.push(row(0, 1, 1, 42)).unwrap();
    stimulus.push(row(0, 0, 1, 42)).unwrap();
    stimulus.push(row(0, 1, 255, 9)).unwrap();
    for _ in 0..260 {
        stimulus.push(row(0, 0, 255, 9)).unwrap();
    }
    stimulus.push(row(1, 0, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0, 0)).unwrap();
    assert_eq!(stimulus.cycles(), 274);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );
}
