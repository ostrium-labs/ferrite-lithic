//! Width rules for every constructor, and the errors they refuse.
//!
//! Widths are runtime values ([ADR-0003](https://github.com/ostrium-labs/ferrite-lithic/blob/dev/docs/adr/0003-runtime-widths-not-const-generics.md)),
//! so every width rule here is a runtime check with a runtime message. The
//! messages are the compensation for giving up const generics, so
//! `tests/errors.rs`-style assertions on their text are part of this file's job.

#![allow(clippy::unwrap_used)]

use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{CaseArm, Circuit, Error, Node, NodeId, Op};

fn bits(value: u64, width: u32) -> Bits {
    Bits::constant(value, width).unwrap()
}

fn fresh() -> Circuit {
    Circuit::new()
}

// ------------------------------------------------------------------ literals

#[test]
fn a_wire_refuses_zero_width() {
    let err = fresh().wire(0).unwrap_err();
    assert!(matches!(err, Error::ZeroWidth { .. }), "{err:?}");
    assert!(err.to_string().contains("zero-width"), "{err}");
}

#[test]
fn a_wires_declared_width_is_its_width() {
    let mut c = fresh();
    for width in [1u32, 7, 64, 65, 200] {
        let w = c.wire(width).unwrap();
        assert_eq!(c.width_of(w), width);
    }
}

#[test]
fn a_constants_width_comes_from_the_value() {
    let mut c = fresh();
    let k = c.constant(bits(0xff, 12));
    assert_eq!(c.width_of(k), 12);
}

// ---------------------------------------------------------------- arithmetic

#[test]
fn add_and_sub_require_equal_widths() {
    let mut c = fresh();
    let a = c.constant(bits(0, 8));
    let b = c.constant(bits(0, 16));
    for err in [c.add(a, b).unwrap_err(), c.sub(a, b).unwrap_err()] {
        let text = err.to_string();
        assert!(text.contains("8 bits"), "{text}");
        assert!(text.contains("16 bits"), "{text}");
        assert!(
            text.contains("discarding whatever falls off the top"),
            "{text}"
        );
    }
}

#[test]
fn add_returns_the_left_operands_width() {
    let mut c = fresh();
    let a = c.constant(bits(0xff, 8));
    let b = c.constant(bits(0x01, 8));
    let sum = c.add(a, b).unwrap();
    let difference = c.sub(a, b).unwrap();
    assert_eq!(c.width_of(sum), 8);
    assert_eq!(c.width_of(difference), 8);
}

#[test]
fn mul_widens_and_so_takes_mismatched_widths() {
    let mut c = fresh();
    let a = c.constant(bits(0xff, 8));
    let b = c.constant(bits(0xff, 16));
    let product = c.mul(a, b).unwrap();
    assert_eq!(c.width_of(product), 24);
}

#[test]
fn division_and_remainder_take_the_left_operands_width() {
    let mut c = fresh();
    let a = c.constant(bits(0, 8));
    let b = c.constant(bits(0, 8));
    for id in [
        c.udiv(a, b).unwrap(),
        c.sdiv(a, b).unwrap(),
        c.urem(a, b).unwrap(),
        c.srem(a, b).unwrap(),
    ] {
        assert_eq!(c.width_of(id), 8);
    }
}

#[test]
fn division_by_zero_is_a_simulation_error_not_a_graph_error() {
    // The graph is legal; the *values* are what make it unsimulatable. Refusing
    // here would mean a design that can never be built, even if the divisor is
    // provably never zero.
    let mut c = fresh();
    let a = c.constant(bits(0, 8));
    let z = c.constant(bits(0, 8));
    assert!(c.udiv(a, z).is_ok());
}

// ------------------------------------------------------------------ bitwise

#[test]
fn bitwise_operators_require_equal_widths() {
    let mut c = fresh();
    let a = c.constant(bits(0, 8));
    let b = c.constant(bits(0, 32));
    for err in [
        c.bit_and(a, b).unwrap_err(),
        c.bit_or(a, b).unwrap_err(),
        c.bit_xor(a, b).unwrap_err(),
    ] {
        assert!(matches!(err, Error::WidthMismatch { .. }), "{err:?}");
    }
}

#[test]
fn not_preserves_width() {
    let mut c = fresh();
    for width in [1u32, 9, 64, 129] {
        let a = c.constant(bits(0, width));
        let inverted = c.not(a);
        assert_eq!(c.width_of(inverted), width);
    }
}

// -------------------------------------------------------------- comparisons

#[test]
fn every_comparison_is_one_bit() {
    let mut c = fresh();
    let a = c.constant(bits(0, 8));
    let b = c.constant(bits(1, 8));
    let results = [
        c.eq(a, b).unwrap(),
        c.ult(a, b).unwrap(),
        c.ule(a, b).unwrap(),
        c.ugt(a, b).unwrap(),
        c.uge(a, b).unwrap(),
        c.slt(a, b).unwrap(),
        c.sle(a, b).unwrap(),
        c.sgt(a, b).unwrap(),
        c.sge(a, b).unwrap(),
    ];
    for id in results {
        assert_eq!(c.width_of(id), 1);
    }
}

// ------------------------------------------------------- structural operators

#[test]
fn cat_widens_to_the_sum() {
    let mut c = fresh();
    let hi = c.constant(bits(0, 3));
    let lo = c.constant(bits(0, 5));
    let joined = c.cat(hi, lo).unwrap();
    assert_eq!(c.width_of(joined), 8);
}

#[test]
fn cat_takes_mismatched_widths_because_that_is_its_purpose() {
    let mut c = fresh();
    let hi = c.constant(bits(0, 1));
    let lo = c.constant(bits(0, 200));
    let joined = c.cat(hi, lo).unwrap();
    assert_eq!(c.width_of(joined), 201);
}

#[test]
fn replicate_multiplies_width_by_count() {
    let mut c = fresh();
    let v = c.constant(bits(0, 4));
    let thrice = c.replicate(v, 3).unwrap();
    let once = c.replicate(v, 1).unwrap();
    assert_eq!(c.width_of(thrice), 12);
    assert_eq!(c.width_of(once), 4);
}

#[test]
fn replicate_refuses_a_zero_count() {
    let mut c = fresh();
    let v = c.constant(bits(0, 4));
    let err = c.replicate(v, 0).unwrap_err();
    assert!(matches!(err, Error::ZeroWidth { .. }), "{err:?}");
}

#[test]
fn replicate_refuses_a_count_that_overflows_the_width_field() {
    let mut c = fresh();
    let v = c.constant(bits(0, 1 << 20));
    let err = c.replicate(v, 1 << 20).unwrap_err();
    assert!(matches!(err, Error::WidthOverflow { .. }), "{err:?}");
}

#[test]
fn cat_refuses_a_sum_that_overflows_the_width_field() {
    let mut c = fresh();
    let hi = c.constant(bits(0, 1 << 31));
    let lo = c.constant(bits(0, 1 << 31));
    let err = c.cat(hi, lo).unwrap_err();
    assert!(matches!(err, Error::WidthOverflow { .. }), "{err:?}");
}

// ------------------------------------------------------------------- select

#[test]
fn an_ite_selects_and_so_requires_matching_branches() {
    let mut c = fresh();
    let cond = c.constant(bits(1, 1));
    let eight = c.constant(bits(0, 8));
    let sixteen = c.constant(bits(0, 16));
    let err = c.ite(cond, eight, sixteen).unwrap_err();
    assert!(
        matches!(
            err,
            Error::WidthMismatch {
                lhs: 8,
                rhs: 16,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn an_ite_condition_must_be_exactly_one_bit() {
    let mut c = fresh();
    let cond = c.constant(bits(1, 8));
    let eight = c.constant(bits(0, 8));
    let err = c.ite(cond, eight, eight).unwrap_err();
    assert!(matches!(err, Error::NotOneBit { width: 8, .. }), "{err:?}");
    assert!(err.to_string().contains("condition"), "{err}");
}

#[test]
fn an_ite_result_is_the_branch_width() {
    let mut c = fresh();
    let cond = c.constant(bits(1, 1));
    let eight = c.constant(bits(0, 8));
    let selected = c.ite(cond, eight, eight).unwrap();
    assert_eq!(c.width_of(selected), 8);
}

// --------------------------------------------------------------------- case

#[test]
fn a_case_needs_at_least_one_arm() {
    let mut c = fresh();
    let s = c.constant(bits(0, 4));
    let v = c.constant(bits(0, 8));
    let err = c.case_(s, Vec::new(), v).unwrap_err();
    assert!(matches!(err, Error::CaseNoArms { .. }), "{err:?}");
}

#[test]
fn every_case_arm_and_the_default_must_share_one_width() {
    let mut c = fresh();
    let s = c.constant(bits(0, 4));
    let eight = c.constant(bits(0, 8));
    let sixteen = c.constant(bits(0, 16));
    let arms = vec![CaseArm::new(bits(0, 4), eight)];
    let err = c.case_(s, arms, sixteen).unwrap_err();
    assert!(
        matches!(
            err,
            Error::CaseArmWidthMismatch {
                lhs: 8,
                rhs: 16,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_case_reports_which_arm_is_the_odd_one_out() {
    let mut c = fresh();
    let s = c.constant(bits(0, 4));
    let eight = c.constant(bits(0, 8));
    let sixteen = c.constant(bits(0, 16));
    let arms = vec![
        CaseArm::new(bits(0, 4), eight),
        CaseArm::new(bits(1, 4), eight),
        CaseArm::new(bits(2, 4), sixteen),
    ];
    let err = c.case_(s, arms, eight).unwrap_err();
    assert!(
        matches!(err, Error::CaseArmWidthMismatch { arm: 2, .. }),
        "{err:?}"
    );
}

#[test]
fn match_values_must_be_the_scrutinees_width() {
    let mut c = fresh();
    let s = c.constant(bits(0, 4));
    let eight = c.constant(bits(0, 8));
    let arms = vec![CaseArm::new(bits(0, 8), eight)];
    let err = c.case_(s, arms, eight).unwrap_err();
    assert!(
        matches!(err, Error::WidthMismatch { lhs: 4, rhs: 8, .. }),
        "{err:?}"
    );
}

#[test]
fn a_multi_value_arm_is_allowed() {
    let mut c = fresh();
    let s = c.constant(bits(0, 4));
    let eight = c.constant(bits(0, 8));
    let arms = vec![CaseArm {
        matches: vec![bits(1, 4), bits(2, 4), bits(3, 4)],
        value: eight,
    }];
    assert!(c.case_(s, arms, eight).is_ok());
}

// -------------------------------------------------------------------- state

#[test]
fn clock_reset_and_clear_must_be_one_bit() {
    let mut c = fresh();
    let data = c.constant(bits(0, 8));
    let wide = c.constant(bits(0, 8));
    let one = c.constant(bits(1, 1));
    for (operand, err) in [
        ("clock", c.reg(data, wide, one, one).unwrap_err()),
        ("reset", c.reg(data, one, wide, one).unwrap_err()),
        ("clear", c.reg(data, one, one, wide).unwrap_err()),
    ] {
        assert!(matches!(err, Error::NotOneBit { .. }), "{operand}: {err:?}");
        assert!(err.to_string().contains(operand), "{operand}: {err}");
    }
}

#[test]
fn a_registers_width_is_its_data_inputs_width() {
    let mut c = fresh();
    let one = c.constant(bits(1, 1));
    let wide = c.constant(bits(0, 12));
    let narrow = c.constant(bits(0, 3));
    let wide_reg = c.reg(wide, one, one, one).unwrap();
    let narrow_reg = c.reg(narrow, one, one, one).unwrap();
    assert_eq!(c.width_of(wide_reg), 12);
    assert_eq!(c.width_of(narrow_reg), 3);
}

#[test]
fn memories_refuse_zero_width_or_zero_depth() {
    let mut c = fresh();
    assert!(matches!(c.mem(0, 4).unwrap_err(), Error::ZeroWidth { .. }));
    assert!(matches!(c.mem(8, 0).unwrap_err(), Error::ZeroWidth { .. }));
}

#[test]
fn a_read_ports_width_is_its_memorys_word_width() {
    let mut c = fresh();
    let en = c.constant(bits(1, 1));
    for depth in [2u32, 4, 16, 256] {
        let addr = c.constant(bits(0, ferrite_lithic_ir::address_width_for_depth(depth)));
        let mem = c.mem(12, depth).unwrap();
        let port = c.read_port(mem, addr, en).unwrap();
        assert_eq!(c.width_of(port), 12);
    }
}

#[test]
fn an_address_too_narrow_to_reach_every_word_is_refused() {
    let mut c = fresh();
    let en = c.constant(bits(1, 1));
    let mem = c.mem(8, 16).unwrap();
    let too_narrow = c.constant(bits(0, 3));
    let err = c.read_port(mem, too_narrow, en).unwrap_err();
    assert!(
        matches!(
            err,
            Error::MemoryAddressTooNarrow {
                depth: 16,
                address_width: 4,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_write_ports_data_must_be_one_whole_word() {
    let mut c = fresh();
    let mem = c.mem(8, 4).unwrap();
    let addr = c.constant(bits(0, 2));
    let en = c.constant(bits(1, 1));
    let narrow = c.constant(bits(0, 4));
    let err = c.write_port(mem, narrow, addr, en).unwrap_err();
    assert!(
        matches!(err, Error::WidthMismatch { lhs: 8, rhs: 4, .. }),
        "{err:?}"
    );
    // And the port was not added, so the memory is still read-only.
    assert!(c.write_ports_of(mem).unwrap().is_empty());
}

#[test]
fn a_write_ports_enable_must_be_one_bit() {
    let mut c = fresh();
    let mem = c.mem(8, 4).unwrap();
    let addr = c.constant(bits(0, 2));
    let data = c.constant(bits(0, 8));
    let wide = c.constant(bits(1, 2));
    let err = c.write_port(mem, data, addr, wide).unwrap_err();
    assert!(matches!(err, Error::NotOneBit { .. }), "{err:?}");
}

#[test]
fn an_instance_refuses_a_zero_output_width() {
    let mut c = fresh();
    assert!(matches!(
        c.instance("sub", Vec::new(), Vec::new(), 0).unwrap_err(),
        Error::ZeroWidth { .. }
    ));
}

#[test]
fn an_instance_refuses_an_empty_name() {
    // Reported as its own error rather than as `ZeroWidth`: the name becomes the
    // instance identifier in the generated Verilog, so an empty one is a
    // different mistake with a different fix, and `ZeroWidth` names neither.
    let mut c = fresh();
    let err = c.instance("", Vec::new(), Vec::new(), 8).unwrap_err();
    assert!(matches!(err, Error::EmptyInstanceName { .. }));
    assert!(
        err.to_string().contains("non-empty name"),
        "message should name the problem: {err}"
    );
}

#[test]
fn an_instances_output_width_is_the_declared_one() {
    let mut c = fresh();
    let input = c.constant(bits(0, 4));
    let sub = c
        .instance("sub", vec![("W".to_string(), bits(3, 2))], vec![input], 9)
        .unwrap();
    assert_eq!(c.width_of(sub), 9);
}

// --------------------------------------------------------------------- check

#[test]
fn check_catches_a_stored_width_that_lies() {
    // Only reachable through `add_node`, which is the point: every constructor
    // derives the width, so a disagreement means a hand-built node.
    let mut c = fresh();
    let scrutinee = c.constant(bits(0, 4));
    let arm = c.constant(bits(0, 8));
    let lying = c
        .add_node(Node::Case {
            scrutinee,
            arms: vec![CaseArm::new(bits(0, 4), arm)],
            default: arm,
            width: 16, // the truth is 8
        })
        .unwrap();
    let err = c.check().unwrap_err();
    assert!(
        matches!(err, Error::WidthInvariant { node, stored: 16, derived: 8 } if node == lying),
        "{err:?}"
    );
}

#[test]
fn check_catches_a_read_port_that_claims_the_wrong_width() {
    let mut c = fresh();
    let en = c.constant(bits(1, 1));
    let addr = c.constant(bits(0, 2));
    let mem = c.mem(8, 4).unwrap();
    let lying = c
        .add_node(Node::ReadPort {
            mem,
            address: addr,
            enable: en,
            data_width: 32,
        })
        .unwrap();
    let err = c.check().unwrap_err();
    assert!(
        matches!(err, Error::WidthInvariant { node, stored: 32, derived: 8 } if node == lying),
        "{err:?}"
    );
}

#[test]
fn check_catches_a_dangling_operand() {
    let mut c = fresh();
    let ghost = NodeId::from_index(99);
    let orphan = c
        .add_node(Node::Add {
            left: ghost,
            right: ghost,
        })
        .unwrap();
    // The node itself exists; it is the operand that does not, so the failure has
    // to come from `check` and not from `get`.
    assert!(c.get(orphan).is_ok());
    let err = c.check().unwrap_err();
    assert!(
        matches!(err, Error::UnknownNode { id, len: _ } if id == ghost),
        "{err:?}"
    );
}

#[test]
fn a_healthy_graph_checks_clean() {
    let mut c = fresh();
    let en = c.constant(bits(1, 1));
    let addr = c.constant(bits(0, 3));
    let data = c.constant(bits(9, 8));
    let mem = c.mem(8, 8).unwrap();
    c.write_port(mem, data, addr, en).unwrap();
    let read = c.read_port(mem, addr, en).unwrap();
    let one_bit = c.constant(bits(1, 1));
    let one_word = c.constant(bits(1, 8));
    let acc = c.wire(8).unwrap();
    let sum = c.add(read, one_word).unwrap();
    let sum = c.add(sum, one_word).unwrap();
    let next = c.reg(sum, one_bit, one_bit, one_bit).unwrap();
    c.drive(acc, next).unwrap();
    c.set_name(acc, "acc").unwrap();
    c.check().unwrap();
    assert_eq!(c.name(acc), Some("acc"));
    assert_eq!(c.memory_shape(mem).unwrap(), (8, 8));
}

// ------------------------------------------------- memory diagnostics are about memory

#[test]
fn using_a_non_memory_where_a_memory_is_wanted_says_so() {
    // Every one of these used to report `NotAWire`, which named the wrong
    // requirement: the caller wanted a memory, and "create a wire" sends them
    // somewhere else entirely. A wire *is* a legal target for `drive`, so the
    // message was not merely terse, it was misleading about the same node kind.
    let mut c = fresh();
    let en = c.constant(bits(1, 1));
    let addr = c.constant(bits(0, 3));
    let data = c.constant(bits(9, 8));
    let not_a_memory = c.constant(bits(0, 8));

    let err = c.write_port(not_a_memory, data, addr, en).unwrap_err();
    assert!(matches!(err, Error::NotAMemory { .. }));
    assert!(
        err.to_string().contains("as a memory"),
        "message should say a memory was wanted: {err}"
    );

    let err = c.read_port(not_a_memory, addr, en).unwrap_err();
    assert!(matches!(err, Error::NotAMemory { .. }));

    let err = c.memory_shape(not_a_memory).unwrap_err();
    assert!(matches!(err, Error::NotAMemory { .. }));

    let err = c.write_ports_of(not_a_memory).unwrap_err();
    assert!(matches!(err, Error::NotAMemory { .. }));
}

#[test]
fn driving_a_wire_still_reports_not_a_wire() {
    // The positive and negative case of the same predicate, side by side, so a
    // future refactor cannot swap them.
    let mut c = fresh();
    let wire = c.wire(8).unwrap();
    let driver = c.constant(bits(3, 8));
    c.drive(wire, driver).unwrap();

    let err = c.drive(driver, driver).unwrap_err();
    assert!(matches!(err, Error::NotAWire { .. }));
    assert!(
        err.to_string().contains("rather than a wire"),
        "message should name the wire requirement: {err}"
    );
}

// --------------------------------------------------------------------- select

#[test]
fn a_select_is_len_bits_wide() {
    let mut c = fresh();
    let wide = c.wire(16).unwrap();
    // Bound first: `c.width_of(c.select(..))` does not borrow-check, which is the
    // whole reason the front-end DSL takes `&self` instead of `&mut self`.
    let narrow = c.select(wide, 3, 5).unwrap();
    assert_eq!(c.width_of(narrow), 5);
    let whole = c.select(wide, 0, 16).unwrap();
    assert_eq!(c.width_of(whole), 16);
    let top = c.select(wide, 15, 1).unwrap();
    assert_eq!(c.width_of(top), 1);
}

#[test]
fn a_select_takes_the_bits_it_claims() {
    let mut c = fresh();
    let value = c.constant(bits(0b1101_1010, 8));
    let low = c.select(value, 0, 4).unwrap();
    let high = c.select(value, 4, 4).unwrap();
    let bit = c.select(value, 7, 1).unwrap();
    // The node records the window, so the offsets are checkable directly.
    match c.get(low).unwrap() {
        Node::Select { offset, len, .. } => {
            assert_eq!((*offset, *len), (0, 4));
        }
        other => panic!("expected a Select, got {}", other.kind()),
    }
    assert!(matches!(
        c.get(bit).unwrap(),
        Node::Select {
            offset: 7,
            len: 1,
            ..
        }
    ));
    assert!(matches!(
        c.get(high).unwrap(),
        Node::Select { offset: 4, .. }
    ));
}

#[test]
fn a_select_that_runs_past_the_end_is_refused() {
    let mut c = fresh();
    let value = c.wire(8).unwrap();
    // Off by one either way, which is exactly how this is written by accident.
    assert!(matches!(
        c.select(value, 4, 5).unwrap_err(),
        Error::SelectOutOfRange {
            offset: 4,
            len: 5,
            value_width: 8,
            ..
        }
    ));
    assert!(matches!(
        c.select(value, 9, 1).unwrap_err(),
        Error::SelectOutOfRange { .. }
    ));
    let err = c.select(value, 4, 5).unwrap_err();
    assert!(
        err.to_string().contains("runs past the end"),
        "message should say where it ran off: {err}"
    );
}

#[test]
fn a_select_of_zero_bits_is_refused() {
    let mut c = fresh();
    let value = c.wire(8).unwrap();
    assert!(matches!(
        c.select(value, 0, 0).unwrap_err(),
        Error::ZeroWidth { op: Op::Select }
    ));
}

#[test]
fn a_hand_built_select_that_overruns_is_caught_by_check() {
    // `add_node` does no validation, so `check` is the only thing standing between
    // a malformed node and a graph that claims more bits than it has.
    let mut c = fresh();
    let value = c.wire(8).unwrap();
    c.add_node(Node::Select {
        value,
        offset: 6,
        len: 4,
    })
    .unwrap();
    let err = c.check().unwrap_err();
    assert!(
        matches!(err, Error::WidthInvariant { .. }),
        "expected a width invariant failure, got {err:?}"
    );
}

#[test]
fn a_well_formed_hand_built_select_checks_clean() {
    let mut c = fresh();
    let value = c.wire(8).unwrap();
    let driver = c.constant(bits(7, 8));
    c.drive(value, driver).unwrap();
    c.add_node(Node::Select {
        value,
        offset: 4,
        len: 4,
    })
    .unwrap();
    c.check().unwrap();
}
