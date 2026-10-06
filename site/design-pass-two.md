# Ferrite Lithic: second-pass design and validation

## Visual thesis and grid specification

Hardware expressed as Rust: a technical instrument on drafting paper, measured and inspectable. Preserve Archivo for reading, IBM Plex Mono for signals, paper #e9ebe9, raised paper #f5f6f4, ink #14181a, secondary ink #4e5754, signal #a34816 and fault #9e2b1c. Amber indicates signal or location; red indicates a defect.

The implementation uses a 96rem shell, fluid 20-56px gutters, twelve columns and a 34rem prose measure. Diagrams and evidence can span the shell. Chapter roles include narrative/code, full-width architecture, sticky cycle inspection, lineage chain, dependency inspection and paired verification evidence. Desktop docs use 14rem navigation / article / 13rem context above 80rem. At 60-80rem contents becomes a disclosure; below 60rem both navigation and contents appear as disclosures above the article. Important metadata is at least 13px; body is 16px and controls have generous targets. Mobile diagrams retain readable coordinates and scroll within named focusable regions.

## A. Read-only audit: what was found

Before this pass, all 18 routes were direct-loaded in Chromium at 1440x900. The baseline had no route errors or page overflow. Home, learning index, bits and benchmarks were captured at 320x800, 375x812, 430x932, 768x1024, 1024x768, 1280x800, 1440x900, 1600x1000 and 1920x1080. All routes also received desktop screenshots. Source inspection covered tokens, styles, routes, root, header, docs shell, MDX vocabulary, registries, data, ticker, canvas bindings and demos. Baseline images remain in the local temporary `ferrite-pass2/before` folder.

| Group | Priority | Finding |
| --- | --- | --- |
| Visual design | P1 | Hero hierarchy separated the definition, CTAs and trace; lineage was left-only prose; crates had no visible dependency relationships. Pipeline occupied excessive height and used red/green for ordinary architecture. |
| UX | P1/P2 | Flat document navigation, redundant crate names, no learning entry paths, no code copying or meaningful diagram controls. The contents rail's numbering and active selection were inaccurate. |
| Responsive | P0 | 1100-1200px canvas coordinates shrank to phone widths, making 9-12px labels unreadable. Laptop contents occupied excessive inline space. |
| Accessibility | P1 | Docs/index lacked a shared main landmark, skip destination was not focusable, definition terms lacked a dl, and overflowing code/tables needed keyboard focus. |
| Performance | P1 | CycleWalk redrew perpetually, including reduced motion. Mux mounted 336 uncancelled timers. Huffman recreated its table every frame. |
| Architecture | P1 | Monolithic CSS had conflicting overrides and dead reveal/backdrop rules. Header imported the complete MDX registry. Project counts and reading order were duplicated. |
| Credibility | P0/P1 | FNV and mux evidence had no styling. Shared lfsr appeared as a 22nd algorithm. Historical test counts disagree across source documents; a current total must not be fabricated. |

## B. Major changes and decisions

| Problem | Design decision | Implementation | Why this is better |
| --- | --- | --- | --- |
| Viewport underuse and repeated compositions | Give evidence its own grid role | Twelve-column shell; narrow prose, wide instruments, bespoke chapter layouts | Uses wide screens without making prose hard to read. |
| Weak hero entry hierarchy | Put definition, actions and proof together | Two-line desktop title, nearby CTAs, provenance, four-cycle trace and full-width stats | Readers can identify the product and their starting point immediately. |
| Architecture lacked inspectable relationships | Show actual paths and dependencies | Focus/touch/click stage controls, downstream highlighting and production crate dependencies | Makes one graph and two backends legible without hiding essential information. |
| Mobile labels were too small | Preserve logical text size | Focusable instrument scroll regions and hints | Signals remain readable and keyboard/touch inspectable. |
| Docs were a flat inventory | Present a learning map | Three entry routes, three semantic groups, reading sequence, prerequisites and canonical pagination | Readers can choose a route appropriate to their knowledge. |
| Verification looked like a claim | Put checks beside evidence | External reference / timing / backend-agreement loop, unit-bearing benchmark bars and aligned FNV rows | Readers can evaluate checks and see limitations directly. |
| Timers and perpetual redraws wasted work | Animate meaningful changes only | Finite traces, replay/pause, shared visibility-aware RAF, static mux cells and cached Huffman data | Idle pages remain quiet; reduced motion remains useful. |
| Metadata and CSS ownership overlapped | Assign a single owner to each domain | Canonical navigation/data/benchmarks and five domain stylesheets | Future changes have predictable places to live. |
| Production preview served wrong nested HTML | Preview through deployment's routing runtime | `npm run preview` now invokes `wrangler dev --local` | Direct clean URLs use the same static-asset behavior as the configured host. |

Additional correctness fixes: DEFLATE marks the actual outgoing bit; Huffman advances by decoded code length rather than one bit. Register traces show delayed q values. Contents tracking evaluates all headings rather than only the latest observer batch. Semantic fixes include one focusable main, a working keyboard skip link, valid definition lists and focusable overflowing tables/code. Hydration fallback and error metadata are explicit; duplicate article/context React keys are fixed. Instrument labels use Plex Mono and important text uses the higher-contrast palette.

## C. Individual file inventory

Paths below are relative to `site/`.

| File | Purpose |
| --- | --- |
| `app/tokens.css` | Palette, typography, shell/prose measures, gutters and offsets. |
| `app/app.css` | Font/token entry point; removes conflicting monolithic rules. |
| `app/styles/foundation.css` | Text, focus, semantic tables, code and motion primitives. |
| `app/styles/layout.css` | Shell, twelve-column grid, header, sections and footer. |
| `app/styles/landing.css` | Distinct responsive hero and chapter compositions. |
| `app/styles/docs.css` | Learning map, reading layout, rails and disclosures. |
| `app/styles/instruments.css` | Readable scrolling instruments, forensic cells and benchmark bars. |
| `app/root.tsx` | Main/skip landmarks, hydration fallback and safe errors. |
| `app/routes/home.tsx` | Integrated hero, controls and revised landing chapters. |
| `app/routes/docs-index.tsx` | Entry routes, grouped learning map and reading path. |
| `app/routes/doc.tsx` | Metadata safe during loader errors. |
| `app/components/SiteHeader.tsx` | Lightweight metadata and measured sticky-header height. |
| `app/components/DocShell.tsx` | Grouped navigation, prerequisites, pagination and code controls. |
| `app/components/DocRail.tsx` | Authored contents tracking, responsive disclosure and source context. |
| `app/components/CodeBlock.tsx` | Copy controls, fallback selection and reset-timer cleanup. |
| `app/components/Instrument.tsx` | Named frame and focusable local scroll region. |
| `app/components/CrateArchitecture.tsx` | Real production dependencies and accessible inspection. |
| `app/components/VerificationLoop.tsx` | Three complementary checks and their limits. |
| `app/components/BenchmarkFigure.tsx` | Linear bars and shared numeric benchmark table. |
| `app/components/CycleWalk.tsx` | Keyboard/touch inspection, full-node selection and finite drawing. |
| `app/components/canvas.ts` | Owned subscriptions, finite/live drawing and reduced motion. |
| `app/components/mdx.tsx` | Code copying, semantic definitions, shared counts and focusable tables. |
| `app/content/navigation.ts` | Reading order, groups, labels, prerequisites and source root. |
| `app/content/pages.ts` | Joins canonical navigation to authored content and source paths. |
| `app/content/paths.ts` | Derives route paths from navigation. |
| `app/content/toolchain.ts` | Reuses recorded crate counts. |
| `app/content/corpus.ts` | Reuses recorded totals; distinguishes shared recurrence. |
| `app/lib/data.ts` | Crate dependencies and derived project/tier counts. |
| `app/lib/benchmarks.ts` | Recorded speeds, assumed frequency and calculated verdicts. |
| `app/lib/ticker.ts` | Shared idle-safe RAF, cleanup and consistent palette/font. |
| `app/demos/hero.ts` | Readable four-cycle trace and delayed register values. |
| `app/demos/pipeline.ts` | Compact architecture, downstream highlighting and illustration labels. |
| `app/demos/bitdemos.ts` | Correct outgoing bits, code-length decoding and cached lookup data. |
| `app/demos/hexdiff.ts` | Nibble-aligned evidence, text verdicts and reference provenance. |
| `app/demos/mux.ts` | Static meaningful cost cells without mount timers. |
| `content/docs/benchmarks.mdx` | Uses the canonical speed table. |
| `content/docs/hardcaml.mdx` | Uses canonical algorithm count. |
| `content/docs/why-rust.mdx` | Uses canonical algorithm count. |
| `ticker.test.mjs` | RAF registration, pause/resume, removal and disposer assertions. |
| `demos.test.mjs` | Bit order, 80 outgoing DEFLATE bits and 12 Huffman symbol assertions. |
| `verify.mjs` | Direct routes, console, reading chain, fonts and live diagrams. |
| `verify-layout.mjs` | Seventeen-width/eighteen-route matrix and full-page captures. |
| `verify-accessibility.mjs` | Thirty-six axe WCAG A/AA scans. |
| `verify-interactions.mjs` | SPA, keyboard skip, copy, controls, motion and reflow. |
| `verify-lifecycle.mjs` | Observer/listener cleanup across repeated SPA transitions and idle RAF. |
| `verify-browsers.mjs` | Edge, Firefox and WebKit route/console/layout checks. |
| `shoot.mjs` | Five-width chapter screenshot capture. |
| `package.json` | Axe development dependency, unit tests and correct local preview. |
| `package-lock.json` | Two additional axe packages; existing dependency versions preserved. |
| `.gitignore` | Excludes generated screenshots and JSON evidence. |
| `README.md` | Content ownership, preview and verification instructions. |
| `design-pass-two.md` | Audit, decisions, inventory, evidence and limitations. |

## D. Validation results

| Check | Result |
| --- | --- |
| `npm run typecheck` | Pass. |
| `npm test` | Pass: ticker lifecycle; all 80 outgoing DEFLATE bits and 12 fixed-Huffman symbol boundaries. |
| `npm run build` | Pass: all 18 prerendered routes and SPA fallback. |
| Direct routes/console | All 18 routes pass; no console/page errors. Shiki, fonts and visible live demos pass. |
| Reading/navigation | Sixteen-page previous/next chain passes; all sixteen SPA transitions preserve client state. |
| Layout | Production Chromium: 18 routes x 17 widths = 306 checks, zero failures, one main and h1 per route, no page horizontal overflow. |
| Accessibility | All 18 routes at 375 and 1440px: 36 axe WCAG A/AA scans, zero violations. Automated evidence, not a claim of complete conformance. |
| Cross-browser | Edge, Firefox and WebKit each pass all 18 routes at 375 and 1440px: 36 checks per engine; no console/page errors or page overflow. HTTP 304 cached loads count as successful. |
| Controls/keyboard | Replay, pause, exact code copying (Windows newline normalization), dependency inspection, phone disclosure, skip-to-main and contents tracking pass. |
| Motion | Hero settles; paused/offscreen pipeline stays idle; reduced motion renders without animation. |
| Cleanup | Five repeated SPA mount/unmount loops: home stays at six intersection/six resize observers and nine window/media listeners; docs return to one of each observer and seven listeners. No accumulating registrations. Idle docs have zero RAF callbacks. |
| Reflow | Home, index, bits and benchmarks have no page overflow at 720 CSS pixels, the 200% reflow equivalent of a 1440px viewport. Native browser UI zoom is not claimed. |
| Screenshots | Ninety full-page route captures and fifty chapter viewport captures at 375, 768, 1024, 1440 and 1920px. Hero, learning map, docs shell and chapter compositions reviewed. |
| Whitespace | `git diff --check` passes. |

Width matrix: **320, 360, 375, 390, 430, 600, 768, 820, 900, 1024, 1100, 1280, 1366, 1440, 1536, 1600 and 1920px**. Artifacts are in ignored `site/.artifacts/pass-two/`, including layout, accessibility, interaction, browser and lifecycle JSON. The final chapter captures use the production preview.

Rendered critique prompted a four-cycle compact hero to fit its desktop column, readable minimum diagram widths and improved forensic-column placement. The visual language stays paper/amber/Archivo/Plex; no illustration, new font family, generic gradient, glass effect or decorative looping motion was introduced.

## E. Remaining issues and intentional limits

- Historical measurements remain documented snapshots. README's 811 test total differs from the crate rows' 810 sum. This pass does not invent a current test result. A Rust CI-generated manifest pinned to commits should replace those historical counts.
- The 21 algorithms exclude shared `lfsr`, which is labelled separately. Recorded Verilator coverage is evidence from the documented corpus, not a live certification.
- Benchmark design ns assumes 1 GHz and is labelled accordingly. No new hardware/software benchmark run was performed.
- Native browser UI zoom, screen-reader review and field Core Web Vitals measurements remain manual follow-ups. The keyboard, axe and 200% reflow-equivalent checks do not claim to cover these.
- Shared CSS is about 36.2kB / 7.35kB gzip; home JavaScript is about 47.0kB / 15.56kB gzip. These are build sizes, not field performance results.
- Source links intentionally point at current `dev`, while measurements are historical. The docs context rail makes that distinction explicit.
- `vite preview` returned incorrect nested prerender HTML and caused hydration errors. Local Wrangler static-asset routing fixed the preview workflow. No deployment was performed.
