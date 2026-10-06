import type { Route } from "./+types/doc";
import { DocShell } from "../components/DocShell";
import { BY_SLUG } from "../content/pages";

/** The page title, set from the registry so it cannot disagree with the sidebar. */
export function loader({ params }: Route.LoaderArgs) {
  const slug = params.slug;
  const entry = BY_SLUG.get(slug);
  if (!entry) throw new Response("Not found", { status: 404 });
  return { slug, title: entry.title, description: entry.description };
}

export function meta({ loaderData }: Route.MetaArgs) {
  if (!loaderData) return [{ title: "Page not found — Ferrite Lithic" }];
  return [
    { title: `${loaderData.title} — Ferrite Lithic` },
    { name: "description", content: loaderData.description },
  ];
}

export default function DocRoute({ loaderData }: Route.ComponentProps) {
  return <DocShell slug={loaderData.slug} />;
}
