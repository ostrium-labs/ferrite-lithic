//! Shared conversion helpers between `Bits` and the `num-bigint` oracle.
//!
//! `num-bigint` is a dev-dependency only. It is the one place our oracle is *not*
//! Hardcaml, because Hardcaml has no division or remainder to compare against
//! ([ADR-0009](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0009-add-integer-division-and-remainder.md)).
//! Everything else is checked against Hardcaml-derived expectations written out by
//! hand in the unit tests.

#![allow(dead_code)]

use ferrite_lithic_bits::{Bits, bytes_for_width};
use num_bigint::{BigInt, BigUint, Sign};
use num_traits::One;

/// The unsigned value of a `Bits`, as a `BigUint`.
pub(crate) fn to_biguint(value: &Bits) -> BigUint {
    BigUint::from_bytes_le(&value.to_bytes_le())
}

/// A `Bits` of `width` holding the low `width` bits of `value`.
pub(crate) fn from_biguint(value: &BigUint, width: u32) -> Bits {
    let want = bytes_for_width(width);
    let mut buffer = vec![0u8; want];
    for (slot, byte) in buffer.iter_mut().zip(value.to_bytes_le()) {
        *slot = byte;
    }
    Bits::from_bytes_le(width, &buffer).expect("buffer length is exact")
}

/// The two's complement signed value of a `Bits`, as a `BigInt`.
pub(crate) fn to_bigint(value: &Bits) -> BigInt {
    let magnitude = to_biguint(value);
    if value.is_negative() {
        // The stored bits are `magnitude`, which stands for `magnitude - 2^width`.
        let modulus = BigUint::one() << value.width();
        BigInt::from_biguint(Sign::Minus, modulus - magnitude)
    } else {
        BigInt::from_biguint(Sign::Plus, magnitude)
    }
}

/// A `Bits` of `width` holding `value` reduced modulo `2^width`.
pub(crate) fn from_bigint(value: &BigInt, width: u32) -> Bits {
    let modulus = BigUint::one() << width;
    let magnitude = value.magnitude();
    let wrapped = if value.sign() == Sign::Minus {
        (&modulus - (magnitude % &modulus)) % &modulus
    } else {
        magnitude % &modulus
    };
    from_biguint(&wrapped, width)
}

/// `2^width`, as a `BigUint`.
pub(crate) fn modulus(width: u32) -> BigUint {
    BigUint::one() << width
}

/// `Bits` with every bit clear.
pub(crate) fn zero(width: u32) -> Bits {
    Bits::zeros(width).expect("width >= 1")
}

/// `Bits` with every bit set.
pub(crate) fn ones(width: u32) -> Bits {
    Bits::ones(width).expect("width >= 1")
}

/// A deterministic pseudo-random `u64`, so the explicit width sweep is
/// reproducible without pulling in a random-number dependency.
pub(crate) struct Lcg(u64);

impl Lcg {
    pub(crate) fn new(seed: u64) -> Self {
        Lcg(seed)
    }

    /// The next value. Uses the 64-bit PCG-style increment with an xorshift mix;
    /// good enough to produce varied bit patterns, not good enough to be secure.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let mut x = self.0;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x
    }

    /// A `Bits` of `width` with pseudo-random bits.
    pub(crate) fn bits(&mut self, width: u32) -> Bits {
        let mut buffer = vec![0u8; bytes_for_width(width)];
        for slot in buffer.iter_mut() {
            *slot = self.next_u64() as u8;
        }
        Bits::from_bytes_le(width, &buffer).expect("buffer length is exact")
    }
}

/// True when no bit at or above `width` is set, i.e. the zero-upper-bits
/// invariant holds.
///
/// `BigUint::bits` is the count of bits needed to represent the value, and zero
/// for zero, so "highest set bit index below width" is exactly `bits <= width`.
pub(crate) fn invariant_holds(value: &Bits) -> bool {
    to_biguint(value).bits() <= u64::from(value.width())
}
