import { chromium } from "playwright-core";

const EXECUTABLE = "/home/keshav/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome";
const BASE = "http://localhost:5173";

const ROUTES = [
  "/", "/docs",
  "/docs/basics", "/docs/ocaml", "/docs/hardcaml", "/docs/why-rust", "/docs/benchmarks",
  "/docs/bits", "/docs/ir", "/docs/design", "/docs/sim", "/docs/rtl",
  "/docs/derive", "/docs/wave", "/docs/cosim", "/docs/tb",
  "/docs/corpus", "/docs/findings",
];

const browser = await chromium.launch({ executablePath: EXECUTABLE, args: ["--no-sandbox"] });
let failures = 0;

for (const route of ROUTES) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));

  const response = await page.goto(BASE + route, { waitUntil: "networkidle" });
  await page.waitForTimeout(route === "/" ? 2800 : 500);

  const info = await page.evaluate(() => ({
    title: document.title,
    h1: document.querySelector("h1")?.textContent?.trim() ?? "",
    steps: document.querySelectorAll(".step").length,
    sidebar: document.querySelectorAll(".sidebar__item").length,
    current: document.querySelector(".sidebar__link.is-current")?.textContent?.trim() ?? "",
    // Shiki emits spans with inline styles; its presence proves the MDX pipeline ran.
    shiki: document.querySelectorAll("pre code span[style]").length,
    pre: document.querySelectorAll("pre").length,
    figures: document.querySelectorAll(".figure").length,
    notes: document.querySelectorAll(".note").length,
    tables: document.querySelectorAll("table.compare").length,
    next: document.querySelector(".doc-footer__link--next")?.getAttribute("href") ?? "",
    cards: document.querySelectorAll(".index-card").length,
    barLinks: document.querySelectorAll(".sitebar__link").length,
    barCurrent: document.querySelectorAll(".sitebar__link.is-current").length,
  }));

  const problems = [];
  if (response.status() !== 200) problems.push(`status ${response.status()}`);
  if (errors.length) problems.push(`console: ${errors.slice(0, 2).join(" | ")}`);
  if (!info.h1) problems.push("no <h1>");
  if (info.barLinks < 5) problems.push(`header has ${info.barLinks} links, want 5`);
  // /docs is the hub of all four sections, so the bar deliberately highlights none of
  // them there. Every other route must mark exactly one.
  if (route === "/docs") {
    if (info.barCurrent !== 0) problems.push(`the hub highlighted ${info.barCurrent} sections`);
  } else if (info.barCurrent !== 1) {
    problems.push(`header marks ${info.barCurrent} sections, want exactly 1`);
  }
  if (route.startsWith("/docs/") && route !== "/docs") {
    if (info.sidebar !== 16) problems.push(`sidebar ${info.sidebar}, want 16`);
    if (!info.current) problems.push("no current item");
    if (!info.next && !["/docs/findings", "/docs/benchmarks"].includes(route)) problems.push("no next link");
  }
  if (route === "/docs" && info.cards !== 16) problems.push(`${info.cards} cards, want 16`);
  if (["/docs/basics", "/docs/ocaml", "/docs/hardcaml", "/docs/why-rust", "/docs/benchmarks"].includes(route)) {
    if (info.shiki === 0) problems.push("no highlighted code — MDX/Shiki did not run");
    // `benchmarks` is a reference page, not a numbered walkthrough, so no steps is right.
    if (info.steps < 5 && route !== "/docs/benchmarks") problems.push(`only ${info.steps} steps`);
  }

  if (problems.length) failures += 1;
  console.log(
    `${problems.length ? "FAIL" : "ok  "} ${route.padEnd(18)} steps=${String(info.steps).padStart(2)} ` +
    `shiki=${String(info.shiki).padStart(4)} pre=${String(info.pre).padStart(2)} ` +
    `fig=${info.figures} notes=${info.notes} table=${info.tables} side=${info.sidebar} bar=${info.barCurrent}`,
    problems.length ? `\n     -> ${problems.join("; ")}` : "",
  );
  await page.close();
}

// Follow prev/next through the whole order.
const page = await browser.newPage();
await page.goto(`${BASE}/docs/basics`, { waitUntil: "networkidle" });
const seen = ["basics"];
for (let i = 0; i < 20; i += 1) {
  const href = await page.evaluate(() =>
    document.querySelector(".doc-footer__link--next")?.getAttribute("href") ?? null);
  if (!href) break;
  const r = await page.goto(new URL(href, page.url()).href, { waitUntil: "networkidle" });
  if (r.status() !== 200) { failures += 1; console.log(`FAIL chain ${href} ${r.status()}`); break; }
  seen.push(href.replace("/docs/", ""));
}
console.log(`\nchain (${seen.length}): ${seen.join(" -> ")}`);
if (seen.length !== 16) { failures += 1; console.log(`FAIL chain length ${seen.length}, want 16`); }

// Canvas behaviour. Two different contracts, so two different assertions:
//   the hero draws once and then HOLDS (a looping hero competes with the text below it),
//   the scrolling demos animate only while on screen (that is the point of onVisibility).
await page.goto(BASE + "/", { waitUntil: "networkidle" });

const hero = await page.evaluate(async () => {
  const c = document.querySelector("#timing-hero");
  if (!c) return "MISSING";
  const drawn = c.toDataURL().length;
  await new Promise((r) => setTimeout(r, 900));
  const midway = c.toDataURL();
  await new Promise((r) => setTimeout(r, 2600));
  const settled = c.toDataURL();
  // Non-trivial pixel content, changing while drawing, then stable once complete.
  const ctx = c.getContext("2d");
  const d = ctx.getImageData(0, 0, c.width, c.height).data;
  let lit = 0;
  for (let i = 0; i < d.length; i += 400) if (d[i] > 40) lit++;
  return `drawn=${drawn > 5000} changedDuring=${midway !== settled} holdsAfter=${settled === c.toDataURL()} lit=${lit}`;
});
console.log(`hero timing diagram: ${hero}`);
if (!hero.includes("drawn=true") || !hero.includes("lit=") ) failures += 1;
if (hero.includes("MISSING")) { failures += 1; console.log("FAIL hero canvas missing"); }

for (const id of ["pipeline-canvas", "bitstream-canvas", "huffman-canvas"]) {
  const result = await page.evaluate(async (canvasId) => {
    const c = document.querySelector("#" + canvasId);
    if (!c) return "MISSING";
    c.scrollIntoView({ block: "center" });
    await new Promise((r) => setTimeout(r, 700));
    const a = c.toDataURL();
    await new Promise((r) => setTimeout(r, 600));
    const b = c.toDataURL();
    return a !== b ? "animating" : "frozen";
  }, id);
  console.log(`${id}: ${result}`);
  if (result !== "animating") { failures += 1; console.log(`FAIL ${id} ${result}`); }
}

const fontOk = await page.evaluate(() => {
  const h = document.querySelector(".hero__title");
  return {
    archivo: getComputedStyle(document.body).fontFamily.includes("Archivo"),
    mono: getComputedStyle(document.querySelector("code") ?? document.body).fontFamily.includes("Plex Mono"),
    nameplate: h ? getComputedStyle(h).fontVariationSettings : "",
    width: h ? Math.round(h.getBoundingClientRect().width) : 0,
  };
});
console.log(`fonts: archivo=${fontOk.archivo} plexMono=${fontOk.mono} nameplate="${fontOk.nameplate}" w=${fontOk.width}`);
if (!fontOk.archivo || !fontOk.mono) { failures += 1; console.log("FAIL fonts not applied"); }

await browser.close();
console.log(failures === 0 ? "\nALL PASS" : `\n${failures} FAILURES`);
process.exit(failures === 0 ? 0 : 1);
