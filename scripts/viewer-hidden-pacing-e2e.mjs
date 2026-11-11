// A server that doesn't know Itda's "nobody is looking" hint (Apple Screen Sharing, other VNC
// servers: no Continuous Updates, one update per request) gets fewer requests while the page is
// hidden: at most one a second, and the next at once when it is seen again. Real bundled noVNC,
// synthetic RFB server; page visibility is faked (headless Chromium has none). No real host.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const host={id:'pacing-fixture',label:'Mac',name:'Mac',kind:'vnc',os:'mac',opts:{clipboard:true}};
const BACKGROUND=0x49544442;
const peers=[],sockets=new Set();
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 const json=v=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify(v));};
 if(url.pathname==='/api/me')return json({local:true});
 if(url.pathname==='/api/hosts')return json([host]);
 const name=url.pathname==='/'?'index.html':url.pathname.slice(1);
 if(!['index.html','viewer.js','viewer.css','monitors.js','interaction.js','novnc.min.js','manifest.webmanifest'].includes(name)&&!/^(i18n\.js|locales\/[\w.-]+\.js)$/.test(name)){res.writeHead(404);return res.end();}
 res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(path.join(root,/^(i18n\.js|locales\/[\w.-]+\.js)$/.test(name)?'web':'web/viewer',name)));
});
server.on('connection',s=>{sockets.add(s);s.on('close',()=>sockets.delete(s));});
function frame(bytes){let h;if(bytes.length<126)h=Buffer.from([0x82,bytes.length]);else if(bytes.length<65536){h=Buffer.alloc(4);h[0]=0x82;h[1]=126;h.writeUInt16BE(bytes.length,2);}else{h=Buffer.alloc(10);h[0]=0x82;h[1]=127;h.writeBigUInt64BE(BigInt(bytes.length),2);}return Buffer.concat([h,bytes]);}
const u16=n=>{const b=Buffer.alloc(2);b.writeUInt16BE(n);return b;};
const u32=n=>{const b=Buffer.alloc(4);b.writeUInt32BE(n>>>0);return b;};
const rect=(x,y,w,h,e,d=Buffer.alloc(0))=>Buffer.concat([u16(x),u16(y),u16(w),u16(h),u32(e),d]);
const update=rects=>Buffer.concat([Buffer.from([0,0]),u16(rects.length),...rects]);
const solid=(w,h,g)=>{const d=Buffer.alloc(w*h*4);for(let i=0;i<d.length;i+=4)d[i+1]=g;return d;};
server.on('upgrade',(req,socket)=>{
 const accept=createHash('sha1').update(req.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
 socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: '+accept+'\r\nSec-WebSocket-Protocol: binary\r\n\r\n');
 const peer={encodings:[],pointer:[],relative:[],requests:0,times:[],send:d=>socket.write(frame(d))};peers.push(peer);
 peer.send(Buffer.from('RFB 003.008\n'));
 let wire=Buffer.alloc(0),input=Buffer.alloc(0),phase=0;
 const consume=()=>{for(;;){
  if(phase===0){if(input.length<12)return;input=input.subarray(12);phase=1;peer.send(Buffer.from([1,1]));}
  else if(phase===1){if(input.length<1)return;input=input.subarray(1);phase=2;peer.send(u32(0));}
  else if(phase===2){if(input.length<1)return;input=input.subarray(1);phase=3;const name=Buffer.from('Pacing fixture');peer.send(Buffer.concat([u16(64),u16(48),Buffer.from([32,24,0,1,0,255,0,255,0,255,16,8,0,0,0,0]),u32(name.length),name]));}
  else{
   if(!input.length)return;const t=input[0];let n=({0:20,3:10,4:8,5:6,150:10,248:9,0xea:6})[t];
   if(t===2){if(input.length<4)return;n=4+input.readUInt16BE(2)*4;}
   if(t===6){if(input.length<8)return;n=8+Math.abs(input.readInt32BE(4));}
   if(!n)throw Error('Unexpected RFB client message '+t);
   if(input.length<n)return;const m=input.subarray(0,n);input=input.subarray(n);
   if(t===2){const e=[];for(let i=4;i<n;i+=4)e.push(m.readInt32BE(i));peer.encodings.push(e);}
   if(t===3){peer.requests++;peer.times.push(Date.now());const g=peer.requests===1?40:peer.requests%200;setTimeout(()=>peer.send(update([rect(0,0,64,48,0,solid(64,48,g))])),5);}
   if(t===5)peer.pointer.push(m[1]);
   if(t===0xea)peer.relative.push([m[1],m.readInt16BE(2),m.readInt16BE(4)]);
  }
 }};
 socket.on('data',chunk=>{wire=Buffer.concat([wire,chunk]);while(wire.length>=2){const op=wire[0]&15;let n=wire[1]&127,at=2;if(n===126){if(wire.length<4)return;n=wire.readUInt16BE(2);at=4;}else if(n===127){if(wire.length<10)return;n=Number(wire.readBigUInt64BE(2));at=10;}const masked=!!(wire[1]&128),start=at+(masked?4:0);if(wire.length<start+n)return;const d=Buffer.from(wire.subarray(start,start+n));if(masked)for(let i=0;i<n;i++)d[i]^=wire[at+i%4];wire=wire.subarray(start+n);if(op===8)return socket.end();input=Buffer.concat([input,d]);consume();}});
 socket.on('error',()=>{});
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const port=server.address().port;
const until=async(what,ok,ms=3000)=>{const end=Date.now()+ms;while(!ok()){if(Date.now()>end)throw Error('timed out: '+what);await new Promise(r=>setTimeout(r,10));}};
let browser;
try{
 browser=await pw.chromium.launch({...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{}),headless:true});
 const context=await browser.newContext({locale:'ko-KR',viewport:{width:900,height:640}});
 await context.addInitScript(()=>{
  window.ipc={postMessage:m=>(window.ipcEvents||=[]).push(JSON.parse(m))};
  let hidden=false;
  Object.defineProperty(Document.prototype,'hidden',{get:()=>hidden,configurable:true});
  window.setHidden=h=>{hidden=h;document.dispatchEvent(new Event('visibilitychange'));};
 });
 const page=await context.newPage();const errors=[];page.on('pageerror',e=>{errors.push(e.message);console.error('pageerror',e.message);});
 await page.goto(`http://127.0.0.1:${port}/?open=pacing-fixture`);
 await until('connected',()=>peers.length===1&&peers[0].requests>0);
 const p=peers[0];
 const during=async ms=>{const n=p.requests;await page.waitForTimeout(ms);return p.requests-n;};
 const visible=await during(1000);
 assert(visible>=20,`visible: updates keep coming (${visible}/s)`);
 await page.evaluate(()=>window.setHidden(true));
 await page.waitForTimeout(300); // the request already on the wire is answered
 const hidden=await during(3000);
 assert(hidden>=1&&hidden<=4,`hidden: about one request a second (${hidden} in 3 s)`);
 const gaps=p.times.slice(-hidden-1).map((t,i,a)=>i?t-a[i-1]:null).filter(Boolean);
 assert(gaps.every(g=>g>=900),`hidden: a second apart (${gaps.join(', ')} ms)`);
 const n=p.requests,shown=Date.now();
 await page.evaluate(()=>window.setHidden(false));
 await until('a request as soon as it is seen',()=>p.requests>n,300);
 const back=await during(1000);
 assert(back>=20,`visible again: full rate (${back}/s)`);
 assert.deepEqual(errors,[]);
 console.log(`PASS: hidden page paces a server without Continuous Updates: ${visible}/s visible, ${hidden} in 3 s hidden, next request ${p.times.find(t=>t>=shown)-shown} ms after being seen, ${back}/s again`);
}finally{
 await browser?.close();
 for(const s of sockets)s.destroy();
 server.close();
}
