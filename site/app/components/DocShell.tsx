/**
 * The documentation shell.
 *
 * One component renders every page, whether the page is MDX or typed data. That is the
 * whole point: the sidebar, the step numbering, the figure treatment and the prev/next
 * pair are decided once here, so all documentation pages cannot drift apart in structure even
 * though they are authored in two different formats.
 */

import { Link } from "react-router";
import { useEffect, useRef, type ReactNode } from "react";

import { DOC_GROUPS, DOC_NAV } from "../content/navigation";
import { DocRail } from "./DocRail";
import { CodeBlock } from "./CodeBlock";

import { mdxComponents } from "./mdx";
import { BY_SLUG, DOCS, type DocEntry } from "../content/pages";
import type { Page } from "../content/types";

/* -------------------------------------------------------------- doc chrome */

function Sidebar({ current }: { current: string }) {
  const disclosureRef = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const media = globalThis.matchMedia("(min-width: 60.01rem)");
    const sync = () => { if (disclosureRef.current) disclosureRef.current.open = media.matches; };
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  }, [current]);
  return (
    <nav className="sidebar" aria-label="Documentation">
      <details ref={disclosureRef} className="sidebar__disclosure" key={current} open>
      <summary className="sidebar__summary">Documentation / {BY_SLUG.get(current)?.title}</summary>
      <Link className="sidebar__index" to="/docs">Documentation / learning map</Link>
      {DOC_GROUPS.map(group => <section className="sidebar__group" key={group.id}>
        <h2 className="sidebar__heading">{group.title}</h2>
        <ol className="sidebar__list">
          {DOCS.filter(entry => group.sections.includes(entry.section)).map(entry => <li key={entry.slug} className="sidebar__item">
            <Link to={`/docs/${entry.slug}`} className={`sidebar__link${entry.slug === current ? " is-current" : ""}`} aria-current={entry.slug === current ? "page" : undefined}>{entry.label}</Link>
          </li>)}
        </ol>
      </section>)}
      </details>
    </nav>
  );
}

function Breadcrumb({ entry }: { entry: DocEntry }) {
  return (
    <nav className="breadcrumb" aria-label="Breadcrumb">
      <Link className="breadcrumb__link" to="/">
        Ferrite Lithic
      </Link>
      <span className="breadcrumb__sep">/</span>
      <Link className="breadcrumb__link" to="/docs">
        Documentation
      </Link>
      <span className="breadcrumb__sep">/</span>
      <span className="breadcrumb__here">{entry.title}</span>
    </nav>
  );
}

function Header({ entry }: { entry: DocEntry }) {
  return (
    <header className="doc-header">
      <Breadcrumb entry={entry} />
      <p className="doc-header__location">{DOC_GROUPS.find(group => group.sections.includes(entry.section))?.title} / {entry.order + 1} of {DOCS.length}</p>
      {entry.crate || entry.tests ? (
        <p className="doc-header__meta">
          {entry.crate ? <span className="doc-header__crate">{entry.crate}</span> : null}
          {entry.tests ? <span>{entry.tests} tests / recorded snapshot</span> : null}
        </p>
      ) : null}
      <h1 className="doc-header__title">{entry.title}</h1>
      <div className="doc__lede">
        <p>{entry.description}</p>
      </div>
    </header>
  );
}

function Pagination({ slug }: { slug: string }) {
  const index = DOCS.findIndex((entry) => entry.slug === slug);
  const previous = index > 0 ? DOCS[index - 1] : undefined;
  const next = index >= 0 && index < DOCS.length - 1 ? DOCS[index + 1] : undefined;

  return (
    <nav className="doc-footer" aria-label="Pagination">
      {previous ? (
        <Link className="doc-footer__link doc-footer__link--prev" to={`/docs/${previous.slug}`}>
          <span className="doc-footer__direction">← Previous</span>
          {previous.title}
        </Link>
      ) : (
        <Link className="doc-footer__link doc-footer__link--prev" to="/">
          <span className="doc-footer__direction">← Previous</span>
          Landing page
        </Link>
      )}
      {next ? (
        <Link className="doc-footer__link doc-footer__link--next" to={`/docs/${next.slug}`}>
          <span className="doc-footer__direction">Next →</span>
          {next.title}
        </Link>
      ) : null}
    </nav>
  );
}

/* ------------------------------------------------- rendering the typed pages */

function Figure({
  value,
  unit,
  caption,
}: {
  value: string;
  unit?: string;
  caption: string;
}) {
  return (
    <div className="figure">
      <span className="figure__value">{value}</span>
      {unit ? <span className="figure__unit">{unit}</span> : null}
      <span className="figure__caption">{caption}</span>
    </div>
  );
}

/** The tiny inline markup used in the typed content: `code` and **strong**. */
function Rich({ text }: { text: string }) {
  const nodes: ReactNode[] = [];
  const pattern = /`([^`]+)`|\*\*([^*]+)\*\*/g;
  let at = 0;
  let match = pattern.exec(text);

  while (match !== null) {
    if (match.index > at) nodes.push(text.slice(at, match.index));
    if (match[1] !== undefined) nodes.push(<code key={nodes.length}>{match[1]}</code>);
    else if (match[2] !== undefined) nodes.push(<strong key={nodes.length}>{match[2]}</strong>);
    at = match.index + match[0].length;
    match = pattern.exec(text);
  }
  if (at < text.length) nodes.push(text.slice(at));
  return <>{nodes}</>;
}

function renderPage(page: Page) {
  return (
    <>
      {page.lede.length > 0 ? (
        <div className="doc__lede">
          {page.lede.map((paragraph, index) => (
            <p key={index}>
              <Rich text={paragraph} />
            </p>
          ))}
        </div>
      ) : null}

      <section className="doc__section">
        <h2 className="doc__h2">Step by step</h2>
        <ol className="steps">
          {page.steps.map((step, index) => (
            <li className="step" id={`step-${index + 1}`} key={step.title}>
              <div className="step__head">
                <span className="step__number">{String(index + 1).padStart(2, "0")}</span>
                <h3 className="step__title">{step.title}</h3>
              </div>
              <div className="step__detail">
                <p>
                  <Rich text={step.detail} />
                </p>
                {step.figure ? <Figure {...step.figure} /> : null}
                {step.code ? (
                  <CodeBlock><pre tabIndex={0}><code>{step.code}</code></pre></CodeBlock>
                ) : null}
              </div>
            </li>
          ))}
        </ol>
      </section>

      {page.gotchas && page.gotchas.length > 0 ? (
        <section className="doc__section">
          <h2 className="doc__h2">What bites</h2>
          <ul className="gotchas">
            {page.gotchas.map((gotcha) => (
              <li className="gotcha" key={gotcha.title}>
                <h3 className="gotcha__title">{gotcha.title}</h3>
                <div className="gotcha__body">
                  <p>
                    <Rich text={gotcha.body} />
                  </p>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {page.notes && page.notes.length > 0 ? (
        <section className="doc__section">
          <h2 className="doc__h2">Notes</h2>
          <ul className="notes">
            {page.notes.map((note) => (
              <li className={`note note--${note.kind}`} key={note.title}>
                <div className="note__head">
                  <span className={`note__kind note__kind--${note.kind}`}>{note.kind}</span>
                  <h3 className="note__title">{note.title}</h3>
                </div>
                <div className="note__body">
                  <p>
                    <Rich text={note.body} />
                  </p>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </>
  );
}

/* ---------------------------------------------------------------- the page */

export function DocShell({ slug }: { slug: string }) {
  // Declared before any early return: a hook after a conditional return changes the hook
  // count between renders, which React treats as a different component.
  const articleRef = useRef<HTMLElement | null>(null);
  const entry = BY_SLUG.get(slug);

  if (!entry) {
    return (
      <main className="doc doc--error">
        <div className="doc__lede">
          <p className="eyebrow">404</p>
          <h1 className="doc-header__title">No page at /docs/{slug}</h1>
          <p>
            The reading order lists {DOCS.length} pages. <Link to="/docs">See the index</Link>.
          </p>
        </div>
      </main>
    );
  }


  return (
    <div className="doc-layout">
      <Sidebar current={slug} />
      <article className="doc" ref={articleRef} key={slug}>
        <Header entry={entry} />
        {entry.prerequisite ? <p className="doc__prerequisite">Builds on <Link to={`/docs/${entry.prerequisite}`}>{DOC_NAV.find(item => item.slug === entry.prerequisite)?.label}</Link></p> : null}
        {entry.kind === "mdx" && entry.Component ? (
          <entry.Component components={mdxComponents} />
        ) : entry.page ? (
          renderPage(entry.page)
        ) : null}
        <Pagination slug={slug} />
      </article>
      <DocRail key={`rail-${slug}`} articleRef={articleRef} entry={entry} />
    </div>
  );
}
