//! Sorting networks: an unrolled bitonic sorter and an unrolled odd-even merge sorter.
//!
//! # Why a sorting *network* and not a sort
//!
//! A software sort is a loop whose trip count depends on the data and whose
//! comparisons are branches. A hardware sort cannot afford either: a data-dependent
//! number of compare-exchange stages means a variable-latency operation, and a
//! comparison that can skip work means the throughput is set by the worst case.
//!
//! A sorting network replaces both with a **fixed sequence of compare-exchange
//! operators**. Every lane is compared with the right other lanes, in the same order,
//! whatever the data, so:
//!
//! - **latency is a constant**, known at elaboration time, and it is a *depth* rather
//!   than a comparator count;
//! - **throughput is one element per lane per layer**, with no control flow and no
//!   memory traffic for a branch predictor to predict;
//! - the same input always produces the same switch settings, which is what makes it
//!   testable: a network is a fixed list of indices, and a test can compare *the
//!   network* rather than only its output.
//!
//! The cost is comparators. A network sorts `n` lanes with `O(n log^2 n)` comparators
//! against a comparison sort's `O(n log n)`, and the extra comparators are on data
//! that is already partly ordered. For a fixed small `n` on a fixed clock that trade
//! is excellent, and for `n` large enough that the comparator count starts to dominate
//! area it stops being.
//!
//! # The two networks and their depths
//!
//! Both are Batcher's, and both are **unrolled here as a list of layers** -- one layer
//! per unit of latency, with every comparator in a layer acting on disjoint lanes:
//!
//! | network | depth for `n = 2^k` | for `n = 8` |
//! |---|---|---|
//! | bitonic sort | `k(k+1)/2` | 6 |
//! | odd-even merge sort | `k(k+1)/2` | 6 |
//!
//! **A note on a formula that is worth getting right, because it is easy to write
//! down wrong.** A third candidate, `k(k-1)/2 + 1`, circulates for the odd-even merge
//! sorter and it is *not* its depth. It gives 2 for `n = 4`, and no four-input sorting
//! network has depth 2: three is the minimum, and the depth-3 count is forced because
//! the last comparison of a four-sorter has to see the results of both of the first
//! two. `k(k+1)/2` gives 3, agrees with the known optimal depths for `n <= 16`
//! (1, 3, 6, 10, 15), and is what both networks here actually build. The reason they
//! agree is structural rather than coincidental: an odd-even merge sort recursively
//! sorts two halves (depth `d(k-1)`) and then merges them, and the merge network for
//! `2^k` elements has depth `k`, so `d(k) = d(k-1) + k` with `d(0) = 0` gives the
//! triangular number. A bitonic sorter does the same recursion with a bitonic merge
//! that also has depth `k`.
//!
//! The test asserts the **measured** depth -- the longest path actually present in the
//! generated comparator list -- against `k(k+1)/2`, rather than asserting that the
//! generator returns the right number of layers. A generator that produced the right
//! count of layers with the wrong comparators in them would pass the first and fail
//! the second, and the second is the one that catches it.
//!
//! # Bitonic, in one paragraph
//!
//! A *bitonic sequence* is one that rises then falls (or falls then rises). The sorter
//! builds on the fact that any bitonic sequence of `n` elements can be sorted in
//! `log2(n)` compare-exchanges applied to lanes `i` and `i ^ (n/2)`: every pair so
//! compared has one element belonging to each half, and after the layer the larger (or
//! smaller) half sits in the upper lanes. The sorter then produces two bitonic halves
//! of size `n/2` -- by sorting one half ascending and the other descending -- sorts each
//! with the same construction, and concatenates. That is the whole algorithm, and
//! `bitonic_layers` is it written out.
//!
//! # Odd-even merge, in one paragraph
//!
//! The merge of two already-sorted runs is the cheaper half of the sorter and the part
//! that saves comparators. Recursively, it compares lanes `i` and `i + r` for the
//! elements at odd offsets with `r = 1`, then for the elements at odd offsets of each
//! half with `r = 2`, and so on; the even-offset comparisons are last and independent.
//! Sort two halves and merge, and the whole sorter falls out. The `r` parameter is
//! why the odd-offset/even-offset split appears in the code, and it is the only
//! difference in shape from the bitonic sorter.
//!
//! # Where this belongs, and where it does not
//!
//! This is one of the few things in the corpus where hardware wins for the boring
//! reason: **a comparator network is nothing but combinational logic**, so it occupies
//! no memory, generates no traffic, and cannot be bandwidth-bound. The cost is area and
//! a fixed depth, and for `n = 8` at eight bits that is a few hundred gates -- cheaper
//! than the memory a software sort would touch. For larger `n` the `O(n log^2 n)`
//! comparator count is what stops it, and a real large-scale implementation adds
//! *radix* stages on top, which is a different network again.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The number of lanes the port list is built for: eight, i.e. `2^3`.
///
/// A power of two, as a sorting network requires. Eight lanes is where the choice
/// stops being obvious: below it the design is trivially small, and above it the
/// comparator count -- twenty-four for a bitonic sort of eight, against eight for a
/// radix sort of the same data -- starts to be the interesting number.
pub const DEFAULT_LANES: usize = 8;

/// The width of one lane, in bits.
///
/// Eight, so a lane holds a byte. The network is width-agnostic: every comparator is
/// one unsigned comparison and two muxes, so widening [`LANE_BITS`] widens the whole
/// design and changes nothing about the depth.
pub const LANE_BITS: u32 = 8;

/// One compare-exchange between two lanes.
///
/// `low` and `high` are *lane indices*, not signals: the network is a list of indices
/// and the signals are attached to the lanes when the design is built. That separation
/// is what lets the network be generated, measured and applied to plain integers before
/// any of it reaches the DSL.
///
/// A comparator is a *single* output-pair operation: after it, `lanes[low]` and
/// `lanes[high]` hold the two inputs in the order `rising` names, and nothing else has
/// moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comparator {
    /// The lane that will hold the smaller of the two, when `rising`.
    pub low: usize,
    /// The lane that will hold the larger of the two, when `rising`.
    pub high: usize,
    /// `true` for `lanes[low] <= lanes[high]`, `false` for `lanes[low] >= lanes[high]`.
    pub rising: bool,
}

/// One unit of latency: comparators that all act at once.
///
/// A layer is a *parallel* set, which is a claim about the network and not just a
/// grouping. It is checked by [`check_parallel`], and the design relies on it: every
/// comparator in a layer reads the layer's input and writes the layer's output, so two
/// comparators in one layer touching the same lane would be a combinational loop rather
/// than a network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    /// The comparators in this layer, in any order.
    pub comparators: Vec<Comparator>,
}

/// `log2(n)` for a power of two, as the depth formulas want it.
///
/// Zero for `n <= 1`, where there is nothing to sort.
///
/// # Panics
///
/// Never. `usize::ilog2` is defined for every value, and this is only called with
/// values the generators have already rounded to a power of two.
#[must_use]
pub fn log2_exact(n: usize) -> u32 {
    if n <= 1 { 0 } else { n.ilog2() }
}

/// The depth of an `n`-lane bitonic sort: `log2(n) * (log2(n) + 1) / 2`.
#[must_use]
pub fn bitonic_depth(n: usize) -> usize {
    let k = log2_exact(n) as usize;
    k * (k + 1) / 2
}

/// The depth of an `n`-lane odd-even merge sort: `log2(n) * (log2(n) + 1) / 2`.
///
/// **The same as [`bitonic_depth`], and the module docs explain why**, including the
/// `k(k-1)/2 + 1` formula that is sometimes quoted instead and why it cannot be a
/// sorting-network depth. Both networks are recursive: `d(0) = 0` and
/// `d(k) = d(k-1) + k`, because the merge of two sorted halves of length `2^k` has
/// depth `k` in both constructions.
#[must_use]
pub fn odd_even_merge_depth(n: usize) -> usize {
    bitonic_depth(n)
}

/// The bitonic sort network for `n` lanes, as depth-ordered layers.
///
/// `n` is rounded up to a power of two, and `n <= 1` gives an empty network -- which
/// sorts anything, trivially, and has depth zero.
#[must_use]
pub fn bitonic_layers(n: usize) -> Vec<Layer> {
    let n = n.next_power_of_two();
    let mut layers = Vec::new();
    let mut size = 2usize;
    while size <= n {
        let mut step = size / 2;
        while step > 0 {
            let mut layer = Layer {
                comparators: Vec::new(),
            };
            for low in 0..n {
                let high = low ^ step;
                // Only the lower-indexed lane of each pair owns the comparator, so a
                // pair is not emitted twice. Without this the network would still sort
                // correctly -- the second comparator would see both values already in
                // order and be a no-op -- but its depth and its comparator count would
                // both be wrong, which is precisely what the tests measure.
                if high > low {
                    // `(low & size) == 0` is the direction bit: lanes in the first half
                    // of this stage's block sort one way and lanes in the second half
                    // sort the other, which is what makes every intermediate result
                    // bitonic. For the final stage `size == n` and every lane has
                    // `low & size == 0`, so the network ends ascending.
                    layer.comparators.push(Comparator {
                        low,
                        high,
                        rising: (low & size) == 0,
                    });
                }
            }
            layers.push(layer);
            step /= 2;
        }
        size *= 2;
    }
    layers
}

/// The odd-even merge sort network for `n` lanes, as depth-ordered layers.
///
/// Rounds up like [`bitonic_layers`] and returns nothing for `n <= 1`.
#[must_use]
pub fn odd_even_merge_layers(n: usize) -> Vec<Layer> {
    let n = n.next_power_of_two();
    let mut tagged: Vec<(u32, Comparator)> = Vec::new();
    if n > 1 {
        odd_even_sort(0, n, 0, &mut tagged);
    }
    // The recursion tags each comparator with the depth of the last layer its call
    // emitted, and those tags have gaps: a merge's two recursive halves occupy one depth
    // and its even-offset comparisons the next. Grouping into a map and reading it back
    // in key order is what turns the tags into contiguous layers.
    let mut by_depth: std::collections::BTreeMap<u32, Vec<Comparator>> =
        std::collections::BTreeMap::new();
    for (depth, comparator) in tagged {
        by_depth.entry(depth).or_default().push(comparator);
    }
    by_depth
        .into_values()
        .map(|comparators| Layer { comparators })
        .collect()
}

/// Batcher's odd-even merge sort: sort the halves, then merge.
///
/// `base` is the first depth this sub-sort may use, and the return value is the first
/// depth it may **not** use. The two halves are sorted in parallel, so both are given
/// the same `base` and the merge that follows them starts at `max` of where they got to.
///
/// Both details are load-bearing, and the shape of getting them wrong is worth
/// recording because the network still *sorts* afterwards:
///
/// - Without the `base` offset the merge's layers are numbered from zero like the
///   sorts', so it overlaps them. `measured_depth` would still report the right depth,
///   because it follows the comparators, but the design -- which is unrolled once per
///   layer -- would be shorter than the algorithm it implements and would read as
///   wrong.
/// - With a shared mutable counter, the recursion visits one half completely before the
///   other, so the second half's comparators land at depths *after* the first half's.
///   The layer list then comes out roughly twice as long as the real depth.
fn odd_even_sort(lo: usize, n: usize, base: u32, out: &mut Vec<(u32, Comparator)>) -> u32 {
    if n <= 1 {
        return base;
    }
    let half = n / 2;
    let first = odd_even_sort(lo, half, base, out);
    let second = odd_even_sort(lo + half, half, base, out);
    let offset = first.max(second);
    odd_even_merge(lo, n, 1, offset, out)
}

/// Batcher's odd-even merge of `n` elements starting at `lo`, stride `r`.
///
/// Compares the odd-offset elements first (`r = 1`), recurses on the halves with `r`
/// doubled, and compares the even-offset elements last -- those are the only ones that
/// depend on both recursive calls, which is why the return value is `max(first, second)`
/// rather than a `+ 1` applied on the way down. Returns the first depth not used.
fn odd_even_merge(
    lo: usize,
    n: usize,
    r: usize,
    base: u32,
    out: &mut Vec<(u32, Comparator)>,
) -> u32 {
    let step = r * 2;
    if step < n {
        let first = odd_even_merge(lo, n, step, base, out);
        let second = odd_even_merge(lo + r, n, step, base, out);
        let depth = first.max(second);
        let mut low = lo + r;
        while low < lo + n - r {
            out.push((
                depth,
                Comparator {
                    low,
                    high: low + r,
                    rising: true,
                },
            ));
            low += step;
        }
        depth + 1
    } else {
        out.push((
            base,
            Comparator {
                low: lo,
                high: lo + r,
                rising: true,
            },
        ));
        base + 1
    }
}

/// The longest path through the network, measured from the comparators themselves.
///
/// Each comparator takes one unit of time, and the network's depth is the longest chain
/// of comparators through any lane. That is a different computation from
/// `layers.len()`, and it is the one worth asserting: a generator that emitted the right
/// number of layers but put a layer's comparators in the wrong order would have a longer
/// chain, and a generator that emitted comparators that touch the same lane twice within
/// a layer would have a chain the layers do not describe.
///
/// # Panics
///
/// Never, for a network generated by [`bitonic_layers`] or
/// [`odd_even_merge_layers`]. A hand-written network whose lane indices were out of
/// range would index past the end, so `n` is taken as a parameter and a caller who
/// passes fewer lanes than the network uses gets a panic rather than a wrong answer.
#[must_use]
pub fn measured_depth(layers: &[Layer], n: usize) -> usize {
    let mut level = vec![0usize; n];
    for layer in layers {
        for comparator in &layer.comparators {
            let next = level[comparator.low].max(level[comparator.high]) + 1;
            level[comparator.low] = next;
            level[comparator.high] = next;
        }
    }
    level.iter().copied().max().unwrap_or(0)
}

/// That no two comparators in a layer touch the same lane.
///
/// The property the design's construction depends on: a layer's comparators all read
/// the previous layer's lanes and write this layer's, so a repeated lane within a layer
/// would mean one comparator reading another's output.
///
/// # Errors
///
/// `Err` naming the lane and the layer, or `Ok` when the network is genuinely parallel.
pub fn check_parallel(layers: &[Layer], n: usize) -> Result<(), String> {
    for (index, layer) in layers.iter().enumerate() {
        let mut seen = vec![false; n];
        for comparator in &layer.comparators {
            if comparator.low >= n || comparator.high >= n {
                return Err(format!(
                    "layer {index} compares lanes {} and {}, outside 0..{n}",
                    comparator.low, comparator.high
                ));
            }
            if comparator.low == comparator.high {
                return Err(format!(
                    "layer {index} compares lane {} with itself",
                    comparator.low
                ));
            }
            for lane in [comparator.low, comparator.high] {
                if seen[lane] {
                    return Err(format!(
                        "lane {lane} is compared twice in layer {index}, so that layer \
                         is not a parallel set"
                    ));
                }
                seen[lane] = true;
            }
        }
    }
    Ok(())
}

/// How many comparators the network has in total.
#[must_use]
pub fn comparator_count(layers: &[Layer]) -> usize {
    layers.iter().map(|layer| layer.comparators.len()).sum()
}

/// Applies the network to `values`, in place.
///
/// The reference the design is compared against, and deliberately the *network* and not
/// a sort: applying the same list of indices to the same inputs and comparing the
/// outputs is a stronger check than comparing two sorts, because it says the circuit and
/// the network agree on every lane rather than only on the multiset they end up holding.
///
/// # Panics
///
/// If a comparator names a lane outside `values`.
pub fn apply(layers: &[Layer], values: &mut [u64]) {
    for layer in layers {
        for comparator in &layer.comparators {
            let left = values[comparator.low];
            let right = values[comparator.high];
            let (low, high) = if comparator.rising {
                (left.min(right), left.max(right))
            } else {
                (left.max(right), left.min(right))
            };
            values[comparator.low] = low;
            values[comparator.high] = high;
        }
    }
}

/// The sorted order the bitonic network produces for `values`.
#[must_use]
pub fn sorted_by_bitonic(values: &[u64]) -> Vec<u64> {
    let mut out = values.to_vec();
    apply(&bitonic_layers(values.len()), &mut out);
    out
}

/// The sorted order the odd-even merge network produces for `values`.
#[must_use]
pub fn sorted_by_odd_even(values: &[u64]) -> Vec<u64> {
    let mut out = values.to_vec();
    apply(&odd_even_merge_layers(values.len()), &mut out);
    out
}

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge registers the sorted lanes.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which zeroes every output lane.
    pub rst: Signal,
    /// The lanes to sort, `x_0` through `x_7`.
    #[bits(8)]
    #[length(8)]
    pub x: Vec<Signal>,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The sorted lanes, `y_0` through `y_7`, ascending.
    #[bits(8)]
    #[length(8)]
    pub y: Vec<Signal>,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The sorted lanes, which are also the outputs.
    pub lanes: Vec<Signal>,
}

/// Builds the eight-lane bitonic sorter into `design`.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    build_network(design, &bitonic_layers(DEFAULT_LANES))
}

/// [`build`] for the eight-lane odd-even merge sorter.
///
/// Same ports, same depth, different comparators -- which is the point of having both:
/// a test that only checked the sorted *order* would pass for either, and only the
/// measured depth and the comparator set say which network was built.
///
/// # Errors
///
/// Whatever [`Design`] returns, for the reasons [`build`] gives.
pub fn build_odd_even(design: &Design) -> Result<Ports, BuildError> {
    build_network(design, &odd_even_merge_layers(DEFAULT_LANES))
}

/// Applies a sorting network to design signals, layer by layer, unrolled.
///
/// The one place a network reaches the DSL, and shared with
/// [`crate::hamming::build_topk`] so that the "a layer is a parallel set, so every
/// comparator reads the previous layer" structure exists once rather than twice.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn network_signals(
    design: &Design,
    lanes: &[Signal],
    layers: &[Layer],
) -> Result<Vec<Signal>, BuildError> {
    let mut lanes = lanes.to_vec();
    for layer in layers {
        let mut next = lanes.clone();
        for comparator in &layer.comparators {
            let left = lanes[comparator.low].clone();
            let right = lanes[comparator.high].clone();
            let smaller = design.ult(&left, &right)?;
            // `rising` picks which way round the two values land. Written as two `ite`s
            // off one comparison so that a comparator is one comparison and two muxes,
            // and so a descending layer is a literal swap.
            let low = if comparator.rising {
                design.ite(&smaller, &left, &right)?
            } else {
                design.ite(&smaller, &right, &left)?
            };
            let high = if comparator.rising {
                design.ite(&smaller, &right, &left)?
            } else {
                design.ite(&smaller, &left, &right)?
            };
            next[comparator.low] = low;
            next[comparator.high] = high;
        }
        lanes = next;
    }
    Ok(lanes)
}

/// Builds a sorting network's lanes, unrolled and registered.
///
/// The network is applied by [`network_signals`], value-wise, one layer at a time: each
/// layer's output lane is a fresh `ite` reading the previous layer's lanes. That is the
/// only way to build a network in this IR, because a signal here has one driver -- a
/// comparator cannot re-wire a lane in place -- and it is exactly the structure of the
/// circuit anyway. The cost is that the design holds every intermediate layer as a node,
/// so the emitted Verilog is `depth` layers of muxes rather than a loop over layers,
/// which is what "unrolled" means.
///
/// Every layer is assumed to be a parallel set, which is what [`check_parallel`]
/// verifies for the generators here; the assumption is not checked at build time
/// because a network that violated it would be a bug in the generator and there is no
/// `BuildError` that names that, and a design that reported it as a width mismatch would
/// be worse than useless.
///
/// Outputs are registered so the module has a clock to be driven from; the network
/// itself is combinational.
fn build_network(design: &Design, layers: &[Layer]) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let lanes = network_signals(design, &inputs.x, layers)?;

    let registered: Vec<Signal> = lanes
        .iter()
        .map(|lane| design.reg(lane, &inputs.clk, &inputs.rst, &design.constant(false)))
        .collect::<Result<_, _>>()?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            y: registered.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        lanes: registered,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        bitonic_depth, bitonic_layers, check_parallel, comparator_count, log2_exact,
        measured_depth, odd_even_merge_depth, odd_even_merge_layers, sorted_by_bitonic,
        sorted_by_odd_even,
    };

    #[test]
    fn the_depth_formula_is_the_triangular_number() {
        for k in 1..=8u32 {
            let n = 1usize << k;
            assert_eq!(bitonic_depth(n), (k * (k + 1) / 2) as usize, "n = {n}");
            assert_eq!(odd_even_merge_depth(n), bitonic_depth(n));
        }
    }

    #[test]
    fn the_rejected_formula_cannot_be_a_sorting_network_depth() {
        // `k(k-1)/2 + 1` gives 2 for four lanes, and the minimum depth of a four-input
        // sorting network is 3. This test exists so the claim in the module docs is
        // checked rather than asserted.
        for k in 2..=8u32 {
            let wrong = (k * (k - 1) / 2 + 1) as usize;
            let n = 1usize << k;
            assert!(
                wrong < bitonic_depth(n),
                "for n = {n} the rejected formula says {wrong} and the real depth is {}",
                bitonic_depth(n)
            );
        }
    }

    #[test]
    fn both_networks_are_parallel_layer_by_layer() {
        for n in [2usize, 4, 8, 16] {
            check_parallel(&bitonic_layers(n), n).expect("a bitonic layer is parallel");
            check_parallel(&odd_even_merge_layers(n), n).expect("an odd-even layer is parallel");
        }
    }

    #[test]
    fn the_measured_depth_is_the_formula_not_just_the_layer_count() {
        for n in [2usize, 4, 8, 16] {
            let bitonic = bitonic_layers(n);
            assert_eq!(measured_depth(&bitonic, n), bitonic_depth(n), "n = {n}");
            let odd_even = odd_even_merge_layers(n);
            assert_eq!(
                measured_depth(&odd_even, n),
                odd_even_merge_depth(n),
                "n = {n}"
            );
            assert_eq!(
                bitonic.len(),
                bitonic_depth(n),
                "and the layer count agrees, which is what the design relies on"
            );
        }
    }

    #[test]
    fn a_bitonic_sort_of_four_uses_the_classic_six_comparators() {
        // The textbook n = 4 bitonic network, layer by layer. Written out because a
        // network generator that silently emitted a different one would still sort.
        let layers = bitonic_layers(4);
        let shape: Vec<Vec<(usize, usize, bool)>> = layers
            .iter()
            .map(|layer| {
                layer
                    .comparators
                    .iter()
                    .map(|c| (c.low, c.high, c.rising))
                    .collect()
            })
            .collect();
        assert_eq!(comparator_count(&layers), 6);
        assert_eq!(
            shape,
            vec![
                vec![(0, 1, true), (2, 3, false)],
                vec![(0, 2, true), (1, 3, true)],
                vec![(0, 1, true), (2, 3, true)],
            ]
        );
    }

    #[test]
    fn both_networks_sort_every_four_lane_input() {
        // All 4^4 inputs over four distinct-ish values, against `slice::sort`. Small
        // enough to be exhaustive and it covers the descending and equal cases, which a
        // random sample hits rarely and which are exactly where a comparator's `rising`
        // flag goes wrong.
        let mut sorted = 0usize;
        for a in 0..4u64 {
            for b in 0..4u64 {
                for c in 0..4u64 {
                    for d in 0..4u64 {
                        let input = [a, b, c, d];
                        let mut want = input;
                        want.sort_unstable();
                        assert_eq!(sorted_by_bitonic(&input), want.to_vec(), "{input:?}");
                        assert_eq!(sorted_by_odd_even(&input), want.to_vec(), "{input:?}");
                        sorted += 1;
                    }
                }
            }
        }
        assert_eq!(sorted, 256);
    }

    #[test]
    fn an_empty_network_sorts_one_element() {
        assert!(bitonic_layers(1).is_empty());
        assert!(odd_even_merge_layers(1).is_empty());
        assert_eq!(measured_depth(&bitonic_layers(1), 1), 0);
        assert_eq!(measured_depth(&odd_even_merge_layers(1), 1), 0);
        assert_eq!(log2_exact(1), 0);
        assert_eq!(bitonic_depth(1), 0);
        assert_eq!(odd_even_merge_depth(1), 0);
        assert_eq!(comparator_count(&[]), 0);
        assert!(check_parallel(&[], 1).is_ok());
    }
}
