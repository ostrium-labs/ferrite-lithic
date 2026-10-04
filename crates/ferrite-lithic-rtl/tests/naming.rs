//! The naming contract, tested through the only thing a caller can see: the text.
//!
//! The name map itself is private, and that is on purpose. A name map tested
//! directly would be tested against itself; every assertion here reads the emitted
//! Verilog and checks that the identifier a reader will see is the one the contract
//! promises. The three states, and what each one guarantees, are documented on
//! [`naming`](crate::naming); the tests are grouped by state.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::Circuit;
use ferrite_lithic_rtl::{Error, Module, PortNames};

/// Build a combinational module from `build`, with its single output named `out`.
fn emit(name: &str, build: impl FnOnce(&Design) -> Signal) -> String {
    let d = Design::new();
    let out = build(&d);
    d.output("out", out.width(), &out).unwrap();
    module(&d, name).emit().unwrap()
}

/// The combinational [`Module`] for a design's declared ports.
fn module(d: &Design, name: &str) -> Module {
    Module::combinational(
        name,
        d.build().unwrap(),
        d.input_ports().iter().map(|s| s.id()).collect(),
        d.output_ports().iter().map(|s| s.id()).collect(),
    )
}

/// State 1: ports keep their own names, whatever they are.
#[test]
fn a_port_keeps_its_name_verbatim() {
    let text = emit("ports", |d| {
        let a = d.input("a", 8).unwrap();
        let b = d.input("b", 8).unwrap();
        d.add(&a, &b).unwrap()
    });
    assert!(text.contains("input  wire [7:0] a,"), "{text}");
    assert!(text.contains("input  wire [7:0] b,"), "{text}");
}

/// A port named after a Verilog keyword is escaped, because a port list has to
/// parse before anything else about the module can matter.
#[test]
fn a_port_named_after_a_reserved_word_is_escaped() {
    let text = emit("reserved", |d| {
        let clk = d.input("always", 1).unwrap();
        d.not(&clk)
    });
    assert!(text.contains("input  wire [0:0] always_,"), "{text}");
    assert!(!text.contains("[0:0] always,"), "{text}");
}

/// State 2: a user name beats a derived one, and the *first* allocation of a name is
/// the one that keeps it.
#[test]
fn an_explicit_name_beats_a_derived_one() {
    let d = Design::new();
    let a = d.input("a", 8).unwrap();
    let b = d.input("b", 8).unwrap();
    // Three chained adds, the last of which is named. The user's name still wins,
    // because a *named* signal is allocated before any unnamed one — the derived
    // names start after it, not before.
    let first = d.add(&a, &b).unwrap();
    let second = d.add(&first, &b).unwrap();
    let named = d.add(&second, &b).unwrap();
    d.set_name(&named, "total").unwrap();
    let out = d.add(&named, &a).unwrap();
    d.output("out", 8, &out).unwrap();
    let text = module(&d, "named").emit().unwrap();
    assert!(text.contains("wire [7:0] total;"), "{text}");
    assert!(text.contains("wire [7:0] signal_Add;"), "{text}");
    assert!(text.contains("wire [7:0] signal_Add_1;"), "{text}");
    assert!(text.contains("assign signal_Add = a + b;"), "{text}");
    assert!(
        text.contains("assign signal_Add_1 = signal_Add + b;"),
        "{text}"
    );
    assert!(text.contains("assign total = signal_Add_1 + b;"), "{text}");
}

/// State 3: unnamed signals are named for their kind, in graph order rather than
/// depth-first.
#[test]
fn unnamed_signals_are_named_for_their_kind_in_graph_order() {
    let text = emit("kinds", |d| {
        let a = d.input("a", 8).unwrap();
        let b = d.input("b", 8).unwrap();
        let sum = d.add(&a, &b).unwrap();
        let inverted = d.not(&sum);
        let anded = d.and(&inverted, &b).unwrap();
        d.or(&anded, &a).unwrap()
    });
    // Construction order, not the order a depth-first walk reaches them: the `Not`
    // comes *after* the `Add` it depends on, and its declaration follows it.
    let add = text.find("wire [7:0] signal_Add;").unwrap();
    let not = text.find("wire [7:0] signal_Not;").unwrap();
    let bitand = text.find("wire [7:0] signal_BitAnd;").unwrap();
    let bitor = text.find("wire [7:0] signal_BitOr;").unwrap();
    assert!(add < not && not < bitand && bitand < bitor, "{text}");
}

/// Collisions append a numeric suffix, starting at 1.
#[test]
fn colliding_derived_names_get_a_numeric_suffix() {
    let text = emit("collide", |d| {
        let a = d.input("a", 8).unwrap();
        let b = d.input("b", 8).unwrap();
        let first = d.add(&a, &b).unwrap();
        let second = d.add(&first, &b).unwrap();
        d.add(&second, &b).unwrap()
    });
    assert!(text.contains("wire [7:0] signal_Add;"), "{text}");
    assert!(text.contains("wire [7:0] signal_Add_1;"), "{text}");
    assert!(text.contains("wire [7:0] signal_Add_2;"), "{text}");
}

/// Matching is case-insensitive, which is stricter than Verilog needs and is the
/// point: `acc` and `ACC` are two signals to a reader and one hazard to a tool.
#[test]
fn collision_matching_ignores_case() {
    let text = emit("cases", |d| {
        let a = d.input("a", 8).unwrap();
        let b = d.input("b", 8).unwrap();
        let first = d.add(&a, &b).unwrap();
        d.set_name(&first, "acc").unwrap();
        let second = d.add(&first, &b).unwrap();
        d.set_name(&second, "ACC").unwrap();
        d.add(&second, &b).unwrap()
    });
    assert!(text.contains("wire [7:0] acc;"), "{text}");
    assert!(text.contains("wire [7:0] ACC_1;"), "{text}");
}

/// An identifier that cannot be spelled in Verilog is rewritten, not refused: a
/// rewritten name is visible in a diff, a refusal is only visible in an error.
#[test]
fn an_illegal_identifier_is_rewritten() {
    let text = emit("illegal", |d| {
        let a = d.input("a", 8).unwrap();
        let b = d.input("b", 8).unwrap();
        let sum = d.add(&a, &b).unwrap();
        d.set_name(&sum, "9lives").unwrap();
        let other = d.add(&sum, &b).unwrap();
        d.set_name(&other, "a-b").unwrap();
        other
    });
    assert!(text.contains("wire [7:0] _9lives;"), "{text}");
    assert!(text.contains("wire [7:0] a_b;"), "{text}");
}

/// An empty name is refused, because there is nothing to sanitise and no way to
/// tell it apart from "no name at all".
#[test]
fn an_empty_name_is_refused() {
    let mut circuit = Circuit::new();
    let a = circuit.wire(8).unwrap();
    let b = circuit.wire(8).unwrap();
    circuit.set_name(a, "a").unwrap();
    circuit.set_name(b, "b").unwrap();
    let sum = circuit.add(a, b).unwrap();
    let out = circuit.wire(8).unwrap();
    circuit.set_name(out, "out").unwrap();
    circuit.set_name(sum, "").unwrap();
    circuit.drive(out, sum).unwrap();
    let err = Module::combinational("empty", circuit, vec![a, b], vec![out])
        .emit()
        .unwrap_err();
    assert!(matches!(err, Error::EmptySignalName { .. }), "{err:?}");
}

/// An instance label is allocated from the same table as a signal, because a label
/// colliding with a signal is a Verilog error.
#[test]
fn instance_labels_come_from_the_same_namespace() {
    let text = emit("labels", |d| {
        let a = d.input("a", 8).unwrap();
        let first = d.instance("sub", &[], std::slice::from_ref(&a), 8).unwrap();
        let second = d.instance("sub", &[], &[a], 8).unwrap();
        d.concat(&[first, second]).unwrap()
    });
    assert!(text.contains("u_Instance ("), "{text}");
    assert!(text.contains("u_Instance_1 ("), "{text}");
}

/// Positional port names are reserved before any user name, so a signal called `i0`
/// is renamed rather than allowed to shadow a port and break the binding.
#[test]
fn positional_port_names_are_reserved_before_user_names() {
    let d = Design::new();
    let a = d.input("a", 8).unwrap();
    let b = d.input("b", 8).unwrap();
    let sum = d.add(&a, &b).unwrap();
    d.set_name(&sum, "i0").unwrap();
    d.output("out", 8, &sum).unwrap();
    let text = module(&d, "positional")
        .with_port_names(PortNames::Positional)
        .emit()
        .unwrap();
    assert!(text.contains("input  wire [7:0] i0,"), "{text}");
    assert!(text.contains("output wire [7:0] o0"), "{text}");
    assert!(text.contains("wire [7:0] i0_1;"), "{text}");
    assert!(text.contains("assign o0 = i0_1;"), "{text}");
}

/// A port with no name has no ABI, so there is nothing to derive one from.
#[test]
fn an_unnamed_port_is_refused() {
    let mut circuit = Circuit::new();
    let a = circuit.wire(8).unwrap();
    let b = circuit.wire(8).unwrap();
    let sum = circuit.add(a, b).unwrap();
    let out = circuit.wire(8).unwrap();
    circuit.drive(out, sum).unwrap();
    // `a` is a port and has no name: nothing to put in the port list.
    let err = Module::combinational("unnamed", circuit, vec![a, b], vec![out])
        .emit()
        .unwrap_err();
    assert!(matches!(err, Error::UnnamedPort { .. }), "{err:?}");
}

/// A node listed as both an input and an output is an `inout`, which this emitter
/// does not model.
#[test]
fn a_port_in_both_directions_is_refused() {
    let mut circuit = Circuit::new();
    let a = circuit.wire(8).unwrap();
    let b = circuit.wire(8).unwrap();
    let sum = circuit.add(a, b).unwrap();
    let out = circuit.wire(8).unwrap();
    circuit.drive(out, sum).unwrap();
    let err = Module::combinational("both", circuit, vec![a, b], vec![a, out])
        .emit()
        .unwrap_err();
    assert!(
        matches!(err, Error::PortIsBothDirections { id } if id == a),
        "{err:?}"
    );
}

/// A literal whose width is not a multiple of four still renders correctly, because
/// Verilog keeps the *low* bits of an over-wide literal.
#[test]
fn a_literal_is_truncated_to_its_width() {
    let mut circuit = Circuit::new();
    let a = circuit.wire(5).unwrap();
    circuit.set_name(a, "a").unwrap();
    let b = circuit.constant(Bits::constant(0x1f, 5).unwrap());
    let sum = circuit.add(a, b).unwrap();
    let out = circuit.wire(5).unwrap();
    circuit.set_name(out, "out").unwrap();
    circuit.drive(out, sum).unwrap();
    let text = Module::combinational("narrow", circuit, vec![a], vec![out])
        .emit()
        .unwrap();
    assert!(text.contains("5'h1f"), "{text}");
}

/// A one-bit signal is declared `[0:0]` rather than as a scalar net, and a port's
/// declared width is its own.
#[test]
fn widths_are_declared_as_ranges() {
    let text = emit("widths", |d| {
        let a = d.input("a", 1).unwrap();
        let wide = d.zero_extend(&a, 17).unwrap();
        d.concat(&[wide, a]).unwrap()
    });
    assert!(text.contains("input  wire [0:0] a"), "{text}");
    assert!(text.contains("output wire [17:0] out"), "{text}");
    assert!(text.contains("wire [16:0] signal_Cat;"), "{text}");
    assert!(text.contains("wire [0:0] signal_Select;"), "{text}");
}

/// A positional module needs no port names at all, which is the ergonomic half of
/// why the scheme exists: a submodule can be declared without anyone inventing
/// interface names.
#[test]
fn a_positional_module_needs_no_port_names() {
    let mut circuit = Circuit::new();
    let a = circuit.wire(8).unwrap();
    let b = circuit.wire(8).unwrap();
    let sum = circuit.add(a, b).unwrap();
    let out = circuit.wire(8).unwrap();
    circuit.drive(out, sum).unwrap();
    let text = Module::combinational("anonymous", circuit, vec![a, b], vec![out])
        .with_port_names(PortNames::Positional)
        .emit()
        .unwrap();
    assert!(text.contains("input  wire [7:0] i0,"), "{text}");
    assert!(text.contains("output wire [7:0] o0"), "{text}");
    assert!(text.contains("assign o0 = signal_Add;"), "{text}");
}

/// The port identifier a module emits and the one an instantiation site connects
/// are the same string, which is the whole point of the positional scheme.
#[test]
fn an_instantiation_site_binds_to_a_positional_module() {
    // The submodule: two inputs, one output, named positionally.
    let child = Design::new();
    let child_x = child.input("x", 8).unwrap();
    let child_y = child.input("y", 8).unwrap();
    let sum = child.add(&child_x, &child_y).unwrap();
    child.output("q", 8, &sum).unwrap();
    let child_text = module(&child, "adder")
        .with_port_names(PortNames::Positional)
        .emit()
        .unwrap();
    assert!(child_text.contains("input  wire [7:0] i0,"), "{child_text}");
    assert!(child_text.contains("input  wire [7:0] i1,"), "{child_text}");
    assert!(child_text.contains("output wire [7:0] o0"), "{child_text}");
    assert!(
        child_text.contains("assign o0 = signal_Add;"),
        "{child_text}"
    );

    // The parent: an instance whose connections name exactly those ports.
    let parent = Design::new();
    let x = parent.input("x", 8).unwrap();
    let y = parent.input("y", 8).unwrap();
    let sub = parent
        .instance("adder", &[], &[x.clone(), y.clone()], 8)
        .unwrap();
    parent.output("q", 8, &sub).unwrap();
    let parent_text = module(&parent, "wrapper").emit().unwrap();
    assert!(parent_text.contains(".i0 (x),"), "{parent_text}");
    assert!(parent_text.contains(".i1 (y),"), "{parent_text}");
    assert!(
        parent_text.contains(".o0 (signal_Instance)"),
        "{parent_text}"
    );
}
