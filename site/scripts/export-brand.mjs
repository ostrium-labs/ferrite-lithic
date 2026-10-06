import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {chromium} from 'playwright-core';
import {MARK,MICRO_MARK} from '../app/components/brand/geometry.ts';
const tokens=await readFile('app/tokens.css','utf8');
const color=name=>tokens.match(new RegExp(`--${name}:\\s*(#[a-fA-F0-9]+)`))[1];
const ink=color('ink'),paper=color('paper'),signal=color('signal'),bright=color('instrument-signal');
const rect=(r,fill)=>`<rect x="${r.x}" y="${r.y}" width="${r.width}" height="${r.height}" fill="${fill}"/>`;
function shapes(g,fg,accent){return `<path d="${g.branch}" fill="none" stroke="${fg}" stroke-width="2" stroke-linejoin="miter"/>${rect(g.input,accent)}${rect(g.upper,accent)}${rect(g.lower,fg)}`;}
const svg=(box,body)=>`<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box}" role="img" aria-label="Ferrite Lithic — Verified Edge">${body}</svg>\n`;
await mkdir('public/brand',{recursive:true});
for(const [name,fg,accent] of [['primary',ink,signal],['inverse',paper,bright],['mono',ink,ink],['mono-inverse',paper,paper]])await writeFile(`public/brand/ferrite-mark-${name}.svg`,svg(MARK.viewBox,shapes(MARK,fg,accent)));
const font=await readFile('public/fonts/plexmono-latin-bb87500e.woff2');
const fontCSS=`@font-face{font-family:Plex;src:url(data:font/woff2;base64,${font.toString('base64')})}`;
const browser=await chromium.launch({executablePath:process.env.BROWSER_EXECUTABLE||'C:/Program Files/Google/Chrome/Application/chrome.exe'});const page=await browser.newPage({deviceScaleFactor:1});
try{
 for(const size of [16,32,48,180,192,512]){
  const micro=size===16;const scale=micro?1:size/(size>=180?32:24);const g=micro?MICRO_MARK:MARK;
  const icon=svg(`0 0 ${size} ${size}`,`<rect width="${size}" height="${size}" fill="${ink}"/><g transform="scale(${scale}) ${size>=180?'translate(4 4)':''}">${shapes(g,paper,bright)}</g>`);
  const name=size===180?'apple-touch-icon':size>=192?`icon-${size}`:`favicon-${size}`;
  await page.setViewportSize({width:size,height:size});await page.setContent(`<style>body{margin:0}svg{display:block}</style>${icon}`);await page.screenshot({path:`public/brand/${name}.png`});
 }
 await writeFile('public/favicon.svg',svg(MICRO_MARK.viewBox,`<rect width="16" height="16" fill="${ink}"/>${shapes(MICRO_MARK,paper,bright)}`));
 await writeFile('public/brand/avatar.svg',svg('0 0 32 32',`<rect width="32" height="32" fill="${ink}"/><g transform="translate(4 4)">${shapes(MARK,paper,bright)}</g>`));
 const og=`<style>${fontCSS}*{box-sizing:border-box}body{margin:0;width:1200px;height:630px;background:${ink};color:${paper};font-family:Plex;padding:64px}header{display:flex;gap:32px;align-items:center;font-size:40px}svg{width:112px;height:112px}h1{font-size:54px;font-weight:400;line-height:1.25;margin:44px 0 40px}footer{border-top:1px solid #4e5754;padding-top:24px;font-size:24px;color:#a4ada9}</style><header>${svg(MARK.viewBox,shapes(MARK,paper,bright))}<span>ferrite-lithic</span></header><h1>Hardware you write in Rust,<br>as real gates.</h1><footer>One graph. Two backends. One clock.</footer>`;
 await page.setViewportSize({width:1200,height:630});await page.setContent(og);await page.evaluate(()=>document.fonts.ready);await page.screenshot({path:'public/brand/og-ferrite.png'});
}finally{await browser.close();}
console.log('Brand SVGs, favicon pixels, app icons and social card exported');
