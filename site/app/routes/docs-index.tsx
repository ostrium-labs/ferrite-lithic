import { Link } from "react-router";
import { DOCS, type DocEntry } from "../content/pages";
import { DOC_GROUPS, DOC_NAV } from "../content/navigation";

export function meta() {
  return [{ title: "Documentation — Ferrite Lithic" }, { name: "description", content: `${DOCS.length} pages covering concepts, toolchain, corpus, findings and measured benchmarks.` }];
}

function Entry({ entry }: { entry: DocEntry }) {
  const prerequisite = DOC_NAV.find(item => item.slug === entry.prerequisite);
  return <Link className="index-card" to={`/docs/${entry.slug}`}>
    <span className="index-card__number">{String(entry.order + 1).padStart(2, "0")}</span>
    <div><h3 className="index-card__title">{entry.title}</h3><p>{entry.description}</p>
      <div className="index-card__foot">{prerequisite ? <span>After {prerequisite.label}</span> : <span>Start here</span>}<span className="index-card__action">Read →</span></div>
    </div>
  </Link>;
}

export default function DocsIndex() {
  return <div className="doc-layout doc-layout--index">
    <nav className="sidebar" aria-label="Learning map sections">
      <p className="sidebar__heading">Learning map</p>
      <ol className="sidebar__list">{DOC_GROUPS.map(group => <li className="sidebar__item" key={group.id}>
        <a className="sidebar__link" href={`#${group.id}`}>{group.title}<span>{DOCS.filter(entry => group.sections.includes(entry.section)).length}</span></a>
      </li>)}</ol>
      <p className="sidebar__note">{DOCS.length} pages. Read in order, or enter through the question you have.</p>
    </nav>
    <article className="doc doc--index">
      <header className="doc-header">
        <nav className="breadcrumb" aria-label="Breadcrumb"><Link className="breadcrumb__link" to="/">Ferrite Lithic</Link><span aria-hidden="true">/</span><span>Documentation</span></nav>
        <h1 className="doc-header__title">A map of the toolchain.</h1>
        <div className="doc__lede"><p>Learn the circuit model. Follow the graph through two backends. Then inspect the evidence that keeps them honest.</p></div>
      </header>
      <div className="learning-entries" aria-label="Choose a starting point">
        <Link to="/docs/basics"><span>New to hardware?</span><strong>Start with one clock edge</strong><span>Values, state and the meaning of a cycle →</span></Link>
        <Link to="/docs/bits"><span>Ready to build?</span><strong>Follow a value into the graph</strong><span>Bits → IR → Design → backends</span></Link>
        <Link to="/docs/findings"><span>Evaluating the project?</span><strong>Inspect the defects it caught</strong><span>Independent references and measured limits →</span></Link>
      </div>
      <nav className="learning-path" aria-label="Core toolchain reading path">
        <span className="data-label">The core reading path</span>
        <ol>{["bits", "ir", "design", "sim", "rtl", "tb", "cosim", "corpus"].map(slug => <li key={slug}><Link to={`/docs/${slug}`}>{DOC_NAV.find(entry => entry.slug === slug)?.label}</Link></li>)}</ol>
        <p>Simulator and RTL are parallel consumers of the graph. This path is a reading sequence, not a build-dependency graph.</p>
      </nav>
      {DOC_GROUPS.map(group => <section className="doc__section learning-chapter" id={group.id} key={group.id}>
        <div className="learning-chapter__intro"><h2 className="doc__h2">{group.title}</h2><p>{group.description}</p><span className="data-label">{DOCS.filter(entry => group.sections.includes(entry.section)).length} pages</span></div>
        <div className="index-grid">{DOCS.filter(entry => group.sections.includes(entry.section)).map(entry => <Entry key={entry.slug} entry={entry}/>)}</div>
      </section>)}
    </article>
  </div>;
}
