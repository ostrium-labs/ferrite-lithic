# 0018. Cosimulation compares after the clock edge, and the driver is generated

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic-cosim`, `ferrite-lithic-sim`, `ferrite-lithic-rtl`

## Context

ADR-0010 deferred Verilator to A2, where `ferrite-lithic-cosim` checks the
emitted Verilog against `ferrite-lithic-sim`. Three things about that comparison
had to be decided, and each has a failure mode that is quiet rather than loud.

**Which cycle boundary.** `Sim` is a three-phase cycle (`src/cyclesim.ml:32-46`):
drive the clock high and settle, commit registers and memories, drive it low and
settle again. It returns *two* snapshots — `before` and `after`. Verilator, driven
by a C++ `main`, has no such notion: there is a signal and an `eval()`. Comparing
the simulator's `before` against a Verilator read after the rising edge would put
the two backends one register update apart on every registered output, and the
report would be a wall of divergences that says nothing about the design.

**Who owns the clock.** `Sim::before_clock_edge` writes `1` into the clock's word
itself and `after_clock_edge` writes `0`. The stimulus's clock column is therefore
overwritten by the simulator and is documentation, not data — while Verilator's
`clk` *is* driven from the file. Two different mechanisms that have to produce the
same edge count.

**The reset state.** Verilator zero-initialises its state. `Sim` allocates its
machine zero-filled. Both start at zero, so the two agree, but neither is a property
of the design: it is a coincidence of the two tools that would break the moment
either changed.

## Decision

**Compare after the edge.** The generated driver does clock low, apply inputs,
clock high, `eval()`, then read the outputs. That is `Sim::step`'s `after`
snapshot. `simulate` takes `step()`'s second return value and ignores the first,
and says why.

**Drive the clock once per cycle, in both backends, from the plan's clock port.**
The plan names the clock, the driver toggles it, and the simulator drives its own.
The stimulus still carries a value for the clock column because the module declares
it as an input, but nothing reads it. That is stated in the crate docs rather than
left to be discovered.

**State the reset assumption instead of relying on it.** The generated C++ carries
a comment saying registers start at zero because Verilator zero-initialises and
`Sim` does too, and `Plan::initial_values` exists so a caller can assert the
reset state rather than inherit it. A future `--x-initial random` or a `Sim` that
started registers at `x` would then be a loud failure.

**Generate the driver from the port list.** The C++ `main` assigns each input port
from one field and prints each output port, so it is *about* the port list; a
checked-in driver would be a second hand-maintained copy of it that goes stale
silently. `driver_source` is a pure function of a `Plan`, which means the driver is
checked as text without compiling it.

**Refuse a port wider than 64 bits, in the generated source.** The driver's I/O is
`unsigned long long`, so a wider port would be silently truncated and the
cosimulation would compare the wrong value against the right one. The refusal is a
`static_assert` rather than a runtime check, so the failure is a compile error in the
harness where the stack trace points at the design.

## Consequences

- One rising edge per cycle on both sides, by construction rather than by
  coincidence.
- A register's new value is observable in the same cycle on both sides, so a test
  can assert on a register the cycle it changes.
- Wide designs (more than 64 bits on a port) cannot be cosimulated yet. The error is
  explicit and names the limit.
- The generated C++ is checked as text. That is most of what can go wrong without a
  compiler, and it is not the same as compiling it — see `DETAILS.md` for what is
  still unrun.

## Alternatives considered

- **Read the outputs before the edge**, matching a `before`-snapshot reading. Rejected:
  it disagrees with Verilator on every register, so the report is noise.
- **Have `Sim` not drive the clock**, so the stimulus column is the single source of
  the edge. Rejected: `Sim`'s three-phase cycle is the settled execution model
  (Hardcaml has the same one) and giving it up for symmetry would be a large change
  to the crate most worth trusting.
- **Compare every snapshot phase**, not just one. Rejected for now: it triples the
  comparison with no additional signal, because a design that disagrees at `after`
  has already failed, and one that disagrees only at `before` disagrees at `after`
  too.
