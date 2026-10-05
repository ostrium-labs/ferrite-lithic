import { type RouteConfig, index, route } from "@react-router/dev/routes";

/**
 * Four routes and no nesting.
 *
 * `/docs/:slug` resolves a slug to an MDX file rather than to a component, so adding a
 * page means adding a file in `content/docs/` and nothing else — no route entry, no import,
 * no registry to keep in sync. That is the whole reason for moving to MDX.
 *
 * A single segment rather than a splat: every page in the registry is one segment, and a
 * named param means the loader reads `params.slug` instead of guessing which key the splat
 * arrived under.
 */
export default [
  index("routes/home.tsx"),
  route("docs", "routes/docs-index.tsx"),
  route("docs/:slug", "routes/doc.tsx"),
] satisfies RouteConfig;
