# Verified Edge brand system

The reference boards are design references, not production images. The mark is authored vector geometry; none of the reference PNGs are cropped or embedded.

## Source of truth

`app/components/brand/geometry.ts` defines the 24-module mark and its explicitly pixel-snapped 16px adaptation. The two-unit trace branches through 45-degree chamfers into two four-unit square terminals. There are no curves, filters, masks, shadows or raster layers in the mark.

`FerriteMark.tsx` renders those primitives. Primary: ink branch/lower terminal, signal input/upper terminal. Inverse: paper branch/lower terminal and existing bright instrument amber. Mono follows currentColor. `FerriteLogo.tsx` supplies a horizontal or stacked lowercase Plex Mono wordmark. Linked header accessibility is exactly "Ferrite Lithic"; its SVG is decorative. Standalone marks have a single accessible graphic name.

The header keeps its existing height and the name remains visible on phones. The hero uses a quiet 20px identity in the existing provenance line. Footer stays typographic to avoid excessive repetition. No public brand route was added.

## Size and spacing

Use canonical marks at 20px or larger in UI, preferably 24-32px. Use the micro geometry at 16px for favicons. Lockup spacing is .7em with a small optical baseline correction. Keep one terminal-square width of clear space around standalone marks; avatar/app exports include extra padding. Compact browser favicons are the deliberate clear-space exception.

## Motion

Only the hero identity animates. One 720ms CSS sequence presents input, clock edge, upper endpoint, lower endpoint and settlement to the primary asymmetric state. A session-local flag prevents replay after SPA navigation. There are no timers, RAF callbacks, observers, motion libraries or event listeners. Reduced motion renders the static resolved state immediately. The header and favicons are static.

## Exports

Run from `site/`:

```sh
node scripts/export-brand.mjs
python scripts/export-wordmark.py
```

The first exporter reads canonical geometry and site color tokens, then exports SVG variants and browser-rendered icon/social PNGs. It requires the existing Playwright package and installed Chrome (override with BROWSER_EXECUTABLE). The second reads generated marks and outlines the existing `plexmono-latin-bb87500e.woff2` with Python fontTools. FontTools is an export-only local prerequisite; no browser dependency or additional web font is introduced.

Generated files under `public/brand/`:

- `ferrite-mark-primary.svg`, `ferrite-mark-inverse.svg`, `ferrite-mark-mono.svg`, `ferrite-mark-mono-inverse.svg`
- `ferrite-logo-primary.svg`, `ferrite-logo-inverse.svg`, `ferrite-logo-mono.svg`, `ferrite-logo-mono-inverse.svg`, `ferrite-logo-stacked.svg`
- `avatar.svg`
- `favicon-16.png`, `favicon-32.png`, `favicon-48.png`
- `apple-touch-icon.png` (180px), `icon-192.png`, `icon-512.png`
- `og-ferrite.png` (1200x630)

`public/favicon.svg` replaces the obsolete purple starter favicon with the micro mark on a dark tile. A consistent dark tile works against either browser-chrome color; it does not use runtime theme detection. No favicon.ico is retained or referenced. PNGs are intentional favicon/app/social exports, not replacements for the live SVG identity.

## Integration changes

- Added `geometry.ts`, `FerriteMark.tsx`, `FerriteLogo.tsx`, `FerriteAnimatedMark.tsx` under `app/components/brand/`.
- Added `app/styles/brand.css`, both deterministic export scripts and `verify-brand.mjs`.
- `SiteHeader.tsx`: replaces the generic square with the reusable logo and an explicit accessible link name.
- `home.tsx`: uses the small one-shot identity in the existing provenance line.
- `tokens.css`: names the existing bright instrument amber as `--instrument-signal`; existing site colors remain authoritative.
- `root.tsx`: imports brand CSS, links SVG/PNG favicon and touch icon, and supplies shared OG/Twitter image metadata. Existing route titles/descriptions remain intact.
- `layout.css`: removes obsolete `.sitebar__mark` rules.
- `site/README.md`: links the brand guide and brand verification command.
- No changes to repository README branding or external profiles. Portable assets are ready for later use.

## Validation and screenshots

TypeScript, unit tests and build pass. All 18 routes, the 16-page reading chain, font/code rendering and live diagrams pass without console errors. All 36 axe route/viewport checks pass. Existing interaction checks and five repeated SPA lifecycle loops pass with constant observer/listener counts and zero idle RAF. The full layout matrix passes all 306 route/width checks with zero failures. Edge, Firefox and WebKit each pass all 36 direct-route checks at phone and desktop widths. Brand-specific verification also passes against the built Cloudflare local preview. No lint command is configured in this repository.

Brand QA checks the header/home/docs at 320, 360, 375, 390, 430, 768, 1024, 1280, 1440, 1600 and 1920px. It checks compact header height, no page overflow, retained wordmark, asset HTTP responses in a clean browser context, resolved reduced-motion colors, no running animations after settlement and no replay on SPA return. Scale sheets cover 16, 20, 24, 32, 48, 64, 128 and 256px in primary, inverse, black and white forms. Favicon pixels and avatar 64/256px are captured separately.

Representative screenshots are in ignored `.artifacts/brand/`: `header-1440.png`, `header-375.png`, `hero-1440.png`, `hero-375.png`, `docs-1440.png`, `docs-375.png`, `scale-variants.png`, `favicon-avatar.png`. The production social card is `public/brand/og-ferrite.png`. Actual native browser tab-chrome screenshots were unavailable; favicon images were reviewed at actual pixel sizes in Chromium, and requested assets are explicitly checked. No claim is made about a screenshot of native browser chrome.

No commit, push or deployment was performed.
