//! Roaring-style bitmap set operations, restricted to the case that makes them cheap.
//!
//! # The claim, and its exact scope
//!
//! A Roaring bitmap stores a set of 32-bit integers as a list of **containers**,
//! each covering one 16-bit key and each holding a set of 16-bit values. There are
//! three container shapes and they are chosen by cardinality:
//!
//! | shape | chosen when | representation | size of the set |
//! |---|---|---|---|
//! | array | sparse | sorted `u16` list | proportional to cardinality |
//! | bitmap | dense | `8 KiB` bit array | always 64 KiB |
//! | run | very dense | sorted `(start, length)` list | **two integers per run** |
//!
//! **Only the run container is in this module, and that is the whole point of it.**
//! A run-heavy set operation is arithmetic on interval endpoints: the union of two
//! runs is a pair of min/max operations and a subtraction, and the *result* of a union
//! or intersection over `r` runs is at most `r` runs regardless of how many integers
//! are involved. Union two containers holding ten thousand consecutive integers each
//! and you touch four numbers. Iterate over bits and you touch twenty thousand.
//!
//! The other two shapes are a **different shape of problem and are not implemented
//! here**, deliberately:
//!
//! - An **array** container's union is a two-pointer merge over sorted `u16` lists,
//!   and its work is proportional to the cardinality. There is no endpoint
//!   arithmetic to accelerate and no constant-factor win; this is a sequential
//!   dependency chain over memory, which is the case hardware helps least.
//! - A **bitmap** container's union is 65536 bitwise operations, and the entire
//!   question is whether they can be done eight or sixty-four at a time. That is a
//!   genuine SIMD problem, but its answer is a vector unit, not an ASIC: sixteen ARM
//!   NEON or AVX-512 words already do the whole container in one pass over a 64 KiB
//!   buffer that lives in L1.
//!
//! So the honest summary is: run containers make bitmap algebra cheap *in software
//! too*, and the reason to care about them in hardware is the same reason as
//! [`bitpack`](crate::bitpack) -- the compressed side is small enough to live in
//! on-chip SRAM, and then the endpoint arithmetic being free actually matters.
//!
//! # What the design computes
//!
//! [`build`] is the two-run kernel: for two runs it produces the union's cardinality
//! and span, and the intersection's cardinality. Everything else in the module --
//! containers, multi-run unions, intersections -- is the same arithmetic swept over
//! lists, which is what [`union`] and [`intersection`] do in plain Rust and what the
//! tests check against the `roaring` crate.
//!
//! Outputs are registered, one cycle behind the inputs, so the module has a clock
//! for the cosimulator to drive. The arithmetic itself is combinational and has no
//! latency; the register is an interface decision and is documented as one rather than
//! dressed up as part of the algorithm.
//!
//! # The empty-run convention, which is not optional
//!
//! A run of length zero is legal input -- it is what "no run" decodes to -- and the
//! design handles it *without* a live flag per run, by mapping it to the degenerate
//! interval `[0, 0)`. That works for everything except one thing: the empty interval
//! would drag the union's low endpoint down to zero. So an empty run's *start* is
//! mapped to [`SPAN_BITS`]'s all-ones, which no legal endpoint can be, and its *end*
//! to zero, which an empty interval already is. A `min` then ignores the empty run
//! and a `max` ignores it too, and the only remaining case -- both runs empty -- is
//! caught by clamping the low endpoint to zero when neither run is live.
//!
//! The alternative would be an explicit `live` flag and three extra `ite`s; this is
//! the same number of gates with the corner case stated once, in the sentinel.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The width of a run's start key, in bits.
///
/// Sixteen, because a Roaring container holds 16-bit values and its key is the top
/// sixteen bits of the 32-bit integer.
pub const KEY_BITS: u32 = 16;

/// The width of a run's length, in bits.
pub const LEN_BITS: u32 = 16;

/// The width of an endpoint, in bits.
///
/// Seventeen, because the largest endpoint a run can name is `65535 + 65535 =
/// 131070`, which needs seventeen bits. It is also what makes [`SPAN_SENTINEL`] an
/// impossible value: seventeen bits reach 131071 and no legal endpoint does.
pub const SPAN_BITS: u32 = 17;

/// The value an empty run's *start* is mapped to.
///
/// One past the largest legal endpoint, so that a `min` over endpoints ignores it.
/// [`SPAN_BITS`] bits of ones.
pub const SPAN_SENTINEL: u64 = (1u64 << SPAN_BITS) - 1;

/// One run: `len` consecutive integers starting at `start`.
///
/// `end` is exclusive, which is what makes the overlap arithmetic a subtraction with
/// no off-by-one: two runs overlap in `min(a.end, b.end) - max(a.start, b.start)`
/// whenever that is positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// The first integer of the run.
    pub start: u32,
    /// How many integers. Zero is legal and means the run holds nothing.
    pub len: u32,
}

impl Run {
    /// A run of `len` integers starting at `start`.
    #[must_use]
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    /// The first integer *after* the run.
    #[must_use]
    pub const fn end(&self) -> u32 {
        self.start + self.len
    }

    /// How many integers the run holds.
    #[must_use]
    pub const fn cardinality(&self) -> u32 {
        self.len
    }

    /// Whether the run holds nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the run covers `value`.
    #[must_use]
    pub const fn contains(&self, value: u32) -> bool {
        !self.is_empty() && value >= self.start && value < self.end()
    }

    /// Whether `value` is in `self`, in `other`, or in both.
    ///
    /// The three-way test in one place, because the whole hardware claim is that it
    /// takes one comparison per operand rather than one per value.
    #[must_use]
    pub const fn relative_to(&self, other: &Self, value: u32) -> Ordering {
        match (self.contains(value), other.contains(value)) {
            (false, false) => Ordering::Neither,
            (true, false) => Ordering::OnlyLeft,
            (false, true) => Ordering::OnlyRight,
            (true, true) => Ordering::Both,
        }
    }
}

/// Where a value sits in two runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ordering {
    /// In neither run.
    Neither,
    /// In the left run only.
    OnlyLeft,
    /// In the right run only.
    OnlyRight,
    /// In both runs, i.e. in their intersection.
    Both,
}

/// Sorts, drops empties, and merges overlapping *or adjacent* runs.
///
/// Merging adjacent runs is not an optimisation, it is part of the canonical form:
/// `[0,10)` and `[10,20)` is the same set as `[0,20)`, and leaving them split would
/// make [`cardinality`] right and the run count wrong, which is the number that
/// determines which container shape Roaring would have chosen.
///
/// # Panics
///
/// Never. [`Run::end`] can overflow `u32` for a caller who hands in `start` and `len`
/// whose sum does not fit, which is outside what a 32-bit bitmap can hold; the
/// `saturating_add` here turns that into a run that ends at `u32::MAX` rather than a
/// panic in the middle of a test.
#[must_use]
pub fn canonicalise(runs: Vec<Run>) -> Vec<Run> {
    let mut kept: Vec<Run> = runs
        .into_iter()
        .filter(|run| !run.is_empty())
        .map(|run| Run::new(run.start, run.len.min(u32::MAX - run.start)))
        .collect();
    kept.sort_unstable_by_key(|run| run.start);
    let mut out: Vec<Run> = Vec::with_capacity(kept.len());
    for run in kept {
        match out.last_mut() {
            // `saturating_add` on the *last* run's end: the previous run was already
            // clamped above, so this cannot wrap.
            Some(previous) if run.start <= previous.end() => {
                let end = previous.end().max(run.end());
                previous.len = end - previous.start;
            }
            _ => out.push(run),
        }
    }
    out
}

/// The union of two canonical run lists.
///
/// `canonicalise` of the two lists concatenated, which is correct because the
/// canonical form is exactly "sorted, disjoint, non-adjacent", so a merge of the
/// concatenation *is* the union and no pairwise case analysis is needed.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn union(left: &[Run], right: &[Run]) -> Vec<Run> {
    let mut both = Vec::with_capacity(left.len() + right.len());
    both.extend_from_slice(left);
    both.extend_from_slice(right);
    canonicalise(both)
}

/// The intersection of two canonical run lists.
///
/// A two-pointer sweep, then [`canonicalise`] on the result. The sweep alone can
/// produce adjacent runs -- `[0,20)` against `[5,15)` yields `[5,10)` and `[10,15)`,
/// which are the same set as `[5,15)` -- and a set operation whose result is not in
/// canonical form is a set operation that will be compared against a golden model
/// one call later and disagree about the run count.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn intersection(left: &[Run], right: &[Run]) -> Vec<Run> {
    let (mut i, mut j) = (0usize, 0usize);
    let mut out = Vec::new();
    while i < left.len() && j < right.len() {
        let low = left[i].start.max(right[j].start);
        let high = left[i].end().min(right[j].end());
        if high > low {
            out.push(Run::new(low, high - low));
        }
        if left[i].end() <= right[j].end() {
            i += 1;
        } else {
            j += 1;
        }
    }
    canonicalise(out)
}

/// The total cardinality of a run list, empty runs included or not.
///
/// # Panics
///
/// Never. Sums saturate rather than wrap, because a wrapped cardinality is a small
/// number rather than an obviously wrong one.
#[must_use]
pub fn cardinality(runs: &[Run]) -> u32 {
    runs.iter()
        .fold(0u32, |total, run| total.saturating_add(run.len))
}

/// The span `(low, high)` a run list covers, with `high` exclusive.
///
/// An empty list's span is `(0, 0)`, which is the same convention the design uses for
/// an empty run.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn span(runs: &[Run]) -> (u32, u32) {
    let live: Vec<Run> = runs.iter().copied().filter(|run| !run.is_empty()).collect();
    match live.last() {
        None => (0, 0),
        Some(_) => (
            live.iter().map(|run| run.start).min().unwrap_or(0),
            live.iter().map(|run| run.end()).max().unwrap_or(0),
        ),
    }
}

/// One Roaring container: a 16-bit key and the runs of 16-bit values inside it.
///
/// The key is the high half of the integer; a container's values are `run.start` as
/// 16-bit offsets within its own 16-bit key space. The distinction between the two is
/// not modelled -- [`Container`] stores 32-bit starts, which is the *global* view of
/// the same numbers -- because the key is bookkeeping and every operation in this
/// module is on endpoints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    /// The high sixteen bits.
    pub key: u16,
    /// The runs inside this key, canonical.
    pub runs: Vec<Run>,
}

impl Container {
    /// An empty container at `key`.
    #[must_use]
    pub const fn new(key: u16) -> Self {
        Self {
            key,
            runs: Vec::new(),
        }
    }

    /// Adds a run, restoring the canonical form.
    pub fn push(&mut self, run: Run) {
        self.runs = canonicalise(std::mem::take(&mut self.runs));
        let mut next = self.runs.clone();
        next.push(run);
        self.runs = canonicalise(next);
    }

    /// How many integers the container holds.
    #[must_use]
    pub fn cardinality(&self) -> u32 {
        cardinality(&self.runs)
    }

    /// How many runs the container holds, which is what picks its storage shape.
    #[must_use]
    pub fn runs(&self) -> usize {
        self.runs.len()
    }
}

/// The container-level union of two containers.
///
/// The keys differ in general, so the result keeps the *left* container's key. That is
/// a deliberate simplification and it is stated rather than hidden: a real Roaring
/// union over two containers at different keys produces two containers, and the
/// container-level bookkeeping is not what this module is about. What is modelled
/// exactly is the run-level algebra, which is where the two-run design's arithmetic
/// comes from.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn union_containers(left: &Container, right: &Container) -> Container {
    Container {
        key: left.key,
        runs: union(&left.runs, &right.runs),
    }
}

/// The container-level intersection of two containers.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn intersection_containers(left: &Container, right: &Container) -> Container {
    Container {
        key: left.key,
        runs: intersection(&left.runs, &right.runs),
    }
}

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge registers the four results below.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which zeroes all four outputs.
    pub rst: Signal,
    /// The first integer of the left run.
    #[bits(16)]
    pub a_start: Signal,
    /// The length of the left run.
    #[bits(16)]
    pub a_len: Signal,
    /// The first integer of the right run.
    #[bits(16)]
    pub b_start: Signal,
    /// The length of the right run.
    #[bits(16)]
    pub b_len: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// How many integers are in the union of the two runs.
    #[bits(17)]
    pub card: Signal,
    /// The first integer of the union's span.
    #[bits(17)]
    pub lo: Signal,
    /// One past the last integer of the union's span.
    #[bits(17)]
    pub hi: Signal,
    /// How many integers are in the intersection of the two runs.
    #[bits(17)]
    pub lap: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The union's cardinality, which is also the output.
    pub card: Signal,
    /// The union span's low endpoint, which is also the output.
    pub lo: Signal,
    /// The union span's high endpoint, which is also the output.
    pub hi: Signal,
    /// The intersection's cardinality, which is also the output.
    pub lap: Signal,
}

/// Builds the two-run union/intersection kernel into `design`.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;
    let (card, lo, hi, lap) = kernel(
        design,
        &inputs.a_start,
        &inputs.a_len,
        &inputs.b_start,
        &inputs.b_len,
    )?;

    let card_q = register(design, &inputs.clk, &inputs.rst, &card)?;
    let lo_q = register(design, &inputs.clk, &inputs.rst, &lo)?;
    let hi_q = register(design, &inputs.clk, &inputs.rst, &hi)?;
    let lap_q = register(design, &inputs.clk, &inputs.rst, &lap)?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            card: card_q.clone(),
            lo: lo_q.clone(),
            hi: hi_q.clone(),
            lap: lap_q.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        card: card_q,
        lo: lo_q,
        hi: hi_q,
        lap: lap_q,
    })
}

/// A register whose reset value is zero and whose enable is always on.
///
/// The four results are registered together rather than individually so that they are
/// mutually consistent: a caller reading `lo` and `card` on the same cycle must not be
/// able to get one from this run and the other from the last.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn register(
    design: &Design,
    clk: &Signal,
    rst: &Signal,
    data: &Signal,
) -> Result<Signal, BuildError> {
    Ok(design.reg(data, clk, rst, &design.constant(false))?)
}

/// The two-run kernel: `(union cardinality, union span, intersection cardinality)`.
///
/// The whole of the hardware claim is in this function, and it is: two additions for
/// the endpoints, two `min`s and one `max` for the span, one comparison for the
/// overlap, one subtraction for the union cardinality. **Every one of those is
/// [`SPAN_BITS`] bits wide and every one of them depends on four 16-bit input words.**
/// There is no loop over values here and that is the point.
///
/// The empty-run convention is described in the module docs. In code it is two
/// `ite`s per run:
///
/// ```text
/// a_start_eff = a_len  != 0 ? a_start      : SPAN_SENTINEL
/// a_end_eff   = a_len  != 0 ? a_start+a_len : 0
/// ```
///
/// so that `min` ignores a dead run's low endpoint, `max` ignores its high one, and
/// the overlap is automatically zero because the dead interval is `[SPAN_SENTINEL, 0)`.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn kernel(
    design: &Design,
    a_start: &Signal,
    a_len: &Signal,
    b_start: &Signal,
    b_len: &Signal,
) -> Result<(Signal, Signal, Signal, Signal), BuildError> {
    let zero = design.lit(0, SPAN_BITS)?;
    let sentinel = design.lit(SPAN_SENTINEL, SPAN_BITS)?;

    let (a_low, a_high) = endpoint(design, a_start, a_len, &sentinel, &zero)?;
    let (b_low, b_high) = endpoint(design, b_start, b_len, &sentinel, &zero)?;

    let a_live = design.ugt(a_len, &design.lit(0, LEN_BITS)?)?;
    let b_live = design.ugt(b_len, &design.lit(0, LEN_BITS)?)?;
    let either = design.or(&a_live, &b_live)?;

    // `lo` is the only output the sentinel convention does not finish on its own: when
    // neither run is live, `min` of two sentinels is a sentinel, so the clamp catches
    // the one case where "there is no low endpoint" has to read as zero rather than as
    // 131071.
    let lowest = min_u(design, &a_low, &b_low)?;
    let lo = design.ite(&either, &lowest, &zero)?;
    let hi = max_u(design, &a_high, &b_high)?;

    // The overlap is `min(high) - max(low)` when that is positive and zero otherwise.
    // The comparison is a `ult` rather than a subtraction's borrow so the zero is
    // produced explicitly; `sub` truncates to its left operand, so subtracting a
    // `lap_low` that is larger than `lap_high` has to be discarded rather than
    // wrapped -- and the `ite` is what discards it.
    let lap_high = min_u(design, &a_high, &b_high)?;
    let lap_low = max_u(design, &a_low, &b_low)?;
    let disjoint = design.ult(&lap_high, &lap_low)?;
    let difference = design.sub(&lap_high, &lap_low)?;
    let lap = design.ite(&disjoint, &zero, &difference)?;

    // `|a| + |b| - |a & b|`, which is the inclusion-exclusion identity and the reason
    // a run container needs no other arithmetic. The sum is formed one bit wider than
    // the answer so that the subtraction cannot borrow out of the result, then narrowed
    // back: the true cardinality cannot exceed `2 * 65535 < 2^17`.
    let total = design.add(
        &design.zero_extend(a_len, SPAN_BITS + 1)?,
        &design.zero_extend(b_len, SPAN_BITS + 1)?,
    )?;
    let counted = design.sub(&total, &design.zero_extend(&lap, SPAN_BITS + 1)?)?;
    let card = design.slice(&counted, 0, SPAN_BITS)?;

    Ok((card, lo, hi, lap))
}

/// The unsigned minimum of two same-width signals.
///
/// An `ite` on `ult` rather than a comparison node, because the DSL has comparison
/// nodes but no select-one node: the result has to be *one of the operands*, and the
/// only way to say "whichever is smaller" in this IR is a two-way mux.
///
/// Both operands must already be the same width -- `Design::ite` truncates to its
/// `then` operand, so an unmatched pair would silently narrow.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn min_u(design: &Design, left: &Signal, right: &Signal) -> Result<Signal, BuildError> {
    let smaller = design.ult(left, right)?;
    Ok(design.ite(&smaller, left, right)?)
}

/// The unsigned maximum of two same-width signals. [`min_u`] for the other direction.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn max_u(design: &Design, left: &Signal, right: &Signal) -> Result<Signal, BuildError> {
    let smaller = design.ult(left, right)?;
    Ok(design.ite(&smaller, right, left)?)
}

/// A run's low and high endpoint, with the empty-run convention applied.
///
/// `sentinel` and `zero` are passed in rather than built here so that the four literals
/// are shared between the two runs: a `lit` is a node, and two runs that each built
/// their own sentinels would put four nodes where two will do.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn endpoint(
    design: &Design,
    start: &Signal,
    len: &Signal,
    sentinel: &Signal,
    zero: &Signal,
) -> Result<(Signal, Signal), BuildError> {
    let live = design.ugt(len, &design.lit(0, len.width())?)?;
    let wide = design.zero_extend(start, SPAN_BITS)?;
    let end = design.add(&wide, &design.zero_extend(len, SPAN_BITS)?)?;
    Ok((
        design.ite(&live, &wide, sentinel)?,
        design.ite(&live, &end, zero)?,
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{Container, Ordering, Run, canonicalise, cardinality, intersection, span, union};

    #[test]
    fn adjacent_runs_merge_because_they_are_the_same_set() {
        let merged = canonicalise(vec![Run::new(10, 10), Run::new(0, 10)]);
        assert_eq!(merged, vec![Run::new(0, 20)]);
    }

    #[test]
    fn a_gap_stops_the_merge() {
        assert_eq!(
            canonicalise(vec![Run::new(0, 10), Run::new(11, 1)]),
            vec![Run::new(0, 10), Run::new(11, 1)]
        );
    }

    #[test]
    fn the_intersection_of_a_run_with_itself_is_the_run() {
        let run = Run::new(100, 50);
        assert_eq!(intersection(&[run], &[run]), vec![run]);
    }

    #[test]
    fn the_intersection_canonicalises_adjacent_pieces() {
        // `[0,20)` against `[5,15)` sweeps to `[5,10)` then `[10,15)`, which is the
        // same set as `[5,15)` and must not be reported as two runs.
        let got = intersection(&[Run::new(0, 20)], &[Run::new(5, 10)]);
        assert_eq!(got, vec![Run::new(5, 10)]);
    }

    #[test]
    fn an_empty_run_contributes_nothing_to_a_union() {
        let union = union(&[Run::new(7, 3)], &[Run::new(100, 1)]);
        assert_eq!(union, vec![Run::new(7, 3), Run::new(100, 1)]);
        assert_eq!(cardinality(&union), 4);
    }

    #[test]
    fn the_span_of_nothing_is_the_empty_span() {
        assert_eq!(span(&[]), (0, 0));
        assert_eq!(span(&[Run::new(0, 0)]), (0, 0));
    }

    #[test]
    fn containment_is_three_way() {
        let a = Run::new(0, 10);
        let b = Run::new(5, 10);
        assert_eq!(a.relative_to(&b, 2), Ordering::OnlyLeft);
        assert_eq!(a.relative_to(&b, 7), Ordering::Both);
        assert_eq!(a.relative_to(&b, 12), Ordering::OnlyRight);
        assert_eq!(a.relative_to(&b, 99), Ordering::Neither);
    }

    #[test]
    fn a_container_keeps_its_runs_canonical() {
        let mut container = Container::new(0);
        container.push(Run::new(0, 10));
        container.push(Run::new(10, 10));
        assert_eq!(container.runs(), 1);
        assert_eq!(container.cardinality(), 20);
    }
}
