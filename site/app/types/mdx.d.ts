/**
 * Types for `.mdx` content.
 *
 * `fumadocs-mdx/vite` compiles MDX to a component that takes a `components` map, but ships
 * no ambient declaration for the import. Without this every MDX import is `any` and the
 * `components` prop goes unchecked — which is exactly the class of mistake this project
 * documents as its most expensive bug.
 */

declare module "*.mdx" {
  import type { ComponentType } from "react";
  import type { mdxComponents } from "../components/mdx";

  const MDXContent: ComponentType<{ components?: typeof mdxComponents }>;

  export default MDXContent;
}
