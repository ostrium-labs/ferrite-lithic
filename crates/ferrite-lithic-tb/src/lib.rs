//! Step testbenches: a test that reads as a sequence of clocked steps.
//!
//! # What this is for
//!
//! A2's assertions are written directly against
//! [`WaveData`], which means a test drives the
//! simulator by index and counts cycles itself:
//!
//! ```text
//! sim.set_input("d", ...)?;
//! sim.step()?;
//! assert_eq!(sim.peek("q")?, 5);
//! ```
//!
//! That is correct and it is unpleasant. Every step is three statements, the cycle
//! count is implicit in how many times a loop body ran, and the interesting part --
//! *what the design should do* -- is buried between plumbing calls.
//!
//! So a test here is an `async` block where **every `.await` is one rising edge**:
//!
//! ```no_run
//! # use ferrite_lithic::Design;
//! # use ferrite_lithic_tb::Testbench;
//! # fn build() -> Design { Design::new() }
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let tb = Testbench::new(&build())?;
//! tb.run(|tb| async move {
//!     tb.pulse("rst").await;
//!     tb.drive("d", 5).await;
//!     assert_eq!(tb.value("q"), 5);
//!     tb.drive("d", 0).await;
//!     assert_eq!(tb.value("q"), 5, "the accumulator holds");
//! })?;
//! # Ok(())
//! # }
//! ```
//!
//! The cycle count is now the number of `.await`s, and that is the property that
//! matters: a test that has to count `step()` calls to know where it is has already
//! lost the thing it was meant to express.
//!
//! # These futures do not suspend
//!
//! An `async` block usually means "may suspend, to be resumed by something else".
//! Here it does not, and the reason is worth stating because it reads like a
//! limitation and is the opposite: **advancing a cycle is work, not a yield.**
//! [`Testbench::run`] drives the body on one thread and resolves every cycle
//! boundary itself, so a future provided by this crate does its work on the first
//! poll and then completes. There is no executor to suspend into and nothing to
//! wake.
//!
//! The upshot is that this needs no nightly `generators`, no `async-trait`, no
//! per-step boxed allocation and no second thread. It also means [`Testbench::run`]
//! *refuses* a future that does return `Pending` ([`Error::Suspended`]) rather than
//! hanging on it, because a foreign future waiting for a wake is waiting for
//! something this testbench will never do.
//!
//! # Errors surface when the body returns
//!
//! A step cannot return `Result`, because then every `.await` would need a `?` and
//! the plumbing would come straight back. So a failing step records the first error
//! and [`Testbench::run`] reports it when the body completes -- which also means a
//! body that recorded a failure and *then* returned a value is a failed run, not a
//! passing one. `run` checks for that first, deliberately.
//!
//! # Where the boundary is read
//!
//! After the edge, like the cosimulator (ADR-0018). [`Handle::value`] reads the
//! settled post-edge value, so the first `.await` in a test already sees the effect
//! of that edge. Both backends agree on this, and neither can be made to agree on a
//! pre-edge read.
//!
//! # Recording
//!
//! Every step records the post-edge value of every output port, so [`Testbench::series`]
//! gives a cycle-indexed series with no extra setup. The ports are registered from
//! the first snapshot rather than from a list written out by hand, because a
//! hand-written port list is a second copy of the design's to keep in step.
//!
//! [`WaveData`]: ferrite_lithic_wave::WaveData

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_sim::Sim;
use ferrite_lithic_wave::{Port as WavePort, WaveData};

use crate::edge::{Drive, Edge, Pulse, Settle};

mod edge;
mod error;

pub use crate::edge::{Drive as Step, Edge as StepEdge, Pulse as StepPulse, Settle as SettleOnly};
pub use crate::error::Error;

/// The simulator, its port list, and the first recorded failure.
///
/// `Rc<RefCell<_>>` rather than anything thread-safe, on purpose: a testbench is
/// single-threaded by construction, and the `async` syntax is the only concurrency
/// this crate offers. Sharing would buy nothing and would cost the ability to hold a
/// non-`Send` handle.
pub(crate) struct Shared {
    sim: RefCell<Sim>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    error: RefCell<Option<Error>>,
    waves: RefCell<WaveData>,
    recorded: RefCell<bool>,
}

impl Shared {
    /// The width of a declared port, by name.
    fn width_of(&self, name: &str) -> Option<u32> {
        self.sim
            .borrow()
            .program()
            .port(name)
            .map(|port| port.width)
    }

    /// Records the first failure. Later ones are dropped, because the first is the
    /// one that explains the rest.
    pub(crate) fn fail(&self, error: Error) {
        let mut slot = self.error.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    fn take_failure(&self) -> Option<Error> {
        self.error.borrow_mut().take()
    }

    fn unknown(&self, port: &str) -> Error {
        Error::UnknownPort {
            name: port.to_string(),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
        }
    }

    /// Applies an input value, recording rather than returning a failure.
    pub(crate) fn set_input(&self, port: &str, value: &Bits) {
        match self.width_of(port) {
            None => self.fail(self.unknown(port)),
            Some(width) if value.width() != width => self.fail(Error::PortWidth {
                port: port.to_string(),
                expected: width,
                got: value.width(),
            }),
            Some(_) => {
                if let Err(error) = self.sim.borrow_mut().set_input(port, value.clone()) {
                    self.fail(Error::Sim(Box::new(error)));
                }
            }
        }
    }

    /// Settles the combinational logic without an edge.
    pub(crate) fn comb(&self) -> Result<(), ferrite_lithic_sim::Error> {
        self.sim.borrow_mut().comb()
    }

    /// One cycle.
    ///
    /// A cycle is a rising edge when the design has a clock, and a settle when it
    /// does not -- the same rule the cosimulator's generated driver follows, where a
    /// clockless module gets an `eval()` instead of an edge. Without this a
    /// register-free design could not be driven at all, because every step would
    /// fail on `NoClock`; and a testbench that cannot drive a combinational design
    /// is a testbench that can only test half the corpus.
    pub(crate) fn step(&self) {
        if self.sim.borrow().program().clock().is_none() {
            let settled = self.sim.borrow_mut().comb();
            match settled {
                Ok(()) => match self.sim.borrow_mut().initial() {
                    Ok(snapshot) => self.record(&snapshot),
                    Err(error) => self.fail(Error::Sim(Box::new(error))),
                },
                Err(error) => self.fail(Error::Sim(Box::new(error))),
            }
            return;
        }
        let outcome = self.sim.borrow_mut().step();
        match outcome {
            Ok((_before, after)) => self.record(&after),
            Err(error) => self.fail(Error::Sim(Box::new(error))),
        }
    }

    /// Records every output port's post-edge value.
    ///
    /// The ports are registered on the first cycle because the snapshot is where the
    /// names come from. Registering them up front from a written-out list would make
    /// that list a second port list to keep in step with the design's.
    fn record(&self, snapshot: &ferrite_lithic_sim::Snapshot) {
        let mut waves = self.waves.borrow_mut();
        if !*self.recorded.borrow() {
            for (name, value) in snapshot.outputs() {
                if let Err(error) = waves.register(name, value.width()) {
                    self.fail(Error::Wave(Box::new(error)));
                    return;
                }
            }
            *self.recorded.borrow_mut() = true;
        }
        if let Err(error) = waves.record(snapshot.cycle(), |name| snapshot.get(name).cloned()) {
            self.fail(Error::Wave(Box::new(error)));
        }
    }
}

/// A design under test, with a cycle count you do not have to keep.
///
/// Cheap to clone. The cycle count keeps running across `run` calls, which is what
/// makes a testbench a sequence of *phases* rather than a pile of unrelated designs:
/// a reset phase, a data phase and a drain phase can each be their own `run` and
/// still be one continuous simulation.
#[derive(Clone)]
pub struct Testbench {
    shared: Rc<Shared>,
}

impl std::fmt::Debug for Testbench {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Testbench")
            .field("cycle", &self.cycle())
            .finish_non_exhaustive()
    }
}

impl Testbench {
    /// Compiles a design into a simulator and starts it at cycle zero.
    ///
    /// # Errors
    ///
    /// [`Error::Sim`] if the design does not compile or does not settle. Registers
    /// start at zero, which is a property of the simulator and of Verilator rather
    /// than of the design (ADR-0018).
    pub fn new(design: &Design) -> Result<Self, Error> {
        let sim = Sim::new(design)?;
        let inputs = design
            .input_ports()
            .iter()
            .filter_map(|signal| signal.name())
            .collect();
        let outputs = design
            .output_ports()
            .iter()
            .filter_map(|signal| signal.name())
            .collect();
        Ok(Self {
            shared: Rc::new(Shared {
                sim: RefCell::new(sim),
                inputs,
                outputs,
                error: RefCell::new(None),
                waves: RefCell::new(WaveData::new("top")),
                recorded: RefCell::new(false),
            }),
        })
    }

    /// A handle for use inside an `async` body.
    #[must_use]
    pub fn handle(&self) -> Handle {
        Handle {
            shared: Rc::clone(&self.shared),
        }
    }

    /// Runs an `async` body to completion, then reports the first failure.
    ///
    /// # Errors
    ///
    /// [`Error::Suspended`] if the body returned `Pending`, and otherwise the first
    /// failure any step recorded. A recorded failure is reported *in preference to*
    /// the body's value: a body that failed and then returned something is a failed
    /// run, and handing back the value would hide that.
    pub fn run<Fut, T>(&self, body: impl FnOnce(Handle) -> Fut) -> Result<T, Error>
    where
        Fut: Future<Output = T>,
    {
        let mut body = Box::pin(body(self.handle()));
        match poll_once(&mut body) {
            Poll::Ready(value) => match self.shared.take_failure() {
                Some(error) => Err(error),
                None => Ok(value),
            },
            Poll::Pending => Err(Error::Suspended),
        }
    }

    /// Edges applied since the design was created.
    #[must_use]
    pub fn cycle(&self) -> u64 {
        self.shared.sim.borrow().cycle()
    }

    /// The recorded value of an output port, cycle by cycle.
    ///
    /// Empty before the first step: there is nothing to record until then.
    #[must_use]
    pub fn series(&self, port: &str) -> Vec<Bits> {
        self.shared
            .waves
            .borrow()
            .series_named(port)
            .unwrap_or_default()
    }

    /// Every recorded output port name.
    #[must_use]
    pub fn recorded_ports(&self) -> Vec<String> {
        let waves = self.shared.waves.borrow();
        let ports: Vec<WavePort> = waves.ports().to_vec();
        ports.iter().map(|port| port.name().to_string()).collect()
    }

    /// The recorded series of every output port, for a VCD.
    #[must_use]
    pub fn waves(&self) -> Vec<(String, Vec<Bits>)> {
        let names = self.recorded_ports();
        names
            .into_iter()
            .map(|name| {
                let series = self.series(&name);
                (name, series)
            })
            .collect()
    }

    /// The first failure recorded so far, without clearing it.
    #[must_use]
    pub fn failure(&self) -> Option<String> {
        self.shared.error.borrow().as_ref().map(ToString::to_string)
    }
}

/// What an `async` body drives the simulator with.
///
/// Cheap to clone, and the only thing a body needs: it refers to the same
/// simulator as the [`Testbench`] it came from.
#[derive(Clone)]
pub struct Handle {
    shared: Rc<Shared>,
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle").finish_non_exhaustive()
    }
}

impl Handle {
    /// Applies an input value and takes one rising edge.
    ///
    /// The width is resolved against the port here rather than on the await, so a
    /// `u64` is checked against the design's actual port widths instead of being
    /// quietly truncated into whatever the value's own width happened to be.
    pub fn drive(&self, port: &str, value: u64) -> Drive {
        let bits = match self.shared.width_of(port) {
            None => {
                self.shared.fail(self.shared.unknown(port));
                None
            }
            Some(width) if width == 0 => {
                self.shared.fail(Error::PortWidth {
                    port: port.to_string(),
                    expected: width,
                    got: 1,
                });
                None
            }
            Some(width) if width > 64 => {
                self.shared.fail(Error::TooWide {
                    port: port.to_string(),
                    width,
                });
                None
            }
            Some(width) => match Bits::constant(value, width) {
                Ok(bits) => Some(bits),
                Err(_) => {
                    // `Bits::constant` refuses a width of zero and a value wider
                    // than the width; both are already excluded above, so this is
                    // unreachable and is reported rather than papered over.
                    self.shared.fail(Error::PortWidth {
                        port: port.to_string(),
                        expected: width,
                        got: 0,
                    });
                    None
                }
            },
        };
        Drive::new(Rc::clone(&self.shared), port.to_string(), bits)
    }

    /// Applies an input value of an explicit width and takes one rising edge.
    ///
    /// For a port wider than 64 bits, and for a value that is already a [`Bits`].
    pub fn drive_bits(&self, port: &str, value: Bits) -> Drive {
        Drive::new(Rc::clone(&self.shared), port.to_string(), Some(value))
    }

    /// A rising edge with no input change.
    pub fn step(&self) -> Edge {
        Edge::new(Rc::clone(&self.shared))
    }

    /// Asserts a one-bit input for one cycle, then releases it for one.
    pub fn pulse(&self, port: &str) -> Pulse {
        Pulse::new(Rc::clone(&self.shared), port.to_string())
    }

    /// Settles the combinational logic without an edge.
    ///
    /// For a register-free design. On a design with a register it is harmless but
    /// usually a mistake: the registers will not have updated.
    pub fn settle(&self) -> Settle {
        Settle::new(Rc::clone(&self.shared))
    }

    /// The settled value of a signal, without taking an edge.
    ///
    /// For combinational outputs, for reading an input back, and for inspecting a
    /// registered output *before* the next edge. For the value a test most often
    /// wants after an edge, [`Handle::value`] is the same read -- the design is
    /// already settled after a step, so the two agree.
    #[must_use]
    pub fn peek(&self, port: &str) -> Bits {
        match self.shared.sim.borrow_mut().peek(port) {
            Ok(bits) => bits,
            Err(_) => {
                self.shared.fail(self.shared.unknown(port));
                Bits::constant(0, 1).expect("a one-bit constant is representable")
            }
        }
    }

    /// [`Handle::peek`] as a `u64`.
    ///
    /// Never panics. An unknown port, or one wider than 64 bits, records a failure
    /// and yields zero, so the assertion downstream reports the real problem rather
    /// than the symptom.
    #[must_use]
    pub fn value(&self, port: &str) -> u64 {
        let bits = self.peek(port);
        match bits.to_u64() {
            Ok(value) => value,
            Err(_) => {
                self.shared.fail(Error::TooWide {
                    port: port.to_string(),
                    width: bits.width(),
                });
                0
            }
        }
    }

    /// The recorded series of an output port, from inside a body.
    #[must_use]
    pub fn series(&self, port: &str) -> Vec<Bits> {
        self.shared
            .waves
            .borrow()
            .series_named(port)
            .unwrap_or_default()
    }

    /// The cycle count as the body sees it.
    #[must_use]
    pub fn cycle(&self) -> u64 {
        self.shared.sim.borrow().cycle()
    }
}

/// Polls a future exactly once.
///
/// Once, not until `Ready`: looping would turn "this future never completes" into a
/// hang instead of [`Error::Suspended`], and a hang is the one outcome a test suite
/// cannot report.
fn poll_once<Fut: Future>(future: &mut Pin<Box<Fut>>) -> Poll<Fut::Output> {
    // `Waker::noop` rather than a hand-written one: this crate's futures never
    // suspend, so a wake is not a thing that can happen, and the standard library
    // already says that.
    let mut cx = Context::from_waker(Waker::noop());
    future.as_mut().poll(&mut cx)
}
