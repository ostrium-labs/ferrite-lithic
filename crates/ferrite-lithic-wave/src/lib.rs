//! Waveforms for the Ferrite Lithic hardware DSL.
//!
//! # What this crate is for
//!
//! Two backends now produce values per cycle — the simulator and, in A2's
//! cosimulator, the emitted Verilog — and the question "what did `acc` hold in
//! cycle 7" needs one answer that does not depend on which backend ran. That
//! answer is [`WaveData`]: cycle-indexed
//! [`Bits`](ferrite_lithic_bits::Bits), one per declared signal.
//!
//! [`Vcd`] renders that to a file a person can open. It is a *rendering*, and
//! deliberately not the thing tests assert on, because Hardcaml's own golden
//! waveforms are tied to `hardcaml_waveterm`, a proprietary external library
//! ([`README.md:112-114`](https://github.com/jane-street/hardcaml)) whose bytes
//! we could not reproduce if we tried. What we can reproduce is the data
//! ([`src/wave_data_in_cycles.ml`](https://github.com/jane-street/hardcaml)), so
//! that is the first-class value and the file is derived from it.
//!
//! ```
//! use ferrite_lithic_bits::Bits;
//! use ferrite_lithic_wave::{Mismatch, WaveData};
//!
//! let mut wave = WaveData::new("shifter");
//! let data = wave.register("data", 8).unwrap();
//!
//! for (cycle, value) in [0x01u64, 0x02, 0x04].into_iter().enumerate() {
//!     wave.set(cycle as u64, &data, &Bits::constant(value, 8).unwrap()).unwrap();
//! }
//!
//! // The assertion a test writes: values, not a picture.
//! let expected: Vec<Bits> = [1u64, 2, 4]
//!     .into_iter()
//!     .map(|value| Bits::constant(value, 8).unwrap())
//!     .collect();
//! wave.assert_series(&data, &expected).unwrap();
//!
//! // And the file, for a person.
//! let text = ferrite_lithic_wave::Vcd::new(&wave).render();
//! assert!(text.contains("$scope module shifter $end"));
//! ```
//!
//! # No wall-clock in the file
//!
//! No `$date`. Two runs of the same design must produce the same bytes, or a
//! golden file is a test of the clock. See [`Vcd`] for the rest of what the
//! format does and does not allow.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod data;
mod error;
mod vcd;

pub use data::{Port, WaveData, render_value};
pub use error::{Error, Mismatch};
pub use vcd::{DEFAULT_TIMESCALE, Vcd, identifier_for};
