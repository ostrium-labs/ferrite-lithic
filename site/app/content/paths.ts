import { DOC_NAV } from "./navigation";

// Shared with the page registry so prerendering includes every documentation page.
// Keep this module free of MDX imports: the build config runs outside Vite.
export const MDX_SLUGS = {
  basics: "basics",
  ocaml: "ocaml",
  hardcaml: "hardcaml",
  whyRust: "why-rust",
  benchmarks: "benchmarks",
} as const;

export const DOC_PATHS = DOC_NAV.map(({ slug }) => `/docs/${slug}`);
