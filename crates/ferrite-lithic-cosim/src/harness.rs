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
use ferrite_lithic_rtl::{Module, PortNames};

use crate::error::Error;
use crate::stimulus::Stimulus;

/// One port of the module under test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Port {
    /// The port's name, as the emitted Verilog spells it.
    pub name: String,
    /// Its width in bits.
    pub width: u32,
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

    /// The plan for a design and the module emitted from it.
    ///
    /// The port names come from the design, or from the positional scheme if the
    /// module was declared with [`PortNames::Positional`] — and the plan has to
    /// agree with whatever the emitter wrote, because the generated driver
    /// assigns `dut->name` and a name the emitter never declared would be a C++
    /// compile error rather than a disagreement about the design.
    #[must_use]
    pub fn of(design: &Design, module: &Module) -> Self {
        let positional = module.port_names() == PortNames::Positional;
        let inputs: Vec<Port> = design
            .input_ports()
            .iter()
            .enumerate()
            .map(|(index, signal)| Port {
                name: port_name(signal, index, positional),
                width: signal.width(),
            })
            .collect();
        let outputs = design
            .output_ports()
            .iter()
            .enumerate()
            .map(|(index, signal)| Port {
                name: port_name(signal, index, positional),
                width: signal.width(),
            })
            .collect();
        // The clock is a module field, so it is one of the ports, and the plan
        // says which one. Searching the declared inputs for its node id is the
        // only way to get from an id back to a name without the design offering a
        // lookup by id, and it cannot fail to find it: a module with a clock that
        // is not a declared input is a module whose clock nothing drives.
        let clock = module.clock().and_then(|id| {
            let found = design
                .input_ports()
                .iter()
                .position(|signal| signal.id() == id);
            found
                .and_then(|index| inputs.get(index))
                .map(|port| port.name.clone())
        });
        Self {
            top: module.name().to_string(),
            clock,
            inputs,
            outputs,
        }
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

/// A port's name as the emitted module spells it.
fn port_name(signal: &Signal, index: usize, positional: bool) -> String {
    if positional {
        return format!("i{index}");
    }
    signal_name(signal, index)
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
/// The stimulus is a stream of whitespace-separated hex words, `inputs.len()`
/// words per cycle, and the output is one hex word per output port per cycle,
/// space separated and newline terminated. A blank line is skipped rather than
/// counted, so a trailing newline in the stimulus file does not produce a cycle
/// of nothing.
#[must_use]
pub fn driver_source(plan: &Plan) -> String {
    let class = format!("V{}", plan.top);
    let mut source = String::new();

    source.push_str("// Generated by ferrite-lithic-cosim. Do not edit: it is rebuilt\n");
    source.push_str("// from the module's port list every time.\n");
    source.push_str("//\n");
    source.push_str("// Registers start at zero here because Verilator zero-initialises its\n");
    source.push_str("// state, and that is what ferrite-lithic-sim does too. It is a property\n");
    source.push_str("// of the two tools, not of the design.\n");
    source.push_str("#include \"verilated.h\"\n");
    source.push_str(&format!("#include \"{class}.h\"\n\n"));
    source.push_str(
        r#"#include <cstdio>
#include <cstdlib>

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
    source.push_str(&format!("  {class}* dut = new {class}(&context);\n"));

    match &plan.clock {
        Some(clock) => {
            source.push_str(&format!("  dut->{clock} = 0;\n"));
            source.push_str("  dut->eval();\n");
        }
        None => source.push_str(
            "  // No clock: a combinational module has no edge, so each cycle is one\n  \
             // settle.\n",
        ),
    }

    source.push_str("\n  char word[512];\n  unsigned cycle = 0;\n");
    source.push_str(&format!("  const int ports = {};\n", plan.inputs.len()));

    source.push_str(
        r#"  while (ports == 0 || std::fscanf(in, "%511s", word) == 1) {
    // Drive the clock low first: inputs are applied while it is low, so the
    // next rising edge sees them already in place.
"#,
    );
    if let Some(clock) = &plan.clock {
        source.push_str(&format!("    dut->{clock} = 0;\n    dut->eval();\n"));
    }
    for index in 0..plan.inputs.len() {
        let port = &plan.inputs[index];
        source.push_str(&format!(
            "    dut->{name} = std::strtoull(word, nullptr, 16) & mask<{width}>;\n",
            name = port.name,
            width = port.width,
        ));
        if index + 1 < plan.inputs.len() {
            source.push_str(
                "    if (std::fscanf(in, \"%511s\", word) != 1) { \\\n      \
                 std::fprintf(stderr, \"stimulus ended mid-cycle %u\\n\", cycle); \\\n      \
                 return 2; }\\n",
            );
        }
    }
    source.push_str(
        r#"
    // The rising edge, then the read. Reading after the edge is what makes this
    // comparable with the simulator's `after` snapshot.
"#,
    );
    if let Some(clock) = &plan.clock {
        source.push_str(&format!("    dut->{clock} = 1;\n    dut->eval();\n"));
    } else {
        source.push_str("    dut->eval();\n");
    }

    for index in 0..plan.outputs.len() {
        if index > 0 {
            source.push_str("    std::printf(\" \");\n");
        }
        let port = &plan.outputs[index];
        source.push_str(&format!(
            "    std::printf(\"%0{}llx\", (unsigned long long)(dut->{name} & mask<{width}>));\n",
            port.width.div_ceil(4),
            name = port.name,
            width = port.width,
        ));
    }
    source.push_str("    std::printf(\"\\n\");\n");
    source.push_str("    cycle++;\n  }\n\n");
    source.push_str("  std::fclose(in);\n  dut->final();\n  delete dut;\n");
    source.push_str("  return 0;\n}\n\n");
    source.push_str(
        r#"// A port wider than 64 bits is not representable in this harness's I/O, and
// truncating it would make a wide design cosimulate against the wrong thing.
// So the driver refuses to build for one rather than quietly comparing low words.
template <int W> static unsigned long long mask() {
  static_assert(W <= 64, "ferrite-lithic-cosim drives ports up to 64 bits wide");
  return W >= 64 ? ~0ULL : ((1ULL << W) - 1);
}
"#,
    );
    source
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
            .arg("--x-initial")
            .arg("zero")
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
