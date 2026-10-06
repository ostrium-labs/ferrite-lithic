# ferrite-lithic-site

The documentation site for [Ferrite Lithic](../README.md): an embedded hardware DSL
in Rust, with a corpus of twenty-one real algorithms that are each checked three ways.

## What it is

A [React Router 8](https://reactrouter.com) app in framework mode, with
[`fumadocs-mdx`](https://fumadocs.dev) as the content layer. Three routes own fifteen
documentation pages plus the landing page.

```
/                      the landing page: one argument, made with a timing diagram
/docs                  the reading order
/docs/:slug            a page
```

`fumadocs-mdx` is used for what it is good at — compiling MDX and highlighting code with
Shiki at build time. The layout, the design system and the components are ours, because
`fumadocs-ui` ships its own opinionated chrome and this site has its own.

## Two kinds of page

| | Authored as | Rendered by | Why |
|---|---|---|---|
| Background | `content/docs/*.mdx` | MDX + `app/components/mdx.tsx` | Prose, and Shiki highlights fenced code at build time so a broken fence fails the build |
| Crate walkthroughs | `app/content/*.ts` | `app/components/DocShell.tsx` | The step/figure/notes structure is worth having checked by the compiler rather than trusted to prose |

`app/content/pages.ts` is the single registry: it decides what exists, what order it is
read in, and what the sidebar shows. Adding a page means adding a file and one entry.

## The design system

Built from the subject matter rather than from a template. The reasoning is written down in
`app/tokens.css`, which is the file to read before changing a colour:

- **Paper ground, not dark.** This is a hardware project, and dark-with-neon is the reflex
  choice for anything hardware — which is why it was rejected. Most of the site is
  long-form documentation, and a signal drawn on paper is what real test equipment looks
  like anyway.
- **Six core colours.** One accent (sodium amber, the colour of a trace) and one fault red.
  Everything else is ink at an alpha.
- **Two typefaces.** Archivo for prose, IBM Plex Mono for everything that is an identifier
  — in a hardware DSL the port names and bit widths *are* the content, and setting them in
  a proportional face misrepresents their shape. Both self-hosted; no third-party runtime
  request.
- **Rules, not cards.** A rounded rectangle here means "a control". Prose and code are text,
  so both are on paper; only the waveform diagrams flip to the dark instrument ground.

## Commands

```sh
npm run dev        # dev server on :5173
npm run build      # production build
npm run start      # serve the build
npm run typecheck  # react-router typegen && tsc
```

## Cloudflare Workers deployment

The build prerenders the landing page, documentation index, and every documentation
slug into `build/client`. `wrangler.jsonc` deploys these as static assets to the
`ferrite` Worker; no runtime server is required. Documentation paths are shared with
the page registry in `app/content/paths.ts`. Add new MDX slugs there as well as their
registry entries; crate walkthrough paths come from the typed page objects.

Connect `ostrium-labs/ferrite-lithic` in Cloudflare Workers Builds with:

| Setting | Value |
|---|---|
| Production branch | `dev` |
| Root directory | `site` |
| Build command | `npm run build` |
| Deploy command | `npx wrangler deploy` |
| Build token | Existing `ostrium build token`, if it permits deploying `ferrite` |

The committed npm lockfile selects npm for dependency installation. Enable automatic
builds on pushes to the production branch. The token needs permission to edit Workers
in the target account; read-only access cannot create the Worker or its build connection.

For a local preview or authenticated manual deployment:

```sh
npm ci
npm run build
npm run start     # preview with Workers static asset routing
npm run deploy    # requires Cloudflare write access
```

After the first successful deployment, add `ferrite.ostriumlabs.org` under the Worker's
Settings → Domains & Routes → Add → Custom Domain in Cloudflare.

## Verifying

`verify.mjs` drives a real browser over every route and asserts the things that are easy to
break silently: that each page renders content, that the sidebar marks the current page,
that the prev/next chain reaches all fifteen pages, that Shiki actually ran on the MDX
pages, that the hero timing diagram draws once and then holds, that the scrolling demos
animate only while on screen, and that both typefaces are applied.

```sh
npm run dev &
node verify.mjs     # exits non-zero on any failure
```

It needs a Chromium binary. The path is hard-coded to the locally installed Playwright
build and overridable with `CHROME=/path/to/chrome`.

`shoot.mjs` takes screenshots for design review: `node shoot.mjs "/=landing" "/docs=docs"`.
