# Step testbenches: `.await` is one clock edge

## Status

Accepted. Supersedes nothing; refines ADR-0018.

## Context

A2 replaced Hardcaml's golden *files* with cycle-indexed values
(`ferrite-lithic-wave`). That is the right substitution for assertions, but the
*drivers* were still written by hand against `ferrite-lithic-sim`, and a hand-written
driver is three statements per step:

```rust
sim.set_input("d", value)?;
sim.step()?;
assert_eq!(sim.peek("q")?, expected);
```

which is correct and unpleasant. The cycle count is implicit in how many times a
loop body ran, the interesting part of the test is buried between plumbing calls,
and a design with a handshake needs the test to say when it asserted `valid` and
when it expected `ready` — bookkeeping that becomes the reader's job.

Hardcaml's answer is OCaml coroutines: a testbench function runs until it yields,
the driver resumes it at each clock edge, and the test reads as straight-line code
with `yield` where the edge is. That is a good fit and OCaml has built-in support
for it. Rust has three ways to spell it and none of them are built in:

- **`async`/`await`.** Stable, but a future is only resumed by something that polls
  it, so `.await` has to mean "the thing that resumes me", and the natural
  implementation needs a waker, a task queue, and a thread.
- **Generators.** `feature(generators)` is nightly, and the project has an MSRV of
  1.97 stable.
- **Threads.** One scoped thread per testbench, with the simulator driven by the
  other end. This is the standard trick and it works — but it needs the simulator
  to be `Send`, which it is not, and it makes the failure mode of a testbench bug a
  deadlock rather than an error.

## Decision

**`ferrite-lithic-tb` gives an `async` body to the testbench and defines every
`.await` as one rising edge, with no executor.**

The key observation is that **advancing a cycle is work, not a yield.** A step
takes an edge, records what came out, and completes. There is nothing to wait for
and nothing to wake, so a future provided by this crate does its work on its first
poll and returns `Ready`.

That makes `Testbench::run` a *single poll* rather than a loop:

```rust
pub fn run<Fut, T>(&self, body: impl FnOnce(Handle) -> Fut) -> Result<T, Error>
where
    Fut: Future<Output = T>,
```

No `Waker::from(Arc<...>)` per step, no boxed-per-step allocation, no second
thread, no nightly. `Waker::noop()` is correct because a wake is not a thing that
can happen here.

Because there is no executor, `run` **refuses** a future that does return `Pending`
(`Error::Suspended`) instead of spinning until it completes. A foreign future
waiting for a wake is waiting for something this testbench will never do, and a
test run that never returns is the one outcome a suite cannot report.

## Consequences

**A step cannot return `Result`.** Otherwise every `.await` needs a `?` and the
plumbing comes straight back. A failing step records the first error and `run`
reports it when the body completes. This also means a body that recorded a failure
and *then* returned a value is a **failed** run — `run` checks for that before
handing back the value, deliberately.

**The read is after the edge.** `Handle::value` reads the settled post-edge value,
so the first `.await` in a test already sees the effect of that edge. This is
ADR-0018's rule and it is not negotiable: neither backend can be made to agree on a
pre-edge read.

**A design with no clock settles instead of stepping.** A "cycle" for a
register-free design is a settle, which is what the cosimulator's generated driver
already did. Without this, half the corpus would be undrivable, because every step
would fail with `NoClock`.

**Recording is free and always on.** Every step records the post-edge value of
every output port into a `WaveData`, so a series is available with no setup. The
ports are registered from the first snapshot rather than from a list written out by
hand, because a hand-written port list is a second copy of the design's.

**The cycle count keeps running across `run` calls.** A testbench is a sequence of
phases — reset, data, drain — and each phase being its own `run` is what makes a
long test readable. Restarting the count per call would make that worse.

## What this buys

The corpus in `ferrite-lithic-corpus` exists because a tool tested with inputs
chosen by whoever wrote it drifts toward what the tool was built to do. A design
written the way hardware wants it is the other direction. `crc3` — a three-bit
LFSR, one byte per cycle — is the first one, and it found four defects that 471
tests of the tools had not:

- `Design::sll` and `Design::srl` were **exactly swapped** for the whole of A1 and
  A2, and `sra` with them. `srl` built `{slice, zeros}`, which is a *left* shift of
  the right width. The builder tests checked the width a shift produced, which is
  the one thing a swapped shift still gets right.
- The `cat` property in `tests/ops.rs` set its low half to **zeros**, which cannot
  distinguish "shifted into place" from "left where it was" — shifting zero moves
  nothing. So the one property covering the operation every width-changing op is
  built from could not fail.
- `WaveData::record` padded skipped cycles with zeros, which directly contradicted
  `value`'s documented promise that an unrecorded cycle "reads as absent rather than
  as zero". Since the simulator numbers its first edge cycle 1, every series from a
  simulator began with a fabricated zero row.
- The emitter's reserved-word list was missing `byte` (and `break`, `chandle`,
  `throw`, `assume`), so a port called `byte` emitted `input wire [7:0] byte` — a
  syntax error in every tool that matters, and one nothing noticed because nothing
  had ever been named `byte`.

None of those is a subtle numerical bug. Every one is a case where a test asserted
the property that was easy to assert rather than the property that mattered, which
is the argument for the corpus crate and against adding more examples to the
existing suites.