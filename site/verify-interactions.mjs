import assert from 'node:assert/strict';
import {chromium} from 'playwright-core';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
const base=process.env.SITE_BASE_URL||'http://127.0.0.1:5173';
const browser=await chromium.launch({executablePath:process.env.BROWSER_EXECUTABLE||'C:/Program Files/Google/Chrome/Application/chrome.exe'});
const context=await browser.newContext({viewport:{width:1440,height:900},permissions:['clipboard-read','clipboard-write']});
const page=await context.newPage(); const passed=[];
await page.addInitScript(()=>{window.drawCounts={};const clear=CanvasRenderingContext2D.prototype.clearRect;CanvasRenderingContext2D.prototype.clearRect=function(...args){const id=this.canvas.id||'walk';window.drawCounts[id]=(window.drawCounts[id]||0)+1;return clear.apply(this,args);};});
try{
 await page.goto(base);await page.waitForTimeout(6500);
 const hero=await page.evaluate(()=>window.drawCounts['timing-hero']);await page.waitForTimeout(450);assert.equal(await page.evaluate(()=>window.drawCounts['timing-hero']),hero);passed.push('Finite hero stops drawing');
 await page.getByRole('button',{name:'Replay trace'}).click();await page.waitForTimeout(250);assert.ok(await page.evaluate(()=>window.drawCounts['timing-hero'])>hero);passed.push('Replay redraws hero');
 await page.locator('.code-surface').first().scrollIntoViewIfNeeded();const expected=await page.locator('.code-surface pre').first().textContent();await page.locator('.code-surface button').first().click();assert.equal(await page.evaluate(()=>navigator.clipboard.readText()).then(text=>text.replaceAll("\r\n","\n")),expected);passed.push('Copy preserves exact rendered code');
 await page.locator('.crate-architecture__controls button').filter({hasText:/^rtl$/}).click();assert.equal(await page.locator('.crate-architecture__controls button[aria-pressed=true]').textContent(),'rtl');assert.match(await page.locator('.crate-architecture__relation').textContent(),/bits, ir/);passed.push('Crate inspection exposes real dependencies');
 await page.locator('#pipeline-canvas').scrollIntoViewIfNeeded();await page.getByRole('button',{name:'Pause signal',exact:true}).click();await page.waitForTimeout(150);const pipeline=await page.evaluate(()=>window.drawCounts['pipeline-canvas']);await page.waitForTimeout(450);assert.equal(await page.evaluate(()=>window.drawCounts['pipeline-canvas']),pipeline);passed.push('Paused pipeline stops drawing');
 await page.locator('.hero').scrollIntoViewIfNeeded();await page.waitForTimeout(200);const offscreen=await page.evaluate(()=>window.drawCounts['pipeline-canvas']);await page.waitForTimeout(450);assert.equal(await page.evaluate(()=>window.drawCounts['pipeline-canvas']),offscreen);passed.push('Offscreen pipeline stays idle');
 await page.emulateMedia({reducedMotion:'reduce'});await page.reload();await page.locator('#pipeline-canvas').scrollIntoViewIfNeeded();await page.waitForTimeout(450);const reduced=await page.evaluate(()=>({...window.drawCounts}));await page.waitForTimeout(450);assert.deepEqual(await page.evaluate(()=>window.drawCounts),reduced);passed.push('Reduced motion renders without animation');
 await page.goto(base);await page.keyboard.press('Tab');assert.match(await page.locator(':focus').textContent(),/Skip/);await page.keyboard.press('Enter');assert.equal(await page.locator(':focus').getAttribute('id'),'main');passed.push('Keyboard skip focuses main');
 const slugs=Array.from((await readFile('app/content/navigation.ts','utf8')).matchAll(/slug: "([^"]+)"/g),m=>m[1]);await page.goto(base+'/docs');await page.evaluate(()=>window.spaProbe='preserved');
 for(const slug of slugs){await page.locator(`a[href="/docs/${slug}"]`).first().click();await page.waitForURL('**/docs/'+slug);assert.equal(await page.evaluate(()=>window.spaProbe),'preserved');assert.equal(await page.locator('main').count(),1);}
 passed.push('All 16 docs navigate through SPA with state preserved');
 await page.goto(base+'/docs/bits');const heading=page.locator('.step').last();await heading.scrollIntoViewIfNeeded();await page.waitForTimeout(350);assert.ok(await page.locator('.onpage [aria-current]').count());passed.push('Reading rail tracks a scrolled section');
 await page.setViewportSize({width:375,height:812});await page.goto(base+'/docs/bits');await page.locator('.sidebar__disclosure summary').click();assert.ok(await page.locator('.sidebar__link').first().isVisible());passed.push('Phone navigation disclosure works');
 await page.setViewportSize({width:720,height:450});for(const route of ['/','/docs','/docs/bits','/docs/benchmarks']){await page.goto(base+route);assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1),false);}passed.push('200% reflow equivalent (1440 → 720 CSS pixels) has no page overflow');
 await mkdir('.artifacts/pass-two',{recursive:true});await writeFile('.artifacts/pass-two/interactions.json',JSON.stringify(passed,null,2));console.log(passed.join('\n'));
}finally{await browser.close();}
