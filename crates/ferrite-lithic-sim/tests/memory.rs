//! Memories: zero-filled, written at the edge, read combinationally.
//!
//! A memory is the one place where a value is written in one phase and read in
//! another, which makes it the place where the phase boundary is easiest to get
//! wrong. In particular, a write port's data is captured *before* the edge, not
//! after, so a design that computes the write data from a register sees the
//! register's pre-edge value.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_sim::Sim;

/// A memory with one write port and one read port, plus a clock.
struct Fixture {
    design: Design,
}

impl Fixture {
    fn new(data_width: u32, depth: u32) -> Self {
        let d = Design::new();
        let clk = d.input("clk", 1).unwrap();
        let addr = d.input("addr", 8).unwrap();
        let waddr = d.input("waddr", 8).unwrap();
        let wdata = d.input("wdata", data_width).unwrap();
        let wen = d.input("wen", 1).unwrap();
        let ren = d.input("ren", 1).unwrap();

        let mem = d.mem(data_width, depth).unwrap();
        d.write_port(&mem, &wdata, &waddr, &wen).unwrap();
        let rdata = d.read_port(&mem, &addr, &ren).unwrap();
        d.output("rdata", data_width, &rdata).unwrap();
        // The clock is unused by the memory itself, but a memory-only design has
        // no register and therefore no clock, so borrow it for a pass-through
        // register to give the simulation an edge to apply writes on.
        let held = d
            .reg(&rdata, &clk, &d.constant(false), &d.constant(false))
            .unwrap();
        d.output("held", data_width, &held).unwrap();
        Fixture { design: d }
    }

    fn sim(&self) -> Sim {
        Sim::new(&self.design).unwrap()
    }
}

#[test]
fn a_memory_starts_zero_filled() {
    let f = Fixture::new(8, 4);
    let mut sim = f.sim();
    sim.set_input("ren", Bits::constant(1, 1).unwrap()).unwrap();
    for addr in 0..4u64 {
        sim.set_input("addr", Bits::constant(addr, 8).unwrap())
            .unwrap();
        sim.comb().unwrap();
        assert_eq!(
            sim.peek("rdata").unwrap().to_u64().unwrap(),
            0,
            "word {addr}"
        );
    }
}

#[test]
fn a_write_lands_in_the_memory_and_a_read_sees_it_on_the_next_settle() {
    let f = Fixture::new(8, 4);
    let mut sim = f.sim();
    sim.set_input("ren", Bits::constant(1, 1).unwrap()).unwrap();

    // Write 0xab to word 2.
    sim.set_input("waddr", Bits::constant(2, 8).unwrap())
        .unwrap();
    sim.set_input("wdata", Bits::constant(0xab, 8).unwrap())
        .unwrap();
    sim.set_input("wen", Bits::constant(1, 1).unwrap()).unwrap();

    // Before the edge the memory is still empty: the write has not been applied.
    sim.comb().unwrap();
    sim.set_input("addr", Bits::constant(2, 8).unwrap())
        .unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("rdata").unwrap().to_u64().unwrap(), 0);

    // The edge commits it, and the read port -- which is combinational -- sees it
    // in the same cycle's settle that follows.
    sim.step().unwrap();
    assert_eq!(sim.peek("rdata").unwrap().to_u64().unwrap(), 0xab);

    // And it is still there next cycle: a memory is not a register.
    sim.step().unwrap();
    assert_eq!(sim.peek("rdata").unwrap().to_u64().unwrap(), 0xab);
}

#[test]
fn a_disabled_write_enable_changes_nothing() {
    let f = Fixture::new(8, 4);
    let mut sim = f.sim();
    sim.set_input("ren", Bits::constant(1, 1).unwrap()).unwrap();
    sim.set_input("waddr", Bits::constant(1, 8).unwrap())
        .unwrap();
    sim.set_input("wdata", Bits::constant(0xff, 8).unwrap())
        .unwrap();
    sim.set_input("wen", Bits::constant(0, 1).unwrap()).unwrap();
    sim.step().unwrap();
    sim.set_input("addr", Bits::constant(1, 8).unwrap())
        .unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("rdata").unwrap().to_u64().unwrap(), 0);
}

#[test]
fn a_disabled_read_enable_forces_zero() {
    let f = Fixture::new(8, 4);
    let mut sim = f.sim();
    sim.set_input("wen", Bits::constant(1, 1).unwrap()).unwrap();
    sim.set_input("waddr", Bits::constant(0, 8).unwrap())
        .unwrap();
    sim.set_input("wdata", Bits::constant(0x77, 8).unwrap())
        .unwrap();
    sim.step().unwrap();

    sim.set_input("addr", Bits::constant(0, 8).unwrap())
        .unwrap();
    sim.set_input("ren", Bits::constant(1, 1).unwrap()).unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("rdata").unwrap().to_u64().unwrap(), 0x77);

    sim.set_input("ren", Bits::constant(0, 1).unwrap()).unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("rdata").unwrap().to_u64().unwrap(), 0);
}

#[test]
fn a_read_past_the_end_of_the_memory_reads_zero() {
    let f = Fixture::new(8, 4);
    let mut sim = f.sim();
    sim.set_input("ren", Bits::constant(1, 1).unwrap()).unwrap();
    sim.set_input("addr", Bits::constant(9, 8).unwrap())
        .unwrap();
    sim.comb().unwrap();
    // There is no `x` in a two-state simulator and no trap in hardware, so zero is
    // the only answer that is neither a lie nor a failure.
    assert_eq!(sim.peek("rdata").unwrap().to_u64().unwrap(), 0);
}

#[test]
fn a_write_past_the_end_of_the_memory_is_dropped() {
    let f = Fixture::new(8, 4);
    let mut sim = f.sim();
    sim.set_input("wen", Bits::constant(1, 1).unwrap()).unwrap();
    sim.set_input("waddr", Bits::constant(200, 8).unwrap())
        .unwrap();
    sim.set_input("wdata", Bits::constant(0xff, 8).unwrap())
        .unwrap();
    sim.step().unwrap();

    // Nothing landed, and nothing was corrupted either.
    sim.set_input("ren", Bits::constant(1, 1).unwrap()).unwrap();
    for addr in 0..4u64 {
        sim.set_input("addr", Bits::constant(addr, 8).unwrap())
            .unwrap();
        sim.comb().unwrap();
        assert_eq!(
            sim.peek("rdata").unwrap().to_u64().unwrap(),
            0,
            "word {addr}"
        );
    }
}

#[test]
fn a_wide_word_spans_several_buffer_words() {
    // 200 bits is four words, which is where an off-by-one in the word count would
    // show up as a corrupted neighbouring signal rather than as a wrong answer.
    let f = Fixture::new(200, 2);
    let mut sim = f.sim();
    sim.set_input("wen", Bits::constant(1, 1).unwrap()).unwrap();
    sim.set_input("waddr", Bits::constant(1, 8).unwrap())
        .unwrap();

    // Ascending bytes, so a word-order mistake cannot coincidentally produce the
    // right answer.
    let value = Bits::from_bytes_le(200, &(1..=25u8).collect::<Vec<_>>()).unwrap();
    sim.set_input("wdata", value.clone()).unwrap();
    sim.step().unwrap();

    sim.set_input("ren", Bits::constant(1, 1).unwrap()).unwrap();
    sim.set_input("addr", Bits::constant(1, 8).unwrap())
        .unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("rdata").unwrap(), value);
}

#[test]
fn several_write_ports_commit_in_the_same_edge() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let addr = d.input("addr", 8).unwrap();
    let waddr_a = d.input("waddr_a", 8).unwrap();
    let waddr_b = d.input("waddr_b", 8).unwrap();
    let wdata_a = d.input("wdata_a", 8).unwrap();
    let wdata_b = d.input("wdata_b", 8).unwrap();

    let mem = d.mem(8, 4).unwrap();
    d.write_port(&mem, &wdata_a, &waddr_a, &d.constant(true))
        .unwrap();
    d.write_port(&mem, &wdata_b, &waddr_b, &d.constant(true))
        .unwrap();
    // The read port is the output, not the pass-through register: a read is
    // combinational, so it reflects the memory as of the last edge without needing
    // another one. The register exists only to give the design a clock.
    let rdata = d.read_port(&mem, &addr, &d.constant(true)).unwrap();
    d.output("rdata", 8, &rdata).unwrap();
    let _held = d
        .reg(&rdata, &clk, &d.constant(false), &d.constant(false))
        .unwrap();

    let mut sim = Sim::new(&d).unwrap();
    sim.set_input("waddr_a", Bits::constant(0, 8).unwrap())
        .unwrap();
    sim.set_input("wdata_a", Bits::constant(0x11, 8).unwrap())
        .unwrap();
    sim.set_input("waddr_b", Bits::constant(3, 8).unwrap())
        .unwrap();
    sim.set_input("wdata_b", Bits::constant(0x22, 8).unwrap())
        .unwrap();
    sim.step().unwrap();

    for (at, want) in [(0u64, 0x11u64), (3, 0x22)] {
        sim.set_input("addr", Bits::constant(at, 8).unwrap())
            .unwrap();
        sim.comb().unwrap();
        assert_eq!(
            sim.peek("rdata").unwrap().to_u64().unwrap(),
            want,
            "word {at}"
        );
    }
}

#[test]
fn feedback_into_a_memory_write_port_is_legal() {
    // `Mem` is terminal for loop checking, so a counter feeding its own write port
    // is not a combinational loop. It is what a scratchpad buffer looks like.
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let pos = d.wire(4).unwrap();
    let mem = d.mem(8, 16).unwrap();
    d.write_port(
        &mem,
        &d.lit(0xa5, 8).unwrap(),
        &d.zero_extend(&pos, 8).unwrap(),
        &d.constant(true),
    )
    .unwrap();
    let next = d.add(&pos, &d.lit(1, 4).unwrap()).unwrap();
    let next = d
        .ite(&d.eq(&pos, &d.lit(15, 4).unwrap()).unwrap(), &pos, &next)
        .unwrap();
    let next = d
        .reg(&next, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.drive(&pos, &next).unwrap();
    d.output("pos", 4, &pos).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    // The circuit holds at 15 rather than wrapping, so the expected value saturates.
    for expected in 1..=20u64 {
        sim.step().unwrap();
        assert_eq!(
            sim.peek("pos").unwrap().to_u64().unwrap(),
            expected.min(15),
            "cycle {expected}"
        );
    }
}
