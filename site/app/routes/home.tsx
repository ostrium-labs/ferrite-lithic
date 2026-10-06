import { FerriteAnimatedMark } from "../components/brand/FerriteAnimatedMark";
/**
 * The landing page.
 *
 * Its job is to make one argument — *one graph, two backends, one clock* — and then get
 * out of the way. So the hero is a timing diagram rather than a logo or a screenshot, the
 * claims are specific enough to check, and the page states its own gaps before a reader
 * has to ask.
 */

import { useEffect, useRef, useState } from "react";
import { Link } from "react-router";

import { useCanvas, useOnceCanvas } from "../components/canvas";
import { Instrument } from "../components/Instrument";
import { CodeBlock } from "../components/CodeBlock";
import { CrateArchitecture } from "../components/CrateArchitecture";
import { VerificationLoop } from "../components/VerificationLoop";
import { SOURCE_ROOT } from "../content/navigation";
import { BenchmarkFigure } from "../components/BenchmarkFigure";
import { CycleWalk } from "../components/CycleWalk";
import { TIERS, PROJECT_STATS } from "../lib/data";
import { drawTimingHero } from "../demos/hero";
import { drawPipeline } from "../demos/pipeline";
import { drawBitStream, drawHuffman } from "../demos/bitdemos";
import { mountHexDiff } from "../demos/hexdiff";
import { mountMux } from "../demos/mux";

const HERO_DURATION = 2200;

export function meta() {
  return [
    { title: "Ferrite Lithic — an embedded hardware DSL in Rust" },
    {
      name: "description",
      content:
        "An embedded hardware DSL, a cycle simulator and a Verilog emitter, written in Rust. Hardcaml's model, reimplemented natively. A corpus of independently checked algorithms, each checked three ways.",
    },
  ];
}

/* ------------------------------------------------------------------- pieces */

function Stat({
  label,
  value,
  suffix,
  note,
}: {
  label: string;
  value: number;
  suffix?: string;
  note: string;
}) {
  return (
    <div className="stat">
      <dt>{label}</dt>
      <dd>
        <span>{value}</span>
        {suffix}
        <p>{note}</p>
      </dd>
    </div>
  );
}

/** Renders a code sample with the same four-token highlighting the design uses elsewhere. */
function Code({ source }: { source: string }) {
  const tokens = source.split("\n");
  return (
    <pre tabIndex={0}>
      {tokens.map((line, index) => {
        const comment = line.includes("//") ? line.indexOf("//") : -1;
        const code = comment >= 0 ? line.slice(0, comment) : line;
        const tail = comment >= 0 ? line.slice(comment) : "";
        return (
          // eslint-disable-next-line react/no-array-index-key
          <span key={index}>
            {highlight(code)}
            {tail ? <span className="tok-c">{tail}</span> : null}
            {index < tokens.length - 1 ? "\n" : null}
          </span>
        );
      })}
    </pre>
  );
}

/** Four roles only: keyword, call, type, string. No fifth hue. */
function highlight(code: string) {
  const pattern = /(\b(?:use|let|fn|pub|struct|impl|mod|const|match|for|if|else|return)\b)|(\b[A-Za-z_][A-Za-z0-9_]*\s*(?=\())|(&[a-z_]+)|("[^"]*")/g;
  const out: React.ReactNode[] = [];
  let at = 0;
  let match = pattern.exec(code);

  while (match !== null) {
    if (match.index > at) out.push(code.slice(at, match.index));
    const kind = match[1] ? "tok-k" : match[2] ? "tok-f" : match[3] ? "tok-k" : "tok-str";
    out.push(
      <span className={kind} key={match.index}>
        {match[0]}
      </span>,
    );
    at = match.index + match[0].length;
    match = pattern.exec(code);
  }
  if (at < code.length) out.push(code.slice(at));
  return out;
}

function Hero() {
  const [replay, setReplay] = useState(0);
  const ref = useOnceCanvas(HERO_DURATION, drawTimingHero, replay);
  return <figure className="hero__diagram">
    <Instrument title="Register timing / illustrative trace" description="Scrollable register timing diagram" compact controls={<button type="button" onClick={() => setReplay(value => value + 1)}>Replay trace</button>}>
      <canvas ref={ref} id="timing-hero" width={720} height={360} role="img" aria-label="Illustrative register trace: reset clears q at the first edge. On subsequent rising clock edges, q takes d's previous value. A witness bracket marks the setup interval before the third edge." />
    </Instrument>
    <div className="hero__contract"><Link to="/docs/ir">One graph</Link><span aria-hidden="true">→</span><Link to="/docs/sim">Simulator</Link><span aria-hidden="true">⇄</span><Link to="/docs/cosim">Emitted RTL / Verilator</Link></div>
    <figcaption>One clock. The same stimulus. Compare the outputs at every edge.</figcaption>
  </figure>;
}

/* --------------------------------------------------------------- the canvas demos */

function Pipeline() {
  const [paused, setPaused] = useState(false);
  const [selected, setSelected] = useState<number | null>(null);
  const ref = useCanvas((context, width, height, elapsed) => drawPipeline(context, width, height, elapsed, selected), paused);
  const stages = ["Rust builder", "Design / IR", "Simulator", "RTL emitter", "Sim output", "Verilator output", "Plan + stimulus", "Compare"];
  return <figure className="pipeline">
    <Instrument title="Construction → execution → comparison" description="Scrollable Ferrite architecture diagram" controls={<button type="button" onClick={() => setPaused(value => !value)} aria-pressed={paused}>{paused ? "Resume signal" : "Pause signal"}</button>}>
      <canvas ref={ref} id="pipeline-canvas" width={1200} height={300} role="img" aria-label="Rust builds one Design graph. The simulator executes it; the RTL emitter produces Verilog run by Verilator. A Plan selects matching columns and stimulus. The comparison reports agreement or the first differing cycle." />
    </Instrument>
    <div className="pipeline__stages" aria-label="Trace a stage’s downstream path">{stages.map((stage, index) => <button type="button" key={stage} aria-pressed={selected === index} onMouseEnter={() => setSelected(index)} onFocus={() => setSelected(index)} onClick={() => setSelected(selected === index ? null : index)}>{stage}</button>)}</div>
    <figcaption>Inspect a stage to trace its downstream path. <code>Plan</code> chooses the columns; identical stimulus makes the two runs comparable. This is an architectural illustration, not a live verification run.</figcaption>
  </figure>;
}

function BitStream() {
  const [paused, setPaused] = useState(false);
  const ref = useCanvas(drawBitStream, paused);
  return (
    <>
      <Instrument title="DEFLATE / bit order" description="Scrollable least-significant-bit-first explanation" controls={<button type="button" onClick={() => setPaused(value => !value)} aria-pressed={paused}>{paused ? "Resume" : "Pause"}</button>}>
      <canvas ref={ref} id="bitstream-canvas" role="img" aria-label="DEFLATE emits the bits of each byte in increasing bit order, starting at bit zero." width={1100} height={300} />
      </Instrument>
      <div className="demo__foot">
        Bit 0 of each byte goes first, then bit 1, up to bit 7, then the next byte's bit
        0. Watch the marker: nine cycles per byte, because this design will not load a byte
        over bits it has not emitted yet.
      </div>
    </>
  );
}

function Huffman() {
  const [paused, setPaused] = useState(false);
  const ref = useCanvas(drawHuffman, paused);
  return (
    <>
      <Instrument title="Huffman / decode lookup" description="Scrollable nine-bit Huffman lookup" controls={<button type="button" onClick={() => setPaused(value => !value)} aria-pressed={paused}>{paused ? "Resume" : "Pause"}</button>}>
      <canvas ref={ref} id="huffman-canvas" role="img" aria-label="Nine incoming bits are reversed into a Huffman table index to retrieve a symbol and its code length." width={1100} height={340} />
      </Instrument>
      <div className="demo__foot">
        The reversal is a wire permutation and costs no gates in silicon. Here it is nine
        one-bit selects, because the DSL has no way to say "the same bits, in the other
        order". Run against DEFLATE's fixed literal/length code — the RFC's own published
        numbers.
      </div>
    </>
  );
}

/** The hex diff and the mux tree are DOM rather than canvas, so they mount imperatively. */
function useDomDemo<T extends HTMLElement>(mount: (node: T) => (() => void) | void) {
  const ref = useRef<T | null>(null);
  useEffect(() => {
    if (!ref.current) return;
    return mount(ref.current);
  }, []);
  return ref;
}

function HexDiff() {
  const ref = useDomDemo<HTMLDivElement>((node) => mountHexDiff(node));
  return <div className="hexdiff" ref={ref} />;
}

function Mux() {
  const ref = useDomDemo<HTMLDivElement>((node) => mountMux(node));
  return <div className="mux" ref={ref} />;
}

/* ------------------------------------------------------------------- the page */

export default function Home() {
  return (
    <>
      <header className="hero shell">
        <div className="hero__provenance"><span className="hero__identity"><FerriteAnimatedMark /><span>ferrite-lithic</span></span><a href={`${SOURCE_ROOT}README.md`}>Apache-2.0 / source & verification</a></div>
        <div className="hero__inner page-grid">
          <div className="hero__copy">
            <h1 className="hero__title"><span>Hardware you write</span><span>in Rust, as real gates.</span></h1>
            <p className="lede">An embedded hardware DSL, a cycle simulator and a Verilog emitter. Build one graph in Rust. Check that both backends agree on every clock edge.</p>
            <div className="landing-cta"><Link className="cta-button cta-button--primary" to="/docs">Explore the toolchain</Link><Link className="cta-button" to="/docs/basics">Start with circuit basics →</Link></div>
            <p className="hero__lineage">Hardcaml’s model, implemented natively in Rust.</p>
          </div>
          <Hero />
          <dl className="stats">
            <Stat label="tests / recorded snapshot" value={PROJECT_STATS.tests} note="unit, integration and documentation tests" />
            <Stat label="corpus algorithms" value={PROJECT_STATS.designs} note={`${PROJECT_STATS.tiers} tiers, ${PROJECT_STATS.checksPerDesign} complementary checks`} />
            <Stat label="crates" value={PROJECT_STATS.crates} note="nine tools and one adversarial corpus" />
            <Stat label="Verilator coverage" value={PROJECT_STATS.cosimCoverage} suffix="%" note="recorded corpus equivalence coverage" />
          </dl>
        </div>
      </header>

      <div className="landing-chapters">
        <section className="section" id="what">
          <h2>A library, not a language</h2>
          <div className="prose">
            <p>
              Most hardware is described in a language that is not yours. This one is a
              Rust library: you call a builder, it hands you back a <code>Design</code>, and
              that design is a graph of nodes and edges with widths — the same thing a
              simulator needs and the same thing an emitter needs.
            </p>
            <p>
              One graph, two backends, and an equivalence check between them on every
              single corpus design. That check is the point. A simulator and an emitter that
              both produce plausible answers will agree on every test anybody bothers to
              write — until one of them is wrong in a corner.
            </p>
          </div>

          <div className="code-panel">
            <div className="code-panel__head">
              <span className="dot" />
              <span className="code-panel__name">the whole shape of a design</span>
            </div>
            <CodeBlock label="A design, two consumers"><Code
              source={`use ferrite_lithic::Design;

let design = Design::new();
let clk  = design.clock();
let rst  = design.reset();

// a bit of state and one combinational path
let state = design.reg(&next, &clk, &rst, &hold)?;
let out   = design.eq(&state, &target)?;

// the two backends, from the same graph
ferrite_lithic_sim::Sim::new(&design);   // cycle-accurate
design.emit_verilog("top");              // structural`}
            /></CodeBlock>
          </div>
        </section>

        <section className="section" id="pipeline">
          <h2>One graph, two backends, one arbiter</h2>
          <p className="prose">
            The pipeline below is animated on a real clock. Data flows left to right; the
            equivalence check at the end compares the two right-hand branches{" "}
            <em>cycle by cycle</em> over a real stimulus, and reports the first cycle they
            disagree on.
          </p>
          <Pipeline />
          <div className="callout">
            <h3>Why the equivalence check fails the build when it is skipped</h3>
            <p>
              Verilator and Icarus are looked for rather than required, so{" "}
              <code>cargo test</code> works without them — and both{" "}
              <strong>print what they skipped</strong>. The CI job installs both and then{" "}
              <em>fails if anything skipped</em>, because a green run in which every
              equivalence test took its skip path is green and meaningless.
            </p>
          </div>
        </section>

        <CycleWalk />

        <section className="section" id="lineage">
          <h2>Where the design came from</h2>
          <div className="prose">
            <p>
              The architecture is a port of <strong>Hardcaml</strong>, Jane Street's OCaml
              hardware DSL, and that is worth understanding before reading any of the nine
              crates — several choices that look arbitrary are Hardcaml's choices that
              survived the translation.
            </p>
            <p>
              Hardcaml designs are OCaml programs that build a data structure describing a
              circuit. Rust does not check widths at compile time here, and it cannot make
              combinational loops impossible; what it gives instead is memory safety without
              a garbage collector, handles that cannot outlive their graph, and{" "}
              <code>Result</code> on every builder operation. The{" "}
              <Link to="/docs/why-rust">honest accounting</Link> is four pages long and worth
              the read.
            </p>
          </div>
          <ol className="lineage-chain" aria-label="Ferrite’s architectural lineage">
            <li><Link to="/docs/ocaml">OCaml</Link><p>A program constructs hardware, rather than describing its execution.</p></li>
            <li><Link to="/docs/hardcaml">Hardcaml</Link><p>The graph, clock-edge model and explicit checks establish the contract.</p></li>
            <li><Link to="/docs/why-rust">Ferrite / Rust</Link><p>Native ownership and error handling. Runtime widths, with verification doing the work.</p></li>
          </ol>
        </section>

        <section className="section" id="crates">
          <h2>Ten crates, in dependency order</h2>
          <p className="prose">
            Small crates with explicit contracts. Inspect the production dependencies, then
            follow the walkthroughs in reading order. Test counts are the recorded snapshot.
          </p>
          <CrateArchitecture />
        </section>

        <section className="section section--alt" id="corpus">
          <h2>Why a corpus, and what it cost</h2>
          <div className="prose">
            <p>
              Every other crate is a tool, and a tool gets tested with inputs chosen by
              whoever wrote it — so its tests drift toward what the tool was built to do. A
              corpus entry is a real algorithm written the way hardware would want it,
              checked three ways: against the <strong>original crate</strong> as the golden
              model, through the <strong>step testbench</strong>, and against{" "}
              <strong>Verilator</strong>.
            </p>
            <p>
              It paid for itself immediately. <code>crc3</code> — a three-bit LFSR, one
              byte per cycle, the smallest design with a register and a bit stream — found
              four defects that 471 tests of the tools had not, including{" "}
              <code>Design::sll</code> and <code>Design::srl</code> being{" "}
              <strong>exactly swapped</strong> for the whole of A1 and A2. Every crate
              agreed with every other crate about it, because they were all wrong the same
              way.
            </p>
          </div>

          <VerificationLoop />
          <div className="tiers">
            <h3>{PROJECT_STATS.tiers} tiers, {PROJECT_STATS.designs} algorithms</h3>
            <ul className="tier-list">
              {TIERS.map((tier) => (
                <li className="tier" key={tier.name}>
                  <span className="tier__name" title={tier.why}>
                    {tier.name}
                  </span>
                  <span className="tier__designs">
                    {tier.designs.map((design) => (
                      <code className="tier__chip" key={design}>
                        {design}
                      </code>
                    ))}
                  </span>
                  <span className="tier__n">{tier.designs.length}</span>
                </li>
              ))}
            </ul>
          </div>
          <p className="corpus__recurrence"><code>lfsr</code> is the shared reflected recurrence, not an additional corpus algorithm.</p>

          <div className="callout callout--loss">
            <h3>The honest benchmark: the CPU wins memchr, by 4.2×</h3>
            <p>
              The automaton design is a real design, and it still loses. Measured per byte
              against the <code>memchr</code> crate at an assumed 1&nbsp;GHz: the design
              retires <strong>one byte per clock edge</strong>, the crate retires{" "}
              <strong>32 bytes per AVX2 instruction</strong>. That is the entire gap, and
              the argument for hardware is not that it wins here.
            </p>
            <BenchmarkFigure />
            <p className="callout__foot">
              Across the corpus, two of five designs beat their reference implementation:{" "}
              <code>crc3</code> by 2.2× and <code>hex</code> by 1.4×. Both win by{" "}
              <em>doing less</em> — a three-bit state and a lookup table have no generality
              to pay for. The ones that lose lose because the CPU has more parallelism
              available than a 31-node circuit uses.{" "}
              <Link to="/docs/benchmarks">Why hardware loses here, and when it wins →</Link>
            </p>
            <p>
              It is in the corpus anyway, because it is the baseline the other two automata
              are measured against, and because a design that claimed to beat AVX2 at
              single-byte search would have been dishonest. The surviving argument for the
              tier is the <em>multi-pattern</em> search, where no SIMD form exists and a
              19-state automaton shares one ROM across lanes.
            </p>
          </div>
        </section>

        <section className="section" id="bugs">
          <h2>Defects that plausible answers hide</h2>
          <p className="prose">
            Every real defect this corpus has found looks the same: a constant that looks
            right, builds a graph of exactly the right width, and means the opposite of
            what it says. Here are two you can watch happen.
          </p>

          <figure className="demo" id="fnv-demo">
            <figcaption className="demo__head">
              <span className="tag tag--bug">one digit</span>
              <h3>FNV-1a's offset basis</h3>
            </figcaption>
            <div className="demo__body">
              <p className="demo__lede">
                The offset basis is a published constant. It was mistyped by one hex digit —
                and the test that should have caught it{" "}
                <strong>used the same wrong constant</strong>, so it agreed with itself. Only
                the vectors checked against an external source disagreed.
              </p>
              <HexDiff />
              <div className="demo__foot">
                <Link to="/docs/findings#step-2">Read the FNV finding</Link>. A constant copied into both the code and its own test is not checked twice.
                That is the whole lesson, and it cost one digit to learn.
              </div>
            </div>
          </figure>

          <figure className="demo" id="bits-demo">
            <figcaption className="demo__head">
              <span className="tag">bit order</span>
              <h3>DEFLATE peels from the bottom</h3>
            </figcaption>
            <div className="demo__body">
              <p className="demo__lede">
                RFC 1951 packs data into bytes <em>in order of increasing bit number</em>. A
                design that peels bits off the top produces a perfectly plausible bit stream
                that no DEFLATE decoder has ever accepted — so this is the single thing
                worth watching.
              </p>
              <BitStream />
            </div>
          </figure>

          <figure className="demo" id="huffman-demo">
            <figcaption className="demo__head">
              <span className="tag">a table lookup</span>
              <h3>Nine bits in, one symbol out</h3>
            </figcaption>
            <div className="demo__body">
              <p className="demo__lede">
                A Huffman code is written <em>most</em> significant bit first into a stream
                that arrives least significant bit first. The decoder peeks nine bits,
                reverses them into a table index, and gets back a symbol and the number of
                bits that symbol cost.
              </p>
              <Huffman />
            </div>
          </figure>
        </section>

        <section className="section section--alt" id="gap">
          <h2>The capability boundary</h2>
          <p className="prose">
            There is no initialised-memory node in the IR. <code>Design::mem</code> is
            zero-filled in both backends, so every lookup table in the corpus is a{" "}
            <code>case</code> or a multiplexer tree. That is close to the truth for a nibble
            LUT and a lie for a 256-entry alphabet map.
          </p>
          <figure className="demo">
            <Mux />
            <figcaption className="demo__foot">
              <code>Design::rom(address, &amp;table, width)</code> builds exactly what a
              human would write with <code>case</code>: one arm per entry, emitting{" "}
              <code>always @* case</code>. A 256-entry table is 256 case arms. A synthesis tool
              would infer block RAM from the same source in seconds — which is the point
              being recorded rather than hidden.
            </figcaption>
          </figure>
        </section>

        <section className="section" id="limits">
          <h2>What is not here</h2>
          <ul className="limits">
            <li>
              <strong>No initialised memory.</strong> The highest-value addition left to the
              toolchain. <code>Design::rom</code> documents the cost instead of hiding it.
            </li>
            <li>
              <strong>No dynamic Huffman tree.</strong> <code>Table</code> is built from code
              lengths at elaboration time. Parsing a dynamic block header — nineteen code
              lengths, a seven-bit code-length code, the 16/17/18 run codes — is a different
              module.
            </li>
            <li>
              <strong>No LZ77 window.</strong> The DEFLATE entry is the bit layer and nothing
              above it: no block structure, no error detection, no back-references.
            </li>
            <li>
              <strong>Huffman is not one symbol per cycle.</strong> A decode spends{" "}
              <code>code_len</code> bits and the host supplies one bit per cycle, so the rate
              is one symbol per code length — seven, for DEFLATE's shortest fixed code. FSE{" "}
              <em>can</em>, because an FSE transition is allowed to cost zero bits. That
              difference is the whole distinction between the two decoders.
            </li>
            <li>
              <strong>BLAKE3 and the FFT are placeholders.</strong> Files with no
              implementation behind them, not even declared as modules.
            </li>
          </ul>
        </section>
      </div>

      <footer className="footer">
        <p>
          Ferrite Lithic — Apache-2.0. An embedded hardware DSL in Rust, with a verified
          corpus that keeps finding bugs the tools' own tests could not.
        </p>
        <p className="footer__meta">
          {PROJECT_STATS.tests} tests, {PROJECT_STATS.designs} designs, {PROJECT_STATS.tiers} tiers, {PROJECT_STATS.crates} crates. MIT-licensed dependencies audited with{" "}
          <code>cargo deny</code>.
        </p>
      </footer>
    </>
  );
}
