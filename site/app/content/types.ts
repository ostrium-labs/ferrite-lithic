/**
 * The shape of a documentation page.
 *
 * Content lives in typed data rather than in eleven hand-written HTML files, for one
 * reason: eleven hand-written files drift. The moment one of them has a different
 * heading level or a missing "gotchas" block, the set stops reading as one document. A
 * renderer over a shared shape makes that structurally impossible.
 *
 * Every `note` carries a `kind`, because the distinction this project cares about most
 * is between a thing that is true and a thing that is a trap. "Measured", "asserted" and
 * "wrong" are different claims and they should not look the same on the page.
 */

export type NoteKind = "measured" | "decision" | "trap" | "gap";

export interface Note {
  kind: NoteKind;
  title: string;
  body: string;
}

export interface Step {
  /** What this step accomplishes, in one line. */
  title: string;
  /** Why it is done this way rather than the obvious way. */
  detail: string;
  code?: string;
  /** A measured number, rendered as a figure with its unit. */
  figure?: { value: string; unit: string; caption: string };
}

export interface Gotcha {
  title: string;
  body: string;
}

export interface Page {
  /** URL slug, matching the file name under `docs/`. */
  slug: string;
  /** Display title. */
  title: string;
  /** The crate name, if this page documents one. */
  crate?: string;
  /** Phase that produced it. */
  tier?: string;
  /** One sentence for the table of contents. */
  summary: string;
  /** The opening paragraphs of the page. */
  lede: string[];
  /** The numbered walkthrough. */
  steps: Step[];
  /** Things that bite. */
  gotchas?: Gotcha[];
  /** Everything else worth knowing. */
  notes?: Note[];
  /** Test count, and how it is split. */
  tests?: { total: number; breakdown: string };
  /** The previous and next page in reading order. */
  prev?: string;
  next?: string;
}

/** The reading order. Every page's prev/next comes from this list, so it cannot disagree. */
export const READING_ORDER = [
  "index",
  "bits",
  "ir",
  "design",
  "sim",
  "rtl",
  "derive",
  "wave",
  "tb",
  "cosim",
  "corpus",
  "findings",
] as const;

export type Slug = (typeof READING_ORDER)[number];

export const NOTE_LABELS: Record<NoteKind, string> = {
  measured: "Measured",
  decision: "Decision",
  trap: "Trap",
  gap: "Gap",
};