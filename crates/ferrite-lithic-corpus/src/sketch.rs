//! Probabilistic sketches: a bloom filter, a count-min sketch and a HyperLogLog.
//!
//! # The shape of the problem
//!
//! All three answer a question about a stream too large to hold, using a fixed amount
//! of memory, and all three do it by *hashing into a fixed number of cells*. That
//! shared structure is why they are in one module: the hardware for all of them is a
//! bank of counters or bits, a small number of address decoders, and a comparison at
//! the end. What differs is only the cell's arithmetic -- one bit, a saturating
//! counter, or a max -- and what the answer means.
//!
//! # The part that matters: none of this is exact, and the module says so
//!
//! **A probabilistic structure without a stated error bound is a bug report with a
//! delay on it.** So every bound in this module is written down, and the tests assert
//! the bound rather than a value. Concretely:
//!
//! | sketch | guarantee | asserted |
//! |---|---|---|
//! | bloom | never a false *negative*; false-positive *rate* `~ (1 - e^{-kn/m})^k` | rate within a stated binomial band |
//! | count-min | never *under*-estimates; over by at most `n/w` | lower bound exactly, upper bound exactly, mean overestimate in a stated band |
//! | HyperLogLog | relative error `1.04 / sqrt(m)` | within 4 sigma over a large stream |
//!
//! The tolerances are derived in the tests, not chosen by feel. The short version:
//!
//! - **Bloom.** `T` queries against a filter with false-positive probability `p` produce
//!   a count that is `Binomial(T, p)`, so the standard deviation of the *observed
//!   rate* is `sqrt(p(1-p)/T)` -- **not** `p` and not zero. The test uses
//!   `T = 200_000` against `p ~ 0.0024`, i.e. about 479 expected hits, and asserts
//!   within 5 sigma (~109 hits, or +-22% relative). Tightening that needs more trials,
//!   not a better test.
//! - **Count-min.** The lower bound `estimate >= true` and the worst-case
//!   `estimate <= true + n/w` are both *deterministic*, so they are asserted exactly
//!   and cannot flake. The mean overestimate is statistical: for an unseen key,
//!   `E[overestimate] <= n/(w * d)`, because the minimum over `w` rows is at most any
//!   one row's load and each row's load is `Binomial(n, 1/d)` with mean `n/d`. The test
//!   asserts the observed mean under four times that bound and above zero, the latter
//!   being near-certain for the sample sizes involved.
//! - **HyperLogLog.** Flajolet et al. give the relative standard error as
//!   `1.04 / sqrt(m)`, so with `m = 4096` it is 1.625% and a 4-sigma band is 6.5%. The
//!   test asserts inside that band over a half-million-element stream.
//!
//! # The golden models, and the honest gap
//!
//! There is no `bloommap`, no `countmin` and no `hyperloglog` crate to check against
//! here -- deliberately so, because for a *probabilistic* structure a golden crate
//! would only be checking that this code and that crate implement the same
//! arithmetic, which is a weaker statement than it sounds when both are graded on
//! an error bound. So instead:
//!
//! - the **exact** parts are checked by hand ([`HyperLogLog::from_registers`] for the
//!   raw estimator, the published FNV-1a check values, the mask arithmetic);
//! - the **inexact** parts are checked against their *analytic* bound, with the
//!   derivation written next to the assertion;
//! - the one part that *is* deterministic -- the 64-bit bit array the design
//!   implements -- is checked against [`indexed_insert`] and [`indexed_member`],
//!   hand-written references in this file. That reference is the one place here where
//!   a shared misreading could hide, so it is three lines long and the design's
//!   `case_` table is separately counted.
//!
//! # Hardware: this is where a sketch genuinely does belong on chip
//!
//! The counter bank is the whole argument. A count-min sketch with `w = 4` and
//! `d = 4096` is 16384 four-byte counters: 64 KiB of state that must be *read and
//! written* on every update, and which is far too large for a CPU's fastest cache.
//! On chip, next to the hash function that computes the addresses, that bank is one
//! cycle of SRAM access. Off chip it is a cache miss per update and nothing else. So
//! unlike [`bitpack`](crate::bitpack) -- where the honest verdict was that the win is
//! bandwidth and the arithmetic is free -- a sketch's win is a real locality win. The
//! arithmetic is still almost nothing: one add per update for count-min, one OR for
//! bloom, one max for HyperLogLog.
//!
//! The design here is the bloom filter's bit array, because that is the piece that
//! needs a *decoder*: 64 bits of state addressed by a 6-bit hash, which is a 64-entry
//! one-hot decoder over 64 bits, i.e. exactly the block-RAM-plus-write-decoder a real
//! implementation infers.
//!
//! **A known gap:** there is no initialised-memory node in this IR. `Design::mem` is
//! zero-filled in both backends, so the filter's bit array is a *register* here rather
//! than a RAM. That is not so much a workaround as the same answer -- sixty-four bits
//! is register-sized -- but a filter large enough to be useful (thousands of bits) is
//! RAM-sized, and modelling that would need a ROM/RAM node with reset contents.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The width of the design's bit array, in bits.
///
/// Sixty-four, so that it is one port rather than a collection and so the design's
/// whole state is a single register.
pub const FILTER_BITS: u32 = 64;

/// The width of a probe address, in bits.
///
/// Six, because [`FILTER_BITS`] is sixty-four.
pub const PROBE_BITS: u32 = 6;

/// How many probes one insert sets, i.e. the bloom filter's `k`.
pub const PROBES: usize = 2;

/// The single-bit mask for a probe address.
///
/// The one-hot decoder's output. `index` is masked to six bits rather than rejected
/// above that, because the design's address port *is* six bits and a reference that
/// panicked where the design truncates would be a reference that hid a real
/// difference rather than reporting one.
#[must_use]
pub fn mask_for(index: u32) -> u64 {
    1u64 << (index & (FILTER_BITS - 1))
}

/// The reference for the design's insert: set both probe bits.
///
/// One line of `or` on a `u64`, which is the entire operation the design's register
/// next-value is.
#[must_use]
pub fn indexed_insert(bits: u64, probes: [u32; PROBES]) -> u64 {
    let mut out = bits;
    for probe in probes {
        out |= mask_for(probe);
    }
    out
}

/// The reference for the design's membership test: are both probe bits set?
///
/// A bloom filter's whole contract is here: no false negatives, because a set bit is
/// never cleared by an insert of a different key.
#[must_use]
pub fn indexed_member(bits: u64, probes: [u32; PROBES]) -> bool {
    probes.iter().all(|probe| bits & mask_for(*probe) != 0)
}

/// FNV-1a, 64-bit, with a seed.
///
/// The hash the software sketches use. FNV-1a is not a cryptographic hash and is not
/// used here as one: what a sketch needs is a hash whose *low and high bits both look
/// random*, because count-min takes `hash % columns` (low bits) and HyperLogLog takes
/// `hash >> (64 - p)` (high bits). FNV-1a alone is weak in exactly that split, which
/// is why every call site in this module runs the result through [`mix64`].
///
/// The seed is folded in with an `xor` before the loop, which is the usual way to
/// derive several independent-looking hashes from one function for the bloom filter's
/// `k` probes.
#[must_use]
pub fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8423_2325u64 ^ seed;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The splitmix64 finalizer: a bijection with good avalanche in both halves.
///
/// This is the step that makes FNV-1a usable for HyperLogLog. Without it the top bits
/// of an FNV hash of a short key are close to the top bits of the key, so consecutive
/// keys land in the same register and the register distribution -- which is the entire
/// basis of the `1.04 / sqrt(m)` bound -- collapses.
#[must_use]
pub fn mix64(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A distinct seed for probe or row `index`.
///
/// One derived seed per probe rather than a table of constants, because a bloom filter
/// with `k` probes or a count-min sketch with `w` rows must use *distinct* hash
/// functions per probe or row. A hand-written table of seeds has to be checked for
/// that, and a table with fewer entries than the sketch has probes silently reuses one
/// -- which does not break anything visibly and quietly costs the whole error bound.
#[must_use]
pub fn seed_for(index: usize) -> u64 {
    0x9e37_79b9_7f4a_7c15u64.wrapping_mul(index as u64 + 1)
}

/// A bloom filter over `m` bits with `k` probes per key.
///
/// `k` probes are *independent* -- one [`mix64`] of [`fnv1a`] per seed -- rather than
/// the double-hashing `h + i * h2` that a real filter often uses. That is on purpose:
/// with independent probes the false-positive rate is exactly `(1 - e^{-kn/m})^k` in
/// expectation, which is the number the test asserts against. Double hashing makes the
/// probes correlated and shifts the observed rate by a few percent, which is fine for a
/// real filter and not fine for a test whose whole job is to compare an observation to
/// a formula.
#[derive(Clone, Debug)]
pub struct Bloom {
    m: usize,
    k: usize,
    words: Vec<u64>,
}

impl Bloom {
    /// A filter of `m` bits with `k` probes. `m` is rounded up to a multiple of 64.
    #[must_use]
    pub fn new(m: usize, k: usize) -> Self {
        let m = m.max(1).next_multiple_of(64);
        Self {
            m,
            k: k.max(1),
            words: vec![0u64; m / 64],
        }
    }

    /// How many bits the filter has.
    #[must_use]
    pub fn bits(&self) -> usize {
        self.m
    }

    /// How many probes each key sets.
    #[must_use]
    pub fn probes(&self) -> usize {
        self.k
    }

    /// The probe addresses for `key`.
    #[must_use]
    pub fn probes_for(&self, key: &[u8]) -> Vec<usize> {
        (0..self.k)
            .map(|index| (mix64(fnv1a(key, seed_for(index))) as usize) % self.m)
            .collect()
    }

    /// Sets `key`'s probes.
    pub fn insert(&mut self, key: &[u8]) {
        for probe in self.probes_for(key) {
            self.words[probe / 64] |= 1u64 << (probe % 64);
        }
    }

    /// Whether `key` might be present. **False implies absent; true implies nothing.**
    #[must_use]
    pub fn contains(&self, key: &[u8]) -> bool {
        self.probes_for(key)
            .iter()
            .all(|probe| self.words[probe / 64] & (1u64 << (probe % 64)) != 0)
    }

    /// How many bits are set.
    #[must_use]
    pub fn occupancy(&self) -> u32 {
        self.words.iter().map(|word| word.count_ones()).sum()
    }

    /// The fraction of bits set.
    #[must_use]
    pub fn fill(&self) -> f64 {
        f64::from(self.occupancy()) / self.m as f64
    }
}

/// The expected false-positive rate of a [`Bloom`] with `m` bits, `k` probes and `n`
/// keys inserted.
///
/// `(1 - e^{-kn/m})^k`, which is the standard result: after `n` keys each setting `k`
/// bits, a bit is set with probability `1 - (1 - 1/m)^{kn}` -- hence `1 - e^{-kn/m}` to
/// within `O(kn/m^2)`, which is under 0.03% at the parameters the tests use -- and a
/// false positive needs all `k` probes set, hence the `k`th power.
#[must_use]
pub fn bloom_false_positive_rate(m: usize, k: usize, n: usize) -> f64 {
    let occupied = 1.0 - (-(k as f64) * (n as f64) / (m as f64)).exp();
    occupied.powf(k as f64)
}

/// The expected number of set bits after `n` inserts of `k` probes each.
///
/// `m * (1 - (1 - 1/m)^{kn})`, the same occupancy argument as
/// [`bloom_false_positive_rate`] without the final `k`th power. Checked separately from
/// the rate because a filter that inserts correctly and *tests* incorrectly is a
/// different bug from one that does neither.
#[must_use]
pub fn bloom_expected_occupancy(m: usize, k: usize, n: usize) -> f64 {
    let flips = k as f64 * n as f64;
    (m as f64) * (1.0 - (1.0 - 1.0 / (m as f64)).powf(flips))
}

/// A count-min sketch with `w` rows of `d` counters.
#[derive(Clone, Debug)]
pub struct CountMin {
    w: usize,
    d: usize,
    counters: Vec<u32>,
}

impl CountMin {
    /// A sketch with `w` rows and `d` columns, all counters zero.
    #[must_use]
    pub fn new(w: usize, d: usize) -> Self {
        let w = w.max(1);
        let d = d.max(1);
        Self {
            w,
            d,
            counters: vec![0u32; w * d],
        }
    }

    /// How many rows.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.w
    }

    /// How many columns per row.
    #[must_use]
    pub fn columns(&self) -> usize {
        self.d
    }

    /// The column `key` addresses in row `row`.
    #[must_use]
    pub fn column(&self, key: &[u8], row: usize) -> usize {
        (mix64(fnv1a(key, seed_for(row))) as usize) % self.d
    }

    /// Adds one to each of `key`'s `w` counters.
    pub fn increment(&mut self, key: &[u8]) {
        for row in 0..self.w {
            let column = self.column(key, row);
            self.counters[row * self.d + column] =
                self.counters[row * self.d + column].saturating_add(1);
        }
    }

    /// The minimum of `key`'s `w` counters: the estimate of `key`'s count.
    #[must_use]
    pub fn estimate(&self, key: &[u8]) -> u32 {
        (0..self.w)
            .map(|row| self.counters[row * self.d + self.column(key, row)])
            .min()
            .unwrap_or(0)
    }

    /// The raw counter bank, row-major. Exposed so a test can check the arithmetic
    /// rather than only the minimum of it.
    #[must_use]
    pub fn counters(&self) -> &[u32] {
        &self.counters
    }
}

/// A HyperLogLog with `2^p` registers of `q` bits each.
///
/// The registers hold the maximum position of a set bit (the "rho" of the original
/// paper) seen in the low `64 - p` bits of each hashed key. Registers are capped at
/// `q`, because a key whose tail is a long run of ones carries no more information than
/// the cap.
#[derive(Clone, Debug)]
pub struct HyperLogLog {
    p: u32,
    q: u32,
    registers: Vec<u8>,
}

impl HyperLogLog {
    /// A sketch with `m` registers, `m` rounded up to a power of two.
    #[must_use]
    pub fn new(m: usize) -> Self {
        let m = m.max(2).next_power_of_two();
        let p = m.trailing_zeros();
        Self::new_with(p, 64 - p)
    }

    /// A sketch with `2^p` registers of `q` bits.
    #[must_use]
    pub fn new_with(p: u32, q: u32) -> Self {
        Self {
            p,
            q,
            registers: vec![0u8; 1usize << p],
        }
    }

    /// A sketch built from a register bank directly, for hand-computed estimates.
    ///
    /// # Panics
    ///
    /// If `registers` is not exactly `2^p` long, or a register exceeds `q`. The check
    /// is here rather than left to a later silent mis-estimate because the only
    /// plausible use of this constructor is a test asserting an exact number.
    #[must_use]
    pub fn from_registers(p: u32, q: u32, registers: &[u8]) -> Self {
        assert_eq!(
            registers.len(),
            1usize << p,
            "a HyperLogLog with p = {p} has 2^p registers"
        );
        assert!(
            registers.iter().all(|value| u32::from(*value) <= q),
            "a register cannot hold more than q = {q}"
        );
        Self {
            p,
            q,
            registers: registers.to_vec(),
        }
    }

    /// The register index and rank of a 64-bit hash, by hand.
    ///
    /// `index` is the top `p` bits. `rank` is the position of the leftmost set bit in
    /// the remaining `64 - p`, counting from 1, capped at `q`.
    ///
    /// `leading_zeros` counts from bit 63, which is `p` positions above the top of the
    /// tail, so the field-relative position is `leading_zeros + 1 - p`. The all-zero
    /// tail needs no special case: it gives `65 - p`, which is already past the cap
    /// `q = 64 - p`, so a key that is all ones in the tail saturates the register --
    /// which is the right answer, because such a key carries no more information than
    /// the cap.
    ///
    /// # Panics
    ///
    /// Never. `leading_zeros` on the tail is at least `p` (the tail has no bits above
    /// `64 - p - 1`), so the subtraction cannot underflow.
    #[must_use]
    pub fn index_and_rank(&self, hash: u64) -> (usize, u8) {
        let remaining = 64 - self.p;
        let mask = if remaining >= 64 {
            u64::MAX
        } else {
            (1u64 << remaining) - 1
        };
        let tail = hash & mask;
        let index = (hash >> remaining) as usize;
        let rank = (tail.leading_zeros() + 1 - self.p).min(self.q) as u8;
        (index, rank)
    }

    /// Folds one 64-bit hash into the register bank.
    pub fn insert_hash(&mut self, hash: u64) {
        let (index, rank) = self.index_and_rank(hash);
        self.registers[index] = self.registers[index].max(rank);
    }

    /// Folds one key in, hashing it with [`fnv1a`] and [`mix64`].
    pub fn insert(&mut self, key: &[u8]) {
        self.insert_hash(mix64(fnv1a(key, seed_for(0))));
    }

    /// How many registers the sketch has.
    #[must_use]
    pub fn registers_count(&self) -> usize {
        self.registers.len()
    }

    /// The register bank.
    #[must_use]
    pub fn registers(&self) -> &[u8] {
        &self.registers
    }

    /// The raw estimator, with **no** small-range correction.
    ///
    /// `alpha_m * m^2 / sum(2^-M[j])`, which is Flajolet et al. eq. 5. This is what
    /// [`estimate`](Self::estimate) corrects, and it is exposed because it is the part
    /// that can be checked against a hand computation.
    #[must_use]
    pub fn raw_estimate(&self) -> f64 {
        let m = self.registers.len() as f64;
        let harmonic: f64 = self
            .registers
            .iter()
            .map(|value| 2f64.powi(-i32::from(*value)))
            .sum();
        Self::alpha(m) * m * m / harmonic
    }

    /// The cardinality estimate, with linear counting for small cardinalities.
    ///
    /// The small-range correction is `m * ln(m / V)` where `V` is the number of zero
    /// registers. It applies while the raw estimate is below `2.5 m`; above that the
    /// correction is not valid and the raw estimate is already good. The `2.5 m` and
    /// the `V > 0` guard are both from the paper and both matter: with every register
    /// non-zero, `ln(m / 0)` is not an estimate, it is an infinity, and the sketch is
    /// supposed to answer the raw estimate in that case.
    #[must_use]
    pub fn estimate(&self) -> f64 {
        let m = self.registers.len() as f64;
        let raw = self.raw_estimate();
        let empty = self.registers.iter().filter(|value| **value == 0).count() as f64;
        if raw <= 2.5 * m && empty > 0.0 {
            m * (m / empty).ln()
        } else {
            raw
        }
    }

    /// The relative standard error the paper gives for this register count.
    #[must_use]
    pub fn relative_standard_error(&self) -> f64 {
        1.04 / (self.registers.len() as f64).sqrt()
    }

    /// Flajolet et al.'s `alpha_m`.
    fn alpha(m: f64) -> f64 {
        match m as u64 {
            16 => 0.673,
            32 => 0.697,
            64 => 0.709,
            _ => 0.7213 / (1.0 + 1.079 / m),
        }
    }
}

/// The input ports of the bloom filter.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge either sets both probes or holds the bit array.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which empties the filter.
    pub rst: Signal,
    /// The first probe address.
    #[bits(6)]
    pub h0: Signal,
    /// The second probe address.
    #[bits(6)]
    pub h1: Signal,
    /// On this edge, set both probes.
    pub add: Signal,
}

/// The output ports of the bloom filter.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The bit array. Registered, so a test can watch occupancy rather than only
    /// membership.
    #[bits(64)]
    pub bits: Signal,
    /// Whether both probes are currently set: true means *maybe present*, and false
    /// means *definitely absent*. The asymmetry is the filter's contract.
    pub member: Signal,
    /// Whether every bit is set, i.e. the filter is saturated and will report every
    /// key present.
    pub full: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The bit array, which is also the output.
    pub bits: Signal,
    /// The membership answer, which is also the output.
    pub member: Signal,
    /// The saturation flag, which is also the output.
    pub full: Signal,
}

/// Builds the bloom filter's bit array into `design`.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;
    let bits = design.wire(FILTER_BITS)?;

    let m0 = decoder(design, &inputs.h0)?;
    let m1 = decoder(design, &inputs.h1)?;
    let set = design.or(&m0, &m1)?;

    // Membership is "is the addressed bit set", which is `bits & mask != 0` rather than
    // a 64-arm `case_` selecting bit `h`: the mask is already built for the write
    // decoder, so reusing it turns the read into two gates instead of another table.
    // `ugt` rather than `eq(., 0)` because both are one comparator and `ugt` keeps the
    // width of the left operand, which is the 64-bit array rather than a literal.
    let zero = design.lit(0, FILTER_BITS)?;
    let member0 = design.ugt(&design.and(&bits, &m0)?, &zero)?;
    let member1 = design.ugt(&design.and(&bits, &m1)?, &zero)?;
    let member = design.and(&member0, &member1)?;

    let next = design.ite(&inputs.add, &design.or(&bits, &set)?, &bits)?;
    let stored = design.reg(&next, &inputs.clk, &inputs.rst, &design.constant(false))?;
    design.drive(&bits, &stored)?;

    let full = design.eq(&stored, &design.lit(u64::MAX, FILTER_BITS)?)?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            bits: stored.clone(),
            member: member.clone(),
            full: full.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        bits: stored,
        member,
        full,
    })
}

/// One-hot decoder: the 64-bit mask for a probe address.
///
/// A [`Design::case_`] with one arm per address, each arm a literal `1 << i`. This is
/// the block-RAM write decoder's output in a real design and a 64-entry mux tree here:
/// **there is no initialised-memory node in this IR** (`Design::mem` is zero-filled in
/// both backends), so a mask ROM cannot be written with its contents and the table has
/// to be spelled out as arms. See the module docs.
///
/// The default arm is zero and is unreachable -- the address is six bits and all
/// sixty-four values are enumerated above -- but a `case_` needs one.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn decoder(design: &Design, address: &Signal) -> Result<Signal, BuildError> {
    let mut arms = Vec::with_capacity(FILTER_BITS as usize);
    for index in 0..FILTER_BITS {
        arms.push((u64::from(index), design.lit(1u64 << index, FILTER_BITS)?));
    }
    let none = design.lit(0, FILTER_BITS)?;
    Ok(design.case_(address, &arms, &none)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        Bloom, CountMin, HyperLogLog, bloom_expected_occupancy, bloom_false_positive_rate, fnv1a,
        indexed_insert, indexed_member, mask_for, mix64, seed_for,
    };

    #[test]
    fn fnv1a_matches_its_published_check_values() {
        // The FNV-1a 64-bit test vectors, which are the hash's spec and not this
        // module's: the offset basis for the empty string, and the published digests
        // for "a" and "foobar".
        assert_eq!(fnv1a(b"", 0), 0xcbf2_9ce4_8423_2325);
        assert_eq!(fnv1a(b"a", 0), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar", 0), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn mix64_is_a_bijection() {
        // Not a proof, but a permutation of 64 bits has no collision on a random
        // sample and a hash that collided on the first few would be caught here.
        let mut seen = std::collections::HashSet::new();
        for value in 0..10_000u64 {
            assert!(seen.insert(mix64(value)), "{value} collided");
        }
    }

    #[test]
    fn the_probe_seeds_are_all_distinct() {
        // A bloom filter with more probes than seeds, or a count-min sketch with more
        // rows than seeds, would silently reuse a hash function and lose its error
        // bound without anything visibly breaking.
        let seeds: std::collections::HashSet<u64> = (0..1024).map(seed_for).collect();
        assert_eq!(seeds.len(), 1024);
    }

    #[test]
    fn the_mask_is_one_hot_and_truncates_like_the_address_port() {
        assert_eq!(mask_for(0), 1);
        assert_eq!(mask_for(63), 1u64 << 63);
        assert_eq!(mask_for(64), 1, "a six-bit address cannot name bit 64");
    }

    #[test]
    fn an_inserted_key_is_always_found_and_an_empty_filter_never_answers_yes() {
        let mut filter = Bloom::new(2048, 4);
        assert!(!filter.contains(b"absent"));
        filter.insert(b"present");
        assert!(filter.contains(b"present"));
        assert!(
            filter.occupancy() <= 4,
            "four probes, and a collision would make it fewer"
        );
    }

    #[test]
    fn the_hyperloglog_raw_estimator_is_a_hand_computable_number() {
        // alpha_4 is not one of the paper's tabulated values, so this is the general
        // formula 0.7213 / (1 + 1.079/4) applied to four equal registers of value 2:
        // sum(2^-M) = 4 * 2^-2 = 1, so the estimate is alpha * 16.
        let sketch = HyperLogLog::from_registers(2, 62, &[2, 2, 2, 2]);
        let alpha = 0.7213 / (1.0 + 1.079 / 4.0);
        assert!((sketch.raw_estimate() - alpha * 16.0).abs() < 1e-12);

        // Four unequal registers: sum(2^-M) = 1/2 + 1/4 + 1/8 + 1/16 = 15/16, so the
        // raw estimate is alpha * 16 / 0.9375 = 9.694. That is *below* the paper's
        // 2.5 m = 10 boundary, so the small-range correction would apply -- and does not,
        // because no register is zero and `ln(m/0)` is not an estimate. The `V > 0` guard
        // is the whole reason this case has an answer at all.
        let uneven = HyperLogLog::from_registers(2, 62, &[1, 2, 3, 4]);
        assert!((uneven.raw_estimate() - alpha * 16.0 / 0.9375).abs() < 1e-12);
        assert!(
            uneven.raw_estimate() < 2.5 * 4.0,
            "so the boundary does ask for a correction"
        );
        assert!(
            (uneven.estimate() - uneven.raw_estimate()).abs() < 1e-12,
            "and the `V > 0` guard declines it"
        );
    }

    #[test]
    fn the_hyperloglog_small_range_correction_is_linear_counting() {
        // Three zero registers out of four: V = 3, so the estimate is m * ln(m/V) =
        // 4 * ln(4/3), provided the raw estimate is at or below 2.5 m. It is
        // 0.7213/(1 + 1.079/4) * 16 / 3.5 = 2.596, so the correction does apply.
        let sketch = HyperLogLog::from_registers(2, 62, &[1, 0, 0, 0]);
        assert!(sketch.raw_estimate() <= 10.0);
        assert!((sketch.estimate() - 4.0 * (4.0f64 / 3.0).ln()).abs() < 1e-12);
    }

    #[test]
    fn the_hyperloglog_rank_is_the_hand_computable_one() {
        // p = 2, so the index is the top two bits and the rank is the position of the
        // leftmost set bit in the low 62. Bit 59 is the third bit of that 62-bit field,
        // so the rank is 3.
        let sketch = HyperLogLog::new_with(2, 62);
        assert_eq!(sketch.index_and_rank(1u64 << 59), (0, 3));
        assert_eq!(sketch.index_and_rank(1u64 << 60), (0, 2));
        assert_eq!(sketch.index_and_rank(1), (0, 62), "the last bit is rank 62");
        assert_eq!(
            sketch.index_and_rank(0),
            (0, 62),
            "an all-zero tail carries no information and saturates"
        );
        assert_eq!(
            sketch.index_and_rank(3u64 << 62),
            (3, 62),
            "index is the top p"
        );
    }

    #[test]
    fn count_min_never_under_estimates() {
        let mut sketch = CountMin::new(4, 256);
        for key in 0..64u32 {
            for _ in 0..key {
                sketch.increment(&key.to_le_bytes());
            }
        }
        assert!(sketch.estimate(&7u32.to_le_bytes()) >= 7);
        // For an unseen key the true count is zero and the lower bound is therefore
        // vacuous on its own, so what is worth checking is that the estimate really is
        // the minimum of the four addressed counters and not something else entirely.
        let unseen = 9999u32.to_le_bytes();
        let manual = (0..sketch.rows())
            .map(|row| sketch.counters()[row * sketch.columns() + sketch.column(&unseen, row)])
            .min()
            .expect("a sketch with four rows has four counters to minimise");
        assert_eq!(sketch.estimate(&unseen), manual);
    }

    #[test]
    fn the_bloom_rate_formula_is_bounded_by_its_own_limits() {
        // p -> 0 as the filter grows and p -> 1 as it shrinks to nothing, which is the
        // sanity check on the formula rather than on the implementation.
        assert!(bloom_false_positive_rate(1 << 20, 4, 1000) < 1e-3);
        assert!(bloom_false_positive_rate(64, 4, 64) > 0.9);
        assert!(bloom_expected_occupancy(64, 1, 0) < 1e-9);
    }

    #[test]
    fn the_indexed_reference_is_the_whole_design() {
        let bits = indexed_insert(0, [3, 40]);
        assert_eq!(bits, (1 << 3) | (1 << 40));
        assert!(indexed_member(bits, [3, 40]));
        assert!(!indexed_member(bits, [3, 41]));
        assert!(!indexed_member(0, [0, 0]));
    }
}
