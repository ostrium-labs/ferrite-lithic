//! Roaring set operations, the three sketches, Hamming distance and sorting networks.
//!
//! Four modules, four different kinds of evidence, and the distinctions are the point:
//!
//! - **roaring** is checked against the **`roaring` crate** -- the run-container
//!   algebra, including the canonical form, and the design's two-run kernel against
//!   `union_len`/`intersection_len`.
//! - **sketch** has no golden crate, and could not usefully have one: the answers are
//!   probabilities. Every assertion is therefore against a *stated bound*, with the
//!   derivation next to it, and the deterministic parts (a bloom filter's
//!   false-negative rate, count-min's two-sided bound, HyperLogLog's raw estimator on a
//!   hand-built register bank) are checked exactly.
//! - **hamming** is checked against a one-line reference and against a hand-computed
//!   worst case, plus the comparator network's *measured* depth against its formula.
//! - **sorting** is checked against `slice::sort`, through the *design*, so a comparator
//!   that fires in the wrong order is caught rather than hidden by a second sort.
//!
//! Every design is also cosimulated against Verilator. The port-name and stimulus-arity
//! assertions live next to the designs they pin, because a rename that lands in one
//! place and not another turns every differential above into a failure that has nothing
//! to do with the algorithm.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::{hamming, roaring, sketch, sorting};
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::{Handle, Testbench};
use proptest::prelude::*;

/// Applies one set of input values and takes exactly one clock edge.
///
/// The testbench applies **one input port per rising edge**, so a cycle in which several
/// inputs change is `set` for all of them and `drive` for the last.
async fn clock_with(tb: &Handle, values: &[(&str, u64)]) {
    let (last, rest) = values.split_last().expect("a cycle sets at least one port");
    for (name, value) in rest {
        tb.set(name, *value).await;
    }
    tb.drive(last.0, last.1).await;
}

/// The cosimulation plan for a built design.
///
/// [`Plan::of`] is what knows how to exclude the clock from the stimulus columns, so the
/// plan is always derived rather than written out.
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

/// Runs a stimulus through Verilator and asserts the two backends agree.
///
/// Prints which Verilator is doing it and says so loudly when there is none, because a
/// silently-skipped equivalence test is worse than no test: it reads as a pass.
fn cosimulate(design: &Design, plan: &Plan, stimulus: &Stimulus, what: &str) {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the {what} equivalence did not run. The \
             differentials above do not need it."
        );
        return;
    };
    println!("cosimulating {what} with {}", verilator.display());
    let verilog = ferrite_lithic_cosim::emit(design, plan).unwrap();
    let harness = Harness::build(plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(design, plan, stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree on {what}: {report}\nfirst: {:?}",
        report.first()
    );
}

// ===================================================================== roaring

/// The `roaring` crate's bitmap for one run.
fn golden_run(run: roaring::Run) -> ::roaring::RoaringBitmap {
    let mut bitmap = ::roaring::RoaringBitmap::new();
    if !run.is_empty() {
        bitmap.insert_range(run.start..run.end());
    }
    bitmap
}

/// The `roaring` crate's answer for a pair of runs: `(union len, intersection len)`.
fn golden_pair(left: roaring::Run, right: roaring::Run) -> (u64, u64) {
    let a = golden_run(left);
    let b = golden_run(right);
    (a.union_len(&b), a.intersection_len(&b))
}

/// What the two-run design answered: `(card, lo, hi, lap)`.
fn roaring_hardware(left: roaring::Run, right: roaring::Run) -> (u64, u64, u64, u64) {
    let design = Design::new();
    roaring::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        clock_with(
            &tb,
            &[
                ("rst", 0),
                ("a_len", u64::from(left.len)),
                ("b_start", u64::from(right.start)),
                ("b_len", u64::from(right.len)),
                ("a_start", u64::from(left.start)),
            ],
        )
        .await;
        (
            tb.value("card"),
            tb.value("lo"),
            tb.value("hi"),
            tb.value("lap"),
        )
    })
    .expect("the roaring design drives only ports it declares")
}

#[test]
fn the_two_run_kernel_agrees_with_the_roaring_crate() {
    // The corner cases of interval arithmetic, in one sweep: disjoint, touching,
    // identical, nested either way, one inside the other by one, and each of those
    // with an empty run on one side or the other. Every one of them has a hand-computable
    // answer and every one of them is a case where an inclusive/exclusive mix-up in the
    // overlap shows up as an answer that is off by exactly one.
    let cases = [
        (roaring::Run::new(0, 10), roaring::Run::new(20, 10)),
        (roaring::Run::new(0, 10), roaring::Run::new(10, 10)),
        (roaring::Run::new(5, 10), roaring::Run::new(5, 10)),
        (roaring::Run::new(0, 100), roaring::Run::new(10, 10)),
        (roaring::Run::new(10, 10), roaring::Run::new(0, 100)),
        (roaring::Run::new(0, 11), roaring::Run::new(10, 11)),
        (roaring::Run::new(0, 10), roaring::Run::new(0, 0)),
        (roaring::Run::new(0, 0), roaring::Run::new(0, 10)),
        (roaring::Run::new(0, 0), roaring::Run::new(0, 0)),
        (roaring::Run::new(1000, 1), roaring::Run::new(1000, 1)),
    ];
    for (left, right) in cases {
        let (card, lo, hi, lap) = roaring_hardware(left, right);
        let (want_card, want_lap) = golden_pair(left, right);
        assert_eq!(card, want_card, "union of {left:?} and {right:?}");
        assert_eq!(lap, want_lap, "intersection of {left:?} and {right:?}");

        let union = roaring::union(&[left], &[right]);
        let (want_lo, want_hi) = roaring::span(&union);
        assert_eq!(lo, u64::from(want_lo), "span low of {left:?} and {right:?}");
        assert_eq!(
            hi,
            u64::from(want_hi),
            "span high of {left:?} and {right:?}"
        );
        assert_eq!(card, u64::from(roaring::cardinality(&union)));
    }
}

#[test]
fn the_two_run_kernel_agrees_with_the_crate_at_the_widest_endpoints_the_ports_allow() {
    // Sixteen-bit starts and sixteen-bit lengths put the largest endpoint at 131070, so
    // this is the case where the span ports (seventeen bits) and the cardinality would
    // overflow if they were one bit narrower.
    //
    // `left` is [65535, 131070) and `right` is [0, 65535), so they meet at exactly one
    // point -- 65535 -- which is `right`'s *exclusive* end and therefore in neither.
    let left = roaring::Run::new(65_535, 65_535);
    let right = roaring::Run::new(0, 65_535);
    let (card, lo, hi, lap) = roaring_hardware(left, right);
    assert_eq!(card, 131_070, "every integer from 0 to 131069");
    assert_eq!((lo, hi), (0, 131_070));
    assert_eq!(lap, 0, "two runs that touch at one endpoint are disjoint");
    let (want_card, want_lap) = golden_pair(left, right);
    assert_eq!((card, lap), (want_card, want_lap), "and the crate agrees");

    // And a case where the span has to hold a value no single input could:
    // `1 + 65535 = 65536` is seventeen bits, which is exactly why
    // [`roaring::SPAN_BITS`] is seventeen and not sixteen. A sixteen-bit span port would
    // wrap it to zero.
    let overlapping = roaring::Run::new(1, 65_535);
    assert_eq!(
        roaring_hardware(overlapping, right),
        (65_536, 0, 65_536, 65_534),
        "1..65536 against 0..65535: the span needs seventeen bits, the overlap is 65534"
    );
}

#[test]
fn the_software_run_union_matches_the_roaring_crate() {
    let left = [
        roaring::Run::new(0, 100),
        roaring::Run::new(200, 50),
        roaring::Run::new(1000, 1),
    ];
    let right = [
        roaring::Run::new(50, 200),
        roaring::Run::new(1001, 1),
        roaring::Run::new(5000, 7),
    ];
    let mut a = ::roaring::RoaringBitmap::new();
    let mut b = ::roaring::RoaringBitmap::new();
    for run in left {
        a.insert_range(run.start..run.end());
    }
    for run in right {
        b.insert_range(run.start..run.end());
    }

    let union = roaring::union(&left, &right);
    assert_eq!(
        u64::from(roaring::cardinality(&union)),
        a.union_len(&b),
        "the crate's union_len is the cardinality of the merged runs"
    );
    let overlap = roaring::intersection(&left, &right);
    assert_eq!(
        u64::from(roaring::cardinality(&overlap)),
        a.intersection_len(&b)
    );

    // The canonical form, which is what makes the run *count* meaningful: no empty runs,
    // sorted, disjoint, and no two runs adjacent.
    for pair in union.windows(2) {
        assert!(pair[0].end() < pair[1].start, "{:?} are not disjoint", pair);
    }
    assert!(union.iter().all(|run| !run.is_empty()));
}

#[test]
fn a_container_union_and_intersection_match_the_crates() {
    let mut a = roaring::Container::new(0);
    a.push(roaring::Run::new(10, 100));
    a.push(roaring::Run::new(300, 5));
    let mut b = roaring::Container::new(0);
    b.push(roaring::Run::new(50, 100));
    b.push(roaring::Run::new(302, 3));

    let union = roaring::union_containers(&a, &b);
    let overlap = roaring::intersection_containers(&a, &b);
    assert_eq!(a.cardinality(), 105);
    assert_eq!(
        union.cardinality(),
        145,
        "10..150 is 140 and 300..305 is 5: the shared 50..110 is counted once"
    );
    assert_eq!(
        overlap.cardinality(),
        63,
        "50..110 is 60, plus 302..305 is 3"
    );

    let mut bitmap = ::roaring::RoaringBitmap::new();
    for run in &union.runs {
        bitmap.insert_range(run.start..run.end());
    }
    assert_eq!(u64::from(union.cardinality()), bitmap.len());
}

proptest! {
    /// The two-run kernel, on arbitrary intervals inside the port range.
    ///
    /// Starts and lengths are drawn below `1 << 16` because that is the port width, and
    /// the design does not clamp: a start of 70000 does not fit in a sixteen-bit port
    /// and the testbench would report a port-width failure rather than a wrong answer.
    #[test]
    fn the_two_run_kernel_agrees_with_the_roaring_crate_on_arbitrary_runs(
        a in (0u32..1 << 16, 0u32..1 << 16),
        b in (0u32..1 << 16, 0u32..1 << 16),
    ) {
        let left = roaring::Run::new(a.0, a.1);
        let right = roaring::Run::new(b.0, b.1);
        let (card, _lo, _hi, lap) = roaring_hardware(left, right);
        let (want_card, want_lap) = golden_pair(left, right);
        prop_assert!(card == want_card, "union of {left:?} and {right:?} gave {card}, wanted {want_card}");
        prop_assert!(lap == want_lap, "intersection of {left:?} and {right:?} gave {lap}, wanted {want_lap}");
    }
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_roaring_kernel() {
    let design = Design::new();
    let ports = roaring::build(&design).unwrap();
    let plan = plan_of(&design, "roaring_run");
    assert_eq!(
        plan.inputs
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "a_start", "a_len", "b_start", "b_len"],
        "the clock is a port but not a stimulus column"
    );
    let _ = ports;

    // A reset, then the same corner-case sweep the differential walks, so the
    // equivalence sees the empty-run convention as well as the arithmetic.
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, a_start: u64, a_len: u64, b_start: u64, b_len: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(a_start, 16).unwrap(),
            Bits::constant(a_len, 16).unwrap(),
            Bits::constant(b_start, 16).unwrap(),
            Bits::constant(b_len, 16).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0, 0)).unwrap();
    let cases = [
        (0, 10, 20, 10),
        (0, 10, 10, 10),
        (5, 10, 5, 10),
        (0, 100, 10, 10),
        (10, 10, 0, 100),
        (0, 11, 10, 11),
        (0, 10, 0, 0),
        (0, 0, 0, 10),
        (0, 0, 0, 0),
        (1000, 1, 1000, 1),
        (65_535, 65_535, 0, 65_535),
        (65_535, 65_535, 65_535, 65_535),
        (0, 65_535, 65_535, 65_535),
    ];
    for (a_start, a_len, b_start, b_len) in cases {
        stimulus
            .push(row(0, a_start, a_len, b_start, b_len))
            .unwrap();
        stimulus
            .push(row(
                0,
                a_start ^ 0x1234,
                a_len ^ 0x5678,
                b_start ^ 0x9abc,
                b_len ^ 0xdef0,
            ))
            .unwrap();
    }
    assert_eq!(stimulus.cycles(), 1 + 2 * cases.len());

    cosimulate(&design, &plan, &stimulus, "the roaring run kernel");
}

// ====================================================================== sketch

/// A key for the sketches: eight bytes, so that the key is not a short string.
///
/// Eight bytes rather than four because FNV-1a avalanches a four-byte key poorly: its
/// high bits stay weakly mixed, and HyperLogLog reads exactly the high bits.
fn key(index: u32) -> [u8; 8] {
    let [a, b, c, d] = index.to_le_bytes();
    [a, b, c, d, d, c, b, a]
}

#[test]
fn a_bloom_filter_has_no_false_negatives_at_any_size() {
    // The *exact* guarantee. A filter that answered "absent" for a key it had been given
    // would be a bug with no tolerance to hide behind, so this is asserted rather than
    // bounded.
    for (m, k, n) in [(64usize, 1usize, 8usize), (2048, 4, 128), (4096, 6, 500)] {
        let mut filter = sketch::Bloom::new(m, k);
        for index in 0..n as u32 {
            filter.insert(&key(index));
        }
        for index in 0..n as u32 {
            assert!(
                filter.contains(&key(index)),
                "m = {m}, k = {k}: inserted key {index} is absent, which cannot happen"
            );
        }
    }
}

#[test]
fn a_bloom_fills_about_as_many_bits_as_the_occupancy_formula_says() {
    // `m * (1 - (1 - 1/m)^(kn))`, the expected number of set bits. The count is a sum of
    // `m` independent-ish Bernoulli trials, so its standard deviation is
    // `sqrt(m * p * (1 - p))` with `p` the per-bit set probability. Five sigma at
    // m = 2048, k = 4, n = 128 is about 94 bits on an expectation of 453.
    let (m, k, n) = (2048usize, 4usize, 128usize);
    let mut filter = sketch::Bloom::new(m, k);
    for index in 0..n as u32 {
        filter.insert(&key(index));
    }
    let occupied = f64::from(filter.occupancy());
    let expected = sketch::bloom_expected_occupancy(m, k, n);
    let per_bit = expected / m as f64;
    let sigma = (m as f64 * per_bit * (1.0 - per_bit)).sqrt();
    assert!(
        (occupied - expected).abs() <= 5.0 * sigma,
        "{occupied} bits set against an expectation of {expected} +/- {sigma}"
    );
}

#[test]
fn the_bloom_false_positive_rate_is_within_five_sigma_of_its_formula() {
    // `(1 - e^{-kn/m})^k` is the *expected* rate, not a per-query bound, and the thing
    // that is actually random here is the **count** of false positives over `T` queries,
    // which is Binomial(T, p). So the assertion is on the count:
    //
    // ```text
    // p     = (1 - e^{-4*128/2048})^4 = 0.002394
    // T     = 200_000, so E[hits] = 478.8
    // sigma = sqrt(T p (1 - p))   = 21.9
    // ```
    //
    // Five sigma is about 109 hits, i.e. a tolerance of 5.5e-4 on a rate of 2.4e-3 --
    // 22% relative. Tightening it needs more queries, not a better formula: the
    // relative error of a rate estimate goes as `1 / sqrt(T p)`, so it takes sixteen
    // times the queries to halve it.
    let (m, k, n) = (2048usize, 4usize, 128usize);
    let trials = 200_000usize;
    let mut filter = sketch::Bloom::new(m, k);
    for index in 0..n as u32 {
        filter.insert(&key(index));
    }
    let expected_rate = sketch::bloom_false_positive_rate(m, k, n);
    assert!(
        (0.0..1.0).contains(&expected_rate),
        "the formula is a probability: {expected_rate}"
    );

    let hits = (0..trials)
        .filter(|index| filter.contains(&key(1_000_000 + *index as u32)))
        .count();
    let observed = hits as f64 / trials as f64;
    let sigma = (trials as f64 * expected_rate * (1.0 - expected_rate)).sqrt();
    assert!(
        (observed - expected_rate).abs() <= 5.0 * sigma / trials as f64,
        "{hits} false positives in {trials} queries is a rate of {observed}, against \
         {expected_rate} +/- {} at five sigma",
        5.0 * sigma / trials as f64
    );
}

#[test]
fn count_min_never_under_estimates_and_never_over_by_more_than_n_over_w() {
    // Both of these are **deterministic**, so both are asserted exactly and neither can
    // flake. The lower bound holds because every row's counter only ever increases; the
    // upper bound holds because a row's counter for a given key is at most the total
    // number of updates `n`, and the estimate is the minimum over `w` rows, so at most
    // one row can be carrying all of `n` -- giving `true + n/w`.
    let (w, d, n) = (4usize, 256usize, 512usize);
    let mut sketch = sketch::CountMin::new(w, d);
    let mut truth = std::collections::HashMap::new();
    for index in 0..n as u32 {
        // A repeated key rather than a permutation of `n` distinct ones: real streams
        // are skewed, and a skew is what makes the `n/w` bound reachable rather than
        // comfortably loose.
        let item = index % 128;
        sketch.increment(&key(item));
        *truth.entry(item).or_insert(0u32) += 1;
    }

    for (item, count) in &truth {
        let estimate = sketch.estimate(&key(*item));
        assert!(
            estimate >= *count,
            "item {item} was inserted {count} times and estimated at {estimate}"
        );
        assert!(
            estimate <= count + (n / w) as u32,
            "item {item} estimated at {estimate}, above the worst case of {}",
            count + (n / w) as u32
        );
    }
    assert_eq!(
        sketch
            .counters()
            .iter()
            .map(|counter| u64::from(*counter))
            .sum::<u64>(),
        (w * n) as u64,
        "every update touched exactly one counter per row"
    );
}

#[test]
fn the_count_min_overestimate_is_statistically_tiny() {
    // The worst case above is `n/w`, which is loose by orders of magnitude. For an
    // *unseen* key, row `r`'s counter is Binomial(n, 1/d) with mean `n/d`, and the
    // estimate is the minimum over `w` rows, so
    //
    // ```text
    // E[overestimate] <= n / (w * d) = 512 / 1024 = 0.5
    // ```
    //
    // Four times that bound is 2.0, and the true value is about 0.8 because the minimum
    // of four Poisson(2) variables is well below its mean. Asserted on the mean over
    // 20,000 unseen keys, and above zero because the chance that every key lands on an
    // empty column in all four rows is about `(1 - 1/256)^(4*512)` = 3e-4.
    let (w, d, n) = (4usize, 256usize, 512usize);
    let mut sketch = sketch::CountMin::new(w, d);
    for index in 0..n as u32 {
        sketch.increment(&key(index));
    }
    let queries = 20_000u32;
    let total: u64 = (0..queries)
        .map(|index| u64::from(sketch.estimate(&key(5_000_000 + index))))
        .sum();
    let mean = total as f64 / f64::from(queries);
    let bound = n as f64 / (w * d) as f64;
    assert!(
        mean > 0.0,
        "a mean overestimate of zero would mean no key ever collided"
    );
    assert!(
        mean <= 4.0 * bound,
        "mean overestimate {mean} against a bound of {bound}"
    );
}

#[test]
fn the_hyperloglog_estimator_is_exact_on_a_hand_computed_register_bank() {
    // The raw estimator with no correction: `alpha_m * m^2 / sum(2^-M[j])`. Every number
    // below is arithmetic on four hand-written registers, so this is the exact part of a
    // probabilistic structure being checked exactly.
    let alpha = 0.7213 / (1.0 + 1.079 / 4.0);
    let equal = sketch::HyperLogLog::from_registers(2, 62, &[2, 2, 2, 2]);
    assert!((equal.raw_estimate() - alpha * 16.0).abs() < 1e-12);

    let uneven = sketch::HyperLogLog::from_registers(2, 62, &[1, 2, 3, 4]);
    assert!((uneven.raw_estimate() - alpha * 16.0 / 0.9375).abs() < 1e-12);

    // Linear counting: three of four registers zero, so V = 3 and the estimate is
    // `m * ln(m/V) = 4 ln(4/3)`. It applies because the raw estimate is 2.6, below the
    // `2.5 m = 10` the paper sets as the boundary.
    let sparse = sketch::HyperLogLog::from_registers(2, 62, &[1, 0, 0, 0]);
    assert!((sparse.estimate() - 4.0 * (4.0f64 / 3.0).ln()).abs() < 1e-12);

    // Every register set means `V = 0`, and `ln(m/0)` is not an estimate: the raw one is
    // the answer, and the guard is what stops this returning an infinity.
    let full = sketch::HyperLogLog::from_registers(2, 62, &[1, 1, 1, 1]);
    assert!(full.estimate().is_finite());
    assert!((full.estimate() - full.raw_estimate()).abs() < 1e-12);
}

#[test]
fn the_hyperloglog_is_within_four_sigma_of_the_cardinality_it_counted() {
    // Flajolet et al. give the relative standard error as `1.04 / sqrt(m)`. With
    // `m = 4096` that is 1.625%, so four sigma is 6.5%. The stream is half a million
    // distinct keys, which is `122` registers' worth of load -- deep enough that the
    // large-cardinality asymptotics the error bound assumes are the right ones.
    let mut sketch = sketch::HyperLogLog::new(4096);
    assert_eq!(sketch.registers_count(), 4096);
    let n = 500_000u64;
    for index in 0..n as u32 {
        sketch.insert(&key(index));
    }
    let estimate = sketch.estimate();
    let relative = ((estimate - n as f64) / n as f64).abs();
    let four_sigma = 4.0 * sketch.relative_standard_error();
    assert!(
        relative <= four_sigma,
        "estimated {estimate} for {n}, a relative error of {relative} against a \
         four-sigma band of {four_sigma}"
    );
}

#[test]
fn the_bloom_designs_bit_array_agrees_with_the_reference() {
    // The design's register against `indexed_insert`, cycle by cycle, and its membership
    // output against `indexed_member`. Insert the same probes twice: a bloom filter that
    // is not idempotent has no false negatives on the *second* insert of a key, which is
    // the property the whole structure rests on.
    let design = Design::new();
    sketch::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let probes = [(1u32, 40u32), (40, 1), (0, 63), (63, 0), (17, 17)];
    let observed = tb
        .run(|tb| async move {
            clock_with(&tb, &[("rst", 1), ("h0", 0), ("h1", 0), ("add", 0)]).await;
            let mut rows = Vec::new();
            for round in 0..2 {
                for (h0, h1) in probes {
                    clock_with(
                        &tb,
                        &[
                            ("rst", 0),
                            ("h0", u64::from(h0)),
                            ("h1", u64::from(h1)),
                            ("add", 1),
                        ],
                    )
                    .await;
                    rows.push((
                        h0,
                        h1,
                        round,
                        tb.value("bits"),
                        tb.value("member"),
                        tb.value("full"),
                    ));
                }
            }
            rows
        })
        .unwrap();

    let mut bits = 0u64;
    for (h0, h1, round, got_bits, got_member, got_full) in observed {
        bits = sketch::indexed_insert(bits, [h0, h1]);
        assert_eq!(got_bits, bits, "round {round}, probes {h0} and {h1}");
        assert_eq!(
            got_member,
            u64::from(sketch::indexed_member(bits, [h0, h1])),
            "round {round}: a just-inserted pair is present"
        );
        assert_eq!(got_full, u64::from(bits == u64::MAX));
        assert!(
            sketch::indexed_member(bits, [h0, h1]),
            "and the filter never clears a bit, so nothing is ever a false negative"
        );
    }
}

#[test]
fn the_bloom_designs_full_flag_fires_only_when_the_filter_is_saturated() {
    // Thirty-two inserts of `(i, i + 32)` for `i` in `0..32` set all sixty-four bits, so
    // `full` rises exactly once and the filter reports every key present thereafter.
    let design = Design::new();
    sketch::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            clock_with(&tb, &[("rst", 1), ("h0", 0), ("h1", 0), ("add", 0)]).await;
            let empty = tb.value("full");
            let mut flags = Vec::new();
            for index in 0..32u64 {
                clock_with(
                    &tb,
                    &[("rst", 0), ("h0", index), ("h1", index + 32), ("add", 1)],
                )
                .await;
                flags.push((tb.value("full"), tb.value("bits").count_ones()));
            }
            (empty, flags)
        })
        .unwrap();
    assert_eq!(observed.0, 0);
    for (index, (full, ones)) in observed.1.iter().enumerate() {
        assert_eq!(
            *full,
            u64::from(*ones == 64),
            "after {} inserts of two distinct probes each",
            index + 1
        );
    }
    assert_eq!(observed.1.last().expect("thirty-two inserts").0, 1);
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_bloom_filter() {
    let design = Design::new();
    let ports = sketch::build(&design).unwrap();
    let plan = plan_of(&design, "bloom_filter");
    assert_eq!(
        plan.inputs
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "h0", "h1", "add"]
    );
    let _ = ports;

    let mut stimulus = Stimulus::new();
    let row = |rst: u64, h0: u64, h1: u64, add: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(h0, 6).unwrap(),
            Bits::constant(h1, 6).unwrap(),
            Bits::constant(add, 1).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0)).unwrap();
    // Two rounds over every probe pair, so the register's idempotence and the whole
    // one-hot decoder's address range are both exercised.
    for round in 0..2u64 {
        for index in 0..64u64 {
            stimulus
                .push(row(0, index, (index + 32) % 64, round))
                .unwrap();
        }
    }
    assert_eq!(stimulus.cycles(), 1 + 2 * 64);

    cosimulate(&design, &plan, &stimulus, "the bloom filter");
}

// ===================================================================== hamming

/// What the distance design answered for two 128-bit codewords.
fn hamming_hardware(a: [u64; 2], b: [u64; 2]) -> u64 {
    let design = Design::new();
    hamming::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        clock_with(
            &tb,
            &[
                ("rst", 0),
                ("a0", a[0]),
                ("a1", a[1]),
                ("b0", b[0]),
                ("b1", b[1]),
            ],
        )
        .await;
        tb.value("dist")
    })
    .expect("the hamming design drives only ports it declares")
}

#[test]
fn the_popcount_tree_agrees_with_the_software_popcount() {
    // The cases that a popcount gets wrong are the ones with runs of ones, because an
    // adder tree that forgets a carry at a level boundary produces the right answer for
    // isolated bits and the wrong answer for a block of them. `u64::MAX` is 64, not 63.
    let cases = [
        ([0u64, 0], [0, 0]),
        ([u64::MAX, u64::MAX], [u64::MAX, u64::MAX]),
        ([u64::MAX, u64::MAX], [0, 0]),
        ([0xffff_ffff_ffff_ffff, 0], [0, 0]),
        ([0x5555_5555_5555_5555, 0xaaaa_aaaa_aaaa_aaaa], [0, 0]),
        ([0x0100_0000_0000_0000, 0x8000_0000_0000_0000], [0, 0]),
        ([0x8000_0000_0000_0000, 0], [0, 0]),
    ];
    for (a, b) in cases {
        assert_eq!(
            hamming_hardware(a, b),
            u64::from(hamming::hamming(&a, &b)),
            "{a:016x?} against {b:016x?}"
        );
    }
}

#[test]
fn the_distance_is_one_hundred_and_twenty_eight_at_the_extremes() {
    // A seven-bit distance output would report 127 for two complementary codewords,
    // which is the reason the output is eight bits. Asserting the extremes directly is
    // what pins that.
    assert_eq!(hamming_hardware([0, 0], [u64::MAX, u64::MAX]), 128);
    assert_eq!(hamming_hardware([u64::MAX, 0], [0, u64::MAX]), 128);
    assert_eq!(hamming_hardware([0, u64::MAX], [u64::MAX, 0]), 128);
    assert_eq!(hamming_hardware([1, 0], [0, 0]), 1);
}

#[test]
fn the_distance_is_symmetric_and_follows_the_port_pairing() {
    let a = [0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210];
    let b = [0xdead_beef_cafe_babe, 0x0f1e_2d3c_4b5a_6978];
    assert_eq!(
        hamming_hardware(a, b),
        hamming_hardware(b, a),
        "distance is symmetric"
    );

    // Swapping **both** sides, or crossing them, preserves the pairing between the two
    // 64-bit ports and so must not change the answer. Swapping only one side does not:
    // the design pairs `a0` with `b0` and `a1` with `b1`, so a one-sided swap makes a
    // different 64-bit difference in each pair and a different total. That is a fact
    // about the *structure* -- two half-width XORs summed, not one 128-bit XOR -- and it
    // is worth pinning, because a design that really did concatenate first would answer
    // differently here and the port split would be hiding it.
    let straight = hamming_hardware(a, b);
    assert_eq!(
        hamming_hardware([b[0], b[1]], [a[0], a[1]]),
        straight,
        "crossing the operands keeps the pairing"
    );
    assert_eq!(
        hamming_hardware([a[1], a[0]], [b[1], b[0]]),
        straight,
        "and so does swapping both of them"
    );
    assert_eq!(
        hamming_hardware([a[1], a[0]], b),
        u64::from(hamming::hamming(&[a[1], a[0]], &b)),
        "a one-sided swap pairs differently, and the reference agrees"
    );
}

proptest! {
    /// The popcount tree, on arbitrary codewords.
    ///
    /// Sixty-four cases is plenty: the tree has six levels and the failure mode it is
    /// being tested for -- a lost carry at a level boundary -- shows up on roughly half
    /// of all random inputs.
    #[test]
    fn the_popcount_tree_agrees_with_the_software_popcount_on_arbitrary_codes(
        a in prop::array::uniform8(any::<u64>()),
        b in prop::array::uniform8(any::<u64>()),
    ) {
        let left: [u64; 2] = [a[0], a[1]];
        let right: [u64; 2] = [b[0], b[1]];
        prop_assert_eq!(
            hamming_hardware(left, right),
            u64::from(hamming::hamming(&left, &right))
        );
    }
}

/// What the top-k design returned, `k_0` through `k_3`.
fn topk_hardware(distances: [u64; 4]) -> [u64; 4] {
    let design = Design::new();
    hamming::build_topk(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        clock_with(
            &tb,
            &[
                ("rst", 0),
                ("d_0", distances[0]),
                ("d_1", distances[1]),
                ("d_2", distances[2]),
                ("d_3", distances[3]),
            ],
        )
        .await;
        [
            tb.value("k_0"),
            tb.value("k_1"),
            tb.value("k_2"),
            tb.value("k_3"),
        ]
    })
    .expect("the top-k design drives only ports it declares")
}

#[test]
fn the_comparator_network_returns_the_distances_in_sorted_order() {
    // The reference is a `sort`, not a network -- see `hamming::top_k`'s docs for why
    // that is the right thing to compare against. What is being asserted is the design's
    // *output order*, which is what a top-k consumer reads; the network's internal order
    // is an implementation detail that no consumer can see and that this test
    // deliberately does not pin.
    let cases = [
        [3u64, 1, 4, 1],
        [0, 0, 0, 0],
        [255, 254, 253, 252],
        [7, 7, 7, 7],
        [1, 2, 3, 4],
        [4, 3, 2, 1],
        [0, 255, 0, 255],
        [128, 1, 128, 1],
    ];
    for distances in cases {
        let mut want = distances;
        want.sort_unstable();
        assert_eq!(
            topk_hardware(distances),
            want,
            "the network's output order for {distances:?}"
        );
        let reference = hamming::top_k(&distances, hamming::TOP_K);
        assert_eq!(
            &want[..reference.len()],
            reference.as_slice(),
            "and the two smallest agree with the reference"
        );
    }
}

#[test]
fn the_topk_network_depth_is_the_formula_and_not_just_the_layer_count() {
    // `log2(4) * (log2(4) + 1) / 2 = 3`. The *measured* longest path is checked rather
    // than the layer count, because a network that emitted three layers with the wrong
    // comparators in them would have the right count and the wrong answer.
    let layers = sorting::bitonic_layers(hamming::CANDIDATES);
    assert_eq!(layers.len(), 3);
    assert_eq!(
        sorting::measured_depth(&layers, hamming::CANDIDATES),
        hamming::topk_depth()
    );
    assert_eq!(hamming::topk_depth(), 3);
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_hamming_distance() {
    let design = Design::new();
    let ports = hamming::build(&design).unwrap();
    let plan = plan_of(&design, "hamming_dist");
    assert_eq!(
        plan.inputs
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "a0", "a1", "b0", "b1"],
        "the codewords are two 64-bit ports each, not one 128-bit port: a port wider \
         than 64 bits does not cosimulate"
    );
    let _ = ports;

    let mut stimulus = Stimulus::new();
    let row = |rst: u64, a0: u64, a1: u64, b0: u64, b1: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(a0, 64).unwrap(),
            Bits::constant(a1, 64).unwrap(),
            Bits::constant(b0, 64).unwrap(),
            Bits::constant(b1, 64).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0, 0)).unwrap();
    // Complement pairs reach the maximum distance of 128, and single-bit differences
    // reach the minimum of one, so both ends of the adder tree are exercised.
    for index in 0..32u64 {
        stimulus.push(row(0, u64::MAX >> index, 0, 0, 0)).unwrap();
        stimulus.push(row(0, 0, u64::MAX, u64::MAX, 0)).unwrap();
        stimulus
            .push(row(
                0,
                0x5555_5555_5555_5555,
                0xaaaa_aaaa_aaaa_aaaa,
                index,
                !index,
            ))
            .unwrap();
    }
    assert_eq!(stimulus.cycles(), 1 + 3 * 32);

    cosimulate(&design, &plan, &stimulus, "the hamming distance");
}

// ===================================================================== sorting

/// Runs a sorting network over `values` and returns the eight output lanes.
///
/// `bitonic` picks the generator; the two designs have the same ports and the same
/// depth, so a test that only checked the sorted order would pass for either.
fn sorter_hardware(values: [u64; 8], bitonic: bool) -> [u64; 8] {
    let design = Design::new();
    if bitonic {
        sorting::build(&design).unwrap();
    } else {
        sorting::build_odd_even(&design).unwrap();
    }
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        clock_with(
            &tb,
            &[
                ("rst", 0),
                ("x_0", values[0]),
                ("x_1", values[1]),
                ("x_2", values[2]),
                ("x_3", values[3]),
                ("x_4", values[4]),
                ("x_5", values[5]),
                ("x_6", values[6]),
                ("x_7", values[7]),
            ],
        )
        .await;
        [
            tb.value("y_0"),
            tb.value("y_1"),
            tb.value("y_2"),
            tb.value("y_3"),
            tb.value("y_4"),
            tb.value("y_5"),
            tb.value("y_6"),
            tb.value("y_7"),
        ]
    })
    .expect("the sorting design drives only ports it declares")
}

/// Runs a bitonic sorter over `values` and returns the eight output lanes.
fn bitonic_hardware(values: [u64; 8]) -> [u64; 8] {
    sorter_hardware(values, true)
}

/// Runs an odd-even merge sorter over `values` and returns the eight output lanes.
fn odd_even_hardware(values: [u64; 8]) -> [u64; 8] {
    sorter_hardware(values, false)
}

#[test]
fn both_networks_sort_what_a_sort_sorts() {
    // The design is compared against `slice::sort` on the *output*, because what a
    // consumer of a sorter can see is the sorted lane order. The networks themselves are
    // compared on their comparator count in the test below.
    let cases = [
        [3u64, 1, 4, 1, 5, 9, 2, 6],
        [0, 0, 0, 0, 0, 0, 0, 0],
        [255, 254, 253, 252, 251, 250, 249, 248],
        [7, 7, 7, 7, 0, 0, 0, 0],
        [1, 2, 3, 4, 5, 6, 7, 8],
        [8, 7, 6, 5, 4, 3, 2, 1],
        [128, 0, 255, 1, 127, 2, 254, 3],
        [200, 100, 50, 25, 12, 6, 3, 1],
    ];
    for values in cases {
        let mut want = values;
        want.sort_unstable();
        assert_eq!(bitonic_hardware(values), want, "bitonic on {values:?}");
        assert_eq!(odd_even_hardware(values), want, "odd-even on {values:?}");
    }
}

#[test]
fn a_sorter_output_is_always_sorted_whatever_goes_in() {
    // Forty random vectors, which is enough that a comparator that fires in the wrong
    // direction -- a `rising` flag inverted, say -- would produce an unsorted output
    // within a couple of them, and the assertion is the property rather than a value.
    let mut seed = 0x1234_5678_9abc_def0u64;
    for _ in 0..40 {
        let mut values = [0u64; 8];
        for lane in values.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            // Eight bits, because a lane is an eight-bit port: an unmasked value would
            // be truncated by the port and the reference would then be sorting something
            // the design never saw.
            *lane = (seed >> 24) & 0xff;
        }
        let bitonic = bitonic_hardware(values);
        let odd_even = odd_even_hardware(values);
        assert!(
            bitonic.windows(2).all(|pair| pair[0] <= pair[1]),
            "bitonic output {bitonic:?} is not sorted, from {values:?}"
        );
        assert!(
            odd_even.windows(2).all(|pair| pair[0] <= pair[1]),
            "odd-even output {odd_even:?} is not sorted, from {values:?}"
        );
        assert_eq!(
            bitonic, odd_even,
            "and the two networks agree on {values:?}"
        );
        let mut want = values;
        want.sort_unstable();
        assert_eq!(
            bitonic, want,
            "and both match `slice::sort`, so no value is dropped or duplicated"
        );
    }
}

#[test]
fn both_networks_are_six_layers_deep_for_eight_lanes() {
    // `log2(8) * (log2(8) + 1) / 2 = 3 * 4 / 2 = 6`. The measured longest path is what
    // is asserted, because that is the property the eight lanes of mux logic have; the
    // layer count is a second opinion.
    for (name, layers) in [
        ("bitonic", sorting::bitonic_layers(sorting::DEFAULT_LANES)),
        (
            "odd-even merge",
            sorting::odd_even_merge_layers(sorting::DEFAULT_LANES),
        ),
    ] {
        assert_eq!(
            sorting::measured_depth(&layers, sorting::DEFAULT_LANES),
            6,
            "the {name} network's longest path"
        );
        assert_eq!(layers.len(), 6, "and its layer count");
        assert_eq!(sorting::log2_exact(sorting::DEFAULT_LANES), 3);
        assert!(
            sorting::check_parallel(&layers, sorting::DEFAULT_LANES).is_ok(),
            "every {name} layer must be a parallel set, or the design has a loop in it"
        );
    }
    assert_eq!(sorting::bitonic_depth(sorting::DEFAULT_LANES), 6);
    assert_eq!(sorting::odd_even_merge_depth(sorting::DEFAULT_LANES), 6);
    // Batcher's classic counts for eight lanes: twenty-four comparators for the bitonic
    // sorter and nineteen for the odd-even merge sorter. The difference is the whole
    // reason the second one exists -- same depth, fewer gates.
    assert_eq!(sorting::comparator_count(&sorting::bitonic_layers(8)), 24);
    assert_eq!(
        sorting::comparator_count(&sorting::odd_even_merge_layers(8)),
        19
    );
}

proptest! {
    /// The sorters, on arbitrary lanes.
    #[test]
    fn both_networks_sort_arbitrary_lanes(raw in prop::array::uniform8(any::<u64>())) {
        // Masked to a lane's width, for the reason the seeded generator above is: the
        // port truncates, and a reference that sorts the unmasked values is sorting
        // something the design never received.
        let mut values = raw;
        for lane in values.iter_mut() {
            *lane &= 0xff;
        }
        let mut want = values;
        want.sort_unstable();
        prop_assert_eq!(bitonic_hardware(values), want);
        prop_assert_eq!(odd_even_hardware(values), want);
        prop_assert_eq!(sorting::sorted_by_bitonic(&values), want);
        prop_assert_eq!(sorting::sorted_by_odd_even(&values), want);
    }
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_bitonic_sorter() {
    let design = Design::new();
    let ports = sorting::build(&design).unwrap();
    let plan = plan_of(&design, "bitonic_sort8");
    assert_eq!(
        plan.inputs
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>(),
        [
            "rst", "x_0", "x_1", "x_2", "x_3", "x_4", "x_5", "x_6", "x_7"
        ],
        "a collection port contributes one stimulus column per lane"
    );
    let _ = ports;

    let mut stimulus = Stimulus::new();
    let row = |rst: u64, lanes: [u64; 8]| {
        let mut row = vec![Bits::constant(rst, 1).unwrap()];
        row.extend(lanes.iter().map(|lane| Bits::constant(*lane, 8).unwrap()));
        row
    };
    // A reset row carries every stimulus column, not just the reset bit: the
    // cosimulator's arity check is the first thing that sees a short row.
    stimulus.push(row(1, [0; 8])).unwrap();
    stimulus.push(row(0, [0; 8])).unwrap();
    stimulus.push(row(0, [255; 8])).unwrap();
    let cases = [
        [3u64, 1, 4, 1, 5, 9, 2, 6],
        [255, 0, 128, 127, 1, 254, 2, 253],
        [7, 7, 7, 7, 7, 7, 7, 7],
        [8, 7, 6, 5, 4, 3, 2, 1],
        [0, 255, 0, 255, 0, 255, 0, 255],
    ];
    for lanes in cases {
        stimulus.push(row(0, lanes)).unwrap();
    }
    assert_eq!(stimulus.cycles(), 3 + cases.len());

    cosimulate(&design, &plan, &stimulus, "the bitonic sorter");
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_odd_even_sorter() {
    let design = Design::new();
    let ports = sorting::build_odd_even(&design).unwrap();
    let plan = plan_of(&design, "odd_even_sort8");
    let _ = ports;

    let mut stimulus = Stimulus::new();
    let row = |rst: u64, lanes: [u64; 8]| {
        let mut row = vec![Bits::constant(rst, 1).unwrap()];
        row.extend(lanes.iter().map(|lane| Bits::constant(*lane, 8).unwrap()));
        row
    };
    stimulus.push(row(1, [0; 8])).unwrap();
    stimulus.push(row(0, [0; 8])).unwrap();
    stimulus.push(row(0, [255; 8])).unwrap();
    for index in 0..16u64 {
        stimulus
            .push(row(
                0,
                [
                    index * 37 % 256,
                    (255 - index * 37 % 256),
                    index % 256,
                    0,
                    0,
                    0,
                    0,
                    0,
                ],
            ))
            .unwrap();
    }
    assert_eq!(stimulus.cycles(), 3 + 16);

    cosimulate(&design, &plan, &stimulus, "the odd-even merge sorter");
}
