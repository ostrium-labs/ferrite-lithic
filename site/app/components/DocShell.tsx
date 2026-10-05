/**
 * The documentation shell.
 *
 * One component renders every page, whether the page is MDX or typed data. That is the
 * whole point: the sidebar, the step numbering, the figure treatment and the prev/next
 * pair are decided once here, so the fifteen pages cannot drift apart in structure even
 * though they are authored in two different formats.
 */

import { Link } from "react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";

import { mdxComponents } from "./mdx";
import { BY_SLUG, DOCS, type DocEntry } from "../content/pages";
import type { Page } from "../content/types";

/* -------------------------------------------------------------- doc chrome */

function Sidebar({ current }: { current: string }) {
  return (
    <nav className="sidebar" aria-label="Documentation">
      <p className="sidebar__heading">Reading order</p>
      <ol className="sidebar__list">
        {DOCS.map((entry) => (
          <li key={entry.slug} className="sidebar__item">
            <Link
              to={`/docs/${entry.slug}`}
              className={`sidebar__link${entry.slug === current ? " is-current" : ""}`}
              aria-current={entry.slug === current ? "page" : undefined}
            >
              {entry.title}
            </Link>
            {entry.crate ? <span className="sidebar__crate">{entry.crate}</span> : null}
          </li>
        ))}
      </ol>
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
      {entry.crate || entry.tier || entry.tests ? (
        <p className="doc-header__meta">
          {entry.crate ? <span className="doc-header__crate">{entry.crate}</span> : null}
          {entry.tier ? <span>phase {entry.tier}</span> : null}
          {entry.tests ? <span>{entry.tests} tests</span> : null}
        </p>
      ) : null}
      <h1 className="doc-header__title">{entry.title}</h1>
      <div className="doc__lede">
        <p>{entry.description}</p>
      </div>
    </header>
  );
}

/**
 * In-page contents. Steps really are a sequence, so numbering them is information.
 *
 * For a typed page the titles are already in the registry. For an MDX page they are not —
 * they live inside a Markdown file, and parsing that file at runtime to read them out would
 * mean shipping the source twice. So they are read back off the rendered DOM after mount
 * instead, which is one query and cannot disagree with what the reader can see.
 *
 * Rendered in the wrong order? No: the contents sits above the article in the markup, so it
 * is empty on the first paint and fills in immediately after. It is a navigation aid, and
 * arriving a frame late costs nothing.
 */
function OnThisPage({ titles, articleRef }: { titles: string[]; articleRef: React.RefObject<HTMLElement | null> }) {
  const [domTitles, setDomTitles] = useState<string[]>([]);

  useEffect(() => {
    const root = articleRef.current;
    if (!root) return;
    const found = Array.from(root.querySelectorAll<HTMLElement>(".step__title"))
      .map((node) => node.textContent?.trim() ?? "")
      .filter(Boolean);
    setDomTitles(found);
  }, [articleRef, titles]);

  const resolved = titles.length > 0 ? titles : domTitles;
  if (resolved.length === 0) return null;

  return (
    <nav className="onpage" aria-label="On this page">
      <p className="onpage__heading">On this page</p>
      <ol className="onpage__list">
        {resolved.map((title, index) => (
          <li key={`${index}-${title}`}>
            <a className="onpage__link" href={`#step-${index + 1}`}>
              {title}
            </a>
          </li>
        ))}
      </ol>
    </nav>
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
          {previous.title}
        </Link>
      ) : (
        <Link className="doc-footer__link doc-footer__link--prev" to="/">
          Landing page
        </Link>
      )}
      {next ? (
        <Link className="doc-footer__link doc-footer__link--next" to={`/docs/${next.slug}`}>
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
                  <div className="code-block">
                    <pre>{step.code}</pre>
                  </div>
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
      <main className="doc">
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

  const stepTitles = entry.page ? entry.page.steps.map((step) => step.title) : [];

  return (
    <div className="doc-layout">
      <Sidebar current={slug} />
      <article className="doc" ref={articleRef}>
        <Header entry={entry} />
        <OnThisPage titles={stepTitles} articleRef={articleRef} />
        {entry.kind === "mdx" && entry.Component ? (
          <entry.Component components={mdxComponents} />
        ) : entry.page ? (
          renderPage(entry.page)
        ) : null}
        <Pagination slug={slug} />
      </article>
    </div>
  );
}
