import { BENCHMARK_CLOCK_GHZ, SPEED_RESULTS, speedVerdict } from "../lib/benchmarks";

export function BenchmarkFigure() {
  return <figure className="benchmark-figure">
    <figcaption>Recorded ns / byte at an assumed {BENCHMARK_CLOCK_GHZ} GHz. Shorter is faster.</figcaption>
    {SPEED_RESULTS.filter(row => row.design === "crc3" || row.design === "memchr").map(row => {
      const designNs = row.cyclesPerByte / BENCHMARK_CLOCK_GHZ;
      const maximum = Math.max(designNs, row.softwareNs);
      return <div className="benchmark-pair" key={row.design}>
        <p><code>{row.design}</code><span>{speedVerdict(row)}</span></p>
        <dl>{[["design", designNs], [row.reference, row.softwareNs]].map(([name, ns]) => <div className="benchmark-row" key={name}>
          <dt>{name}</dt><dd><span className="benchmark-track" aria-hidden="true"><span style={{ width: `${Number(ns) / maximum * 100}%` }} /></span><span>{Number(ns).toFixed(2)}</span></dd>
        </div>)}</dl>
      </div>;
    })}
  </figure>;
}

export function BenchmarkTable() {
  return <div className="table-wrap" role="region" tabIndex={0} aria-label="Recorded design and software performance, scroll horizontally">
    <table className="compare compare--numeric">
      <caption>Recorded speed.rs results; design ns assumes {BENCHMARK_CLOCK_GHZ} GHz.</caption>
      <thead><tr><th scope="col">Design</th><th scope="col">Reference</th><th scope="col">Cycles / byte</th><th scope="col">Design ns</th><th scope="col">Software ns</th><th scope="col">Result</th></tr></thead>
      <tbody>{SPEED_RESULTS.map(row => <tr key={row.design}><th scope="row"><code>{row.design}</code></th><td><code>{row.reference}</code></td><td>{row.cyclesPerByte.toFixed(2)}</td><td>{(row.cyclesPerByte / BENCHMARK_CLOCK_GHZ).toFixed(2)}</td><td>{row.softwareNs}</td><td>{speedVerdict(row)}</td></tr>)}</tbody>
    </table>
  </div>;
}
