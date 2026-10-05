import { chromium } from "playwright-core";
const EXE = "/home/keshav/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome";
const BASE = "http://localhost:5173";
const targets = process.argv.slice(2).length
  ? process.argv.slice(2).map((a) => { const [r, n] = a.split("="); return [r, n]; })
  : [["/", "landing"], ["/docs", "docs"], ["/docs/why-rust", "why-rust"], ["/docs/ocaml", "ocaml"]];
const b = await chromium.launch({ executablePath: EXE, args: ["--no-sandbox"] });
for (const [route, name] of targets) {
  const p = await b.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 2 });
  const errs = [];
  p.on("console", (m) => { if (m.type() === "error") errs.push(m.text()); });
  p.on("pageerror", (e) => errs.push(String(e)));
  await p.goto(BASE + route, { waitUntil: "networkidle" });
  await p.waitForTimeout(3000);
  if (name === "landing") {
    await p.screenshot({ path: `/tmp/kilo/${name}-hero.png` });
    // Scroll to the scrollytelling section and let a step become active.
    await p.evaluate(() => document.querySelector("#cycle")?.scrollIntoView({ block: "center" }));
    await p.waitForTimeout(1400);
    await p.screenshot({ path: "/tmp/kilo/landing-walk.png" });
  } else {
    await p.screenshot({ path: `/tmp/kilo/${name}.png`, fullPage: true });
  }
  console.log(name, errs.length ? `ERRORS ${errs.slice(0,2).join(" | ")}` : "clean");
  await p.close();
}
await b.close();
