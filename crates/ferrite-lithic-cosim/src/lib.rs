//! Cosimulation: the emitted Verilog, checked against the simulator.
//!
//! # The comparison this crate exists for
//!
//! Three backends read the same signal graph — [`ferrite_lithic_sim`], the
//! Verilog emitter, and this one, which runs the emitted Verilog under Verilator
//! and checks it against the simulator cycle by cycle. ADR-0010 deferred
//! Verilator to A2; this is A2's half of that. Hardcaml's own harness lives in
//! the separate `janestreet/hardcaml_verilator` repository, so the harness here
//! is net-new work rather than a port, and its natural first targets are the
//! register and memory designs — the ones that already have both a definition and
//! an expected output.
//!
//! # Why it is split the way it is
//!
//! Verilator is a vendor toolchain and most contributors will not have it. So the
//! crate is arranged so that **everything except the build and the run** is
//! ordinary Rust that the test suite exercises unconditionally:
//!
//! - [`Stimulus`] is the single list of input values both backends are driven
//!   from, so no disagreement can be blamed on two different input sequences.
//! - [`harness::driver_source`] is the generated C++, and it is a pure function of
//!   the [`Plan`], so the driver is checked without compiling it.
//! - [`compare()`] is pure, so the comparison and its failure reporting are checked
//!   without either backend.
//! - Only [`harness::Harness::build`] and [`run`](harness::Harness::run) need the
//!   toolchain, and the integration tests there skip — loudly, on stdout — when
//!   `verilator` is absent.
//!
//! ```
//! use ferrite_lithic_cosim::{compare, Plan, Port, Stimulus};
//!
//! // A module with one 8-bit input and one 8-bit output.
//! let plan = Plan {
//!     top: "passthrough".to_string(),
//!     clock: Some("clk".to_string()),
//!     inputs: vec![Port { name: "d".to_string(), width: 8 }],
//!     outputs: vec![Port { name: "q".to_string(), width: 8 }],
//! };
//! let mut stimulus = Stimulus::new();
//! stimulus.push(vec![ferrite_lithic_bits::Bits::constant(0xa5, 8).unwrap()]).unwrap();
//!
//! // Compared here rather than under Verilator: the comparison is the part with
//! // the arithmetic in it, and it does not need a toolchain to be right.
//! let simulator = vec![vec![ferrite_lithic_bits::Bits::constant(0xa5, 8).unwrap()]];
//! let report = compare(&plan.output_names(), &simulator, &simulator).unwrap();
//! assert!(report.is_equivalent());
//! assert_eq!(report.cycles, 1);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod equivalence;
pub mod error;
pub mod generator;
pub mod harness;
pub mod stimulus;

pub use equivalence::{Divergence, Source, compare};
pub use error::{Error, Mismatch, Report};
pub use harness::{Harness, Plan, Port, driver_source, verilator};
pub use stimulus::Stimulus;

use ferrite_lithic::Design;
use ferrite_lithic_sim::Sim;

/// One stimulus, run through both backends.
///
/// # Errors
///
/// [`Error::Sim`] if the simulator rejects an input name or value, and whatever
/// [`compare()`](compare) reports for a length disagreement. A *value* disagreement
/// is not an error here: it is a [`Report`], because reporting all the divergences
/// of a run is more useful than stopping at the first.
pub fn run(design: &Design, plan: &Plan, stimulus: &Stimulus) -> Result<Report, Error> {
    // Verilating a design is the slow part by orders of magnitude, so a caller
    // running several stimuli against one design should build once and use
    // `run_with`. This convenience does not, because a convenience that silently
    // rebuilds per call is how a test suite goes from four seconds to four
    // minutes.
    let harness = Harness::build(plan, &emit(design, plan)?, &plan.work_dir())?;
    run_with(design, plan, stimulus, &harness)
}

/// One stimulus against an already-built harness.
///
/// # Errors
///
/// Whatever [`run`] reports, and [`Error::NoVerilator`] never — that is the
/// point of taking the harness.
pub fn run_with(
    design: &Design,
    plan: &Plan,
    stimulus: &Stimulus,
    harness: &Harness,
) -> Result<Report, Error> {
    let simulator = simulate(design, plan, stimulus)?;
    let verilator = harness.run(plan, stimulus)?;
    compare(&plan.output_names(), &simulator, &verilator)
}

/// The simulator's side of a stimulus, cycle by cycle.
///
/// # Errors
///
/// [`Error::Sim`] if the simulator refuses a port name or value.
pub fn simulate(
    design: &Design,
    plan: &Plan,
    stimulus: &Stimulus,
) -> Result<Vec<Vec<ferrite_lithic_bits::Bits>>, Error> {
    let mut sim = Sim::new(design).map_err(|inner| Error::Sim(Box::new(inner)))?;
    let mut cycles = Vec::with_capacity(stimulus.cycles());
    for cycle in 0..stimulus.cycles() as u64 {
        let Some(inputs) = stimulus.cycle(cycle) else {
            break;
        };
        for (port, value) in plan.inputs.iter().zip(inputs) {
            sim.set_input(&port.name, value.clone())
                .map_err(|inner| Error::Sim(Box::new(inner)))?;
        }
        // The *second* snapshot, which is what a Verilator read after the rising
        // edge sees. Comparing the first would put the two backends a register
        // update apart on every port that is one.
        let (_before, after) = sim.step().map_err(|inner| Error::Sim(Box::new(inner)))?;
        cycles.push(
            after
                .outputs()
                .iter()
                .map(|(_, value)| value.clone())
                .collect(),
        );
    }
    Ok(cycles)
}

/// The Verilog for a plan's module.
///
/// # Errors
///
/// [`Error::Emit`] if the emitter refuses the design.
pub fn emit(design: &Design, plan: &Plan) -> Result<String, Error> {
    let inputs: Vec<ferrite_lithic_ir::NodeId> =
        design.input_ports().iter().map(|s| s.id()).collect();
    let outputs: Vec<ferrite_lithic_ir::NodeId> =
        design.output_ports().iter().map(|s| s.id()).collect();
    let module = match plan.clock_id(design)? {
        Some(clock) => {
            ferrite_lithic_rtl::Module::new(&plan.top, design.build()?, clock, inputs, outputs)
        }
        None => {
            ferrite_lithic_rtl::Module::combinational(&plan.top, design.build()?, inputs, outputs)
        }
    };
    Ok(module.emit()?)
}
