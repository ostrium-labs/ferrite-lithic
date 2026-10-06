import {chromium} from 'playwright-core';
import {mkdir} from 'node:fs/promises';
const browser=await chromium.launch({executablePath:process.env.BROWSER_EXECUTABLE||'C:/Program Files/Google/Chrome/Application/chrome.exe'});
const base=process.env.SITE_BASE_URL||'http://127.0.0.1:5173';
try{for(const width of [375,768,1024,1440,1920]){const page=await browser.newPage({viewport:{width,height:1000},reducedMotion:'reduce'});await page.goto(base,{waitUntil:'networkidle'});await mkdir(`.artifacts/pass-two/chapters/${width}`,{recursive:true});for(const id of ['hero','what','pipeline','cycle','lineage','crates','corpus','bugs','gap','limits']){const section=page.locator(id==='hero'?'.hero':'#'+id);await section.scrollIntoViewIfNeeded();await page.waitForTimeout(120);await page.screenshot({path:`.artifacts/pass-two/chapters/${width}/${id}.png`});}await page.close();console.log(`${width}px chapter captures complete`);}}finally{await browser.close();}
