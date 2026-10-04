//! What `#[derive(PortList)]` promises, checked on shapes that use every attribute.
//!
//! One shape with every field kind, because the interesting failures are the
//! *combinations*: a clock after a collection, an optional port in the middle of
//! a shape, a width that disagrees with a signal. Testing each kind in isolation
//! would miss all three.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::{
    __private, Design, PortList, Signal, WaveFormat, assign, inputs, outputs, wires,
};
use ferrite_lithic_derive::PortList;

#[derive(PortList)]
struct Lane {
    #[clock]
    clock: Signal,
    clear: Signal,
    #[bits(8)]
    d: Signal,
    #[bits(4)]
    #[length(3)]
    lanes: Vec<Signal>,
    #[exists]
    ready: Option<Signal>,
    #[bits(1)]
    #[wave_format("hex")]
    full: Signal,
}

#[derive(PortList)]
#[rtlmangle]
struct Reserved {
    signed: Signal,
    #[bits(2)]
    #[rtlname("wire bus")]
    data: Signal,
}

#[derive(PortList)]
#[rtlprefix("in_")]
#[rtlsuffix("_q")]
struct Rewritten {
    a: Signal,
    b: Signal,
}

/// A module's inputs and outputs are usually *different* port sets, so they are
/// two shapes rather than one. This is Hardcaml's shape too: an interface type
/// for `i`, one for `o`, and a builder between them.
#[derive(PortList)]
struct AddInputs {
    #[bits(8)]
    a: Signal,
    #[bits(8)]
    b: Signal,
}

#[derive(PortList)]
struct AddOutputs {
    #[bits(8)]
    y: Signal,
}

/// The error half of a `Result`, without needing `Debug` on the port list.
///
/// A port list holds signals, and `Signal`'s `Debug` prints node ids, which is
/// noise in a test that only wants to read the error. So these shapes are not
/// `Debug`, and the error is taken apart instead.
fn expect_error<T>(result: Result<T, ferrite_lithic::PortError>) -> ferrite_lithic::PortError {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(error) => error,
    }
}

#[test]
fn the_shape_is_the_declared_interface() {
    assert_eq!(
        Lane::PORT_NAMES,
        [
            "clock", "clear", "d", "lanes_0", "lanes_1", "lanes_2", "ready", "full"
        ]
    );
    assert_eq!(Lane::PORT_WIDTHS, [1, 1, 8, 4, 4, 4, 1, 1]);
    assert_eq!(
        Lane::PORT_OPTIONAL,
        [false, false, false, false, false, false, true, false]
    );
    assert_eq!(Lane::REQUIRED_PORTS, 7);
}

#[test]
fn formats_default_to_binary_and_are_read_from_the_attribute() {
    assert_eq!(Lane::PORT_FORMATS.len(), Lane::PORT_WIDTHS.len());
    assert!(
        Lane::PORT_FORMATS[..Lane::PORT_FORMATS.len() - 1]
            .iter()
            .all(|format| *format == WaveFormat::Binary)
    );
    assert_eq!(Lane::PORT_FORMATS[7], WaveFormat::Hex);
}

#[test]
fn the_clock_index_is_a_port_index_not_a_field_index() {
    assert_eq!(Lane::PORT_CLOCK, Some(0));

    // The interesting case: a clock declared *after* a collection, where the
    // port index and the field index differ.
    #[derive(PortList)]
    struct AfterCollection {
        #[bits(2)]
        #[length(3)]
        lanes: Vec<Signal>,
        #[clock]
        clock: Signal,
    }
    assert_eq!(AfterCollection::PORT_CLOCK, Some(3));
}

#[test]
fn a_wired_shape_keeps_its_fields_addressable_by_name() {
    let design = Design::new();
    let ports = inputs::<Lane>(&design).unwrap();

    assert_eq!(ports.clock.width(), 1);
    assert_eq!(ports.d.width(), 8);
    assert_eq!(ports.lanes.len(), 3);
    assert_eq!(ports.lanes[1].width(), 4);
    assert_eq!(ports.ready.as_ref().unwrap().width(), 1);
}

#[test]
fn a_wired_shape_includes_the_optional_port() {
    let design = Design::new();
    let ports = inputs::<Lane>(&design).unwrap();

    // `inputs` declares every declared port, so an `#[exists]` port is present
    // here. It is `from_pattern`, and a shape built without it, that let it be
    // absent.
    assert_eq!(
        ports.present(),
        [true, true, true, true, true, true, true, true]
    );
    assert_eq!(ports.to_signals().len(), 8);
    assert_eq!(ports.to_list().len(), 8);
    assert_eq!(ports.iter().count(), 8);
}

#[test]
fn an_absent_optional_port_shortens_the_list() {
    let design = Design::new();
    let present = inputs::<Lane>(&design).unwrap();
    let signals = present
        .to_signals()
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index != 6)
        .map(|(_, signal)| signal)
        .collect();
    let without_ready = Lane::from_present(signals).unwrap();

    assert_eq!(without_ready.ready, None);
    assert!(!without_ready.present()[6]);
    assert_eq!(without_ready.to_signals().len(), 7);
    assert_eq!(Lane::required_count(), 7);
}

#[test]
fn from_pattern_can_omit_the_optional_port() {
    let design = Design::new();
    let ports = inputs::<Lane>(&design).unwrap();
    let mut present = ports.present();
    present[6] = false;

    let signals = ports
        .to_signals()
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index != 6)
        .map(|(_, signal)| signal)
        .collect();
    let rebuilt = Lane::from_pattern(signals, &present).unwrap();
    assert_eq!(rebuilt.ready, None);
    assert!(!rebuilt.present()[6]);
    assert_eq!(rebuilt.to_signals().len(), 7);
}

#[test]
fn marking_a_required_port_absent_is_an_error_that_names_it() {
    let design = Design::new();
    let ports = inputs::<Lane>(&design).unwrap();
    let signals = ports
        .to_signals()
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index != 2)
        .map(|(_, signal)| signal)
        .collect();

    let mut present = ports.present();
    present[2] = false;
    let error = expect_error(Lane::from_pattern(signals, &present));
    assert!(
        error.to_string().contains("d"),
        "the error must name the port: {error}"
    );
    assert!(matches!(
        error,
        ferrite_lithic::PortError::NotOptional { .. }
    ));
}

#[test]
fn a_presence_list_of_the_wrong_length_is_an_error() {
    let design = Design::new();
    let ports = inputs::<Lane>(&design).unwrap();
    let error = expect_error(Lane::from_pattern(ports.to_signals(), &[true, true]));
    assert!(
        matches!(error, ferrite_lithic::PortError::Arity { .. }),
        "{error}"
    );
}

#[test]
fn the_wrong_number_of_signals_is_an_error() {
    let design = Design::new();
    let mut signals = inputs::<Lane>(&design).unwrap().to_signals();
    signals.pop();

    let error = expect_error(Lane::from_signals(signals));
    assert!(
        matches!(
            error,
            ferrite_lithic::PortError::Arity {
                expected: 8,
                got: 7
            }
        ),
        "{error}"
    );
}

#[test]
fn a_signal_of_the_wrong_width_is_an_error_that_names_both_widths() {
    let design = Design::new();
    let mut signals = inputs::<Lane>(&design).unwrap().to_signals();
    signals[2] = design.lit(0xff, 4).unwrap();

    match expect_error(Lane::from_signals(signals)) {
        ferrite_lithic::PortError::Width {
            port,
            expected,
            got,
        } => {
            assert_eq!(port, "d");
            assert_eq!(expected, 8);
            assert_eq!(got, 4);
        }
        other => panic!("expected a width error, got {other}"),
    }
}

#[test]
fn rtl_names_apply_the_containers_rewrites() {
    assert_eq!(Lane::rtl_names(), Lane::PORT_NAMES);
    assert_eq!(
        Rewritten::rtl_names(),
        ["in_a_q".to_string(), "in_b_q".to_string()]
    );
}

#[test]
fn rtlmangle_legalises_names_that_verilog_cannot_spell() {
    // `#[rtlname]` rewrites the *declared* name too: a shape that says one thing
    // and the emitter another is a shape nobody can read a diff of.
    assert_eq!(Reserved::PORT_NAMES, ["signed", "wire bus"]);
    assert_eq!(Reserved::rtl_names(), ["signed_", "wire_bus"]);
}

#[test]
fn inputs_and_outputs_are_two_shapes_of_one_design() {
    let design = Design::new();

    let in_ports = inputs::<AddInputs>(&design).unwrap();
    let y = &in_ports.a + &in_ports.b;
    let out_ports = outputs::<AddOutputs>(&design, &AddOutputs { y }).unwrap();

    assert_eq!(design.input_ports().len(), 2);
    assert_eq!(design.output_ports().len(), 1);
    assert_eq!(design.name(&out_ports.y).as_deref(), Some("y"));
    assert_eq!(out_ports.y.width(), 8);
}

#[test]
fn one_design_refuses_two_nodes_with_one_port_name() {
    // The consequence, and the reason there are two shapes: a Verilog port list
    // cannot declare one name twice, and an emitter that silently renamed the
    // second would break every instantiation site. `inputs` has already spent
    // `clock`, so a second list with the same names cannot be declared.
    let design = Design::new();
    inputs::<Lane>(&design).unwrap();
    let error = expect_error(inputs::<Lane>(&design));
    assert!(error.to_string().contains("clock"), "{error}");
}

#[test]
fn outputs_declares_driven_ports_under_the_shapes_own_names() {
    let design = Design::new();
    let drivers = wires::<Lane>(&design).unwrap();
    let driven = outputs::<Lane>(&design, &drivers).unwrap();

    assert_eq!(driven.to_signals().len(), 8);
    for (name, signal) in Lane::PORT_NAMES.iter().zip(driven.to_signals()) {
        assert_eq!(
            design.name(&signal).as_deref(),
            Some(*name),
            "port {name} must keep its name"
        );
    }
}

#[test]
fn outputs_declares_only_the_ports_that_are_present() {
    let design = Design::new();
    let drivers = wires::<Lane>(&design).unwrap();

    let mut present = drivers.present();
    present[6] = false;
    let signals = drivers
        .to_signals()
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index != 6)
        .map(|(_, signal)| signal)
        .collect();
    let drivers = Lane::from_pattern(signals, &present).unwrap();

    let driven = outputs::<Lane>(&design, &drivers).unwrap();
    assert_eq!(driven.ready, None);
    assert_eq!(driven.to_signals().len(), 7);
    assert_eq!(design.output_ports().len(), 7);
}

#[test]
fn wires_are_undriven_and_unnamed_so_the_names_are_still_free() {
    let design = Design::new();
    let scratch = wires::<Lane>(&design).unwrap();

    assert_eq!(scratch.to_signals().len(), 8);
    for signal in scratch.to_signals() {
        // No name, so `outputs` can still spend one on this signal later.
        assert_eq!(design.name(&signal), None);
        assert_eq!(design.driver_of(&signal).unwrap(), None);
    }
    assert!(design.input_ports().is_empty());

    // And it is free: the same shape's names can be declared afterwards.
    let declared = outputs::<Lane>(&design, &scratch).unwrap();
    assert_eq!(design.output_ports().len(), 8);
    assert_eq!(
        design.name(&declared.d),
        Some("d".to_string()),
        "the scratch signal became the named output port"
    );
}

#[test]
fn assign_drives_a_wired_list_and_closes_a_loop() {
    let design = Design::new();
    let wired = wires::<Lane>(&design).unwrap();

    // Every wire driven from itself. A wire with no driver and a wire driving
    // itself are different things, and this is the operation that turns an
    // undriven list into a closed one -- the same shape a register input has.
    assign(&design, &wired, &wired).unwrap();
    for signal in wired.to_signals() {
        assert_eq!(
            design.driver_of(&signal).unwrap().unwrap().id(),
            signal.id()
        );
    }
}

#[test]
fn map_and_map2_walk_the_whole_list() {
    let design = Design::new();
    let ports = inputs::<Lane>(&design).unwrap();

    let walked = ports
        .map(|signal| {
            __private::check_width("walked", signal, signal.width())?;
            Ok(signal.clone())
        })
        .unwrap();
    assert_eq!(walked.len(), 8);

    let zipped = ports
        .map2(&ports, |a, b| {
            assert_eq!(a.id(), b.id());
            Ok(a.clone())
        })
        .unwrap();
    assert_eq!(zipped.len(), 8);
}

#[test]
fn a_named_port_lookup_reports_the_names_the_shape_has() {
    let design = Design::new();
    let ports = inputs::<Lane>(&design).unwrap();

    assert!(ports.named("d").is_ok());
    let error = ports.named("nope").unwrap_err();
    assert!(error.to_string().contains("lanes_0"), "{error}");
    assert!(ports.node_ids().len() == 8);
    assert_eq!(ports.clock().unwrap().id(), ports.clock.id());
}

#[test]
fn the_check_arity_helper_is_reachable_under_its_documented_path() {
    assert!(__private::check_arity(2, 2).is_ok());
    assert!(__private::check_arity(1, 2).is_err());
}
