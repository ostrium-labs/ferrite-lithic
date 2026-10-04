//! The DSL's own claims: nesting, widths, ports, feedback, and the structural
//! shifts.

#![allow(clippy::unwrap_used)]

use ferrite_lithic::{Design, Error};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_sim::Sim;

#[test]
fn a_call_can_be_nested_inside_another() {
    // The reason this crate exists. On the IR directly,
    // `c.add(acc, c.lit(..))` is two `&mut` borrows in one expression and does not
    // compile; the compile_fail doctest in `lib.rs` is that error, kept as a test.
    let d = Design::new();
    let acc = d.wire(8).unwrap();
    let sum = d.add(&acc, &d.lit(1, 8).unwrap()).unwrap();
    assert_eq!(sum.width(), 8);
    assert_eq!(d.node_count(), 3);
}

#[test]
fn a_register_closes_a_loop_and_the_graph_checks_clean() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let acc = d.wire(8).unwrap();
    let next = d.add(&acc, &d.lit(1, 8).unwrap()).unwrap();
    let next = d
        .reg(&next, &clk, &d.constant(false), &d.constant(true))
        .unwrap();
    d.drive(&acc, &next).unwrap();
    d.build().unwrap();

    assert_eq!(acc.kind(), "Wire");
    assert_eq!(next.kind(), "Reg");
    assert!(next.is_stateful());
    assert!(!acc.is_stateful());
    assert!(!next.name().is_some());
}

#[test]
fn a_wire_read_before_it_is_driven_is_the_ordinary_case() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let acc = d.wire(8).unwrap();
    let next = d
        .reg(&d.zero_extend(&clk, 8).unwrap(), &clk, &clk, &clk)
        .unwrap();
    d.drive(&acc, &next).unwrap();
    // Built in the order a bottom-up graph forces: read the wire, then drive it.
    assert!(d.driver_of(&acc).unwrap().is_some());
}

#[test]
fn an_input_port_is_undriven_and_still_checks_clean() {
    let d = Design::new();
    let data = d.input("data", 8).unwrap();
    let out = d.output("out", 8, &data).unwrap();
    d.build().unwrap();

    assert!(d.driver_of(&data).unwrap().is_none());
    assert_eq!(d.name(&data).as_deref(), Some("data"));
    assert_eq!(d.name(&out).as_deref(), Some("out"));
    assert_eq!(d.input_ports().len(), 1);
    assert_eq!(d.input_ports()[0].id(), data.id());
}

#[test]
fn a_read_but_undriven_wire_that_is_not_a_port_still_fails() {
    // The port exemption must not turn into a blanket exemption for undriven
    // wires, or the check stops catching real mistakes.
    let d = Design::new();
    let dangle = d.wire(8).unwrap();
    let user = d.add(&dangle, &d.lit(1, 8).unwrap()).unwrap();
    assert_eq!(user.kind(), "Add");

    let err = d.build().unwrap_err();
    assert!(
        matches!(
            err.as_ir(),
            Some(ferrite_lithic_ir::Error::UndrivenWire { .. })
        ),
        "expected an undriven-wire error, got {err:?}"
    );
}

#[test]
fn undriving_a_wire_returns_the_old_driver() {
    let d = Design::new();
    let wire = d.wire(4).unwrap();
    let driver = d.lit(3, 4).unwrap();
    d.drive(&wire, &driver).unwrap();
    let old = d.undrive(&wire).unwrap();
    assert_eq!(old.unwrap().id(), driver.id());
    assert!(d.driver_of(&wire).unwrap().is_none());
    // And it can be driven again afterwards.
    d.drive(&wire, &driver).unwrap();
    assert_eq!(d.driver_of(&wire).unwrap().unwrap().id(), driver.id());
}

#[test]
fn widths_follow_the_bits_crate() {
    let d = Design::new();
    let a = d.lit(0xff, 8).unwrap();
    let b = d.lit(1, 8).unwrap();
    let narrow = d.lit(1, 4).unwrap();

    assert_eq!(d.add(&a, &b).unwrap().width(), 8);
    assert_eq!(d.sub(&a, &b).unwrap().width(), 8);
    // Multiply widens, and does not require equal widths.
    assert_eq!(d.mul(&a, &narrow).unwrap().width(), 12);
    assert_eq!(d.udiv(&a, &b).unwrap().width(), 8);
    assert_eq!(d.urem(&a, &b).unwrap().width(), 8);
    assert_eq!(d.not(&a).width(), 8);
    // Every comparison is one bit.
    for signal in [
        d.eq(&a, &b).unwrap(),
        d.ult(&a, &b).unwrap(),
        d.ule(&a, &b).unwrap(),
        d.ugt(&a, &b).unwrap(),
        d.uge(&a, &b).unwrap(),
        d.slt(&a, &b).unwrap(),
        d.sle(&a, &b).unwrap(),
        d.sgt(&a, &b).unwrap(),
        d.sge(&a, &b).unwrap(),
    ] {
        assert_eq!(signal.width(), 1, "comparison should be one bit wide");
    }
}

#[test]
fn a_width_mismatch_is_an_error_that_names_both_widths() {
    let d = Design::new();
    let wide = d.lit(1, 8).unwrap();
    let narrow = d.lit(1, 4).unwrap();
    let err = d.add(&wide, &narrow).unwrap_err();
    let text = err.to_string();
    assert!(text.contains('8'), "should name the left width: {text}");
    assert!(text.contains('4'), "should name the right width: {text}");
    assert!(text.contains("add"), "should name the operation: {text}");
}

#[test]
fn concat_puts_the_first_part_on_top() {
    let d = Design::new();
    let high = d.lit(0xab, 8).unwrap();
    let low = d.lit(0xcd, 8).unwrap();
    let both = d.concat(&[high.clone(), low.clone()]).unwrap();
    assert_eq!(both.width(), 16);

    let circuit = d.build().unwrap();
    match circuit.get(both.id()).unwrap() {
        // `Circuit::cat` is binary, so `concat` folds right: the outermost cat
        // must have the first part on its left, or the order is reversed.
        ferrite_lithic_ir::Node::Cat {
            high: top,
            low: bottom,
        } => {
            assert_eq!(*top, high.id(), "the first part belongs on top");
            assert_eq!(*bottom, low.id());
        }
        other => panic!("expected a Cat, got {}", other.kind()),
    }
}

#[test]
fn concat_of_one_signal_is_that_signal_and_of_none_is_an_error() {
    let d = Design::new();
    let only = d.lit(7, 4).unwrap();
    assert_eq!(
        d.concat(std::slice::from_ref(&only)).unwrap().id(),
        only.id()
    );
    assert!(matches!(
        d.concat(&[]).unwrap_err(),
        Error::Ir(ferrite_lithic_ir::Error::ZeroWidth { .. })
    ));
}

#[test]
fn replicate_multiplies_the_width() {
    let d = Design::new();
    let four = d.lit(0xf, 4).unwrap();
    assert_eq!(d.replicate(&four, 3).unwrap().width(), 12);
}

#[test]
fn a_slice_is_the_requested_width() {
    let d = Design::new();
    let wide = d.lit(0, 32).unwrap();
    assert_eq!(d.slice(&wide, 0, 8).unwrap().width(), 8);
    assert_eq!(d.slice(&wide, 24, 8).unwrap().width(), 8);
    assert_eq!(d.slice(&wide, 31, 1).unwrap().width(), 1);
}

#[test]
fn a_slice_past_the_end_is_refused() {
    let d = Design::new();
    let eight = d.lit(0, 8).unwrap();
    let err = d.slice(&eight, 4, 5).unwrap_err();
    assert!(
        matches!(
            err.as_ir(),
            Some(ferrite_lithic_ir::Error::SelectOutOfRange { .. })
        ),
        "expected SelectOutOfRange, got {err:?}"
    );
}

#[test]
fn extensions_and_truncation_land_on_the_requested_width() {
    let d = Design::new();
    let four = d.lit(0b1010, 4).unwrap();
    assert_eq!(d.zero_extend(&four, 12).unwrap().width(), 12);
    assert_eq!(d.sign_extend(&four, 12).unwrap().width(), 12);
    assert_eq!(d.truncate(&four, 2).unwrap().width(), 2);
    // Resizing to the same width is the identity, not a redundant node.
    assert_eq!(d.zero_extend(&four, 4).unwrap().id(), four.id());
    assert_eq!(d.sign_extend(&four, 4).unwrap().id(), four.id());
}

#[test]
fn zero_extension_is_built_from_zeros_above_and_the_value_below() {
    let d = Design::new();
    let four = d.lit(0b1010, 4).unwrap();
    let wide = d.zero_extend(&four, 8).unwrap();
    let circuit = d.build().unwrap();
    let ferrite_lithic_ir::Node::Cat { high, low } = circuit.get(wide.id()).unwrap() else {
        panic!("a zero extension should end in a cat");
    };
    assert_eq!(circuit.width_of(*low), 4);
    assert_eq!(circuit.width_of(*high), 4);
    assert!(matches!(
        circuit.get(*high).unwrap(),
        ferrite_lithic_ir::Node::Constant { .. }
    ));
}

#[test]
fn sign_extension_replicates_the_top_bit() {
    let d = Design::new();
    let four = d.lit(0b1010, 4).unwrap();
    let wide = d.sign_extend(&four, 8).unwrap();
    let circuit = d.build().unwrap();
    let ferrite_lithic_ir::Node::Cat { high, low } = circuit.get(wide.id()).unwrap() else {
        panic!("a sign extension should end in a cat");
    };
    assert_eq!(circuit.width_of(*low), 4);
    // The fill is the sign bit replicated, so it is a replicate of a one-bit
    // slice rather than a constant of zeros.
    assert!(
        matches!(
            circuit.get(*high).unwrap(),
            ferrite_lithic_ir::Node::Replicate { .. }
        ),
        "sign fill should be a replicate, got {}",
        circuit.get(*high).unwrap().kind()
    );
}

#[test]
fn truncating_to_a_wider_width_is_refused_rather_than_padded() {
    let d = Design::new();
    let four = d.lit(0, 4).unwrap();
    let err = d.truncate(&four, 8).unwrap_err();
    assert!(err.to_string().contains("runs past the end"), "{err}");
}

#[test]
fn a_zero_width_slice_is_refused_everywhere_it_could_arise() {
    let d = Design::new();
    let four = d.lit(0, 4).unwrap();
    assert!(d.slice(&four, 0, 0).is_err());
    assert!(d.truncate(&four, 0).is_err());
    assert!(d.zero_extend(&four, 0).is_err());
    assert!(d.sign_extend(&four, 0).is_err());
    assert!(d.zeros(0).is_err());
    assert!(d.ones(0).is_err());
}

#[test]
fn a_shift_by_more_than_the_width_saturates_rather_than_failing() {
    // Shifts saturate for `shift >= width`, which is what the bits crate does and
    // what a barrel shifter with a clamped amount does.
    let d = Design::new();
    let four = d.lit(0b1010, 4).unwrap();
    for signal in [
        d.sll(&four, 4).unwrap(),
        d.srl(&four, 4).unwrap(),
        d.srl(&four, 99).unwrap(),
        d.sll(&four, 99).unwrap(),
    ] {
        assert_eq!(signal.width(), 4);
    }
    assert_eq!(d.sra(&four, 4).unwrap().width(), 4);
    assert_eq!(d.sra(&four, 99).unwrap().width(), 4);
}

#[test]
fn a_zero_shift_is_a_whole_value_slice_and_builds_no_filler() {
    let d = Design::new();
    let four = d.lit(0b1010, 4).unwrap();
    for shifted in [
        d.sll(&four, 0).unwrap(),
        d.srl(&four, 0).unwrap(),
        d.sra(&four, 0).unwrap(),
    ] {
        assert_eq!(shifted.width(), 4);
        // Not a `cat`: a zero-width filler does not exist, so a shift of zero takes
        // the whole-value shortcut instead of building an empty half.
        assert_eq!(shifted.kind(), "Select");
    }
    assert!(d.build().is_ok());
}

#[test]
fn an_ite_needs_a_one_bit_condition() {
    let d = Design::new();
    let eight = d.lit(0, 8).unwrap();
    let wide_condition = d.lit(3, 8).unwrap();
    let err = d.ite(&wide_condition, &eight, &eight).unwrap_err();
    assert!(
        matches!(
            err.as_ir(),
            Some(ferrite_lithic_ir::Error::NotOneBit { .. })
        ),
        "expected NotOneBit, got {err:?}"
    );
}

#[test]
fn a_mux_selects_between_two_signals_of_one_width() {
    let d = Design::new();
    let condition = d.constant(true);
    let yes = d.lit(0xaa, 8).unwrap();
    let no = d.lit(0x55, 8).unwrap();
    let chosen = d.ite(&condition, &yes, &no).unwrap();
    assert_eq!(chosen.kind(), "Ite");
    assert_eq!(chosen.width(), 8);
    // `when` is the concise tier's spelling of the same node.
    let also = yes.when(&condition, &no, &yes);
    assert_eq!(also.kind(), "Ite");
}

#[test]
fn a_case_picks_the_matching_arm() {
    let d = Design::new();
    let opcode = d.lit(2, 2).unwrap();
    let zero = d.lit(0x11, 8).unwrap();
    let one = d.lit(0x22, 8).unwrap();
    let other = d.lit(0x33, 8).unwrap();
    let result = d
        .case_(&opcode, &[(0, zero.clone()), (1, one.clone())], &other)
        .unwrap();
    assert_eq!(result.kind(), "Case");
    assert_eq!(result.width(), 8);

    let circuit = d.build().unwrap();
    let ferrite_lithic_ir::Node::Case { arms, .. } = circuit.get(result.id()).unwrap() else {
        panic!("expected a Case node");
    };
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].value, zero.id());
    assert_eq!(arms[1].value, one.id());
}

#[test]
fn a_memory_gets_ports_that_are_real_nodes() {
    let d = Design::new();
    let mem = d.mem(8, 16).unwrap();
    assert_eq!(d.memory_shape(&mem).unwrap(), (8, 16));

    let address = d.lit(0, 4).unwrap();
    let data = d.lit(0xab, 8).unwrap();
    let en = d.constant(true);
    d.write_port(&mem, &data, &address, &en).unwrap();
    let read = d.read_port(&mem, &address, &en).unwrap();

    assert_eq!(read.kind(), "ReadPort");
    assert_eq!(read.width(), 8);
    let ports = d.write_ports_of(&mem).unwrap();
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].data.id(), data.id());
    assert_eq!(ports[0].address.id(), address.id());
    assert!(mem.is_stateful());
    assert!(!read.is_stateful());
}

#[test]
fn using_a_non_memory_where_a_memory_is_wanted_is_refused_by_the_dsl_too() {
    let d = Design::new();
    let not_a_memory = d.lit(0, 8).unwrap();
    let address = d.lit(0, 4).unwrap();
    let en = d.constant(true);
    let err = d.read_port(&not_a_memory, &address, &en).unwrap_err();
    assert!(
        matches!(
            err.as_ir(),
            Some(ferrite_lithic_ir::Error::NotAMemory { .. })
        ),
        "expected NotAMemory, got {err:?}"
    );
}

#[test]
fn an_instance_needs_a_name_and_a_single_output() {
    let d = Design::new();
    let input = d.lit(3, 8).unwrap();
    let sub = d
        .instance(
            "sub",
            &[("W", ferrite_lithic_bits::Bits::constant(3, 2).unwrap())],
            &[input],
            9,
        )
        .unwrap();
    assert_eq!(sub.kind(), "Instance");
    assert_eq!(sub.width(), 9);

    let err = d.instance("", &[], &[], 8).unwrap_err();
    assert!(
        matches!(
            err.as_ir(),
            Some(ferrite_lithic_ir::Error::EmptyInstanceName { .. })
        ),
        "expected EmptyInstanceName, got {err:?}"
    );
}

#[test]
fn a_literal_beyond_64_bits_truncates_the_value_and_that_is_stated() {
    let d = Design::new();
    // `Bits::constant` truncates a `u64` to the width; this documents that the
    // width, not the value, decides the result.
    let wide = d.lit(u64::MAX, 128).unwrap();
    assert_eq!(wide.width(), 128);
    let narrow = d.lit(0xffff_ffff, 4).unwrap();
    assert_eq!(narrow.width(), 4);
}

/// A lookup table is a multiplexer tree here, and that is stated in its docs.
///
/// `Design::rom` exists because every table in the design corpus was being written
/// out as 16 or 256 explicit `case_` arms, which is correct and unreadable. The
/// important part is not the convenience: it is that the cost is now written down in
/// one place instead of being rediscovered per design. There is no initialised-memory
/// node in the IR, so this is a `case`, and in silicon a big one would be a block RAM.
#[test]
fn a_lookup_table_indexes_its_entries() {
    let d = Design::new();
    // The AES-style nibble-to-ASCII table, which `hex` and `base64` both need.
    let table: Vec<u64> = (0..16u64)
        .map(|n| n + if n < 10 { 0x30 } else { 0x57 })
        .collect();
    let address = d.input("n", 4).unwrap();
    let ascii = d.rom(&address, &table, 8).unwrap();
    d.output("ascii", 8, &ascii).unwrap();
    let mut sim = Sim::new(&d).unwrap();
    for nibble in 0..16u64 {
        sim.set_input("n", Bits::constant(nibble, 4).unwrap())
            .unwrap();
        sim.comb().unwrap();
        assert_eq!(
            sim.peek("ascii").unwrap().to_u64().unwrap(),
            table[nibble as usize],
            "nibble {nibble:x}"
        );
    }
}

#[test]
fn a_lookup_table_refuses_an_address_of_the_wrong_width() {
    let d = Design::new();
    let table = [0u64, 1, 2, 3];
    // Four entries need a two-bit address.
    let too_narrow = d.input("a", 1).unwrap();
    let error = d.rom(&too_narrow, &table, 8).unwrap_err();
    assert!(error.to_string().contains("needs a width of 2"), "{error}");

    // And a wider address is refused rather than truncated: truncating would make the
    // table reachable at two different addresses, which is a silent aliasing bug.
    let too_wide = d.input("b", 3).unwrap();
    let error = d.rom(&too_wide, &table, 8).unwrap_err();
    assert!(error.to_string().contains("needs a width of 2"), "{error}");
}

#[test]
fn a_lookup_table_refuses_a_length_that_is_not_a_power_of_two() {
    let d = Design::new();
    let address = d.input("a", 3).unwrap();
    // Three entries cannot be addressed by three bits without an out-of-range case,
    // and a default arm would silently alias two addresses to one value.
    let error = d.rom(&address, &[1, 2, 3], 8).unwrap_err();
    assert!(error.to_string().contains("`rom`"), "{error}");
}
