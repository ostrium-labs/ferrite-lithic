import { useState } from "react";
import { Link } from "react-router";
import { CRATES } from "../lib/data";

export function CrateArchitecture() {
  const [selected, setSelected] = useState("design");
  const current = CRATES.find(crate => crate.doc === selected)!;
  const related = new Set([selected, ...current.dependencies, ...CRATES.filter(crate => crate.dependencies.includes(selected)).map(crate => crate.doc)]);
  return <div className="crate-architecture">
    <div className="crate-architecture__inspect" aria-label="Inspect crate dependencies">
      <p className="data-label">Inspect a crate’s immediate dependencies</p>
      <div className="crate-architecture__controls">{CRATES.map(crate => <button type="button" key={crate.doc} onClick={() => setSelected(crate.doc)} onFocus={() => setSelected(crate.doc)} aria-pressed={selected === crate.doc}>{crate.doc}</button>)}</div>
      <p className="crate-architecture__relation" aria-live="polite"><code>{current.name}</code><br />{current.dependencies.length ? <>Depends on {current.dependencies.join(", ")}</> : "No Ferrite crate dependencies"}. <span>Production dependencies; test dependencies are omitted.</span></p>
    </div>
    <ol className="crates">
      {CRATES.map((crate, index) => <li className={`crate${related.has(crate.doc) ? " is-related" : ""}${selected === crate.doc ? " is-selected" : ""}`} key={crate.doc}>
        <div className="crate__name"><span className="crate__order">{String(index + 1).padStart(2, "0")}</span><Link to={`/docs/${crate.doc}`}>{crate.name}</Link><span className="crate__count">{crate.tests} tests</span></div>
        <p className="crate__what">{crate.what}</p>
        <div className="crate__meta"><span>{crate.dependencies.length ? `Uses ${crate.dependencies.join(", ")}` : "Independent foundation"}</span><Link className="crate__read" to={`/docs/${crate.doc}`} aria-label={`Read ${crate.name}`}>Read →</Link></div>
      </li>)}
    </ol>
  </div>;
}
