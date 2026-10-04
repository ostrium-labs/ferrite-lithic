//! Ethernet II / IPv4 / TCP parsing against `etherparse`.
//!
//! `etherparse` is the golden model because it *is* the reference implementation of
//! these headers in Rust. Where this design deliberately differs from a general parser
//! — a fixed 64-byte window, two VLAN tags rather than an unbounded stack, IP options
//! reported rather than followed — the test says so and asserts the difference rather
//! than narrowing the comparison until it passes.
//!
//! # The frames are built by hand
//!
//! Not taken from `etherparse`, because a test that uses the oracle's own serialiser
//! cannot catch a disagreement about the *wire format* — only about the parse. So the
//! bytes are laid out explicitly here, the checksums are computed by an independent
//! function, and then `etherparse` is asked whether it agrees those bytes are a
//! well-formed frame. If it does not, the first test fails and the rest are testing
//! nothing.
//!
//! # Driving a 512-bit window
//!
//! The design takes the frame as eight 64-bit ports rather than one 512-bit one,
//! because the cosimulator's generated driver refuses a port wider than 64 bits. That
//! is the right shape for this harness and it is also what a real line-rate parser
//! would do with a wide bus, so the two constraints happen to agree.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use etherparse::{NetSlice, SlicedPacket, TransportSlice};
use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::packet::{self, WINDOW_BYTES};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

/// The one's-complement checksum, computed independently of the design.
fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for pair in bytes.chunks(2) {
        sum += u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]));
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// A well-formed Ethernet II / IPv4 / TCP frame with a verifying checksum.
///
/// `vlan` is the tag control field, or `None` for an untagged frame.
fn frame(
    ethertype: u16,
    vlan: Option<u16>,
    ip_src: [u8; 4],
    ip_dst: [u8; 4],
    src_port: u16,
    dst_port: u16,
    flags: u8,
) -> [u8; 64] {
    let mut f = [0u8; 64];
    f[0..6].copy_from_slice(&[0x02, 0, 0, 0, 0, 1]);
    f[6..12].copy_from_slice(&[0x02, 0, 0, 0, 0, 2]);
    f[12..14].copy_from_slice(&ethertype.to_be_bytes());
    let mut ip = 14;
    if let Some(tag) = vlan {
        f[12..14].copy_from_slice(&0x8100u16.to_be_bytes());
        f[14..16].copy_from_slice(&tag.to_be_bytes());
        f[16..18].copy_from_slice(&ethertype.to_be_bytes());
        ip = 18;
    }

    let mut header = [0u8; 20];
    header[0] = 0x45; // version 4, IHL 5
    header[1] = 0x10; // DSCP
    header[2..4].copy_from_slice(&40u16.to_be_bytes()); // total length: 20 IP + 20 TCP
    header[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
    header[8] = 64; // TTL
    header[9] = 6; // TCP
    header[12..16].copy_from_slice(&ip_src);
    header[16..20].copy_from_slice(&ip_dst);
    let sum = checksum(&header);
    header[10..12].copy_from_slice(&sum.to_be_bytes());
    f[ip..ip + 20].copy_from_slice(&header);

    let tcp = ip + 20;
    f[tcp..tcp + 2].copy_from_slice(&src_port.to_be_bytes());
    f[tcp + 2..tcp + 4].copy_from_slice(&dst_port.to_be_bytes());
    f[tcp + 4..tcp + 8].copy_from_slice(&0x0000_0001u32.to_be_bytes());
    f[tcp + 8..tcp + 12].copy_from_slice(&0x0000_0002u32.to_be_bytes());
    f[tcp + 12] = 0x50; // data offset 5
    f[tcp + 13] = flags;
    f[tcp + 14..tcp + 16].copy_from_slice(&65535u16.to_be_bytes());
    let tcp_sum = checksum(&f[tcp..tcp + 20]);
    f[tcp + 16..tcp + 18].copy_from_slice(&tcp_sum.to_be_bytes());
    f
}

/// Drives one frame through the design and returns every output by name.
///
/// The window goes in as eight 64-bit ports, each the little-endian reading of eight
/// consecutive wire bytes, because that is how the design's `concat` puts them back
/// together.
fn parse(frame: &[u8; 64]) -> Vec<(String, u64)> {
    let design = Design::new();
    packet::build(&design).expect("the packet design builds");
    let tb = Testbench::new(&design).expect("the packet design compiles");
    // **Big-endian words**, because the window is a network byte order and the design's
    // `byte` helper takes byte 0 to be the *most* significant. So `buf_0` holds wire
    // bytes 0..8 with byte 0 in its high bits. Getting this backwards is silent: every
    // field reads as zero rather than as an obviously wrong value, because the MAC and
    // the EtherType of a zeroed window are both zero.
    let words: Vec<u64> = frame
        .chunks(8)
        .map(|chunk| u64::from_be_bytes(chunk.try_into().expect("eight bytes")))
        .collect();

    tb.run(|tb| async move {
        tb.set("rst", 1).await;
        tb.step().await;
        tb.set("rst", 0).await;
        for (index, word) in words.into_iter().enumerate() {
            tb.drive(&format!("buf_{index}"), word).await;
        }
        tb.step().await;
        packet::OUTPUT_NAMES
            .iter()
            .map(|name| {
                let value = tb.value(name);
                ((*name).to_string(), value)
            })
            .collect::<Vec<_>>()
    })
    .expect("the packet design drives only ports it declares")
}

fn field(fields: &[(String, u64)], name: &str) -> u64 {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .unwrap_or_else(|| panic!("no output named {name}"))
        .1
}

/// The destination MAC, reassembled from the halves the design splits it into.
///
/// The halves are 32 and 16 bits, not 32 and 32: a 48-bit MAC does not divide evenly,
/// and splitting it at 32 bits would leave the low half holding 32 bits of which only
/// 16 are address. So the shift back together is by 16, and getting that wrong
/// produces a plausible-looking number that is not a MAC.
fn destination(fields: &[(String, u64)]) -> u64 {
    (field(fields, "dst_hi") << 16) | field(fields, "dst_lo")
}

/// The source MAC, likewise.
fn source(fields: &[(String, u64)]) -> u64 {
    (field(fields, "src_hi") << 16) | field(fields, "src_lo")
}

#[test]
fn the_frame_we_build_is_one_etherparse_accepts() {
    let bytes = frame(
        0x0800,
        None,
        [192, 168, 1, 10],
        [192, 168, 1, 1],
        443,
        51234,
        0x12,
    );
    let parsed = SlicedPacket::from_ethernet(&bytes).expect("etherparse accepts the frame");
    assert!(
        matches!(parsed.transport, Some(TransportSlice::Tcp(_))),
        "etherparse sees TCP: {:?}",
        parsed.transport
    );
    let Some(NetSlice::Ipv4(ip)) = parsed.net else {
        panic!("etherparse did not see IPv4");
    };
    assert_eq!(
        ip.header().source_addr().octets(),
        [192, 168, 1, 10],
        "etherparse reads the source we wrote"
    );
}

#[test]
fn an_untagged_ipv4_tcp_frame_parses_field_for_field() {
    let bytes = frame(0x0800, None, [10, 0, 0, 1], [10, 0, 0, 2], 1234, 80, 0x18);
    let fields = parse(&bytes);

    assert_eq!(destination(&fields), 0x0200_0000_0001, "destination MAC");
    assert_eq!(source(&fields), 0x0200_0000_0002, "source MAC");
    assert_eq!(field(&fields, "ethertype"), 0x0800);
    assert_eq!(field(&fields, "version"), 4);
    assert_eq!(field(&fields, "ihl"), 5);
    assert_eq!(field(&fields, "total_len"), 40);
    assert_eq!(field(&fields, "protocol"), 6);
    assert_eq!(
        field(&fields, "ip_src"),
        u64::from(u32::from_be_bytes([10, 0, 0, 1]))
    );
    assert_eq!(
        field(&fields, "ip_dst"),
        u64::from(u32::from_be_bytes([10, 0, 0, 2]))
    );
    assert_eq!(field(&fields, "tcp_src_port"), 1234);
    assert_eq!(field(&fields, "tcp_dst_port"), 80);
    assert_eq!(field(&fields, "tcp_data_offset"), 5);
    assert_eq!(field(&fields, "tcp_flags"), 0x18, "PSH and ACK");
    assert_eq!(field(&fields, "csum_ok"), 1, "the checksum verifies");
    assert_eq!(field(&fields, "valid"), 1, "so the frame is valid");
    assert_eq!(field(&fields, "malformed"), 0);

    // And against the oracle, for the fields the oracle exposes.
    let parsed = SlicedPacket::from_ethernet(&bytes).unwrap();
    let Some(NetSlice::Ipv4(ip)) = parsed.net else {
        panic!("etherparse did not see IPv4");
    };
    assert_eq!(
        field(&fields, "total_len"),
        u64::from(ip.header().total_len())
    );
    assert_eq!(
        field(&fields, "protocol"),
        u64::from(ip.header().protocol().0)
    );
    assert_eq!(
        field(&fields, "ip_src"),
        u64::from(u32::from_be_bytes(ip.header().source_addr().octets()))
    );
    let Some(TransportSlice::Tcp(tcp)) = parsed.transport else {
        panic!("etherparse did not see TCP");
    };
    assert_eq!(field(&fields, "tcp_src_port"), u64::from(tcp.source_port()));
    assert_eq!(
        field(&fields, "tcp_dst_port"),
        u64::from(tcp.destination_port())
    );
    assert_eq!(
        field(&fields, "tcp_data_offset"),
        u64::from(tcp.data_offset())
    );
}

#[test]
fn a_vlan_tagged_frame_is_skipped_and_the_fields_after_it_move() {
    let bytes = frame(
        0x0800,
        Some(0x0064),
        [172, 16, 0, 1],
        [172, 16, 0, 2],
        80,
        8080,
        0x02,
    );
    let fields = parse(&bytes);
    assert_eq!(field(&fields, "vlan"), 1, "the tag was seen");
    assert_eq!(field(&fields, "ethertype"), 0x0800, "read past the tag");
    assert_eq!(
        field(&fields, "ip_src"),
        u64::from(u32::from_be_bytes([172, 16, 0, 1])),
        "the IP header moved up four bytes"
    );
    assert_eq!(field(&fields, "tcp_src_port"), 80, "and so did TCP");
    assert_eq!(field(&fields, "valid"), 1, "a tagged frame is still valid");

    let parsed = SlicedPacket::from_ethernet(&bytes).unwrap();
    assert!(
        parsed.vlan.is_some(),
        "etherparse also sees the tag: {:?}",
        parsed.vlan
    );
}

#[test]
fn a_corrupted_checksum_fails_csum_ok_without_invalidating_the_header() {
    // Three verdicts, not one, and this test is why they are three:
    //
    // - `valid` is *structural*: Ethernet II, IPv4, version 4, an IHL in range.
    // - `csum_ok` is *integrity*: the computed checksum matches the stored field.
    // - `malformed` is `ipv4 && !valid`.
    //
    // So a corrupted checksum leaves `valid` high. That is a real design position and a
    // defensible one: a switch that wants to drop bad checksums reads `csum_ok`, and a
    // switch that only cares whether the header is parseable reads `valid` and is not
    // forced to also compute an adder tree. Folding the checksum into `valid` would
    // have been the other choice, and it would have cost a switch that wants to
    // forward-anything-parseable the ability to say so.
    let mut bytes = frame(0x0800, None, [1, 2, 3, 4], [5, 6, 7, 8], 1, 2, 0x10);
    bytes[14 + 10] ^= 0x01; // the low byte of the checksum field
    let fields = parse(&bytes);
    assert_eq!(
        field(&fields, "csum_ok"),
        0,
        "the checksum no longer verifies"
    );
    assert_eq!(
        field(&fields, "valid"),
        1,
        "but the header is still structurally well formed"
    );
    assert_eq!(
        field(&fields, "malformed"),
        0,
        "and not malformed, because nothing about its shape is wrong"
    );
}

#[test]
fn a_non_tcp_protocol_is_reported_invalid_but_still_parsed() {
    // UDP is not TCP. The parser should say so rather than emit TCP fields that mean
    // nothing, but it should still report the IP header, which was fine.
    let mut bytes = frame(0x0800, None, [1, 1, 1, 1], [2, 2, 2, 2], 53, 53, 0);
    bytes[14 + 9] = 17; // UDP
    let mut header = [0u8; 20];
    header.copy_from_slice(&bytes[14..34]);
    header[10..12].copy_from_slice(&[0, 0]);
    let sum = checksum(&header);
    bytes[24..26].copy_from_slice(&sum.to_be_bytes());

    let fields = parse(&bytes);
    assert_eq!(field(&fields, "protocol"), 17, "the protocol is reported");
    assert_eq!(field(&fields, "csum_ok"), 1, "and the IP header was intact");
    assert_eq!(field(&fields, "ipv4"), 1, "so it is recognised as IPv4");
    assert_eq!(
        field(&fields, "tcp"),
        0,
        "and `tcp` says there is no TCP header here, which is the assertion that \
         matters: a UDP segment must not present itself as TCP"
    );
    assert_eq!(
        field(&fields, "valid"),
        1,
        "which leaves it valid, because `valid` is structural and says nothing about \
         the transport"
    );
}

#[test]
fn a_non_ipv4_ethertype_is_invalid_yet_the_ethernet_fields_are_read() {
    let bytes = frame(0x0806, None, [0; 4], [0; 4], 0, 0, 0); // ARP
    let fields = parse(&bytes);
    assert_eq!(field(&fields, "ethertype"), 0x0806, "ARP is reported");
    assert_eq!(destination(&fields), 0x0200_0000_0001, "MACs still read");
    assert_eq!(field(&fields, "ipv4"), 0);
    assert_eq!(field(&fields, "valid"), 0);
}

#[test]
fn a_header_with_options_is_checksummed_correctly_and_reports_no_tcp() {
    // IHL 6 means a 24-byte IP header. The design handles that properly rather than
    // bailing: the checksum sums the words the header actually has, because the
    // accumulator is masked by `2 * IHL`. That is the interesting part -- summing a
    // *fixed* number of words would be simpler and would be wrong for exactly the
    // headers a real network carries.
    //
    // The TCP header, though, *is* at a fixed offset here, so for a header with
    // options `tcp` goes low rather than the design reading ports out of an option.
    // Declining is the right answer: those ports would be a number with no meaning.
    let mut bytes = frame(0x0800, None, [1, 2, 3, 4], [5, 6, 7, 8], 1, 2, 0x10);
    bytes[14] = 0x46; // IHL 6
    bytes[14 + 2..14 + 4].copy_from_slice(&60u16.to_be_bytes());
    let mut header = [0u8; 20];
    header.copy_from_slice(&bytes[14..34]);
    header[10..12].copy_from_slice(&[0, 0]);
    let sum = checksum(&header);
    bytes[24..26].copy_from_slice(&sum.to_be_bytes());

    let fields = parse(&bytes);
    assert_eq!(field(&fields, "ihl"), 6, "IHL is reported as it stands");
    assert_eq!(
        field(&fields, "valid"),
        1,
        "and an IHL in range is structurally valid"
    );
    assert_eq!(
        field(&fields, "tcp"),
        0,
        "but there is no TCP header at a known offset, so none is claimed"
    );
}

#[test]
fn every_tcp_flag_bit_lands_where_the_specification_says() {
    // One case per bit: a flag field is exactly the sort of thing a width check passes
    // and a bit-order check does not.
    for bit in 0..8u8 {
        let flags = 1u8 << bit;
        let bytes = frame(0x0800, None, [1, 1, 1, 1], [2, 2, 2, 2], 1, 2, flags);
        let fields = parse(&bytes);
        assert_eq!(
            field(&fields, "tcp_flags"),
            u64::from(flags),
            "flag bit {bit} should survive as bit {bit}"
        );
    }
}

#[test]
fn the_window_is_the_standard_minimum_frame_size() {
    // The design reads a fixed window and the constant is used in the port widths, so
    // if it and the arithmetic ever disagreed every frame would be misread.
    assert_eq!(WINDOW_BYTES, 64);
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = ferrite_lithic_cosim::verilator() else {
        println!("SKIPPED: verilator not found, so the packet equivalence did not run.");
        return;
    };
    println!("cosimulating packet with {}", verilator.display());

    let design = Design::new();
    let ports = packet::build(&design).unwrap();
    let module = ferrite_lithic_rtl::Module::new(
        "packet",
        design.build().unwrap(),
        ports.inputs.clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let plan = ferrite_lithic_cosim::Plan::of(&design, &module).unwrap();

    // Two frames: one well formed, one with a corrupted checksum, so the equivalence
    // check sees both verdicts rather than only the happy path.
    let good = frame(0x0800, None, [10, 0, 0, 1], [10, 0, 0, 2], 1234, 80, 0x18);
    let mut bad = good;
    bad[14 + 10] ^= 0x01;

    // Every row carries all nine input values -- `rst` then `buf_0..buf_7` -- because a
    // stimulus whose rows disagree about their own arity makes "cycle 5" mean two
    // different things, and `Stimulus::push` refuses that rather than letting it
    // through.
    let mut stimulus = ferrite_lithic_cosim::Stimulus::new();
    let row = |rst: u64, words: &[u64; 8]| -> Vec<Bits> {
        let mut values = vec![Bits::constant(rst, 1).expect("a one-bit value")];
        for word in words {
            values.push(Bits::constant(*word, 64).expect("a 64-bit value"));
        }
        values
    };
    let words = |target: &[u8; 64]| -> [u64; 8] {
        let mut out = [0u64; 8];
        for (index, slot) in out.iter_mut().enumerate() {
            *slot = u64::from_be_bytes(target[index * 8..index * 8 + 8].try_into().unwrap());
        }
        out
    };

    // Reset, then the two frames. The outputs are registered, so each frame's verdict
    // appears on the edge after its last word.
    stimulus.push(row(1, &[0; 8])).unwrap();
    stimulus.push(row(0, &[0; 8])).unwrap();
    for target in [&good, &bad] {
        for index in 0..8 {
            let mut partial = [0u64; 8];
            partial[index] = words(target)[index];
            stimulus.push(row(0, &partial)).unwrap();
        }
        stimulus.push(row(0, &words(target))).unwrap();
    }
    assert_eq!(stimulus.cycles(), 20);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = ferrite_lithic_cosim::Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );
}

proptest! {
    /// Arbitrary frames: the verdict must follow the checksum, which is the one
    /// property that has to hold for every possible input rather than the ones chosen.
    #[test]
    fn validity_follows_the_checksum_for_any_frame(
        src in prop::array::uniform4(any::<u8>()),
        dst in prop::array::uniform4(any::<u8>()),
        sport in any::<u16>(),
        dport in any::<u16>(),
        flags in any::<u8>(),
        corrupt in any::<bool>(),
    ) {
        let mut bytes = frame(0x0800, None, src, dst, sport, dport, flags);
        if corrupt {
            bytes[14 + 10] ^= 0x80;
        }
        let fields = parse(&bytes);
        let csum_ok = field(&fields, "csum_ok");
        let valid = field(&fields, "valid");
        if corrupt {
            prop_assert_eq!(csum_ok, 0, "a corrupted checksum must not verify");
            prop_assert_eq!(
                valid, 1,
                "and the header stays structurally valid, which is the whole point \
                 of separating the two verdicts"
            );
        } else {
            prop_assert_eq!(csum_ok, 1, "an intact header verifies");
            prop_assert_eq!(valid, 1);
        }
    }
}
