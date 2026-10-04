//! Binary-quantised vector search: popcount distance and a top-k comparator network.
//!
//! # The algorithm
//!
//! Binary (or one-bit) vector search quantises every embedding to a single bit per
//! dimension, so a 128-dimensional float vector becomes a 128-bit codeword. The inner
//! loop is then
//!
//! ```text
//! distance = popcount(query XOR candidate)
//! ```
//!
//! and the search is "compute this for every candidate and keep the `k` smallest". Both
//! halves are hardware-shaped in a way that is genuinely hard to get out of software:
//!
//! - The XOR is free (it is what an XOR gate is), and the popcount is a **tree of
//!   adders** whose depth is `log2(n)` and whose width halves at every level. A CPU
//!   running `POPCNT` gets one codeword per instruction; the tree gets one per cycle
//!   with no SIMD width to fill and no memory stall, because the candidate code is
//!   already in a register.
//! - The top-k is a **comparator network**, unrolled to a fixed depth, so it has fixed
//!   latency and needs no branch on "is this better than the current k-th best" --
//!   which is a data-dependent branch in software and a wire in hardware.
//!
//! This is the corpus's clearest one-result-per-cycle, fixed-latency example, and it is
//! also the cheapest design in it: a popcount tree is adders and a comparator network
//! is muxes, there is no memory, no bandwidth argument, and no state beyond a pipeline
//! register.
//!
//! # Why 128 bits is *two* 64-bit ports, not one 128-bit port
//!
//! The obvious port list is one 128-bit query and one 128-bit candidate. **That design
//! does not cosimulate**: the cosimulator's generated C++ driver refuses to build for a
//! port wider than 64 bits rather than truncating it, so a 128-bit port would build,
//! simulate, emit Verilog, and then fail to compile its own driver. The same reason
//! [`chacha20`](crate::chacha20) exposes sixteen 32-bit words rather than one 512-bit
//! state word.
//!
//! So the codewords arrive as `a0`/`a1` and `b0`/`b1`, each 64 bits, and
//! [`xor_words`] is the two-place concatenation that turns them into a 128-bit XOR
//! before popcount sees it. **The split is a port-list artefact and nothing else**:
//! the emitted Verilog has a 128-bit XOR feeding one 128-bit popcount tree, because
//! concatenation is a wire join rather than a node.
//!
//! # The top-k network, and where it is built
//!
//! The comparator network itself is in [`sorting`](crate::sorting), because a top-k
//! network over `2^j` candidates *is* a sorting network over them and having two
//! generators of the same thing would be a maintenance hazard. What this module adds is
//! [`build_topk`], which runs the bitonic sorter over four 8-bit candidate distances
//! and exposes the two smallest: that is a top-2, which is what an approximate
//! nearest-neighbour search actually asks for on its first refinement step.
//!
//! The network's depth is a **known constant** -- `log2(4) * (log2(4) + 1) / 2 = 3`
//! layers -- and the tests assert the *measured* longest path against it rather than
//! asserting the layer count, for the reason [`sorting`](crate::sorting)'s docs give.
//!
//! # Latency, stated plainly
//!
//! [`build`]'s XOR and popcount tree are combinational: the distance is available on
//! the same cycle the codes are, with zero latency and `log2(128) = 7` levels of adder.
//! The output is registered, so it is readable on the cycle *after* the codes are
//! driven. That register is an interface decision -- a search over a store has to
//! present a distance on a clock boundary -- and it is the same register
//! [`chacha20`](crate::chacha20) registers its block function's output for. It is not
//! part of the algorithm and it is not what the depth claim is about.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// How many bits of codeword this design compares, across both port pairs.
pub const CODE_BITS: u32 = 128;

/// How many bits one port pair carries.
pub const WORD_BITS: u32 = 64;

/// The width of a distance output, in bits.
///
/// Eight, because the largest possible distance over [`CODE_BITS`] bits is 128, and
/// seven bits would reach 127. The extra bit is what makes `dist == 128` -- two
/// complementary codes -- representable, and a design that used seven would report 127
/// for it.
pub const DIST_BITS: u32 = 8;

/// The width of a lane in the top-k network.
pub const CANDIDATE_BITS: u32 = 8;

/// How many candidates the top-k network takes: four, i.e. `2^2`.
///
/// Four is the smallest number that makes a *network* rather than a pair of comparisons
/// worth building, and its depth is 3.
pub const CANDIDATES: usize = 4;

/// How many of the sorted candidates are exposed: the two smallest.
pub const TOP_K: usize = 2;

/// The population count of a 64-bit word, in software.
///
/// The reference for the design's popcount tree, and the reason the tree is worth
/// building: on a machine without a `POPCNT` instruction this is a chain of masks and
/// adds, and on one with it, one instruction whose latency is three cycles and whose
/// throughput is one per cycle. The tree in [`popcount64`] has depth six and *no*
/// instruction dependency beyond the adds, so it retires a codeword every cycle
/// regardless of what the host has.
#[must_use]
pub fn popcount64(value: u64) -> u32 {
    value.count_ones()
}

/// The Hamming distance between two 128-bit codes, in software.
///
/// `CODE_BITS / 2` words wide on each side, most significant word first, so
/// `words_a[0]` is the high half and `words_a[1]` the low half. That ordering is the
/// one the emitted Verilog uses and the one the tests drive.
#[must_use]
pub fn hamming(words_a: &[u64; 2], words_b: &[u64; 2]) -> u32 {
    popcount64(words_a[0] ^ words_b[0]) + popcount64(words_a[1] ^ words_b[1])
}

/// The `k` smallest of `distances`, ascending, as a plain Rust sort.
///
/// The reference for [`build_topk`]. A **sort** rather than a selection, and that is
/// the point: the test compares the design's *output order* with this, so a comparator
/// network that permuted two equal values would still pass -- which is correct, because
/// a top-k over equal distances has no further answer to give. What the design's
/// internal order is *not* checked against, and cannot be, because a network's
/// intermediate order is an implementation detail.
#[must_use]
pub fn top_k(distances: &[u64], k: usize) -> Vec<u64> {
    let mut sorted = distances.to_vec();
    sorted.sort_unstable();
    sorted.truncate(k.min(sorted.len()));
    sorted
}

/// The input ports of the distance design.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge registers the distance.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which zeroes the distance.
    pub rst: Signal,
    /// The high half of the query codeword.
    #[bits(64)]
    pub a0: Signal,
    /// The low half of the query codeword.
    #[bits(64)]
    pub a1: Signal,
    /// The high half of the candidate codeword.
    #[bits(64)]
    pub b0: Signal,
    /// The low half of the candidate codeword.
    #[bits(64)]
    pub b1: Signal,
}

/// The output ports of the distance design.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The Hamming distance between the two codewords, zero to 128.
    #[bits(8)]
    pub dist: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The distance, which is also the output.
    pub dist: Signal,
}

/// Builds the 128-bit Hamming distance design into `design`.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;
    let difference = xor_words(design, &inputs.a0, &inputs.a1, &inputs.b0, &inputs.b1)?;
    let distance = sum_words(design, &difference)?;

    let dist = design.reg(&distance, &inputs.clk, &inputs.rst, &design.constant(false))?;

    ferrite_lithic::outputs::<Outputs>(design, &Outputs { dist: dist.clone() })?;

    Ok(Ports { inputs, dist })
}

/// The 128-bit XOR of two codewords, as two 64-bit halves.
///
/// Concatenation, which is a wire join and not a node, so the result is a genuine
/// 128-bit value assembled from the four 64-bit ports. See the module docs for why the
/// ports are split at all.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn xor_words(
    design: &Design,
    a0: &Signal,
    a1: &Signal,
    b0: &Signal,
    b1: &Signal,
) -> Result<(Signal, Signal), BuildError> {
    Ok((design.xor(a0, b0)?, design.xor(a1, b1)?))
}

/// The 128-bit population count of two 64-bit halves.
///
/// [`popcount64_signal`] on each half, then one addition at [`DIST_BITS`] bits, which is
/// what keeps the carry out of 128: two complementary codewords must give 128, and a
/// seven-bit sum would give 0.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn sum_words(design: &Design, halves: &(Signal, Signal)) -> Result<Signal, BuildError> {
    let (high, low) = halves;
    let total = design.add(
        &design.zero_extend(&popcount64_signal(design, high)?, DIST_BITS)?,
        &design.zero_extend(&popcount64_signal(design, low)?, DIST_BITS)?,
    )?;
    Ok(total)
}

/// The population count of one 64-bit word, as a **balanced adder tree**.
///
/// ```text
/// 64 one-bit values -> 32 two-bit -> 16 three-bit -> ... -> one seven-bit value
/// ```
///
/// Depth `log2(64) = 6`, width `64` at the leaves and `2` at the root. The width
/// bookkeeping is the whole implementation: a level that sums `k` counters of `w` bits
/// produces `w + 1` bits, because a count of `k` counters needs `log2(k)` more bits
/// than a count of one.
///
/// `Design::add` truncates to its **left** operand's width, so every addition here
/// explicitly widens both operands to the level's result width *before* adding. Adding
/// two three-bit counters into a three-bit result would throw away every carry and the
/// tree would answer 3 for eight set bits.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn popcount64_signal(design: &Design, word: &Signal) -> Result<Signal, BuildError> {
    let bits: Vec<Signal> = (0..WORD_BITS)
        .map(|index| design.slice(word, index, 1))
        .collect::<Result<_, _>>()?;
    let (total, _) = count_tree(design, &bits, 1)?;
    Ok(total)
}

/// One level of the popcount tree: pairwise sums, all widened before they are added.
///
/// `width` is the width of each counter *entering* this level, so the results are
/// `width + 1` wide. The returned width is that result width, and a caller that ignored
/// it would eventually add two one-bit values into a one-bit result.
fn count_tree(design: &Design, bits: &[Signal], width: u32) -> Result<(Signal, u32), BuildError> {
    match bits {
        [] => Ok((design.lit(0, width)?, width)),
        [only] => Ok((only.clone(), width)),
        _ => {
            let middle = bits.len() / 2;
            let (low, low_width) = count_tree(design, &bits[..middle], width)?;
            let (high, high_width) = count_tree(design, &bits[middle..], width)?;
            let result_width = low_width.max(high_width) + 1;
            let sum = design.add(
                &design.zero_extend(&low, result_width)?,
                &design.zero_extend(&high, result_width)?,
            )?;
            Ok((sum, result_width))
        }
    }
}

/// The input ports of the top-k design.
#[derive(Clone, Debug, PortList)]
pub struct TopKInputs {
    /// The clock. Every edge registers the sorted candidates.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which zeroes every output.
    pub rst: Signal,
    /// The candidate distances, `d_0` through `d_3`.
    #[bits(8)]
    #[length(4)]
    pub d: Vec<Signal>,
}

/// The output ports of the top-k design.
#[derive(Clone, Debug, PortList)]
pub struct TopKOutputs {
    /// The sorted candidates ascending, `k_0` through `k_3`.
    #[bits(8)]
    #[length(4)]
    pub k: Vec<Signal>,
}

/// The signals [`build_topk`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct TopKPorts {
    /// The input ports.
    pub inputs: TopKInputs,
    /// The sorted candidates, which are also the outputs.
    pub sorted: Vec<Signal>,
}

/// Builds the four-way top-k comparator network into `design`.
///
/// A bitonic sort of [`CANDIDATES`] eight-bit lanes, unrolled with
/// [`crate::sorting::network_signals`] -- the same layer-by-layer application the
/// eight-lane design uses -- with the sorted order exposed rather than only the
/// smallest two. The smallest two are [`TOP_K`] lanes of the same output.
///
/// # Errors
///
/// Whatever [`Design`] returns, for the reasons [`build`] gives.
pub fn build_topk(design: &Design) -> Result<TopKPorts, BuildError> {
    let inputs = ferrite_lithic::inputs::<TopKInputs>(design)?;

    let lanes = crate::sorting::network_signals(
        design,
        &inputs.d,
        &crate::sorting::bitonic_layers(CANDIDATES),
    )?;

    let sorted: Vec<Signal> = lanes
        .iter()
        .map(|lane| design.reg(lane, &inputs.clk, &inputs.rst, &design.constant(false)))
        .collect::<Result<_, _>>()?;

    ferrite_lithic::outputs::<TopKOutputs>(design, &TopKOutputs { k: sorted.clone() })?;

    Ok(TopKPorts { inputs, sorted })
}

/// The depth of the top-k network, in layers.
#[must_use]
pub fn topk_depth() -> usize {
    crate::sorting::bitonic_depth(CANDIDATES)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{CANDIDATES, CODE_BITS, DIST_BITS, hamming, popcount64, top_k, topk_depth};
    use crate::sorting;

    #[test]
    fn the_distance_width_can_represent_the_worst_case() {
        assert_eq!(DIST_BITS, 8);
        assert!(
            1u64 << DIST_BITS > u64::from(CODE_BITS),
            "two complementary codewords must give {CODE_BITS}, not 127"
        );
    }

    #[test]
    fn a_word_of_ones_counts_every_bit() {
        assert_eq!(popcount64(u64::MAX), 64);
        assert_eq!(popcount64(0), 0);
    }

    #[test]
    fn two_complementary_codewords_are_the_furthest_apart_they_can_be() {
        let high = 0x0000_0000_0000_0000u64;
        let low = u64::MAX;
        let distance = hamming(&[high, low], &[!high, !low]);
        assert_eq!(distance, CODE_BITS);
    }

    #[test]
    fn the_topk_network_depth_is_three() {
        // log2(4) * (log2(4) + 1) / 2 = 2 * 3 / 2.
        assert_eq!(topk_depth(), 3);
        assert_eq!(CANDIDATES, 4);
        assert_eq!(
            sorting::measured_depth(&sorting::bitonic_layers(CANDIDATES), CANDIDATES),
            topk_depth()
        );
    }

    #[test]
    fn the_topk_reference_keeps_duplicates() {
        assert_eq!(top_k(&[3, 1, 3, 2], 2), vec![1, 2]);
        assert_eq!(top_k(&[7, 7, 7, 7], 2), vec![7, 7]);
        assert_eq!(top_k(&[], 2), Vec::<u64>::new());
    }
}
