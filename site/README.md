# ferrite-lithic-site

The documentation site for [Ferrite Lithic](../README.md): an embedded hardware DSL
in Rust, with a corpus of twenty-one real algorithms that are each checked three ways.

## What it is

A [React Router 8](https://reactrouter.com) app in framework mode, with
[`fumadocs-mdx`](https://fumadocs.dev) as the content layer. Three route patterns own sixteen
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

`app/content/navigation.ts` owns the document order, labels, groups and prerequisites.
`app/content/pages.ts` joins that metadata to typed content and compiled MDX.
`app/lib/data.ts` owns crate dependencies and recorded project counts;
`app/lib/benchmarks.ts` owns the paired speed measurements.

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
the canonical navigation in `app/content/navigation.ts`, via `app/content/paths.ts`.
Add new documents to that navigation and the content registry.

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

Run `npm run typecheck`, `npm test` and `npm run build`. Preview the production assets
with `npm run preview -- --port 4174`; this uses Cloudflare's local static-asset routing,
including nested prerendered pages. No deployment is performed.

The browser scripts use an installed Chrome by default. Set `BROWSER_EXECUTABLE` to
another Chromium executable and `SITE_BASE_URL` to the running preview (default
`http://127.0.0.1:5173`). Run from `site/`:

```sh
node verify.mjs                 # routes, console, reading order, fonts and live demos
node verify-layout.mjs          # all 18 routes at 17 widths; screenshots at five widths
node verify-accessibility.mjs   # axe WCAG A/AA checks at 375 and 1440px
node verify-interactions.mjs    # keyboard, copy, SPA, controls, motion and reflow
node verify-lifecycle.mjs       # observer counts across repeated SPA mount/unmount
node verify-browsers.mjs        # installed Edge, Playwright Firefox and WebKit
node shoot.mjs                  # chapter screenshots at five widths
```

Set `SKIP_SCREENSHOTS=1` to make the layout matrix verification-only. Browser artifacts
are saved under ignored `.artifacts/pass-two/`. Tests exit nonzero on failed assertions;
missing optional browser engines are reported as failures rather than claimed as tested.
See [the second-pass design report](design-pass-two.md) for decisions and limitations.

## Verified Edge identity

The SVG geometry, component APIs, deterministic export commands, asset inventory and
validation notes are documented in [BRAND.md](BRAND.md). Run `node verify-brand.mjs`
against the local preview for responsive lockups, icon requests and finite/reduced motion.
