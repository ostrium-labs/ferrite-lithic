import { FerriteLogo } from "./brand/FerriteLogo";
/**
 * The persistent site header.
 *
 * Navigation was the weak point: the docs had a sidebar, the landing page had nothing, and
 * neither had a persistent bar. So the answer to "how do I get back?" depended on which page
 * you happened to be on. This bar is in the root layout, so it is on every route, and it
 * marks the section you are in.
 *
 * Deliberately small. Six links, always the same six, in the same order — a reader who
 * learns the bar once never has to think about it again.
 */

import { useEffect, useRef } from "react";
import { DOC_NAV } from "../content/navigation";
import { PROJECT_STATS } from "../lib/data";

import { Link } from "react-router";

/**
 * The sections, in reading order.
 *
 * Every route must belong to exactly one section, or the bar shows nothing current and the
 * reader is left guessing. Rather than hand-maintaining a list of slugs per section — which
 * is how the background pages ended up unmarked — each section matches a *pattern* over the
 * registry order, and `SECTION_OF` is the single place that mapping lives.
 */

function sectionFor(pathname: string) {
  if (!pathname.startsWith("/docs/")) return null;
  return DOC_NAV.find(entry => entry.slug === pathname.replace(/^\/docs\//, "").replace(/\/$/, ""))?.section ?? null;
}

const SECTIONS = [
  { to: "/", label: "Overview", current: (path: string) => path === "/" },
  { to: "/docs/basics", label: "Concepts", current: (path: string) => sectionFor(path) === "background" },
  { to: "/docs/bits", label: "Crates", current: (path: string) => sectionFor(path) === "crates" },
  { to: "/docs/corpus", label: "Corpus", current: (path: string) => sectionFor(path) === "corpus" },
  { to: "/docs/benchmarks", label: "Benchmarks", current: (path: string) => sectionFor(path) === "benchmarks" },
] as const;

/** `/docs` itself has no section of its own; it is the index of all of them. */
function isDocsIndex(pathname: string): boolean {
  return pathname === "/docs" || pathname === "/docs/";
}

export function SiteHeader({ pathname }: { pathname: string }) {
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    const header = ref.current;
    if (!header) return;
    const observer = new ResizeObserver(() => {
      document.documentElement.style.setProperty("--sitebar-height", `${header.getBoundingClientRect().height}px`);
    });
    observer.observe(header);
    return () => { observer.disconnect(); document.documentElement.style.removeProperty("--sitebar-height"); };
  }, []);
  return (
    <header className="sitebar" ref={ref}>
      <div className="sitebar__inner">
        <Link className="sitebar__wordmark" to="/" aria-label="Ferrite Lithic">
          <FerriteLogo />
        </Link>

        <nav className="sitebar__nav" aria-label="Sections">
          {SECTIONS.map((section) => {
            // The docs index highlights nothing: it is the parent of every section, and
            // highlighting one of its children while standing on it would be a lie.
            const active = !isDocsIndex(pathname) && section.current(pathname);
            return (
              <Link
                key={section.to}
                to={section.to}
                className={`sitebar__link${active ? " is-current" : ""}`}
                aria-current={active ? "page" : undefined}
              >
                {section.label}
              </Link>
            );
          })}
        </nav>

        {/* The one number that is not a link, because it is a fact about the repo rather
            than a place to go. */}
        <span className="sitebar__meta">{PROJECT_STATS.tests} tests · {PROJECT_STATS.designs} designs</span>
      </div>
    </header>
  );
}
