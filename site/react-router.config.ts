import type { Config } from "@react-router/dev/config";

/**
 * React Router 8, framework mode.
 *
 * Chosen over the plain Vite MPA this site used before because the documentation
 * content is now MDX with frontmatter, which needs a real router to resolve a slug to a
 * document. The URLs are unchanged in shape — `/docs/sim` rather than `/docs/sim.html` —
 * so a link written for the previous build still resolves.
 *
 * `ssr: true` with everything prerendered: the documentation is fully static, which is
 * what makes it indexable and linkable. The animated parts are effects, so they only ever
 * run in the browser and the prerendered HTML is complete without them.
 */
export default {
  ssr: true,
  appDirectory: "app",
} satisfies Config;
