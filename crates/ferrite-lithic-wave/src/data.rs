//! [`WaveData`] — cycle-indexed values, the first-class form of a waveform.

use std::sync::atomic::{AtomicU64, Ordering};

use ferrite_lithic::{PortList, WaveFormat};
use ferrite_lithic_bits::Bits;

use crate::error::{Error, Mismatch};

/// One declared signal in a [`WaveData`].
///
/// A handle as much as a description: [`WaveData::set`] and
/// [`WaveData::value`] take one, so a caller cannot mix up which signal it meant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Port {
    wave: u64,
    index: usize,
    name: String,
    width: u32,
    format: WaveFormat,
}

impl Port {
    /// The port's name, as the design spells it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The port's width in bits.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// How a viewer should render it.
    #[must_use]
    pub const fn format(&self) -> WaveFormat {
        self.format
    }

    /// The same port with a different rendering.
    ///
    /// The width does not move, so this cannot produce a port the waveform's
    /// values do not fit.
    #[must_use]
    pub const fn with_format(mut self, format: WaveFormat) -> Self {
        self.format = format;
        self
    }

    /// Where this signal sits in its waveform's declaration order.
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }
}

/// A waveform's values, indexed by cycle.
///
/// # Why this type exists at all, when a VCD file is what a person looks at
///
/// Hardcaml's golden waveforms are ASCII renders from `hardcaml_waveterm`, an
/// external proprietary Jane Street library
/// ([`README.md:112-114`](https://github.com/jane-street/hardcaml)). We cannot
/// reproduce those bytes and should not try. What is reproducible is the
/// underlying per-cycle data ([`src/wave_data_in_cycles.ml`](https://github.com/jane-street/hardcaml)),
/// and that is what this type holds: assertions read [`value`](WaveData::value)
/// and [`series`](WaveData::series), and [`Vcd`](crate::Vcd) is a rendering of
/// that data rather than the source of it.
///
/// A test that asserts on a rendered waveform is asserting on the renderer. This
/// split means a bug in [`Vcd`](crate::Vcd) cannot make a design look correct, and a
/// change
/// in rendering style cannot fail a behavioural test.
///
/// ```
/// use ferrite_lithic_bits::Bits;
/// use ferrite_lithic_wave::WaveData;
///
/// let mut wave = WaveData::new("counter");
/// let clk = wave.register("clk", 1).unwrap();
/// let acc = wave.register("acc", 4).unwrap();
///
/// for (cycle, value) in [0b0001u64, 0b0010, 0b0100] .into_iter().enumerate() {
///     wave.set(cycle as u64, &clk, &Bits::constant(cycle as u64, 1).unwrap()).unwrap();
///     wave.set(cycle as u64, &acc, &Bits::constant(value, 4).unwrap()).unwrap();
/// }
///
/// // Asserted on the values, not on a picture of them.
/// let expected = [0b0001u64, 0b0010, 0b0100];
/// for (cycle, want) in expected.iter().enumerate() {
///     assert_eq!(wave.value(cycle as u64, &acc).unwrap().to_u64().unwrap(), *want);
/// }
/// assert_eq!(wave.cycles(), 3);
/// ```
#[derive(Clone, Debug)]
pub struct WaveData {
    wave: u64,
    top: String,
    ports: Vec<Port>,
    /// One column per port, so reading a signal across cycles is a slice rather
    /// than a walk of every cycle's row. Port-major, because the questions this
    /// type exists for — "what was `acc` in cycle 7", "when did it change" — are
    /// all about one signal over time.
    values: Vec<Vec<Bits>>,
}

impl WaveData {
    /// An empty waveform under a scope name.
    ///
    /// `top` becomes the VCD `$scope`, so it is the name a design is opened
    /// under, not a port.
    #[must_use]
    pub fn new(top: impl Into<String>) -> Self {
        Self {
            wave: next_id(),
            top: top.into(),
            ports: Vec::new(),
            values: Vec::new(),
        }
    }

    /// Declares a signal, returning the handle its values are written through.
    ///
    /// # Errors
    ///
    /// [`Error::DuplicateName`] if a port of that name is already declared.
    /// Two ports with one name cannot be told apart in a viewer either.
    pub fn register(&mut self, name: &str, width: u32) -> Result<Port, Error> {
        self.register_with(name, width, WaveFormat::Binary)
    }

    /// Declares a signal with an explicit rendering.
    ///
    /// # Errors
    ///
    /// [`Error::DuplicateName`], or [`Error::ZeroWidth`] for a zero-width port.
    pub fn register_with(
        &mut self,
        name: &str,
        width: u32,
        format: WaveFormat,
    ) -> Result<Port, Error> {
        if width == 0 {
            return Err(Error::ZeroWidth {
                port: name.to_string(),
            });
        }
        if self.ports.iter().any(|port| port.name == name) {
            return Err(Error::DuplicateName {
                name: name.to_string(),
            });
        }
        let port = Port {
            wave: self.wave,
            index: self.ports.len(),
            name: name.to_string(),
            width,
            format,
        };
        self.ports.push(port.clone());
        self.values.push(Vec::new());
        Ok(port)
    }

    /// Declares every port of a port-list shape.
    ///
    /// The shape's `#[wave_format]` and its `#[rtlname]`-adjusted names come
    /// across, so a waveform's signals are named the way the emitted Verilog
    /// names them rather than the way the struct spelled them.
    ///
    /// # Errors
    ///
    /// [`Error::DuplicateName`] if the shape declares two ports that resolve to
    /// one name.
    pub fn register_shape<P: PortList>(&mut self) -> Result<Vec<Port>, Error> {
        let names = P::rtl_names();
        if names.len() != P::PORT_WIDTHS.len() {
            return Err(Error::ShapeMismatch {
                names: names.len(),
                widths: P::PORT_WIDTHS.len(),
            });
        }
        let mut registered = Vec::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            registered.push(self.register_with(
                name,
                P::PORT_WIDTHS[index],
                P::PORT_FORMATS[index],
            )?);
        }
        Ok(registered)
    }

    /// The declared signals, in declaration order.
    #[must_use]
    pub fn ports(&self) -> &[Port] {
        &self.ports
    }

    /// The scope name.
    #[must_use]
    pub fn top(&self) -> &str {
        &self.top
    }

    /// How many cycles have been recorded.
    ///
    /// Recording a cycle out of order is fine and extends the record: a gap is
    /// zeros, which is what an unrecorded cycle means.
    #[must_use]
    pub fn cycles(&self) -> usize {
        self.values.iter().map(Vec::len).max().unwrap_or_default()
    }

    /// Writes one signal's value at one cycle.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownPort`] if the handle is not this waveform's, and
    /// [`Error::Width`] if the value is not the port's width.
    pub fn set(&mut self, cycle: u64, port: &Port, value: &Bits) -> Result<(), Error> {
        let index = self.index_of(port)?;
        if value.width() != port.width {
            return Err(Error::Width {
                port: port.name.clone(),
                expected: port.width,
                got: value.width(),
            });
        }
        let width = port.width;
        let column = &mut self.values[index];
        let zero = Bits::zeros(width).expect("a declared width is never zero");
        while column.len() <= cycle as usize {
            column.push(zero.clone());
        }
        column[cycle as usize] = value.clone();
        Ok(())
    }

    /// Fills every port for one cycle from a lookup by name.
    ///
    /// This is how a simulator or a cosimulator feeds a waveform: the caller has
    /// a way to ask "what is `acc` right now" and not an opinion about which
    /// ports exist.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownSignal`] if a declared port has no value — a missing
    /// signal is a bug in the caller, and writing a zero for it would put a
    /// plausible lie in the file — and [`Error::Width`] on a width mismatch.
    pub fn record<F>(&mut self, cycle: u64, mut lookup: F) -> Result<(), Error>
    where
        F: FnMut(&str) -> Option<Bits>,
    {
        let mut values = Vec::with_capacity(self.ports.len());
        for port in &self.ports {
            let value = lookup(port.name()).ok_or_else(|| Error::UnknownSignal {
                name: port.name.clone(),
            })?;
            if value.width() != port.width {
                return Err(Error::Width {
                    port: port.name.clone(),
                    expected: port.width,
                    got: value.width(),
                });
            }
            values.push(value);
        }
        for (index, value) in values.into_iter().enumerate() {
            let width = self.ports[index].width;
            let column = &mut self.values[index];
            let zero = Bits::zeros(width).expect("a declared width is never zero");
            while column.len() <= cycle as usize {
                column.push(zero.clone());
            }
            column[cycle as usize] = value;
        }
        Ok(())
    }

    /// One signal's value at one cycle.
    ///
    /// [`None`] if the cycle has not been recorded. An unrecorded cycle reads as
    /// absent rather than as zero, because "nothing was written here" and "zero
    /// was written here" are different facts and a test should be able to tell
    /// them apart.
    #[must_use]
    pub fn value(&self, cycle: u64, port: &Port) -> Option<&Bits> {
        let index = self.index_of(port).ok()?;
        self.values.get(index)?.get(cycle as usize)
    }

    /// One signal's values, over the cycles recorded so far.
    #[must_use]
    pub fn series(&self, port: &Port) -> Option<&[Bits]> {
        let index = self.index_of(port).ok()?;
        Some(self.values.get(index)?.as_slice())
    }

    /// One signal's values, over the cycles recorded so far, by name.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownPortName`] if no such signal is declared.
    pub fn series_named(&self, name: &str) -> Result<Vec<Bits>, Error> {
        Ok(self
            .values
            .get(self.index_of_name(name)?)
            .cloned()
            .unwrap_or_default())
    }

    /// One signal's value changes: `(cycle, value)` for each cycle it differs
    /// from the previous.
    ///
    /// This is what a VCD file actually stores, so it is also the smallest
    /// honest answer to "when did this signal change" — a signal that holds its
    /// value for four cycles has one entry, not four.
    #[must_use]
    pub fn series_changes(&self, port: &Port) -> Vec<(u64, Bits)> {
        let mut changes = Vec::new();
        let Some(column) = self.series(port) else {
            return changes;
        };
        let mut previous: Option<&Bits> = None;
        for (cycle, value) in column.iter().enumerate() {
            if previous != Some(value) {
                changes.push((cycle as u64, value.clone()));
                previous = Some(value);
            }
        }
        changes
    }

    /// Asserts one signal's values against a series.
    ///
    /// # Errors
    ///
    /// [`Mismatch`] on the first cycle that differs, carrying both values. The
    /// series is compared up to `expected.len()` cycles and then for length, so
    /// a waveform that ran too long is a failure and not a pass on the prefix.
    pub fn assert_series(&self, port: &Port, expected: &[Bits]) -> Result<(), Mismatch> {
        let observed = self.series(port).unwrap_or_default();
        for (cycle, want) in expected.iter().enumerate() {
            let Some(got) = observed.get(cycle) else {
                return Err(Mismatch {
                    port: port.name.clone(),
                    cycle: cycle as u64,
                    expected: want.clone(),
                    got: None,
                });
            };
            if got != want {
                return Err(Mismatch {
                    port: port.name.clone(),
                    cycle: cycle as u64,
                    expected: want.clone(),
                    got: Some(got.clone()),
                });
            }
        }
        if observed.len() != expected.len() {
            return Err(Mismatch {
                port: port.name.clone(),
                cycle: expected.len() as u64,
                expected: Bits::zeros(port.width).expect("a declared width is never zero"),
                got: observed.get(expected.len()).cloned(),
            });
        }
        Ok(())
    }

    /// A signal's value at one cycle, rendered in its own format.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownCycle`] if that cycle was not recorded.
    pub fn render(&self, cycle: u64, port: &Port) -> Result<String, Error> {
        let value = self.value(cycle, port).ok_or(Error::UnknownCycle {
            cycle,
            recorded: self.cycles(),
        })?;
        Ok(render_value(value, port.format))
    }

    /// The port's identifier for VCD, allocated in declaration order.
    #[must_use]
    pub fn identifiers(&self) -> Vec<String> {
        (0..self.ports.len())
            .map(crate::vcd::identifier_for)
            .collect()
    }

    /// The index a handle refers to, if this waveform declared it.
    ///
    /// Compared whole, not by name: two waveforms can both declare a signal
    /// called `acc`, and a handle from one must not quietly write into the
    /// other. The index is what makes a handle an identity rather than a name.
    fn index_of(&self, port: &Port) -> Result<usize, Error> {
        // Compared on which waveform and which index, not on the format: a
        // format is a rendering hint a caller may change on a copy of the
        // handle, and `with_format` exists for exactly that.
        if port.wave == self.wave
            && self
                .ports
                .get(port.index)
                .is_some_and(|declared| declared.name == port.name)
        {
            return Ok(port.index);
        }
        Err(Error::UnknownPort {
            port: port.name.clone(),
        })
    }

    fn index_of_name(&self, name: &str) -> Result<usize, Error> {
        self.ports
            .iter()
            .position(|declared| declared.name == name)
            .ok_or_else(|| Error::UnknownPortName {
                name: name.to_string(),
                known: self.ports.iter().map(|p| p.name.clone()).collect(),
            })
    }
}

/// The next waveform's identity.
///
/// A plain counter, thread-safe and never reused, so a [`Port`] handle can say
/// which waveform declared it. The alternative — matching on a name — is what
/// makes a handle from one waveform usable in another that happens to declare
/// the same name, which is the mistake this exists to catch.
fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Renders one value the way a format asks for.
///
/// Binary is the VCD form and is a plain digit string; hex and decimal are
/// viewer-side renderings, which is why they are produced here and not written
/// into the file.
#[must_use]
pub fn render_value(value: &Bits, format: WaveFormat) -> String {
    match format {
        WaveFormat::Binary => value.to_binary_digits(),
        WaveFormat::Hex => {
            let digits = (value.width().div_ceil(4)) as usize;
            let rendered = value.to_hex();
            let trimmed = rendered.trim_start_matches("0x").to_string();
            format!("0x{:0>width$}", trimmed, width = digits)
        }
        WaveFormat::Decimal => value.to_u64().map_or_else(
            |_| {
                // Wider than 64 bits has no decimal form that fits in a `u64`, so
                // the binary digits are the honest fallback rather than a
                // truncated number.
                value.to_binary_digits()
            },
            |number| number.to_string(),
        ),
        // `WaveFormat` is `#[non_exhaustive]`, so a format added later renders
        // as binary rather than failing to compile every backend at once.
        other => {
            let _ = other;
            value.to_binary_digits()
        }
    }
}
