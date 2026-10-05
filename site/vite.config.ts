import { fileURLToPath } from "node:url";
import { reactRouter } from "@react-router/dev/vite";
import { defineConfig } from "vite";
import { fumadocsMdx } from "fumadocs-mdx/vite";

/**
 * Vite plugins.
 *
 * Order matters only in that `reactRouter()` owns the build output and the MDX plugin has
 * to see the source files first, so MDX goes before it. Nothing here replaces the design
 * system: Fumadocs is used for the content layer (MDX compilation, Shiki highlighting, and
 * the typed collection API), and the layout is ours.
 */
export default defineConfig({
  plugins: [fumadocsMdx(), reactRouter()],
});

/** Kept so the helper above has a purpose outside the config and stays type-checked. */
export const here = (path: string): string => fileURLToPath(new URL(path, import.meta.url));
