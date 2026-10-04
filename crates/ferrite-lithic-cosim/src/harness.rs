//! Building and running the emitted Verilog under Verilator.
//!
//! # What is generated, and why
//!
//! Two files: the emitted Verilog, and a C++ `main` that drives it. The `main` is
//! generated rather than checked in because it is *about* a module's port list —
//! it assigns each input port from one field and prints each output port — and a
//! checked-in driver would be a second, hand-maintained copy of the port list
//! that silently goes stale.
//!
//! # The cycle boundary that is compared
//!
//! One cycle is: drive the clock low, apply the inputs, drive it high, evaluate,
//! read the outputs. The read happens *after* the rising edge, so what it sees is
//! the state the edge produced — which is `Sim::step`'s second snapshot. That is
//! the only boundary both backends can be made to agree about; reading before the
//! edge would compare the simulator's `before` against Verilator's `after` and
//! report a divergence on every register.
//!
//! # Registers start at zero in both
//!
//! Verilator zero-initialises its state, and `Sim` allocates its machine
//! zero-filled. Neither is a design property, so `Harness::build` says so in the
//! C++ it generates, and [`Plan::initial_values`] exists for a caller that wants
//! to check something other than the reset state.

use std::path::{Path, PathBuf};
use std::process::Command;

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_rtl::{Module, PortNames, mangle_port_name};

use crate::error::Error;
use crate::stimulus::Stimulus;

/// One port of the module under test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Port {
    /// The port's name as the *design* knows it, which is what the simulator takes.
    pub name: String,
    /// The port's name as the emitted Verilog declares it, which the driver assigns.
    ///
    /// Equal to `name` unless legalisation changed it: the emitter escapes a keyword,
    /// so a port called `byte` is declared `byte_`. One name cannot serve both
    /// backends, and using one anyway produces a driver that references a member the
    /// Verilator class does not have -- an error that names the emitter rather than
    /// the plan.
    pub verilog: String,
    /// Its width in bits.
    pub width: u32,
}

impl Port {
    /// A port whose emitted name is its design name.
    ///
    /// Prefer [`Plan::of`] unless the plan is hand-built, because only the emitter
    /// knows what it will actually write.
    #[must_use]
    pub fn new(name: impl Into<String>, width: u32) -> Self {
        let name = name.into();
        Self {
            verilog: name.clone(),
            name,
            width,
        }
    }

    /// Sets the emitted name, for a plan built by hand against known Verilog.
    #[must_use]
    pub fn with_verilog(mut self, verilog: impl Into<String>) -> Self {
        self.verilog = verilog.into();
        self
    }
}

/// Everything the generated driver needs to know about a module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    /// The top module's name, which is also the Verilator class name (`V` + it).
    pub top: String,
    /// The clock port, if the module has one.
    pub clock: Option<String>,
    /// The input ports, in port order.
    pub inputs: Vec<Port>,
    /// The output ports, in port order.
    pub outputs: Vec<Port>,
}

impl Plan {
    /// The input widths, in port order.
    #[must_use]
    pub fn input_widths(&self) -> Vec<u32> {
        self.inputs.iter().map(|port| port.width).collect()
    }

    /// The output names, in port order.
    #[must_use]
    pub fn output_names(&self) -> Vec<String> {
        self.outputs.iter().map(|port| port.name.clone()).collect()
    }

    /// A plan from its parts, refusing a clock that is also a stimulus column.
    ///
    /// # Errors
    ///
    /// [`Error::ClockIsAStimulusColumn`] if `clock` also appears in `inputs`.
    ///
    /// This is the one invariant the whole harness rests on, so it is checked where
    /// a plan is built rather than left to the generated C++. A clock in the
    /// stimulus list means the driver applies the input columns and *then* overwrites
    /// the clock from a stimulus word: if that word is 1 the clock is already high
    /// when the driver raises it again, so no edge happens and no register updates.
    /// That failure is silent — every output simply never changes — and it is the
    /// sort of bug that a text comparison of the driver cannot see, because the
    /// generated text is exactly what was asked for.
    pub fn new(
        top: impl Into<String>,
        clock: Option<String>,
        inputs: Vec<Port>,
        outputs: Vec<Port>,
    ) -> Result<Self, Error> {
        let plan = Self {
            top: top.into(),
            clock,
            inputs,
            outputs,
        };
        if let Some(clock) = &plan.clock
            && plan.inputs.iter().any(|port| &port.name == clock)
        {
            return Err(Error::ClockIsAStimulusColumn {
                clock: clock.clone(),
            });
        }
        Ok(plan)
    }

    /// The plan for a design and the module emitted from it.
    ///
    /// The port names come from the design, or from the positional scheme if the
    /// module was declared with [`PortNames::Positional`] — and the plan has to
    /// agree with whatever the emitter wrote, because the generated driver
    /// assigns `dut->name` and a name the emitter never declared would be a C++
    /// compile error rather than a disagreement about the design.
    ///
    /// **The clock is a port but is not a stimulus column.** It is excluded from
    /// `inputs` for the reason [`Plan::new`] gives, and the design is still emitted
    /// with the clock as a module port — only the stimulus file leaves it out.
    ///
    /// # Errors
    ///
    /// Whatever [`Module::port_identifiers`] reports, which is what [`Module::emit`]
    /// reports: an unnamed port, an empty name, or a graph that fails its own checks.
    pub fn of(design: &Design, module: &Module) -> Result<Self, Error> {
        let positional = module.port_names() == PortNames::Positional;
        let clock_id = module.clock();
        // The emitted identifiers, which is what the driver will assign. Positional
        // modules do not use them, and asking anyway would fail on an unnamed port.
        let emitted = if positional {
            Vec::new()
        } else {
            module.port_identifiers()?
        };
        let all_inputs = design.input_ports();
        let declared: Vec<&Signal> = all_inputs.iter().collect();
        // The clock is a module field, so it is one of the ports, and the plan
        // says which one. Searching the declared inputs for its node id is the
        // only way to get from an id back to a name without the design offering a
        // lookup by id, and it cannot fail to find it: a module with a clock that
        // is not a declared input is a module whose clock nothing drives.
        let clock = clock_id.and_then(|id| {
            declared
                .iter()
                .position(|signal| signal.id() == id)
                .map(|index| port_name(declared[index], index, positional, &emitted))
        });
        let inputs = declared
            .iter()
            .enumerate()
            .filter(|(_, signal)| Some(signal.id()) != clock_id)
            .map(|(index, signal)| Port {
                name: signal_name(signal, index),
                verilog: emitted
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| port_name(signal, index, positional, &emitted)),
                width: signal.width(),
            })
            .collect();
        let outputs = design
            .output_ports()
            .iter()
            .enumerate()
            .map(|(index, signal)| {
                let offset = declared.len() + index;
                Port {
                    name: signal_name(signal, index),
                    verilog: emitted
                        .get(offset)
                        .cloned()
                        .unwrap_or_else(|| port_name(signal, index, positional, &emitted)),
                    width: signal.width(),
                }
            })
            .collect();
        Ok(Self {
            top: module.name().to_string(),
            clock,
            inputs,
            outputs,
        })
    }

    /// Where [`Harness::build`] should put its files.
    ///
    /// Under the system temporary directory rather than the source tree: a build
    /// directory is generated, and a generated directory that can be committed by
    /// accident is a directory somebody will commit by accident. Keyed by module
    /// name so two modules do not overwrite each other's `dut.v`.
    #[must_use]
    pub fn work_dir(&self) -> PathBuf {
        std::env::temp_dir()
            .join("ferrite-lithic-cosim")
            .join(&self.top)
    }

    /// The node id of the clock, for building a [`Module`].
    ///
    /// # Errors
    ///
    /// [`Error::ClockNotDeclared`] if the plan names a clock the design does not
    /// declare as an input. A module whose clock is not a port is one nothing can
    /// drive, and finding that out here is much cheaper than finding it out in
    /// Verilator's output.
    pub fn clock_id(&self, design: &Design) -> Result<Option<ferrite_lithic_ir::NodeId>, Error> {
        let Some(clock) = &self.clock else {
            return Ok(None);
        };
        design
            .input_ports()
            .iter()
            .find(|signal| signal.name().as_deref() == Some(clock.as_str()))
            .map(|signal| Some(signal.id()))
            .ok_or_else(|| Error::ClockNotDeclared {
                clock: clock.clone(),
                known: design
                    .input_ports()
                    .iter()
                    .filter_map(|signal| signal.name())
                    .collect(),
            })
    }

    /// A zero of each register, for a caller that wants to state the reset state
    /// rather than inherit it.
    ///
    /// Deliberately only the widths: this is the reset state both backends already
    /// have, so it exists for a test to assert *that* rather than to change it.
    #[must_use]
    pub fn initial_values(&self) -> Vec<Bits> {
        self.outputs
            .iter()
            .map(|port| Bits::zeros(port.width).expect("a declared width is never zero"))
            .collect()
    }
}

/// A port's name, as the emitted module spells it.
///
/// `emitted` is the emitter's own answer when it has one. [`mangle_port_name`] is
/// the fallback for a hand-built plan, and it cannot reproduce a collision suffix --
/// two ports whose names both legalise to `a_b` come out as `a_b` and `a_b_1` --
/// which is exactly why [`Plan::of`] asks the emitter instead.
fn port_name(signal: &Signal, index: usize, positional: bool, emitted: &[String]) -> String {
    if positional {
        return format!("i{index}");
    }
    emitted
        .get(index)
        .cloned()
        .unwrap_or_else(|| mangle_port_name(&signal_name(signal, index)))
}

/// A signal's own name, or a positional stand-in when it has none.
fn signal_name(signal: &Signal, index: usize) -> String {
    signal.name().unwrap_or_else(|| format!("port {index}"))
}

/// Whether a Verilator binary is available to check against.
///
/// The same trade as the Icarus gate: a hard requirement would make the test
/// suite depend on a vendor toolchain, and a suite that is green because it
/// checked nothing must at least *say* so.
#[must_use]
pub fn verilator() -> Option<PathBuf> {
    let output = Command::new("sh")
        .args(["-c", "command -v verilator"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

/// The C++ driver for one plan.
///
/// Read this to know what the harness does; it is short because the port list is
/// the only thing that varies.
///
/// # The file format
///
/// A decimal cycle count on the first line, then whitespace-separated hex words,
/// `inputs.len()` per cycle. The output is one hex word per output port per cycle,
/// space separated and newline terminated.
///
/// The count is there for two reasons, both of which were bugs first. Without it
/// the loop has to end at end-of-file, which cannot work for a module with **no**
/// input ports: there is nothing to read, so the loop has no way to know a cycle
/// happened, and a free-running design — a counter, an LFSR with no data in — has
/// no inputs at all. The naive fix (`while (ports == 0 || ...)`) loops forever
/// instead. And a count in both directions catches a stimulus file that says one
/// number of cycles and holds another, which end-of-file silently accepts.
///
/// # What this file had wrong before Verilator was installed
///
/// The template function was emitted *after* `main`, which is a C++ compile error
/// the moment a port is driven; the first version of this was never compiled, only
/// compared as text, and a text comparison of a C++ file is exactly the check that
/// cannot see a declaration-order rule. The clock was also listed as a stimulus
/// column, which made the driver overwrite its own clock between the low and high
/// phases. Both are fixed, and the second is now impossible to reintroduce because
/// [`Plan::new`] refuses the shape.
#[must_use]
pub fn driver_source(plan: &Plan) -> String {
    let class = format!("V{}", plan.top);
    let mut out = String::new();
    out.push_str("// Generated by ferrite-lithic-cosim. Do not edit: it is rebuilt\n");
    out.push_str("// from the module's port list every time.\n//\n");
    out.push_str("// Registers start at zero here because Verilator is built with\n");
    out.push_str("// --x-initial 0, and that is what ferrite-lithic-sim does too. It is a\n");
    out.push_str("// property of the two tools, not of the design.\n");
    out.push_str("#include \"verilated.h\"\n");
    out.push_str(&format!("#include \"{class}.h\"\n\n"));
    out.push_str("#include <cstdio>\n#include <cstdlib>\n\n");
    // Before main, not after: C++ requires a name to be declared before its first
    // use, so a definition below main is a compile error the moment a port is
    // driven. This was the first version of this file's mistake -- the template
    // sat at the bottom, and nothing noticed because the driver was compared as
    // text and never compiled.
    //
    // `if constexpr` rather than a ternary because the untaken branch of a ternary
    // is still checked, and `1ULL << 64` in a discarded branch is undefined
    // behaviour the compiler is entitled to complain about.
    out.push_str(
        r#"// A port wider than 64 bits is not representable in this harness's I/O, and
// truncating it would make a wide design cosimulate against the wrong thing. So
// the driver refuses to build for one rather than quietly comparing low words.
template <int W> static unsigned long long mask() {
  static_assert(W >= 1 && W <= 64,
                "ferrite-lithic-cosim drives ports up to 64 bits wide");
  if constexpr (W == 64) {
    return ~0ULL;
  } else {
    return (1ULL << W) - 1;
  }
}

int main(int argc, char** argv) {
  if (argc < 2) {
    std::fprintf(stderr, "usage: %s <stimulus>\n", argv[0]);
    return 2;
  }
  std::FILE* in = std::fopen(argv[1], "r");
  if (in == nullptr) {
    std::perror("stimulus");
    return 2;
  }

  VerilatedContext context;
"#,
    );
    out.push_str(&format!("  {class}* dut = new {class}(&context);\n"));

    if let Some(clock) = &plan.clock {
        out.push_str(&format!("  dut->{clock} = 0;\n  dut->eval();\n"));
    } else {
        out.push_str(
            "  // No clock: a combinational module has no edge, so one cycle is one\n  \
             // settle.\n",
        );
    }

    // The count is first so a module with no input ports still has a defined
    // number of cycles, and so a file whose length disagrees with its contents is
    // an error rather than a short run.
    out.push_str(
        "\n  unsigned long cycles = 0;\n  \
         if (std::fscanf(in, \"%lu\", &cycles) != 1) {\n    \
         std::fprintf(stderr, \"stimulus has no cycle count\\n\");\n    return 2;\n  }\n\n  \
         char word[512];\n  for (unsigned long cycle = 0; cycle < cycles; cycle++) {\n",
    );

    // Clock low, then inputs, so the edge that follows sees them in place.
    if let Some(clock) = &plan.clock {
        out.push_str(&format!("    dut->{clock} = 0;\n    dut->eval();\n"));
    }
    for port in &plan.inputs {
        out.push_str(&format!(
            "    if (std::fscanf(in, \"%511s\", word) != 1) {{\n      \
             std::fprintf(stderr, \"stimulus ended in cycle %lu of %lu (port {name})\\n\", \
             cycle, cycles);\n      return 2;\n    }}\n",
            name = port.name,
        ));
        out.push_str(&format!(
            "    dut->{name} = std::strtoull(word, nullptr, 16) & mask<{width}>();\n",
            name = port.verilog,
            width = port.width,
        ));
    }

    // The rising edge, then the read. Reading after the edge is what makes this
    // comparable with the simulator's `after` snapshot. The clock is raised only
    // here: it is not a stimulus column, so nothing between the low phase and
    // this line can have left it high.
    if let Some(clock) = &plan.clock {
        out.push_str(&format!("    dut->{clock} = 1;\n"));
    }
    out.push_str("    dut->eval();\n\n");

    for (index, port) in plan.outputs.iter().enumerate() {
        if index > 0 {
            out.push_str("    std::printf(\" \");\n");
        }
        out.push_str(&format!(
            "    std::printf(\"%0{}llx\", (unsigned long long)(dut->{name} & mask<{width}>()));\n",
            port.width.div_ceil(4),
            name = port.verilog,
            width = port.width,
        ));
    }
    out.push_str("    std::printf(\"\\n\");\n  }\n\n");

    // A count that understates the file is as wrong as one that overstates it,
    // and end-of-file would have accepted either.
    out.push_str(
        "  if (std::fscanf(in, \"%511s\", word) == 1) {\n    \
         std::fprintf(stderr, \"stimulus has words past its %lu declared cycles\\n\", cycles);\n    \
         return 2;\n  }\n\n",
    );
    out.push_str("  std::fclose(in);\n  dut->final();\n  delete dut;\n  return 0;\n}\n");
    out
}
/// A built Verilator model and its driver, on disk.
#[derive(Clone, Debug)]
pub struct Harness {
    binary: PathBuf,
    root: PathBuf,
}

impl Harness {
    /// Verilates `verilog` and links the generated driver against it.
    ///
    /// # Errors
    ///
    /// [`Error::NoVerilator`] if there is no Verilator, [`Error::Io`] if the
    /// files cannot be written, and [`Error::Process`] with Verilator's own
    /// diagnostics if the build fails.
    pub fn build(plan: &Plan, verilog: &str, root: &Path) -> Result<Self, Error> {
        let verilator = verilator().ok_or(Error::NoVerilator)?;
        std::fs::create_dir_all(root)?;

        let source = root.join("dut.v");
        let driver = root.join("main.cpp");
        let object = root.join("obj");
        let binary = object.join("cosim");

        std::fs::write(&source, verilog)?;
        std::fs::write(&driver, driver_source(plan))?;

        let output = Command::new(&verilator)
            .current_dir(root)
            .arg("--cc")
            .arg("--exe")
            .arg("--build")
            .arg("-j")
            .arg("0")
            // Warnings are information here: a synthesiser's warning about an
            // unused bit is not a reason to refuse to check the design.
            .arg("-Wno-fatal")
            // Two-state, zero-initialised, which is what `Sim` does as well. Said
            // here so the two backends cannot silently differ on reset state.
            //
            // Pinned rather than left at the default, and that matters more than it
            // looks: Verilator's default is `unique`, which randomises the initial
            // value per run to shake out designs that depend on uninitialised
            // state. That is the right default for Verilator's own tests and the
            // wrong one here, because `Sim` zero-fills and a design that relies on
            // the reset state would then diverge at random rather than every time.
            // The argument is `0`, not `zero`.
            .arg("--x-initial")
            .arg("0")
            .arg("--top-module")
            .arg(&plan.top)
            .arg("dut.v")
            .arg("main.cpp")
            .arg("-Mdir")
            .arg(&object)
            .arg("-o")
            .arg(&binary)
            .output()?;

        if !output.status.success() {
            return Err(Error::Process {
                what: format!("verilator --build {}", plan.top),
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(Self {
            binary,
            root: root.to_path_buf(),
        })
    }

    /// Runs the harness over a stimulus, returning each cycle's output values.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] writing the stimulus, [`Error::Process`] if the harness
    /// itself fails, and [`Error::HexWord`] if it printed something that is not
    /// hex — which means the binary is not the one [`build`](Harness::build)
    /// produced.
    pub fn run(&self, plan: &Plan, stimulus: &Stimulus) -> Result<Vec<Vec<Bits>>, Error> {
        let encoded = stimulus.encode(&plan.input_widths())?;
        let path = self.root.join("stimulus.txt");
        std::fs::write(&path, encoded)?;

        let output = Command::new(&self.binary).arg(&path).output()?;
        if !output.status.success() {
            return Err(Error::Process {
                what: format!("cosim harness {}", self.binary.display()),
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let text = String::from_utf8_lossy(&output.stdout);
        let mut cycles = Vec::with_capacity(stimulus.cycles());
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.len() != plan.outputs.len() {
                return Err(Error::Arity {
                    cycle: cycles.len() as u64,
                    expected: plan.outputs.len(),
                    got: words.len(),
                });
            }
            let mut values = Vec::with_capacity(words.len());
            for (index, word) in words.iter().enumerate() {
                values.push(crate::stimulus::parse_hex(word, plan.outputs[index].width)?);
            }
            cycles.push(values);
        }
        Ok(cycles)
    }
}
