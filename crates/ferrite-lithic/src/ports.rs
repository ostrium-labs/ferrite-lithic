//! Port-list shapes: one struct describing a module interface, used twice.
//!
//! # What a shape is, and is not
//!
//! A [`PortList`] is a **shape**: a list of ports with names and widths, and no
//! direction. Hardcaml draws the same line. Its ppx does not infer direction
//! either; the user supplies two separate interface types, one for inputs and
//! one for outputs, and the circuit builder sits between them
//! ([`test/lib/test_fifo.ml:8-31`](https://github.com/jane-street/hardcaml),
//! [`src/circuit.ml:521-522`](https://github.com/jane-street/hardcaml)).
//!
//! We could infer direction from field names or field types, and we do not,
//! because the trait does not know what a design is for: `d` is an input on a
//! FIFO and an output on a load unit, and a convention that guesses would be
//! wrong on exactly the designs where it matters. So the direction is
//! materialised by the builder — [`inputs`] makes undriven wires that are module
//! inputs, [`outputs`] makes wires driven from an expression — and the same
//! shape can be used on either side.
//!
//! ```
//! use ferrite_lithic::Design;
//! use ferrite_lithic::{inputs, outputs};
//! use ferrite_lithic_derive::PortList;
//!
//! #[derive(PortList)]
//! struct AdderInputs {
//!     #[bits(8)] a: ferrite_lithic::Signal,
//!     #[bits(8)] b: ferrite_lithic::Signal,
//! }
//!
//! #[derive(PortList)]
//! struct AdderOutputs {
//!     #[bits(8)] sum: ferrite_lithic::Signal,
//! }
//!
//! # fn main() -> Result<(), ferrite_lithic::PortError> {
//! let design = Design::new();
//!
//! // `a` and `b` come from outside; `sum` is computed from them. Which fields
//! // are which is a fact about the design, stated here, not inferred there --
//! // and because one design cannot hold two nodes with one name, the two sides
//! // are two shapes rather than one.
//! let ports = inputs::<AdderInputs>(&design)?;
//! let sum = &ports.a + &ports.b;
//! let ports = outputs::<AdderOutputs>(&design, &AdderOutputs { sum })?;
//!
//! assert_eq!(AdderInputs::PORT_NAMES, ["a", "b"]);
//! assert_eq!(AdderInputs::PORT_WIDTHS, [8, 8]);
//! assert_eq!(AdderOutputs::PORT_NAMES, ["sum"]);
//! assert_eq!(design.output_ports().len(), 1);
//! # let _ = ports;
//! # Ok(())
//! # }
//! ```
//!
//! # Why the trait is here and the macro is not
//!
//! `#[derive(PortList)]` lives in `ferrite-lithic-derive`, because a proc-macro
//! crate cannot export a trait. The split follows `serde`/`serde_derive`: the
//! trait and the functions that use it are ordinary library code you can read,
//! and the generated code is only the field walk. Nothing here needs a macro to
//! be understood, which is the point — a derive that generated the direction
//! logic as well would be a design rule nobody could read.

use core::fmt;

use ferrite_lithic_ir::NodeId;

use crate::design::Error as DesignError;

use crate::design::{Design, Signal};

/// How a waveform viewer should render one port's value.
///
/// This is a property of the *port*, not of the writer, so it lives here rather
/// than in `ferrite-lithic-wave`: the shape knows it wants hex, and the writer
/// knows that VCD only has a binary scalar form and that hex is a viewer-side
/// annotation. The enum records the intent and the writer decides what it can
/// actually do about it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum WaveFormat {
    /// One `0` or `1` per bit. The only thing VCD's value-change syntax can
    /// express for a vector, and therefore the default.
    #[default]
    Binary,
    /// A `0x`-prefixed whole number, zero-padded to the port's width in nibbles.
    Hex,
    /// A plain unsigned decimal number.
    Decimal,
}

impl WaveFormat {
    /// The spelling used in `#[wave_format("..")]`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::Hex => "hex",
            Self::Decimal => "decimal",
        }
    }

    /// Parses a `#[wave_format("..")]` argument.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "binary" => Some(Self::Binary),
            "hex" => Some(Self::Hex),
            "decimal" => Some(Self::Decimal),
            _ => None,
        }
    }
}

impl fmt::Display for WaveFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Anything that can go wrong while wiring up a [`PortList`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// Building or driving a signal failed inside the design.
    Design(DesignError),
    /// A list of signals did not have the length the shape declares.
    Arity {
        /// How many signals the shape's required ports need.
        expected: usize,
        /// How many were supplied.
        got: usize,
    },
    /// A signal's width did not match the port's declared width.
    Width {
        /// The port whose width did not match.
        port: String,
        /// The width the shape declares.
        expected: u32,
        /// The width the signal has.
        got: u32,
    },
    /// A collection port ran out of signals partway through.
    Truncated {
        /// The collection port.
        port: String,
        /// How many signals that port needs.
        want: usize,
        /// How many were left.
        got: usize,
    },
    /// A required port was marked absent.
    NotOptional {
        /// The port, by declared name.
        port: String,
    },
    /// A name was looked up that the shape does not declare.
    UnknownPort {
        /// The name that was asked for.
        name: String,
        /// Every name the shape does declare.
        known: Vec<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Design(inner) => write!(f, "{inner}"),
            Self::Arity { expected, got } => write!(
                f,
                "this port list needs {expected} signal(s) and got {got}. A shape \
                 declares one port per field, so the count is fixed by the struct: \
                 either build the signals with `wires`/`inputs`/`outputs`, which \
                 return the shape themselves, or check that no field is missing."
            ),
            Self::Width {
                port,
                expected,
                got,
            } => write!(
                f,
                "port `{port}` is declared #[bits({expected})] but the signal is {got} \
                 bit(s) wide. The declared width is what the Verilog port list will \
                 declare, so a mismatch is a wrong module interface rather than a \
                 runtime condition; change the attribute or the width at the source."
            ),
            Self::Truncated { port, want, got } => write!(
                f,
                "collection port `{port}` needs {want} signal(s) but only {got} were \
                 left. Collections are flattened in declaration order, so a short list \
                 means the ports before it took more signals than their declaration \
                 asked for."
            ),
            Self::NotOptional { port } => write!(
                f,
                "port `{port}` is required, so it cannot be marked absent. Only a \
                 field declared #[exists] on an Option<Signal> may be absent; a \
                 caller that thinks a plain port is optional is building a \
                 different module than the shape describes."
            ),
            Self::UnknownPort { name, known } => write!(
                f,
                "no port named `{name}`. This shape declares: {}.",
                known.join(", ")
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::design::Error> for Error {
    fn from(inner: crate::design::Error) -> Self {
        Self::Design(inner)
    }
}

/// A list of ports with names and widths, and no direction.
///
/// Implemented by `#[derive(PortList)]`. Every method that can fail returns
/// [`Result`], for the same reason the builders on [`Design`] do: a shape is
/// checked against the signals it is given rather than trusted.
///
/// # The associated constants are the shape
///
/// [`PORT_NAMES`](PortList::PORT_NAMES), [`PORT_WIDTHS`](PortList::PORT_WIDTHS)
/// and their companions are the interface. They are `const` because a port list
/// is needed at elaboration time — to declare a Verilog port, to open a
/// `$var` section, to write a driver — and building a `Vec` to say "this module
/// has a port called `d`" would put an allocation on that path.
pub trait PortList: Sized {
    /// Every declared port's name, in declaration order, with `#[rtlname]`
    /// overrides applied. Does not include a container's `#[rtlprefix]` or
    /// `#[rtlsuffix]`; see [`rtl_names`](PortList::rtl_names).
    const PORT_NAMES: &'static [&'static str];

    /// Every declared port's width in bits, in declaration order.
    const PORT_WIDTHS: &'static [u32];

    /// Which ports are `#[exists]`. Same length as [`PORT_WIDTHS`](Self::PORT_WIDTHS).
    const PORT_OPTIONAL: &'static [bool];

    /// Every port's `#[wave_format]`, defaulting to [`WaveFormat::Binary`].
    const PORT_FORMATS: &'static [WaveFormat];

    /// The index of the `#[clock]` port, if the shape has one.
    const PORT_CLOCK: Option<usize>;

    /// How many ports the shape declares, optional ones included.
    #[must_use]
    fn port_count() -> usize {
        Self::PORT_WIDTHS.len()
    }

    /// The declared names.
    #[must_use]
    fn names() -> &'static [&'static str] {
        Self::PORT_NAMES
    }

    /// The declared widths, as an owned vector for callers that want to iterate.
    #[must_use]
    fn widths() -> Vec<u32> {
        Self::PORT_WIDTHS.to_vec()
    }

    /// Which ports are optional.
    #[must_use]
    fn optional() -> &'static [bool] {
        Self::PORT_OPTIONAL
    }

    /// How many ports are not optional.
    ///
    /// A list for a module where the `#[exists]` ports are absent has this many
    /// signals, which is why [`from_present`](PortList::from_present) checks
    /// against it rather than against [`port_count`](PortList::port_count).
    #[must_use]
    fn required_count() -> usize {
        Self::PORT_WIDTHS
            .iter()
            .zip(Self::PORT_OPTIONAL)
            .filter(|(_, optional)| !**optional)
            .count()
    }

    /// The declared waveform formats.
    #[must_use]
    fn formats() -> &'static [WaveFormat] {
        Self::PORT_FORMATS
    }

    /// The `#[clock]` port's index.
    #[must_use]
    fn clock_index() -> Option<usize> {
        Self::PORT_CLOCK
    }

    /// The index of a named port.
    #[must_use]
    fn index_of(name: &str) -> Option<usize> {
        Self::PORT_NAMES
            .iter()
            .position(|declared| *declared == name)
    }

    /// The names as the Verilog port list will spell them.
    ///
    /// `PORT_NAMES` is what the design calls its ports. This is what the emitted
    /// module calls them once `#[rtlprefix]`, `#[rtlsuffix]` and `#[rtlmangle]`
    /// have been applied. They are usually the same, and the reason they are two
    /// functions is that mangling cannot happen in a `const`: it needs the
    /// emitter's own legalisation, so a mangled name is only known once
    /// `ferrite-lithic-rtl` is reachable.
    #[must_use]
    fn rtl_names() -> Vec<String> {
        Self::PORT_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    }

    /// Builds the shape from exactly its required ports' signals.
    ///
    /// Optional ports come back as [`None`]: a list that omits them is a list
    /// for a module where they do not exist, and the shape records that rather
    /// than inventing a signal for them.
    ///
    /// # Errors
    ///
    /// [`Error::Arity`] if the count is wrong, [`Error::Width`] if a signal is
    /// not its declared width, and [`Error::Truncated`] if a collection runs out
    /// partway through.
    fn from_signals(signals: Vec<Signal>) -> Result<Self, Error> {
        let present = vec![true; Self::port_count()];
        Self::from_pattern(signals, &present)
    }

    /// Builds the shape from one signal per *present* port, given the pattern.
    ///
    /// This is the constructor that handles `#[exists]` ports: `present` says
    /// which of the declared ports this list has, the signals are those ports'
    /// in declaration order, and the rest come back [`None`]. One shape then
    /// describes both a module that has the optional ports and one that does not.
    ///
    /// # Errors
    ///
    /// [`Error::Arity`] if `present` is not one entry per declared port or the
    /// signal count does not match the ports it claims, [`Error::Width`] if a
    /// signal is not its declared width, and [`Error::Truncated`] if a
    /// collection runs out partway through.
    fn from_pattern(signals: Vec<Signal>, present: &[bool]) -> Result<Self, Error>;

    /// Which of the declared ports this list actually has.
    ///
    /// Always one entry per declared port; a `false` is a declared-but-absent
    /// `#[exists]` port.
    fn present(&self) -> Vec<bool>;

    /// Builds the shape from one signal per non-optional port.
    ///
    /// The convenience case of [`from_pattern`](PortList::from_pattern) for a
    /// module that does not have the `#[exists]` ports: they come back [`None`]
    /// rather than being invented.
    ///
    /// # Errors
    ///
    /// [`Error::Arity`] if the count is not
    /// [`required_count`](PortList::required_count), and whatever
    /// [`from_pattern`](PortList::from_pattern) reports.
    fn from_present(signals: Vec<Signal>) -> Result<Self, Error> {
        let present = Self::PORT_OPTIONAL
            .iter()
            .map(|optional| !optional)
            .collect::<Vec<_>>();
        Self::from_pattern(signals, &present)
    }

    /// The shape's signals, in declaration order, skipping absent optional ports.
    ///
    /// # Errors
    ///
    /// None. This is the one infallible direction, because a shape built by
    /// `from_signals`, `wires`, `inputs` or `outputs` cannot hold a foreign
    /// signal or a wrong-width one — the checks happened when it was built.
    fn to_signals(&self) -> Vec<Signal>;

    /// [`to_signals`](PortList::to_signals), named for the shape it is on.
    ///
    /// The same operation under the name the Hardcaml interface functor uses,
    /// so a port list reads the same as the reference implementation's.
    #[must_use]
    fn to_list(&self) -> Vec<Signal> {
        self.to_signals()
    }

    /// Applies `f` to every signal, in declaration order.
    ///
    /// # Errors
    ///
    /// Whatever `f` returns. Nothing is checked here; a shape is not rebuilt
    /// from the results, so the caller decides whether a mapped list is still
    /// the same interface.
    fn map(&self, f: impl FnMut(&Signal) -> Result<Signal, Error>) -> Result<Vec<Signal>, Error> {
        self.to_signals().iter().map(f).collect()
    }

    /// Applies `f` to every pair of signals from two lists of the same shape.
    ///
    /// # Errors
    ///
    /// [`Error::Arity`] if the two lists have different lengths, then whatever
    /// `f` returns.
    fn map2(
        &self,
        other: &Self,
        mut f: impl FnMut(&Signal, &Signal) -> Result<Signal, Error>,
    ) -> Result<Vec<Signal>, Error> {
        let left = self.to_signals();
        let right = other.to_signals();
        if left.len() != right.len() {
            return Err(Error::Arity {
                expected: left.len(),
                got: right.len(),
            });
        }
        left.iter().zip(&right).map(|(a, b)| f(a, b)).collect()
    }

    /// The signals' node ids, for handing back to the IR.
    #[must_use]
    fn node_ids(&self) -> Vec<NodeId> {
        self.to_signals().iter().map(Signal::id).collect()
    }

    /// One signal by declaration index.
    #[must_use]
    fn port(&self, index: usize) -> Option<Signal> {
        self.to_signals().into_iter().nth(index)
    }

    /// One signal by declared name.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownPort`] if the shape has no such port. The error lists the
    /// names it does have, because the alternative is an `Option` whose `None`
    /// says nothing about whether the name was misspelled or the port is
    /// optional-and-absent.
    fn named(&self, name: &str) -> Result<Signal, Error> {
        let index = Self::index_of(name).ok_or_else(|| Error::UnknownPort {
            name: name.to_string(),
            known: Self::PORT_NAMES.iter().map(|n| (*n).to_string()).collect(),
        })?;
        self.port(index).ok_or_else(|| Error::UnknownPort {
            name: name.to_string(),
            known: Self::PORT_NAMES.iter().map(|n| (*n).to_string()).collect(),
        })
    }

    /// The `#[clock]` signal, if the shape has one and it is present.
    #[must_use]
    fn clock(&self) -> Option<Signal> {
        Self::PORT_CLOCK.and_then(|index| self.port(index))
    }
}

/// Takes one signal for a port, checking its width.
///
/// Generated code calls this; it is public because generated code can only reach
/// public items.
///
/// # Errors
///
/// [`Error::Width`] if the signal is not `width` bits, or [`Error::Arity`] if
/// the list is empty.
pub fn take_one(
    signals: &mut impl Iterator<Item = Signal>,
    port: &str,
    width: u32,
) -> Result<Signal, Error> {
    let signal = signals.next().ok_or(Error::Arity {
        expected: 1,
        got: 0,
    })?;
    check_width(port, &signal, width)?;
    Ok(signal)
}

/// Takes `count` signals for a collection port, checking each width.
///
/// # Errors
///
/// [`Error::Truncated`] if the list ends early, and [`Error::Width`] if any
/// signal is not `width` bits. Every signal taken so far is consumed before the
/// error is returned; a shape is not rebuilt from a failed call.
pub fn take_many(
    signals: &mut impl Iterator<Item = Signal>,
    port: &str,
    count: usize,
    width: u32,
) -> Result<Vec<Signal>, Error> {
    let mut taken = Vec::with_capacity(count);
    for _ in 0..count {
        let signal = signals.next().ok_or_else(|| Error::Truncated {
            port: port.to_string(),
            want: count,
            got: taken.len(),
        })?;
        check_width(port, &signal, width)?;
        taken.push(signal);
    }
    Ok(taken)
}

/// Checks a signal against a declared width.
///
/// # Errors
///
/// [`Error::Width`] naming the port, both widths, and the fix.
pub fn check_width(port: &str, signal: &Signal, width: u32) -> Result<(), Error> {
    if signal.width() == width {
        return Ok(());
    }
    Err(Error::Width {
        port: port.to_string(),
        expected: width,
        got: signal.width(),
    })
}

/// One undriven wire per port, deliberately **unnamed**.
///
/// This is the scratch list: the wires a design computes from before it has
/// decided which of them are module ports. Unnamed on purpose, because a name is
/// spent the moment it is given — one design cannot hold two nodes called
/// `clock` — so a scratch list that took the names would leave nothing for
/// [`inputs`] or [`outputs`] to declare. Use this to compute, and one of those
/// two to declare.
///
/// # Errors
///
/// Whatever `Design::wire` and [`PortList::from_signals`] report. There is no
/// name to collide with, so the only failure is a width.
pub fn wires<P: PortList>(design: &Design) -> Result<P, Error> {
    let mut made = Vec::with_capacity(P::port_count());
    for width in P::PORT_WIDTHS {
        made.push(design.wire(*width)?);
    }
    P::from_signals(made)
}

/// One module input port per port of the shape.
///
/// This is the "direction" step: every signal becomes an undriven wire *and* is
/// registered as an input, which is exactly the criterion the emitter uses to
/// find a module's inputs.
///
/// # Errors
///
/// Whatever [`wires`] reports.
pub fn inputs<P: PortList>(design: &Design) -> Result<P, Error> {
    let mut made = Vec::with_capacity(P::port_count());
    for (index, width) in P::PORT_WIDTHS.iter().enumerate() {
        made.push(design.input(P::PORT_NAMES[index], *width)?);
    }
    P::from_signals(made)
}

/// One module output port per signal of `drivers`, driven by it.
///
/// The drivers are matched to ports by position, so `drivers` has to be a list
/// of the same shape in the same order. Optional ports that are `None` in
/// `drivers` are not declared, which is how a shape with `#[exists]` ports
/// describes two different modules.
///
/// # Errors
///
/// [`Error::Arity`] if `drivers` does not have one signal per declared port, and
/// whatever [`Design::output`] reports for a name already in use.
pub fn outputs<P: PortList>(design: &Design, drivers: &P) -> Result<P, Error> {
    let present = drivers.present();
    let supplied = drivers.to_signals();
    let wanted = present.iter().filter(|here| **here).count();
    if supplied.len() != wanted {
        return Err(Error::Arity {
            expected: wanted,
            got: supplied.len(),
        });
    }
    let mut next = supplied.iter();
    let mut made = Vec::with_capacity(supplied.len());
    for (index, here) in present.iter().enumerate() {
        if !here {
            continue;
        }
        let driver = next.next().ok_or(Error::Arity {
            expected: wanted,
            got: supplied.len(),
        })?;
        made.push(design.output(P::PORT_NAMES[index], P::PORT_WIDTHS[index], driver)?);
    }
    P::from_pattern(made, &present)
}

/// Drives each signal of `targets` from the signal of `drivers` at the same
/// index.
///
/// The counterpart to [`wires`]: build undriven wires, compute something from
/// them, then drive them. `Design::drive` also closes feedback loops, so this is
/// how a register's input gets attached.
///
/// # Errors
///
/// [`Error::Arity`] if the two lists have different lengths, and whatever
/// [`Design::drive`] reports — in particular a signal from another design.
pub fn assign<P: PortList>(design: &Design, targets: &P, drivers: &P) -> Result<(), Error> {
    let target_signals = targets.to_signals();
    let driver_signals = drivers.to_signals();
    if target_signals.len() != driver_signals.len() {
        return Err(Error::Arity {
            expected: target_signals.len(),
            got: driver_signals.len(),
        });
    }
    for (target, driver) in target_signals.iter().zip(&driver_signals) {
        design.drive(target, driver)?;
    }
    Ok(())
}
