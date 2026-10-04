//! The four step futures.
//!
//! Every one of them does its work on its first poll and then completes, which is
//! the property that lets [`Testbench::run`](crate::Testbench::run) be a single poll
//! rather than an executor. See the crate docs for why a cycle boundary is work
//! rather than a suspension.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use ferrite_lithic_bits::Bits;

use crate::Shared;

/// Writes a step's kind and target, which is what is worth seeing when a test fails
/// and prints a future. The handle itself is not interesting and `Shared` holds a
/// simulator, so it is named rather than formatted.
fn debug_step(
    f: &mut fmt::Formatter<'_>,
    kind: &str,
    port: Option<&str>,
    done: bool,
) -> fmt::Result {
    f.debug_struct(kind)
        .field("port", &port)
        .field("applied", &done)
        .finish()
}

/// Applies an input value and takes one rising edge.
///
/// The error is recorded rather than returned because `.await` on a `Result` would
/// put a `?` in every step of every test, which is the plumbing this crate exists to
/// remove. [`Testbench::run`](crate::Testbench::run) reports it.
#[must_use = "a step does nothing unless it is awaited"]
pub struct Drive {
    pub(crate) shared: Rc<Shared>,
    pub(crate) port: String,
    /// `None` when the port was already found to be wrong, in which case the step
    /// still takes an edge so the cycle count stays what the test asked for.
    pub(crate) value: Option<Bits>,
    done: bool,
}

impl Drive {
    pub(crate) fn new(shared: Rc<Shared>, port: String, value: Option<Bits>) -> Self {
        Self {
            shared,
            port,
            value,
            done: false,
        }
    }
}

impl Future for Drive {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(());
        }
        this.done = true;
        if let Some(value) = &this.value {
            this.shared.set_input(&this.port, value);
        }
        this.shared.step();
        Poll::Ready(())
    }
}

/// A rising edge with no input change.
///
/// For a design that is already being fed: a free-running counter, an LFSR with no
/// data input, or a handshake that needs to settle.
#[must_use = "a step does nothing unless it is awaited"]
pub struct Edge {
    pub(crate) shared: Rc<Shared>,
    done: bool,
}

impl Edge {
    pub(crate) fn new(shared: Rc<Shared>) -> Self {
        Self {
            shared,
            done: false,
        }
    }
}

impl Future for Edge {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(());
        }
        this.done = true;
        this.shared.step();
        Poll::Ready(())
    }
}

/// Asserts a one-bit input for one cycle, then releases it for one.
///
/// Both edges happen inside a single poll. They cannot be spread over two polls,
/// because a `Future` may be polled any number of times and this testbench polls
/// once: a future that needed a second poll would look exactly like one that
/// suspends.
///
/// Two cycles rather than one, so the assertion is a cycle wide. A reset asserted
/// for the interval *between* two samples is a reset a recorded waveform cannot
/// show happening.
#[must_use = "a pulse does nothing unless it is awaited"]
pub struct Pulse {
    pub(crate) shared: Rc<Shared>,
    pub(crate) port: String,
    done: bool,
}

impl Pulse {
    pub(crate) fn new(shared: Rc<Shared>, port: String) -> Self {
        Self {
            shared,
            port,
            done: false,
        }
    }
}

impl Future for Pulse {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(());
        }
        this.done = true;
        for value in [1u64, 0] {
            // A one-bit constant always exists, so this cannot fail.
            let bit = Bits::constant(value, 1).expect("a one-bit constant is representable");
            this.shared.set_input(&this.port, &bit);
            this.shared.step();
        }
        Poll::Ready(())
    }
}

/// Settles the combinational logic with no edge.
///
/// For a register-free design, where there is no clock to step. On a design with a
/// register it is harmless but usually a mistake: the registers will not have
/// updated, so a value read afterwards is the pre-edge one.
#[must_use = "a settle does nothing unless it is awaited"]
pub struct Settle {
    pub(crate) shared: Rc<Shared>,
    done: bool,
}

impl Settle {
    pub(crate) fn new(shared: Rc<Shared>) -> Self {
        Self {
            shared,
            done: false,
        }
    }
}

impl Future for Settle {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(());
        }
        this.done = true;
        if let Err(error) = this.shared.comb() {
            this.shared.fail(crate::Error::Sim(Box::new(error)));
        }
        Poll::Ready(())
    }
}
impl fmt::Debug for Drive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_step(f, "Drive", Some(self.port.as_str()), self.done)
    }
}

impl fmt::Debug for Edge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_step(f, "Edge", None, self.done)
    }
}

impl fmt::Debug for Pulse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_step(f, "Pulse", Some(self.port.as_str()), self.done)
    }
}

impl fmt::Debug for Settle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_step(f, "Settle", None, self.done)
    }
}
