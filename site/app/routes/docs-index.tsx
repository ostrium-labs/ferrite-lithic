import { Link } from "react-router";
import { DOCS } from "../content/pages";

export function meta() {
  return [
    { title: "Documentation — Ferrite Lithic" },
    {
      name: "description",
      content:
        "Fifteen pages: the concepts a hardware DSL needs, the OCaml and Hardcaml lineage, why Rust, and then the nine tool crates bottom-up.",
    },
  ];
}

export default function DocsIndex() {
  const background = DOCS.filter((entry) => entry.kind === "mdx");
  const crates = DOCS.filter((entry) => entry.kind === "data");

  const Card = ({ entry }: { entry: (typeof DOCS)[number] }) => (
    <Link className="index-card" to={`/docs/${entry.slug}`}>
      <h3 className="index-card__title">{entry.title}</h3>
      {entry.crate ? <code className="index-card__crate">{entry.crate}</code> : null}
      <p>{entry.description}</p>
      <div className="index-card__foot">
        <span>{entry.slug}</span>
        {entry.tests ? <span>{entry.tests} tests</span> : null}
        {entry.page ? <span>{entry.page.steps.length} steps</span> : null}
        {entry.page?.gotchas?.length ? <span>{entry.page.gotchas.length} traps</span> : null}
      </div>
    </Link>
  );

  return (
    <div className="doc-layout">
      <nav className="sidebar" aria-label="Sections">
        <p className="sidebar__heading">Start here</p>
        <ol className="sidebar__list">
          <li className="sidebar__item">
            <span className="sidebar__link">Background</span>
            <span className="sidebar__crate">{background.length} pages</span>
          </li>
          <li className="sidebar__item">
            <span className="sidebar__link">The toolchain</span>
            <span className="sidebar__crate">{crates.length} pages</span>
          </li>
        </ol>
      </nav>

      <article className="doc doc--index">
        <header className="doc-header">
          <nav className="breadcrumb" aria-label="Breadcrumb">
            <Link className="breadcrumb__link" to="/">
              Ferrite Lithic
            </Link>
            <span className="breadcrumb__sep">/</span>
            <span className="breadcrumb__here">Documentation</span>
          </nav>
          <h1 className="doc-header__title">Documentation</h1>
          <div className="doc__lede">
            <p>
              Fifteen pages in reading order. Start with the four background pages: they
              explain what a clock edge is, where the design came from, and what Rust does
              and does not check. Then the nine tool crates, bottom-up — bits, then the
              graph, then the two backends that read it.
            </p>
            <p>
              Every page is a numbered walkthrough with the code samples transcribed from
              the crates themselves. Where something is a measured number, it is labelled
              as one, and where something is a known gap, it says so.
            </p>
          </div>
        </header>

        <section className="doc__section">
          <h2 className="doc__h2">Background</h2>
          {background.map((entry) => (
            <Card key={entry.slug} entry={entry} />
          ))}
        </section>

        <section className="doc__section">
          <h2 className="doc__h2">The toolchain, bottom-up</h2>
          {crates.map((entry) => (
            <Card key={entry.slug} entry={entry} />
          ))}
        </section>
      </article>
    </div>
  );
}
