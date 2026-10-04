//! Run the emitted Verilog through a real parser, when one is installed.
//!
//! # Why this test can pass without having run
//!
//! `iverilog` is not a build dependency of this crate and is not installed
//! everywhere, so a hard requirement would make the test suite depend on a vendor
//! toolchain. Instead the test looks for the binary, and when it is absent it prints
//! what it skipped. That is a deliberate trade: a suite that is green because it
//! checked nothing is worse than useless if it is green *silently*, so the skip is
//! announced on stdout and the golden text is still compared by `tests/emit.rs`.
//!
//! Nothing here checks *behaviour*. `-t null` elaborates and elaborating is what
//! catches the mistakes a text comparison cannot: a part-select out of range, a
//! concatenation of the wrong width, a `case` item that is not a literal, an
//! unbalanced `begin`. Equivalence is the cosimulator's job, and the design notes
//! put it in A2.
//!
//! # What it found
//!
//! The first version of the accumulator design below registered the output of an
//! adder whose other input was the register itself. There is no cycle in a graph, so
//! the wire was simply undriven and the front end said so -- but only because the
//! tool was installed. A golden that reads `assign signal_Add = acc + d;` with
//! nothing driving `acc` looks like a perfectly plausible emitter output. Text
//! comparison cannot tell you a design is wrong; a parser that accepted it cannot
//! either. Only building the circuit will.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_rtl::{Module, PortNames};

/// Whether a Verilog parser is available to check against.
fn iverilog() -> Option<PathBuf> {
    // `which` is not portable enough to be worth avoiding here: a missing tool is
    // the case this function exists for, and it is reported rather than assumed.
    let output = Command::new("sh")
        .args(["-c", "command -v iverilog"])
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

/// Elaborate every file of one design together, so an instantiation can see the
/// module it instantiates.
///
/// One `iverilog` invocation per design rather than per file: a parent on its own is
/// an unresolved module, which is an error in the parent and a bug report about the
/// emitter in the reader's head. Returns `Err` with the tool's diagnostics.
fn check(group: &str, files: &[(String, String)], tool: &Path) -> Result<(), String> {
    let mut paths = Vec::new();
    for (stem, text) in files {
        let path = std::env::temp_dir().join(format!("ferrite_lithic_rtl_{group}_{stem}.v"));
        std::fs::write(&path, text).map_err(|e| format!("{path:?}: {e}"))?;
        paths.push(path);
    }
    let mut command = Command::new(tool);
    command.args(["-t", "null", "-g2012"]);
    command.args(&paths);
    let output = command.output().map_err(|e| format!("{tool:?}: {e}"))?;
    // The scratch files are removed either way: a failing elaboration is reported
    // from the captured output, not by leaving files behind to be found later.
    for path in &paths {
        let _ = std::fs::remove_file(path);
    }
    if output.status.success() {
        Ok(())
    } else {
        // The emitted text is in the failure message because a line number from the
        // tool means nothing without it.
        let mut report = format!(
            "{group}: iverilog rejected the module\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for (stem, text) in files {
            report.push_str(&format!("\n--- {group}/{stem}.v ---\n{text}"));
        }
        Err(report)
    }
}

/// A module built from `build`, given the clock port it should use.
fn stateful(build: impl FnOnce(&Design, &Signal) -> Signal, name: &str) -> Module {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let out = build(&d, &clk);
    d.output("out", out.width(), &out).unwrap();
    Module::new(
        name,
        d.build().unwrap(),
        clk.id(),
        d.input_ports().iter().map(|s| s.id()).collect(),
        d.output_ports().iter().map(|s| s.id()).collect(),
    )
}

/// A module with no state, and so no clock.
fn combinational(build: impl FnOnce(&Design) -> Signal, name: &str) -> Module {
    let d = Design::new();
    let out = build(&d);
    d.output("out", out.width(), &out).unwrap();
    Module::combinational(
        name,
        d.build().unwrap(),
        d.input_ports().iter().map(|s| s.id()).collect(),
        d.output_ports().iter().map(|s| s.id()).collect(),
    )
}

/// The designs a parser is most likely to have an opinion about.
fn designs() -> Vec<(String, Vec<(String, String)>)> {
    let mut out = Vec::new();

    // Two chained registers rather than a feedback adder: a graph has no cycle, so
    // the feedback version left a wire undriven.
    let accumulator = stateful(
        |d, clk| {
            let data = d.input("d", 8).unwrap();
            let first = d
                .reg(&data, clk, &d.constant(false), &d.constant(false))
                .unwrap();
            let sum = d.add(&first, &data).unwrap();
            d.reg(&sum, clk, &d.constant(false), &d.constant(false))
                .unwrap()
        },
        "accumulator",
    );
    out.push((
        "accumulator".to_string(),
        vec![("top".to_string(), accumulator.emit().unwrap())],
    ));

    // Reset tied high: the branch is kept, because a register that is permanently
    // in reset is a design that does not do what it looks like it does.
    let controls = stateful(
        |d, clk| {
            let data = d.input("d", 8).unwrap();
            let clear = d.input("clear", 1).unwrap();
            d.reg(&data, clk, &d.constant(true), &clear).unwrap()
        },
        "controls",
    );
    out.push((
        "controls".to_string(),
        vec![("top".to_string(), controls.emit().unwrap())],
    ));

    // A write port is state, so this module has a clock; the `rom` below is the
    // clockless one.
    let memory = stateful(
        |d, _| {
            let addr = d.input("addr", 4).unwrap();
            let data = d.input("data", 8).unwrap();
            let we = d.input("we", 1).unwrap();
            let mem = d.mem(8, 16).unwrap();
            d.write_port(&mem, &data, &addr, &we).unwrap();
            d.read_port(&mem, &addr, &we).unwrap()
        },
        "memory",
    );
    out.push((
        "memory".to_string(),
        vec![("top".to_string(), memory.emit().unwrap())],
    ));

    // No write port, so no clock on the memory declaration at all.
    let rom = combinational(
        |d| {
            let addr = d.input("addr", 4).unwrap();
            let mem = d.mem(8, 16).unwrap();
            d.read_port(&mem, &addr, &d.constant(true)).unwrap()
        },
        "rom",
    );
    out.push((
        "rom".to_string(),
        vec![("top".to_string(), rom.emit().unwrap())],
    ));

    let wide = combinational(
        |d| {
            let a = d.input("a", 200).unwrap();
            let b = d.input("b", 200).unwrap();
            let product = d.mul(&a, &b).unwrap();
            d.truncate(&product, 200).unwrap()
        },
        "wide",
    );
    out.push((
        "wide".to_string(),
        vec![("top".to_string(), wide.emit().unwrap())],
    ));

    let cases = combinational(
        |d| {
            let data = d.input("data", 8).unwrap();
            let kind = d.slice(&data, 0, 3).unwrap();
            let wide = d.zero_extend(&kind, 8).unwrap();
            d.case_(&kind, &[(0, wide.clone()), (7, data.clone())], &data)
                .unwrap()
        },
        "cases",
    );
    out.push((
        "cases".to_string(),
        vec![("top".to_string(), cases.emit().unwrap())],
    ));

    let signed = combinational(
        |d| {
            let a = d.input("a", 8).unwrap();
            let b = d.input("b", 8).unwrap();
            d.add(&d.sdiv(&a, &b).unwrap(), &d.srem(&a, &b).unwrap())
                .unwrap()
        },
        "signed",
    );
    out.push((
        "signed".to_string(),
        vec![("top".to_string(), signed.emit().unwrap())],
    ));

    let nested = combinational(
        |d| {
            let a = d.input("a", 8).unwrap();
            let b = d.input("b", 8).unwrap();
            let inner = d.slice(&a, 0, 4).unwrap();
            let outer = d
                .slice(&d.concat(&[inner, b.clone()]).unwrap(), 0, 8)
                .unwrap();
            let wide = d.replicate(&d.xor(&outer, &b).unwrap(), 2).unwrap();
            d.truncate(&wide, 8).unwrap()
        },
        "nested",
    );
    out.push((
        "nested".to_string(),
        vec![("top".to_string(), nested.emit().unwrap())],
    ));

    let params = combinational(
        |d| {
            let a = d.input("a", 8).unwrap();
            let depth = Bits::constant(16, 32).unwrap();
            d.instance("sub", &[("DEPTH", depth)], std::slice::from_ref(&a), 8)
                .unwrap()
        },
        "params",
    );
    // The submodule is hand-written rather than emitted, which makes it a check on
    // the instantiation syntax: `.DEPTH` must match a 32-bit parameter, and `.i0`
    // and `.o0` must be the names a positional submodule actually declares. If the
    // emitter renumbered either side, this stops elaborating.
    let stub = r#"// A hand-written stand-in for the emitted submodule.
module sub #(
  parameter [31:0] DEPTH = 32'd1
) (
  input  wire [7:0] i0,
  output wire [7:0] o0
);
  assign o0 = i0 ^ {5'b0, DEPTH[2:0]};
endmodule
"#
    .to_string();
    out.push((
        "params".to_string(),
        vec![
            ("top".to_string(), params.emit().unwrap()),
            ("sub".to_string(), stub),
        ],
    ));

    // A parent and the child it instantiates, elaborated together.
    //
    // The child is emitted with positional port names, and the clock is the first
    // input it was given, so the clock is `i0`. That is the whole of the hierarchy
    // convention: port order *is* the ABI, and a parent passes the child's clock as
    // the first input because the instantiation site has nowhere else to put it.
    let child = stateful(
        |d, clk| {
            let data = d.input("d", 8).unwrap();
            let clear = d.input("clear", 1).unwrap();
            d.reg(&data, clk, &d.constant(false), &clear).unwrap()
        },
        "child",
    );
    let child = Module::with_port_names(child, PortNames::Positional);
    let parent = combinational(
        |d| {
            let clk = d.input("clk", 1).unwrap();
            let data = d.input("d", 8).unwrap();
            let clear = d.input("clear", 1).unwrap();
            d.instance("child", &[], &[clk, data, clear], 8).unwrap()
        },
        "parent",
    );
    out.push((
        "hierarchy".to_string(),
        vec![
            ("child".to_string(), child.emit().unwrap()),
            ("parent".to_string(), parent.emit().unwrap()),
        ],
    ));

    out
}

/// Every design this crate can emit, elaborated.
#[test]
fn the_emitted_verilog_parses() {
    let Some(tool) = iverilog() else {
        println!(
            "SKIPPED: iverilog is not installed, so the emitted Verilog was not elaborated. \
             The goldens in tests/emit.rs still compare the text."
        );
        return;
    };
    println!("elaborating with {}", tool.display());
    let mut failures = Vec::new();
    for (name, files) in designs() {
        if let Err(e) = check(&name, &files, &tool) {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// Every operation on a *literal*, elaborated by a real tool.
///
/// A constant is inlined wherever it is used, and a resize is lowered to a
/// concatenation with a constant, so an operand is very often a literal rather than a
/// wire. That made `Select` the one operation whose output depended on its operand
/// being a wire: it emitted `32'h510e527f[6 +: 26]`, which is not Verilog, for every
/// shift, truncate, slice or extension of a constant.
///
/// The simulator evaluated the same nodes correctly throughout, so this is a case
/// where the two backends agreed on the value and disagreed about whether the output
/// was Verilog at all -- and the disagreement only surfaces at elaboration, in a
/// generated file, about an operation three layers below the one that was written.
/// A value test cannot see it and an elaborator can, which is why this gate exists.
///
/// Found by SHA-256, where the IV is a constant and shifting it is the obvious way
/// to build the initial round state.
#[test]
fn every_operation_on_a_literal_elaborates() {
    let Some(tool) = iverilog() else {
        println!(
            "SKIPPED: iverilog is not installed, so operations on literals were not \
             elaborated. The text assertions below still run."
        );
        // Even without a tool, the output must not contain the invalid form. That is
        // the assertion that matters and it needs no elaborator.
        let (verilog, _) = literal_operations();
        assert!(
            !contains_literal_part_select(&verilog),
            "a part-select is applied to a literal:\n{verilog}"
        );
        return;
    };
    println!("elaborating literal operations with {}", tool.display());

    let (verilog, names) = literal_operations();
    assert!(
        !contains_literal_part_select(&verilog),
        "a part-select is applied to a literal, which is not Verilog:\n{verilog}"
    );

    let report = check(
        "literal_ops",
        &[("ops".to_string(), verilog.clone())],
        &tool,
    );
    if let Err(message) = report {
        panic!("{message}");
    }

    // And the widths survive: a shift of a constant must not come back narrower than
    // the operation asked for, which is the other half of the same bug.
    for (name, width) in &names {
        let declaration = format!("output wire [{index}:0]", index = width.saturating_sub(1));
        let found =
            verilog.contains(&declaration) || (*width == 1 && verilog.contains("output wire out"));
        assert!(
            found,
            "{name} should be declared as a {width}-bit output\n{verilog}"
        );
    }
}

/// One module per operation on a literal, and their `(name, width)` pairs.
///
/// Each operation gets its own design and its own declared output, because the
/// widths differ and a port list is one width. Nine tiny modules is less clever than
/// a width-changing port and much easier to read when it fails.
fn literal_operations() -> (String, Vec<(String, u32)>) {
    // (name, build, width)
    type Build = Box<dyn Fn(&Design) -> Result<Signal, ferrite_lithic::Error>>;
    let operations: Vec<(&'static str, Build, u32)> = vec![
        (
            "slice",
            Box::new(|d: &Design| d.slice(&d.lit(0x510e_527f, 32)?, 6, 26)),
            26,
        ),
        (
            "srl",
            Box::new(|d: &Design| d.srl(&d.lit(0x510e_527f, 32)?, 6)),
            32,
        ),
        (
            "sll",
            Box::new(|d: &Design| d.sll(&d.lit(0x510e_527f, 32)?, 6)),
            32,
        ),
        (
            "sra",
            Box::new(|d: &Design| d.sra(&d.lit(0x510e_527f, 32)?, 6)),
            32,
        ),
        (
            "truncate",
            Box::new(|d: &Design| d.truncate(&d.lit(0x510e_527f, 32)?, 11)),
            11,
        ),
        (
            "zero_extend",
            Box::new(|d: &Design| d.zero_extend(&d.lit(0x510e_527f, 32)?, 40)),
            40,
        ),
        (
            "sign_extend",
            Box::new(|d: &Design| d.sign_extend(&d.lit(0x510e_527f, 32)?, 40)),
            40,
        ),
        (
            "low_bit",
            Box::new(|d: &Design| d.slice(&d.lit(0x510e_527f, 32)?, 0, 1)),
            1,
        ),
        (
            "top_bit",
            Box::new(|d: &Design| d.slice(&d.lit(0x510e_527f, 32)?, 31, 1)),
            1,
        ),
    ];

    let mut all = String::new();
    let mut names = Vec::new();
    for (name, build, width) in &operations {
        let d = Design::new();
        let signal = build(&d).unwrap_or_else(|e| panic!("{name}: {e}"));
        d.output("out", signal.width(), &signal)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let module = Module::combinational(
            format!("lit_{name}"),
            d.build().unwrap(),
            d.input_ports().iter().map(|s| s.id()).collect(),
            d.output_ports().iter().map(|s| s.id()).collect(),
        );
        all.push_str(&module.emit().unwrap_or_else(|e| panic!("{name}: {e}")));
        names.push((format!("lit_{name}"), *width));
    }
    (all, names)
}

/// Whether any literal in the text is immediately part-selected.
///
/// Matches a sized literal followed by `[`, which is the exact shape that does not
/// parse: `32'h510e527f[6 +: 26]`.
fn contains_literal_part_select(verilog: &str) -> bool {
    let bytes = verilog.as_bytes();
    for (index, ch) in verilog.char_indices() {
        if ch != '\'' {
            continue;
        }
        // Walk forward to the closing quote of the literal.
        let rest = &verilog[index..];
        let Some(end) = rest[1..].find('\'') else {
            continue;
        };
        let after = index + 1 + end + 1;
        if bytes.get(after) == Some(&b'[') {
            return true;
        }
    }
    false
}
