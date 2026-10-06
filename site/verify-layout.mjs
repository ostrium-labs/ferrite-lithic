import { chromium } from "playwright-core";
import { readFile, mkdir, writeFile } from "node:fs/promises";
const widths = [320,360,375,390,430,600,768,820,900,1024,1100,1280,1366,1440,1536,1600,1920];
const routes = ["/", "/docs", ...Array.from((await readFile("app/content/navigation.ts", "utf8")).matchAll(/slug: "([^"]+)"/g), m => `/docs/${m[1]}`)];
const base = process.env.SITE_BASE_URL || "http://127.0.0.1:5173";
const browser = await chromium.launch({executablePath:process.env.BROWSER_EXECUTABLE || "C:/Program Files/Google/Chrome/Application/chrome.exe"});
const results = [], failures = [];
await mkdir(".artifacts/pass-two", {recursive:true});
try {
 for (const width of widths) {
  const context = await browser.newContext({viewport:{width,height:900},reducedMotion:"reduce"});
  const page = await context.newPage();
  for (const route of routes) {
   const errors = []; const onError = e => errors.push(e.message); page.on("pageerror", onError);
   const response = await page.goto(base + route, {waitUntil:"networkidle"});
   const layout = await page.evaluate(() => ({overflow:document.documentElement.scrollWidth > innerWidth + 1,main:document.querySelectorAll("main").length,h1:document.querySelectorAll("h1").length,canvases:[...document.querySelectorAll("canvas")].map(c=>({width:c.getBoundingClientRect().width,scrollable:!!c.closest(".instrument-scroll")}))}));
   const result = {width,route,status:response.status(),...layout,errors}; results.push(result);
   if(result.status!==200 || layout.overflow || layout.main!==1 || layout.h1!==1 || errors.length) failures.push(result);
   page.off("pageerror",onError);
   if(!process.env.SKIP_SCREENSHOTS && [375,768,1024,1440,1920].includes(width)) await page.screenshot({path:`.artifacts/pass-two/${route==='/'?'home':route.slice(1).replaceAll('/','-')}-${width}.png`,fullPage:true});
  }
  await context.close(); console.log(`${width}px: ${routes.length} routes checked`);
 }
 await writeFile(".artifacts/pass-two/layout.json",JSON.stringify({results,failures},null,2));
 console.log(`${results.length} route/width checks; ${failures.length} failures`);
 if(failures.length) throw Error(JSON.stringify(failures));
} finally {await browser.close();}
