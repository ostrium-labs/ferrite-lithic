import assert from 'node:assert/strict';import {readFileSync} from 'node:fs';import ts from 'typescript';
const moduleURL=path=>'data:text/javascript;base64,'+Buffer.from(ts.transpileModule(readFileSync(new URL(path,import.meta.url),'utf8'),{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText).toString('base64');
const tickerURL=moduleURL('app/lib/ticker.ts');
const source=readFileSync(new URL('app/demos/bitdemos.ts',import.meta.url),'utf8').replace('"../lib/ticker"',JSON.stringify(tickerURL));
const demos=await import('data:text/javascript;base64,'+Buffer.from(ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText).toString('base64'));
assert.deepEqual(demos.toBits([0x01,0x80]),[true,false,false,false,false,false,false,false,false,false,false,false,false,false,false,true]);
let labels=[];const context=new Proxy({fillText:(text,x,y)=>labels.push({text:String(text),x,y}),measureText:text=>({width:String(text).length*8})},{get:(obj,key)=>key in obj?obj[key]:(()=>{})});
for(const [index,char] of [...'Hello World!'].entries()){labels=[];demos.drawHuffman(context,1100,340,index*900+750);assert.ok(labels.some(label=>label.text===String(char.charCodeAt(0))&&label.x===280),`symbol ${index} must decode ${char}`);assert.ok(labels.some(label=>label.text==='8'&&label.x===450),'ASCII literal consumes eight bits');}
const bytes=[0x01,0x05,0x00,0xfa,0xff,0x48,0x65,0x6c,0x6c,0x6f];
for(const [byteIndex,byte] of bytes.entries())for(let bit=0;bit<8;bit++){labels=[];demos.drawBitStream(context,1100,300,(byteIndex*9+bit+1)*90+40);const out=labels.find(label=>label.x===367&&label.y===139);assert.equal(out?.text,String((byte>>bit)&1),`byte ${byteIndex}, outgoing bit ${bit}`);}
console.log('Bit order, all 80 outgoing DEFLATE bits and all 12 Huffman symbol boundaries passed');
