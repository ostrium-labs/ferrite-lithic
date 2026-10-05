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

import { Link, NavLink } from "react-router";

/**
 * The sections, in reading order.
 *
 * Every route must belong to exactly one section, or the bar shows nothing current and the
 * reader is left guessing. Rather than hand-maintaining a list of slugs per section — which
 * is how the background pages ended up unmarked — each section matches a *pattern* over the
 * registry order, and `SECTION_OF` is the single place that mapping lives.
 */

const CRATE_SLUGS = ["bits", "ir", "design", "sim", "rtl", "derive", "wave", "cosim", "tb"];
const BACKGROUND_SLUGS = ["basics", "ocaml", "hardcaml", "why-rust"];

/** Which section a documentation page belongs to. Returns null for `/` and `/docs`. */
function sectionFor(pathname: string): "background" | "crates" | "corpus" | "benchmarks" | null {
  const slug = pathname.replace(/^\/docs\/?/, "").replace(/\/$/, "");
  if (slug === "") return null;
  if (slug === "benchmarks") return "benchmarks";
  if (slug === "corpus" || slug === "findings") return "corpus";
  if (CRATE_SLUGS.includes(slug)) return "crates";
  if (BACKGROUND_SLUGS.includes(slug)) return "background";
  // An unregistered slug: fall back to the reading order's first section rather than
  // leaving the bar blank, so the reader always has somewhere to go back to.
  return "background";
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
  return (
    <header className="sitebar">
      <div className="sitebar__inner">
        <Link className="sitebar__wordmark" to="/">
          <span className="sitebar__mark" aria-hidden="true" />
          ferrite-lithic
        </Link>

        <nav className="sitebar__nav" aria-label="Sections">
          {SECTIONS.map((section) => {
            // The docs index highlights nothing: it is the parent of every section, and
            // highlighting one of its children while standing on it would be a lie.
            const active = !isDocsIndex(pathname) && section.current(pathname);
            return (
              <NavLink
                key={section.to}
                to={section.to}
                className={`sitebar__link${active ? " is-current" : ""}`}
                aria-current={active ? "page" : undefined}
              >
                {section.label}
              </NavLink>
            );
          })}
        </nav>

        {/* The one number that is not a link, because it is a fact about the repo rather
            than a place to go. */}
        <span className="sitebar__meta">811 tests · 21 designs</span>
      </div>
    </header>
  );
}
