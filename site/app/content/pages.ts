/**
 * The reading order, and the registry behind it.
 *
 * One list decides what exists, what order it is read in, and what the sidebar shows.
 * Adding a page means adding an entry here and a file — there is no second list to keep in
 * sync, which is the specific thing that goes wrong in a docs site with thirteen pages.
 *
 * Two kinds of page live side by side:
 *
 *   mdx  — authored in Markdown with our components in scope. Used for prose, because
 *          Shiki highlights fenced code at build time and a broken fence fails the build.
 *   data — a typed object from `toolchain.ts` / `corpus.ts`. Used for the crate
 *          walkthroughs, where the step/figure/notes structure is worth having checked by
 *          the compiler rather than trusted to prose.
 *
 * Both render through the same shell, so a reader cannot tell which is which.
 */

import { TOOLCHAIN } from "./toolchain";
import { CORPUS, FINDINGS } from "./corpus";
import type { ComponentType } from "react";

import type { Page } from "./types";
import type { mdxComponents } from "../components/mdx";

import Basics from "../../content/docs/basics.mdx";
import Ocam from "../../content/docs/ocaml.mdx";
import HardcamlPage from "../../content/docs/hardcaml.mdx";
import WhyRust from "../../content/docs/why-rust.mdx";
import Benchmarks from "../../content/docs/benchmarks.mdx";

export interface DocEntry {
  /** The URL segment, and the key into the registry. */
  slug: string;
  title: string;
  description: string;
  crate?: string;
  tier?: string;
  tests?: number;
  order: number;
  /** Either an MDX component or a typed page. */
  kind: "mdx" | "data";
  Component?: ComponentType<{ components?: typeof mdxComponents }>;
  page?: Page;
}

/**
 * Background first, then the crates bottom-up, then the corpus.
 *
 * The order is pedagogical and it is the reverse of the build order on purpose: a reader
 * who does not yet know what a clock edge is does not benefit from being told how the
 * emitter prints one.
 */
const MDX_PAGES: DocEntry[] = [
  {
    slug: "basics",
    title: "How a circuit becomes a value",
    description: "Combinational and sequential logic, clock edges, bit order, and what equivalence checking actually proves.",
    order: 10,
    kind: "mdx",
    Component: Basics,
  },
  {
    slug: "ocaml",
    title: "OCaml, and the idea of a hardware DSL",
    description: "The three ways to write hardware, what a hardware construction language is, and which OCaml features carry weight.",
    order: 20,
    kind: "mdx",
    Component: Ocam,
  },
  {
    slug: "hardcaml",
    title: "Hardcaml, the design being ported",
    description: "Signal.t, Reg_spec, wire and feedback, Cyclesim, ppx interfaces — and the three checks inherited verbatim.",
    order: 30,
    kind: "mdx",
    Component: HardcamlPage,
  },
  {
    slug: "why-rust",
    title: "Why Rust, honestly",
    description: "What the type system does not check, where compile-time width checking was traded away, and what actually verifies the design.",
    order: 40,
    kind: "mdx",
    Component: WhyRust,
  },
];

function fromData(page: Page, order: number): DocEntry {
  return {
    slug: page.slug,
    title: page.title,
    description: page.summary,
    crate: page.crate,
    tier: page.tier === "—" ? undefined : page.tier,
    tests: page.tests?.total,
    order,
    kind: "data",
    page,
  };
}

export const DOCS: DocEntry[] = [
  ...MDX_PAGES,
  fromData(TOOLCHAIN[0], 110), // bits
  fromData(TOOLCHAIN[1], 120), // ir
  fromData(TOOLCHAIN[2], 130), // design
  fromData(TOOLCHAIN[3], 140), // sim
  fromData(TOOLCHAIN[4], 150), // rtl
  fromData(TOOLCHAIN[5], 160), // derive
  fromData(TOOLCHAIN[6], 170), // wave
  fromData(TOOLCHAIN[7], 190), // cosim
  fromData(TOOLCHAIN[8], 180), // tb
  fromData(CORPUS, 200),
  fromData(FINDINGS, 210),
  // Reference, read last: it measures the corpus rather than explaining it.
  {
    slug: "benchmarks",
    title: "Benchmarks, measured",
    description: "Node counts, flops, bits of state and combinational depth for every design, derived from the IR rather than estimated.",
    order: 220,
    kind: "mdx" as const,
    Component: Benchmarks,
  },
].sort((a, b) => a.order - b.order);

export const BY_SLUG = new Map(DOCS.map((entry) => [entry.slug, entry]));

export const slugOf = (path: string): string => path.replace(/^docs\/?/, "").replace(/\/$/, "");
