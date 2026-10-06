import { useEffect, useRef, useState } from "react";
import { SOURCE_ROOT } from "../content/navigation";
import type { DocEntry } from "../content/pages";

interface Heading { id: string; title: string; number?: string; measurement?: string }

/** Context follows the authored headings; no scroll listener or invented percentage. */
export function DocRail({ entry, articleRef }: { entry: DocEntry; articleRef: React.RefObject<HTMLElement | null> }) {
  const [headings, setHeadings] = useState<Heading[]>([]);
  const [active, setActive] = useState("");
  const disclosureRef = useRef<HTMLDetailsElement>(null);

  useEffect(() => {
    const article = articleRef.current;
    if (!article) return;
    const nodes = Array.from(article.querySelectorAll<HTMLElement>(".step__title, h2"))
      .filter(node => node.textContent?.trim() !== "Step by step");
    const targets = nodes.map((node, index) => {
      const step = node.closest<HTMLElement>(".step");
      const target = step ?? node;
      if (!target.id) target.id = `section-${index + 1}`;
      return { node, id: target.id, title: node.textContent?.trim() ?? "", number: step?.querySelector(".step__number")?.textContent ?? undefined, measurement: step?.querySelector(".figure")?.textContent ?? undefined };
    });
    setHeadings(targets);
    const update = () => {
      const offset = (document.querySelector(".sitebar")?.getBoundingClientRect().height ?? 56) + 24;
      const passed = targets.filter(({ node }) => node.getBoundingClientRect().top <= offset + 24);
      setActive((passed.at(-1) ?? targets[0])?.id ?? "");
    };
    const observer = new IntersectionObserver(update, { rootMargin: "-80px 0px -60% 0px", threshold: [0, 1] });
    targets.forEach(({ node }) => observer.observe(node));
    update();
    window.addEventListener("resize", update);
    return () => { observer.disconnect(); window.removeEventListener("resize", update); };
  }, [articleRef, entry.slug]);

  useEffect(() => {
    const media = matchMedia("(min-width: 80rem)");
    const sync = () => { if (disclosureRef.current) disclosureRef.current.open = media.matches; };
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  }, [headings]);

  const current = headings.find(heading => heading.id === active);
  const position = Math.max(0, headings.findIndex(heading => heading.id === active));
  return <aside className="doc-rail" aria-label="Document context">
    <nav className="onpage" aria-label="On this page">
      <details open ref={disclosureRef}>
        <summary className="onpage__heading">On this page <span>{headings.length ? `${position + 1} / ${headings.length}` : ""}</span></summary>
        <ol className="onpage__list">{headings.map(({ id, title, number }) => <li key={id}>
          <a className={`onpage__link${id === active ? " is-current" : ""}`} href={`#${id}`} aria-current={id === active ? "location" : undefined}><span className="onpage__number" aria-hidden="true">{number ?? "—"}</span>{title}</a>
        </li>)}</ol>
      </details>
    </nav>
    <div className="doc-rail__context">
      {entry.crate ? <p className="doc-rail__crate"><code>{entry.crate}</code>{entry.tests ? <span>{entry.tests} tests / recorded snapshot</span> : null}</p> : null}
      {current?.measurement ? <div className="doc-rail__measurement"><p className="data-label">In this section</p><p>{current.measurement}</p></div> : null}
      <a className="doc-rail__source" href={`${SOURCE_ROOT}${entry.source}`}>Read the source ↗</a>
      <p className="doc-rail__note">Code and measurements are documented snapshots. Source is the current <code>dev</code> branch.</p>
    </div>
  </aside>;
}
