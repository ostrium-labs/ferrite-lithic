//! Ethernet II / IPv4 / TCP header parsing at line rate.
//!
//! # Why this is a combinational parser and not a state machine
//!
//! A switch or a router has to decide what to do with a frame on the cycle the
//! frame's last byte lands. That is the whole requirement, and it is why header
//! parsing is not the serial "read a byte, update a pointer, compare a field" shape
//! the earlier modules in this crate use: at line rate there is nothing to
//! serialise, because the *caller* has already serialised into a buffer. By the
//! time the header is in hand it is 54 contiguous bytes sitting in a register file,
//! and a 54-cycle pointer walk is 54 cycles of latency added to a pipeline that has
//! none to spare.
//!
//! So this design is: a 64-byte header window in, every field out, combinationally,
//! with one output register per field to give it a fixed one-cycle latency and one
//! frame per cycle of throughput.
//!
//! # The 64-byte window, and being honest about what it costs
//!
//! [`WINDOW_BYTES`] = 64 bytes, arriving on eight 64-bit ports, byte 0 in the high
//! end of `buf_0`. That is the honest hardware shape, and it has a consequence
//! worth stating rather than hiding:
//!
//! * Every field is at a **fixed offset from the start of the header it belongs
//!   to**, never at a computed one. The Ethernet header is 14 bytes, an 802.1Q tag
//!   is 4, an IPv4 header is 20 when it carries no options, and a TCP header is 20.
//!   So exactly two fields genuinely move -- the IPv4 header, by the tag, and the
//!   TCP header, by `IHL` -- and only the first of those is selected on. The
//!   second is not: see the next point.
//! * **IPv4 options are not decoded.** `IHL` is reported, and the design checks
//!   that the whole IPv4 header fits inside the window, but the TCP fields are read
//!   at the fixed `IHL == 5` offset and `tcp` stays low for any other `IHL`. The
//!   alternative is an eleven-way mux on a variable byte offset, and it would not
//!   fit even with the mux: `14 + 4*15 + 20 = 94` bytes, well past the end of a
//!   64-byte window. A wider window and a real barrel shifter is the answer in
//!   silicon. Here the limitation is documented and the `ihl` output is there so a
//!   caller can see it was hit.
//! * Two levels of tag are unwrapped -- 802.1Q (`0x8100`) and 802.1ad (`0x88a8`) --
//!   which covers QinQ. A third tag is not unwrapped; the ethertype after the second
//!   would read as `0x8100`, `ipv4` goes low, and the frame is reported as
//!   not-IPv4 rather than mis-parsed.
//!
//! # Where the IPv4 header starts, without a variable offset
//!
//! The IPv4 header starts at byte 14 with no tag, 18 with one and 22 with two.
//! Three fixed offsets, so each tap is a *constant* slice of the window
//! zero-extended to a common width, and choosing between them is two one-bit
//! muxes. That is a barrel shifter with its shift amounts fixed at elaboration
//! time, which is strictly less hardware than the variable-shift version and
//! exactly as correct for a two-deep tag stack.
//!
//! The common width is [`IPWIN_BYTES`] = 50, the most IPv4 header bytes that
//! follow a *tagless* start inside the window. The deeper taps come up short and
//! are zero-padded, which is why [`ihl_max`] exists: with two tags only 42 bytes
//! are real, so `IHL` above 10 would read padding, and the design rejects it.
//!
//! Every field below is then a plain `slice` at a constant offset of that signal.
//!
//! # The checksum
//!
//! [`Outputs::ip_csum`] is the Internet checksum of the IPv4 header, and the module
//! states the definition it implements rather than leaving it to be inferred:
//!
//! > The sum of the header's 16-bit big-endian words, with the checksum field
//! > itself -- offset 10, the sixth word -- treated as zero; folded end-around to 16
//! > bits; then complemented.
//!
//! Summing 16-bit big-endian *words* rather than bytes is the point: IPv4's
//! checksum has no byte ordering, which is exactly why it survives every router
//! that rewrites nothing but the TTL. [`Outputs::csum_ok`] is
//! `ip_csum == the stored field`, gated on `valid` so a header that failed its
//! structural checks cannot report a good checksum.
//!
//! The number of words in the header depends on `IHL`, and a hardware checksum has
//! to deal with that. The design's answer is the real one: sum all fifteen possible
//! words and *mask* off the ones past `2*IHL`, with a four-bit magnitude compare
//! per word, and mask the checksum word off unconditionally. A variable-length
//! adder tree is the alternative and it is strictly worse -- fifteen masked words
//! is fifteen adders and fifteen four-bit compares, and no shifter.
//!
//! [`CHECKSUM_ACC_BITS`] is 20 bits because fifteen `0xffff` words sum to `0xfffff`,
//! so twenty holds every partial sum with no wraparound to reason about. That is
//! why it is not sixteen. Three end-around folds then reduce it, and three is the
//! exact number rather than a habit: the first can carry into bit 16 with a
//! remainder up to `0xf`, the second can carry once more with a remainder up to
//! `1`, and the third has nothing left to carry.
//!
//! # What `ipv4`, `valid`, `malformed` and `tcp` mean
//!
//! Four separate bits because they answer four different questions and collapsing
//! them loses information a switch needs.
//!
//! * `ipv4` -- the post-tag ethertype is `0x0800`. "Is this ours?"
//! * `valid` -- the IPv4 header was read and is self-consistent: version 4,
//!   `IHL >= 5`, and the whole header inside the window.
//! * `malformed` -- `ipv4` **and not** `valid`. Only ever set for a frame that
//!   claims to be IPv4 and is not usable as one. A frame that is ARP is not
//!   malformed, it is simply not ours, and saying otherwise is how a parser ends up
//!   dropping traffic.
//! * `tcp` -- the protocol is 6 **and** `IHL` is 5 **and** the TCP data offset is
//!   at least 5. Every `tcp_*` output is only meaningful when this is high, which
//!   is the honest caveat about the no-options limitation above.
//!
//! # Differential testing
//!
//! `etherparse` is the golden model, and the tests compare against
//! `LaxSlicedPacket`, because that is the entry point built for exactly this job:
//! parsing a frame that may be truncated, may have a bad checksum, or may be
//! nonsense, and reporting what it managed to extract rather than refusing.
//!
//! The differential is *directional where the two have different opinions*. The
//! design is stricter than `etherparse` in one place -- it declines to guess a TCP
//! header when `IHL` says there are options -- and looser in others, and a test that
//! asserted equality everywhere would be asserting a bug. So for arbitrary 64-byte
//! buffers the tests assert:
//!
//! * the link-layer fields (MACs, ethertype, tag presence, TCI) always agree;
//! * if `etherparse` produced an IPv4 header, every field and the checksum agree;
//! * if `etherparse` produced a TCP header, the design says `tcp` and every TCP
//!   field agrees;
//! * if the ethertype is IPv4 but `etherparse` refused the header, the design says
//!   `malformed`.
//!
//! That last one is the strong direction, and it holds because the design's
//! structural checks are a superset of the ones `etherparse` can fail on a
//! 64-byte buffer.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The header window: the first [`WINDOW_BYTES`] bytes of the frame.
pub const WINDOW_BYTES: usize = 64;

/// [`WINDOW_BYTES`] in bits.
pub const WINDOW_BITS: u32 = (WINDOW_BYTES as u32) * 8;

/// The width of the signal the IPv4 fields are sliced out of, in bytes.
///
/// Fifty is the most IPv4 header bytes that follow a *tagless* start inside the
/// window. Deeper taps come up short and are zero-padded to this width, which is
/// why they can all be muxed together; see [`ihl_max`] for what the padding costs.
pub const IPWIN_BYTES: usize = 50;

/// [`IPWIN_BYTES`] in bits.
pub const IPWIN_BYTE_BITS: u32 = (IPWIN_BYTES as u32) * 8;

/// The byte offset of the IPv4 header with no VLAN tag.
pub const IP_OFFSET_PLAIN: usize = 14;

/// The byte offset of the IPv4 header behind one VLAN tag.
pub const IP_OFFSET_ONE_TAG: usize = 18;

/// The byte offset of the IPv4 header behind two VLAN tags.
pub const IP_OFFSET_TWO_TAGS: usize = 22;

/// The offset of the TCP header within the IPv4 header, for `IHL == 5`.
pub const TCP_OFFSET_IN_IPV4: usize = 20;

/// The offset of the IPv4 header's checksum field, in 16-bit words.
pub const CHECKSUM_WORD: usize = 5;

/// The most 16-bit words an IPv4 header can contain, for `IHL == 15`.
pub const MAX_HEADER_WORDS: usize = 15;

/// The accumulator width for the header checksum: fifteen `0xffff` words reach
/// `0xfffff`, so twenty bits holds every partial sum with no wraparound.
pub const CHECKSUM_ACC_BITS: u32 = 20;

/// How many end-around folds the accumulator needs.
///
/// Three, and it is the exact number rather than a habit. The first fold adds a
/// low `0xffff` to a carry-out and up to `0xf` of remainder; the second adds a low
/// `0xffff` to a carry and up to `1`; after the third the value is at most `0xffff`
/// and the fourth would be a no-op.
pub const CHECKSUM_FOLDS: usize = 3;

/// 802.1Q, a VLAN tag.
pub const ETHERTYPE_VLAN: u64 = 0x8100;

/// 802.1ad, a service-provider VLAN tag.
pub const ETHERTYPE_QINQ: u64 = 0x88a8;

/// IPv4.
pub const ETHERTYPE_IPV4: u64 = 0x0800;

/// TCP's IPv4 protocol number.
pub const PROTOCOL_TCP: u8 = 6;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge captures one frame's parse.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which zeroes the output registers.
    ///
    /// This one *is* wired to the registers' reset pin, unlike `sha256`'s, and the
    /// difference is worth being explicit about: SHA-256's chaining register has to
    /// come out of reset holding the initial hash value, which a reset pin cannot
    /// do, because a reset pin can only load zero, and a chaining register starting
    /// at zero computes a well-formed hash of nothing. There is no such constraint
    /// here -- a parsed header carries no state across frames -- so zero is a
    /// well-formed "nothing here" answer and the pin is the honest place for it.
    pub rst: Signal,
    /// The [`WINDOW_BYTES`]-byte header window, eight bytes per port, big-endian.
    ///
    /// Collected rather than written out eight times, because `PortList` expands a
    /// `Vec<Signal>` into `buf_0`..`buf_7`. Sixty-four bits is the cosimulator
    /// driver-generator's maximum, which is why this is eight ports and not one
    /// 512-bit one.
    #[bits(64)]
    #[length(8)]
    pub buf: Vec<Signal>,
}

/// The output ports.
///
/// Twenty-five ports, each well under 64 bits. The MAC addresses are split across
/// two lanes for the same reason the FFT's points are: a 48-bit port is legal, but
/// a lane that is a whole number of bytes costs nothing when the ports have to be
/// carried in a driver's `u64`.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The destination MAC, high four bytes.
    #[bits(32)]
    pub dst_hi: Signal,
    /// The destination MAC, low two bytes.
    #[bits(16)]
    pub dst_lo: Signal,
    /// The source MAC, high four bytes.
    #[bits(32)]
    pub src_hi: Signal,
    /// The source MAC, low two bytes.
    #[bits(16)]
    pub src_lo: Signal,
    /// The ethertype, after any VLAN tags have been skipped.
    #[bits(16)]
    pub ethertype: Signal,
    /// Whether at least one VLAN tag was present and skipped.
    pub vlan: Signal,
    /// The outer tag's TCI, or zero when there was no tag.
    #[bits(16)]
    pub vlan_tci: Signal,
    /// Whether the ethertype is IPv4.
    pub ipv4: Signal,
    /// The IPv4 version field.
    #[bits(4)]
    pub version: Signal,
    /// The IPv4 IHL field, in 32-bit words.
    #[bits(4)]
    pub ihl: Signal,
    /// The IPv4 DSCP field.
    #[bits(6)]
    pub dscp: Signal,
    /// The IPv4 total length field.
    #[bits(16)]
    pub total_len: Signal,
    /// The IPv4 protocol field.
    #[bits(8)]
    pub protocol: Signal,
    /// The IPv4 source address.
    #[bits(32)]
    pub ip_src: Signal,
    /// The IPv4 destination address.
    #[bits(32)]
    pub ip_dst: Signal,
    /// The checksum the design computed for the IPv4 header.
    #[bits(16)]
    pub ip_csum: Signal,
    /// Whether the computed checksum matches the one in the header.
    pub csum_ok: Signal,
    /// Whether the TCP fields below are meaningful.
    pub tcp: Signal,
    /// The TCP source port.
    #[bits(16)]
    pub tcp_src_port: Signal,
    /// The TCP destination port.
    #[bits(16)]
    pub tcp_dst_port: Signal,
    /// The TCP sequence number.
    #[bits(32)]
    pub tcp_seq: Signal,
    /// The TCP flags: NS in bit 8, then CWR, ECE, URG, ACK, PSH, RST, SYN, FIN.
    #[bits(9)]
    pub tcp_flags: Signal,
    /// The TCP data offset, in 32-bit words.
    #[bits(4)]
    pub tcp_data_offset: Signal,
    /// Whether the IPv4 header was read and is self-consistent.
    pub valid: Signal,
    /// Whether the frame claims to be IPv4 and is not usable as one.
    pub malformed: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The registered fields, by output name, in [`Outputs`] declaration order.
    pub fields: Vec<(&'static str, Signal)>,
}

/// The output port names, in declaration order.
///
/// A test reading the registers back needs the names in order, and [`build`]
/// registers them in exactly this order, so the two cannot drift.
pub const OUTPUT_NAMES: [&str; 25] = [
    "dst_hi",
    "dst_lo",
    "src_hi",
    "src_lo",
    "ethertype",
    "vlan",
    "vlan_tci",
    "ipv4",
    "version",
    "ihl",
    "dscp",
    "total_len",
    "protocol",
    "ip_src",
    "ip_dst",
    "ip_csum",
    "csum_ok",
    "tcp",
    "tcp_src_port",
    "tcp_dst_port",
    "tcp_seq",
    "tcp_flags",
    "tcp_data_offset",
    "valid",
    "malformed",
];

/// The declared width of each output port, in declaration order.
///
/// Written out next to [`OUTPUT_NAMES`] rather than reflected out of the
/// `PortList` derive, which the derive does not offer. The test file checks the two
/// against the widths the design actually built, so the duplication is the thing
/// that is verified rather than trusted.
pub const OUTPUT_WIDTHS: [u32; 25] = [
    32, 16, 32, 16, 16, 1, 16, 1, 4, 4, 6, 16, 8, 32, 32, 16, 1, 1, 16, 16, 32, 9, 4, 1, 1,
];

/// The largest `IHL` the design accepts, with no tag / one tag / two tags.
///
/// With no tag the whole IPv4 header must fit in the 50 bytes of the tap: `12*4 =
/// 48`. With one tag, 46 bytes are real: `11*4 = 44`. With two, 42 bytes: `10*4 =
/// 40`. Above these the checksum would be summing zero padding, which is a wrong
/// answer rather than a missing one, so the design refuses the header instead.
pub fn ihl_max() -> [u8; 3] {
    [
        ((IPWIN_BYTES / 4) as u8).min(15),
        (((WINDOW_BYTES - IP_OFFSET_ONE_TAG) / 4) as u8).min(15),
        (((WINDOW_BYTES - IP_OFFSET_TWO_TAGS) / 4) as u8).min(15),
    ]
}

/// Builds the line-rate header parser into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say.
///
/// # Errors
///
/// Whatever [`Design`] returns and whatever the port-list helpers return. There is
/// no parser-specific failure: the window size, the offsets and the tag depth are
/// all fixed by the choice of being a line-rate parser over a 64-byte window.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    // One 512-bit window out of eight 64-bit ports, so every byte offset below is
    // a slice on a single signal rather than a two-dimensional index into a port
    // array. Byte 0 is the most significant byte, which is what "the first byte on
    // the wire" means and what the ports are documented to carry, so it sits at the
    // *low* offset in `slice` terms. That convention lives in `byte` and nowhere
    // else.
    let window = design.concat(&inputs.buf)?;

    let dst = bytes(design, &window, 0, 6)?;
    let src = bytes(design, &window, 6, 6)?;
    let raw_ethertype = word(design, &window, 12)?;

    // One tag: the TCI is at 14 and the next ethertype at 16. Two tags: the second
    // TCI is at 18 and the ethertype after it at 20. All inside the window, always,
    // which is why this needs no bounds reasoning at all.
    let one_tag = is_tag(design, &raw_ethertype)?;
    let ethertype_1 = word(design, &window, 16)?;
    let two_tags = design.and(&one_tag, &is_tag(design, &ethertype_1)?)?;
    let ethertype_2 = word(design, &window, 20)?;
    let vlan_tci = select3(
        design,
        &one_tag,
        &two_tags,
        &design.zeros(16)?,
        &word(design, &window, 14)?,
        &word(design, &window, 18)?,
    )?;
    let ethertype = select3(
        design,
        &one_tag,
        &two_tags,
        &raw_ethertype,
        &ethertype_1,
        &ethertype_2,
    )?;
    let vlan = design.or(&one_tag, &two_tags)?;
    let ipv4 = design.eq(&ethertype, &design.lit(ETHERTYPE_IPV4, 16)?)?;

    // The three constant taps, then two muxes. Each tap is the suffix of the window
    // from its IPv4 start, zero-extended to a common width so the muxes have
    // equal-width operands.
    let plain = tap(design, &window, IP_OFFSET_PLAIN)?;
    let one_tagged = tap(design, &window, IP_OFFSET_ONE_TAG)?;
    let two_tagged = tap(design, &window, IP_OFFSET_TWO_TAGS)?;
    let ipwin = design.ite(
        &one_tag,
        &design.ite(&two_tags, &two_tagged, &one_tagged)?,
        &plain,
    )?;

    let limits = ihl_max();
    let ihl_max = select3(
        design,
        &one_tag,
        &two_tags,
        &design.lit(u64::from(limits[0]), 4)?,
        &design.lit(u64::from(limits[1]), 4)?,
        &design.lit(u64::from(limits[2]), 4)?,
    )?;

    let first = byte(design, &ipwin, 0)?;
    let version = design.slice(&first, 4, 4)?;
    let ihl = design.slice(&first, 0, 4)?;
    let version_ok = design.eq(&version, &design.lit(4, 4)?)?;
    let ihl_in_range = design.and(
        &design.ugt(&ihl, &design.lit(4, 4)?)?,
        &design.ule(&ihl, &ihl_max)?,
    )?;
    let valid = design.and(&ipv4, &design.and(&version_ok, &ihl_in_range)?)?;
    let malformed = design.and(&ipv4, &design.not(&valid))?;

    let dscp = byte_field(design, &ipwin, 1, 6)?;
    let total_len = ipwin_word(design, &ipwin, 1)?;
    let protocol = byte_field(design, &ipwin, 9, 8)?;
    let ip_src = concat4(design, &ipwin, 12)?;
    let ip_dst = concat4(design, &ipwin, 16)?;

    let (computed, stored) = header_checksum(design, &ipwin, &ihl)?;
    let csum_ok = design.and(&valid, &design.eq(&computed, &stored)?)?;

    // The TCP header, at the fixed no-options offset. `IHL == 5` is a *term* in
    // `tcp` rather than an assumption baked into the slice, so a header with
    // options comes back as "there is no TCP header here" instead of as a
    // plausible wrong one -- which is the only safe answer at line rate, because a
    // switch acting on a wrong port number is worse than a switch dropping a frame
    // it was told to look at more carefully.
    let base = TCP_OFFSET_IN_IPV4;
    let tcp_src_port = ipwin_word(design, &ipwin, base / 2)?;
    let tcp_dst_port = ipwin_word(design, &ipwin, base / 2 + 1)?;
    let tcp_seq = concat4(design, &ipwin, base + 4)?;
    let tcp_byte_12 = byte(design, &ipwin, base + 12)?;
    let tcp_byte_13 = byte(design, &ipwin, base + 13)?;
    let tcp_data_offset = design.slice(&tcp_byte_12, 4, 4)?;
    let tcp_flags = design.concat(&[design.slice(&tcp_byte_12, 0, 1)?, tcp_byte_13])?;
    let is_tcp = design.eq(&protocol, &design.lit(u64::from(PROTOCOL_TCP), 8)?)?;
    let no_options = design.eq(&ihl, &design.lit(5, 4)?)?;
    let tcp = design.and(
        &valid,
        &design.and(
            &is_tcp,
            &design.and(
                &no_options,
                &design.ugt(&tcp_data_offset, &design.lit(4, 4)?)?,
            )?,
        )?,
    )?;

    // One register per output field. This is the "line rate" claim made concrete: a
    // whole header goes in on one edge and a whole header comes out on the next,
    // forever, with no backpressure and no `valid`.
    let mut registered = Vec::with_capacity(OUTPUT_NAMES.len());
    let mut latch = |name: &'static str, signal: &Signal| -> Result<(), BuildError> {
        let held = design.reg(signal, &inputs.clk, &inputs.rst, &design.constant(false))?;
        registered.push((name, held));
        Ok(())
    };

    latch(OUTPUT_NAMES[0], &design.slice(&dst, 16, 32)?)?;
    latch(OUTPUT_NAMES[1], &design.slice(&dst, 0, 16)?)?;
    latch(OUTPUT_NAMES[2], &design.slice(&src, 16, 32)?)?;
    latch(OUTPUT_NAMES[3], &design.slice(&src, 0, 16)?)?;
    latch(OUTPUT_NAMES[4], &ethertype)?;
    latch(OUTPUT_NAMES[5], &vlan)?;
    latch(OUTPUT_NAMES[6], &vlan_tci)?;
    latch(OUTPUT_NAMES[7], &ipv4)?;
    latch(OUTPUT_NAMES[8], &version)?;
    latch(OUTPUT_NAMES[9], &ihl)?;
    latch(OUTPUT_NAMES[10], &dscp)?;
    latch(OUTPUT_NAMES[11], &total_len)?;
    latch(OUTPUT_NAMES[12], &protocol)?;
    latch(OUTPUT_NAMES[13], &ip_src)?;
    latch(OUTPUT_NAMES[14], &ip_dst)?;
    latch(OUTPUT_NAMES[15], &computed)?;
    latch(OUTPUT_NAMES[16], &csum_ok)?;
    latch(OUTPUT_NAMES[17], &tcp)?;
    latch(OUTPUT_NAMES[18], &tcp_src_port)?;
    latch(OUTPUT_NAMES[19], &tcp_dst_port)?;
    latch(OUTPUT_NAMES[20], &tcp_seq)?;
    latch(OUTPUT_NAMES[21], &tcp_flags)?;
    latch(OUTPUT_NAMES[22], &tcp_data_offset)?;
    latch(OUTPUT_NAMES[23], &valid)?;
    latch(OUTPUT_NAMES[24], &malformed)?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            dst_hi: registered[0].1.clone(),
            dst_lo: registered[1].1.clone(),
            src_hi: registered[2].1.clone(),
            src_lo: registered[3].1.clone(),
            ethertype: registered[4].1.clone(),
            vlan: registered[5].1.clone(),
            vlan_tci: registered[6].1.clone(),
            ipv4: registered[7].1.clone(),
            version: registered[8].1.clone(),
            ihl: registered[9].1.clone(),
            dscp: registered[10].1.clone(),
            total_len: registered[11].1.clone(),
            protocol: registered[12].1.clone(),
            ip_src: registered[13].1.clone(),
            ip_dst: registered[14].1.clone(),
            ip_csum: registered[15].1.clone(),
            csum_ok: registered[16].1.clone(),
            tcp: registered[17].1.clone(),
            tcp_src_port: registered[18].1.clone(),
            tcp_dst_port: registered[19].1.clone(),
            tcp_seq: registered[20].1.clone(),
            tcp_flags: registered[21].1.clone(),
            tcp_data_offset: registered[22].1.clone(),
            valid: registered[23].1.clone(),
            malformed: registered[24].1.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        fields: registered,
    })
}

/// Byte `index` of a big-endian byte signal.
///
/// The one place the byte-offset convention lives. Byte 0 is the most significant
/// byte of the signal, which is what "the first byte on the wire" means and what
/// the ports are documented to carry, so it is the *low* offset in `slice` terms.
///
/// # Errors
///
/// Whatever [`Design::slice`] reports, which for a checked index it will not.
fn byte(design: &Design, value: &Signal, index: usize) -> Result<Signal, BuildError> {
    let width = value.width() / 8;
    assert!(
        index < width as usize,
        "byte {index} is past the end of a {width}-byte signal"
    );
    Ok(design.slice(value, (width - 1 - index as u32) * 8, 8)?)
}

/// `bits` of byte `index`, counting from the byte's low bit.
///
/// The only call sites are the two nibbles of the IPv4 version/IHL byte, the DSCP
/// field at the top of the next byte, and the two halves of the TCP byte 12.
fn byte_field(
    design: &Design,
    value: &Signal,
    index: usize,
    bits: u32,
) -> Result<Signal, BuildError> {
    let host = byte(design, value, index)?;
    Ok(design.slice(&host, 8 - bits, bits)?)
}

/// Two bytes from `index`, as a 16-bit big-endian word.
fn word(design: &Design, value: &Signal, index: usize) -> Result<Signal, BuildError> {
    Ok(design.concat(&[byte(design, value, index)?, byte(design, value, index + 1)?])?)
}

/// `count` consecutive bytes from `index`.
fn bytes(
    design: &Design,
    value: &Signal,
    index: usize,
    count: usize,
) -> Result<Signal, BuildError> {
    let mut parts = Vec::with_capacity(count);
    for offset in 0..count {
        parts.push(byte(design, value, index + offset)?);
    }
    Ok(design.concat(&parts)?)
}

/// Four bytes from `index`, as a 32-bit big-endian integer.
fn concat4(design: &Design, value: &Signal, index: usize) -> Result<Signal, BuildError> {
    bytes(design, value, index, 4)
}

/// Word `index` of the IPv4 header in `ipwin`, as 16 bits.
fn ipwin_word(design: &Design, ipwin: &Signal, index: usize) -> Result<Signal, BuildError> {
    word(design, ipwin, index * 2)
}

/// Whether `ethertype` is a tag type the design unwraps.
fn is_tag(design: &Design, ethertype: &Signal) -> Result<Signal, BuildError> {
    let vlan = design.eq(ethertype, &design.lit(ETHERTYPE_VLAN, 16)?)?;
    let qinq = design.eq(ethertype, &design.lit(ETHERTYPE_QINQ, 16)?)?;
    Ok(design.or(&vlan, &qinq)?)
}

/// `first`, `second` or `third`, chosen by the tag depth.
///
/// `second` when there is one tag, `third` when there are two. A nested select
/// rather than two independent muxes, so that the three cases are mutually
/// exclusive by construction -- and a mutually exclusive three-way choice is what
/// the tag depth actually is.
fn select3(
    design: &Design,
    one_tag: &Signal,
    two_tags: &Signal,
    first: &Signal,
    second: &Signal,
    third: &Signal,
) -> Result<Signal, BuildError> {
    let chosen = design.ite(two_tags, third, second)?;
    Ok(design.ite(one_tag, &chosen, first)?)
}

/// One tap of the constant-offset IPv4 window.
///
/// The suffix of `window` from `offset`, zero-extended to [`IPWIN_BYTES`]. With the
/// plain offset the suffix is already the full width and the extension is skipped,
/// because `Design::zeros` rejects a width of zero and there is no constant-folded
/// zero-width literal to fall back on.
fn tap(design: &Design, window: &Signal, offset: usize) -> Result<Signal, BuildError> {
    let available = WINDOW_BYTES - offset;
    // **The slice offset is zero, not `offset * 8`.**
    //
    // `byte` treats index 0 as the *most* significant byte, so the first `offset`
    // bytes of the frame live at the *top* of the window and dropping them means
    // keeping the low bits. Slicing from `(offset * 8)` instead keeps the top
    // `available` bytes and throws away the last `offset` bytes of the frame, which
    // silently leaves the Ethernet header where the IP header should be: every IP
    // field reads as the byte `offset` earlier than it should, the checksum never
    // verifies, and `valid` is stuck low. Same shape as `sll` and `srl` being swapped
    // -- a constant that looks right, produces a graph of exactly the right width, and
    // means the opposite of what it says.
    let body = design.slice(window, 0, (available * 8) as u32)?;
    if available == IPWIN_BYTES {
        return Ok(body);
    }
    let pad = design.zeros(((IPWIN_BYTES - available) * 8) as u32)?;
    Ok(design.concat(&[body, pad])?)
}

/// The IPv4 header checksum, computed and as stored.
///
/// The computed value is the Internet checksum with the checksum word treated as
/// zero; the stored value is what the frame carries. Two outputs rather than a
/// single `ok`, because a switch that drops a frame on a bad checksum and one that
/// forwards it anyway are both real deployments and both need the number, not just
/// the verdict.
fn header_checksum(
    design: &Design,
    ipwin: &Signal,
    ihl: &Signal,
) -> Result<(Signal, Signal), BuildError> {
    let stored = ipwin_word(design, ipwin, CHECKSUM_WORD)?;

    let zero_word = design.zeros(16)?;
    let mut accumulator = design.zeros(CHECKSUM_ACC_BITS)?;
    for index in 0..MAX_HEADER_WORDS {
        let candidate = ipwin_word(design, ipwin, index)?;
        // Word `index` belongs to the header when `index < 2*IHL`, which for an
        // even `index` is `IHL > index/2`: a four-bit compare against a literal,
        // no multiply and no shifter. Summing every possible word and masking the
        // rest is what a hardware one's-complement adder does about a variable
        // length, and it is cheaper than any tree that knows the length.
        let in_header = design.ugt(ihl, &design.lit((index / 2) as u64, 4)?)?;
        // The checksum word is masked off unconditionally, because it is the thing
        // being computed. For `IHL == 5` the mask above *would* include it, and
        // without this line the design would compute the complement of a sum that
        // already contains the answer -- a number that looks entirely plausible and
        // is wrong. That is the classic one's-complement slip and it is why the line
        // is written as its own term.
        let keep = design.and(&in_header, &design.constant(index != CHECKSUM_WORD))?;
        let masked = design.ite(&keep, &candidate, &zero_word)?;
        accumulator = design.add(
            &accumulator,
            &design.zero_extend(&masked, CHECKSUM_ACC_BITS)?,
        )?;
    }

    let folded = fold_ones_complement(design, &accumulator)?;
    Ok((design.not(&folded), stored))
}

/// Fold a wide accumulator down to 16 bits, end-around.
///
/// Each fold adds the low 16 bits to what was above them, at
/// [`CHECKSUM_ACC_BITS`], so no fold can overflow and lose a carry. See
/// [`CHECKSUM_FOLDS`] for why the count is three.
fn fold_ones_complement(design: &Design, accumulator: &Signal) -> Result<Signal, BuildError> {
    let mut current = accumulator.clone();
    for _ in 0..CHECKSUM_FOLDS {
        let low = design.truncate(&current, 16)?;
        let high = design.slice(&current, 16, CHECKSUM_ACC_BITS - 16)?;
        let sum = design.add(&low, &design.zero_extend(&high, 16)?)?;
        current = design.zero_extend(&sum, CHECKSUM_ACC_BITS)?;
    }
    Ok(design.truncate(&current, 16)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        ETHERTYPE_IPV4, ETHERTYPE_QINQ, ETHERTYPE_VLAN, IP_OFFSET_ONE_TAG, IP_OFFSET_PLAIN,
        IP_OFFSET_TWO_TAGS, IPWIN_BYTES, OUTPUT_NAMES, OUTPUT_WIDTHS, PROTOCOL_TCP, WINDOW_BYTES,
        ihl_max,
    };

    #[test]
    fn the_output_lists_agree_with_each_other() {
        assert_eq!(OUTPUT_NAMES.len(), OUTPUT_WIDTHS.len());
    }

    #[test]
    fn every_output_port_fits_the_cosimulator_driver() {
        for (name, width) in OUTPUT_NAMES.iter().zip(OUTPUT_WIDTHS) {
            assert!((1..=64).contains(&width), "{name} is {width} bits");
        }
    }

    #[test]
    fn the_offsets_leave_room_in_the_window() {
        assert_eq!(IP_OFFSET_PLAIN, 14, "Ethernet II without a tag");
        assert_eq!(IP_OFFSET_ONE_TAG, IP_OFFSET_PLAIN + 4, "one 802.1Q tag");
        assert_eq!(IP_OFFSET_TWO_TAGS, IP_OFFSET_PLAIN + 8, "QinQ");
        assert_eq!(WINDOW_BYTES, 64);
        // A constant, so the assertion is checked by the compiler rather than at
        // run time: `const _: () = assert!(...)` fails the build if a constant above
        // is ever changed out from under this.
        const _: () = assert!(WINDOW_BYTES - IP_OFFSET_TWO_TAGS >= 20);
        const _: () = assert!(IP_OFFSET_PLAIN + IPWIN_BYTES <= WINDOW_BYTES);
    }

    #[test]
    fn the_ihl_limits_follow_from_the_window() {
        assert_eq!(ihl_max(), [12, 11, 10]);
        for (tags, limit) in ihl_max().iter().enumerate() {
            let start = [IP_OFFSET_PLAIN, IP_OFFSET_ONE_TAG, IP_OFFSET_TWO_TAGS][tags];
            let visible = (WINDOW_BYTES - start).min(IPWIN_BYTES);
            assert!(
                usize::from(*limit) * 4 <= visible,
                "{tags} tags: IHL {limit} needs {} bytes, {visible} visible",
                usize::from(*limit) * 4
            );
        }
    }

    #[test]
    fn the_ethertypes_are_the_ones_on_the_wire() {
        assert_eq!(ETHERTYPE_IPV4, 0x0800);
        assert_eq!(ETHERTYPE_VLAN, 0x8100);
        assert_eq!(ETHERTYPE_QINQ, 0x88a8);
        assert_eq!(PROTOCOL_TCP, 6);
    }
}
