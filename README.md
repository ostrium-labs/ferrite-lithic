Ferrite Lithic
==============

An embedded hardware DSL, a cycle simulator, and an RTL generator, in Rust.

Hardcaml's model, reimplemented natively. Not a wrapper, not a binding, not a
transpiler from another language: a Rust library you call from Rust.

Status
------

**Phase 0 (design).** No crates are published yet. The design notes that settle
the IR, the simulator's execution model, and the Verilog emitter's contract are
in `docs/design-notes.md`; the crate split below is the intended shape, not a
promise about what exists.

Crate layout
------------

| Crate | Purpose | Phase |
|---|---|---|
| `ferrite-lithic-bits` | Fixed-width bitvectors over `u64` words, runtime widths | A1 |
| `ferrite-lithic-ir` | Signal graph: arena of nodes, `NodeId` indices, wires resolved after construction so cycles are legal | A1 |
| `ferrite-lithic` | Front-end DSL: `Signal` handle with operator overloads and builder functions | A1 |
| `ferrite-lithic-sim` | Cycle simulator over a flat, topologically sorted op list | A1 |
| `ferrite-lithic-rtl` | Verilog emitter with stable naming | A1 |
| `ferrite-lithic-derive` | `#[derive]` for port-list structs, `map`/`iter`/`of_signal` | A2 |
| `ferrite-lithic-wave` | VCD waveform writer | A2 |
| `ferrite-lithic-cosim` | Verilator equivalence checking against `ferrite-lithic-sim` | A2 |
| `ferrite-lithic-tb` | Coroutine/async-style step testbenches | A3 |

Design decisions
----------------

1. **Runtime widths, not const generics.** Const-generic widths give
   compile-time checks but make graph construction in loops and generators
   painful. We keep runtime checks and invest in the error messages instead.
2. **Embedded DSL, not a new language.** The Hardcaml model. The
   new-language route is already covered by Spade and others.
3. **Own IR first.** Emitting FIRRTL or CIRCT could reuse their optimisers and
   Verilog backends but adds a heavy dependency. Revisit at the end of Phase A2
   with real designs in hand.
4. **Port the design, write tests from behaviour.** Hardcaml is MIT (Jane
   Street), so this is permitted and attributed in `NOTICE`. We take the
   architecture and the semantics, not the code.

Scope
-----

Non-goals, deliberately: writing a new HDL language, and compiling arbitrary
Rust programs to hardware. The bridge to Strata covers a dataflow subset with
static shapes, not general program synthesis.

Relationship to Strata
----------------------

The FPGA path in [`ferrite-strata`](../ferrite-strata) is implemented as a
Strata backend: graph in, bitstream out. That backend is the first real test of
Strata's closed-vendor-plugin idea, because Lithic behaves like a vendor with
its own architecture and its own compiler.

Licence
-------

Apache-2.0. See `LICENSE` and `NOTICE`.
