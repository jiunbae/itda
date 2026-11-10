// Relative pointer and the "nobody is looking" hint in the web viewer, against the real bundled
// noVNC and a synthetic RFB server. The server lists nothing new until it announces relative
// support with Itda's mark; a foreign Pointer Motion Change is ignored; a hidden remote pointer
// locks on the next click; movement goes as 0xEA deltas with no absolute moves meanwhile; the
// click that locked leaves no button held after unlocking; a hidden page asks for 'ITDB'.
// Pointer lock and page visibility are faked (headless Chromium has neither). No real host.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const host={id:'pointer-fixture',label:'Game',name:'Game',kind:'vnc',os:'windows',opts:{clipboard:true}};
const BACKGROUND=0x49544442;
const peers=[],sockets=new Set();
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 const json=v=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify(v));};
 if(url.pathname==='/api/me')return json({local:true});
 if(url.pathname==='/api/hosts')return json([host]);
 const name=url.pathname==='/'?'index.html':url.pathname.slice(1);
 if(!['index.html','viewer.js','viewer.css','monitors.js','interaction.js','novnc.min.js','manifest.webmanifest'].includes(name)){res.writeHead(404);return res.end();}
 res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(path.join(root,'web/viewer',name)));
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
 const peer={encodings:[],pointer:[],relative:[],requests:0,send:d=>socket.write(frame(d))};peers.push(peer);
 peer.send(Buffer.from('RFB 003.008\n'));
 let wire=Buffer.alloc(0),input=Buffer.alloc(0),phase=0;
 const consume=()=>{for(;;){
  if(phase===0){if(input.length<12)return;input=input.subarray(12);phase=1;peer.send(Buffer.from([1,1]));}
  else if(phase===1){if(input.length<1)return;input=input.subarray(1);phase=2;peer.send(u32(0));}
  else if(phase===2){if(input.length<1)return;input=input.subarray(1);phase=3;const name=Buffer.from('Pointer fixture');peer.send(Buffer.concat([u16(64),u16(48),Buffer.from([32,24,0,1,0,255,0,255,0,255,16,8,0,0,0,0]),u32(name.length),name]));}
  else{
   if(!input.length)return;const t=input[0];let n=({0:20,3:10,4:8,5:6,150:10,248:9,0xea:6})[t];
   if(t===2){if(input.length<4)return;n=4+input.readUInt16BE(2)*4;}
   if(t===6){if(input.length<8)return;n=8+Math.abs(input.readInt32BE(4));}
   if(!n)throw Error('Unexpected RFB client message '+t);
   if(input.length<n)return;const m=input.subarray(0,n);input=input.subarray(n);
   if(t===2){const e=[];for(let i=4;i<n;i+=4)e.push(m.readInt32BE(i));peer.encodings.push(e);}
   if(t===3&&!peer.requests++)peer.send(update([rect(0,0,64,48,0,solid(64,48,40))]));
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
// Pointer Motion Change: x 0 = the remote pointer is hidden, y = Itda's mark (or not).
const motion=(hidden,mark=0x4954)=>update([rect(hidden?0:1,mark,0,0,-257)]);
let browser;
try{
 browser=await pw.chromium.launch({...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{}),headless:true});
 const context=await browser.newContext({viewport:{width:900,height:640}});
 await context.addInitScript(()=>{
  // The Mac app's window (?open=): options go through the app.
  window.ipc={postMessage:m=>(window.ipcEvents||=[]).push(JSON.parse(m))};
  let locked=null,hidden=false;
  const fire=t=>setTimeout(()=>document.dispatchEvent(new Event(t)));
  Object.defineProperty(Document.prototype,'pointerLockElement',{get:()=>locked,configurable:true});
  // Asynchronous like a browser's: the locking click's own events come before it.
  Element.prototype.requestPointerLock=function(){window.lockRequests=(window.lockRequests||0)+1;return new Promise(r=>setTimeout(()=>{locked=this;document.dispatchEvent(new Event('pointerlockchange'));r();},30));};
  Document.prototype.exitPointerLock=function(){locked=null;fire('pointerlockchange');};
  Object.defineProperty(Document.prototype,'hidden',{get:()=>hidden,configurable:true});
  window.setHidden=h=>{hidden=h;document.dispatchEvent(new Event('visibilitychange'));};
 });
 const page=await context.newPage();const errors=[];page.on('pageerror',e=>{errors.push(e.message);console.error('pageerror',e.message);});page.on('console',m=>process.env.DEBUG&&console.error('console',m.text()));
 await page.goto(`http://127.0.0.1:${port}/?open=pointer-fixture`);
 await until('connected',()=>peers.length===1&&peers[0].requests>0);
 const p=peers[0];
 await page.waitForFunction(()=>document.querySelector('#screen canvas')?.getContext('2d').getImageData(5,5,1,1).data[1]===40);
 assert(p.encodings[0].includes(-257),'Pointer Motion Change is asked for');
 assert(!p.encodings[0].includes(BACKGROUND),'a visible page is not in the background');
 assert.equal(await page.$('[data-t="mouse-lock"]'),null,'no lock button before the server says it can');

 // A foreign server's Pointer Motion Change (QEMU: no mark) changes nothing.
 p.send(motion(true,0));await page.waitForTimeout(150);
 assert.equal(await page.$('[data-t="mouse-lock"]'),null,'foreign announcement ignored');

 // Itda's announcement: the button appears; then the remote pointer hides.
 p.send(motion(false));
 await page.waitForSelector('[data-t="mouse-lock"]');
 p.send(motion(true));
 await page.waitForFunction(()=>[...document.querySelectorAll('.toast,#toast')].some(t=>/마우스를 숨겼습니다/.test(t.textContent)));
 const box=await page.$eval('#screen canvas',c=>{const r=c.getBoundingClientRect();return{x:r.x+r.width/2,y:r.y+r.height/2};});
 await page.mouse.move(box.x,box.y);
 const absBefore=p.pointer.length;
 await page.mouse.down();
 await page.waitForFunction(()=>window.lockRequests===1&&document.pointerLockElement);
 await page.mouse.up(); // (after the lock: its release goes as a relative message)
 await until('button released as a relative message',()=>p.relative.some(r=>r[0]===0));
 // The locking press may go as an absolute event (a real lock is asynchronous) or be swallowed
 // (this fake one is immediate); either way nothing absolute follows while locked.
 const absLocked=p.pointer.length;
 assert(absLocked-absBefore<=1,'at most the locking press went absolute');
 // Movement while locked: a real move sends no absolute PointerEvent; synthetic movement
 // arrives as deltas, summed and batched per frame.
 await page.mouse.move(box.x+40,box.y+20);
 await page.waitForTimeout(100);
 const total=()=>p.relative.reduce((s,r)=>[s[0]+r[1],s[1]+r[2]],[0,0]);
 const [bx,by]=total(),count=p.relative.length;
 await page.evaluate(()=>{for(let i=0;i<5;i++)document.dispatchEvent(new MouseEvent('mousemove',{movementX:3,movementY:-2,bubbles:true}));});
 await until('deltas',()=>total()[0]-bx>=15);
 await page.waitForTimeout(100);
 assert.deepEqual([total()[0]-bx,total()[1]-by],[15,-10],'movement arrives summed');
 assert(p.relative.length-count<=2,`batched per frame (${p.relative.length-count} messages for 5 moves)`);
 assert.equal(p.pointer.length,absLocked,'no absolute events while locked');
 await page.mouse.wheel(0,100);
 await until('wheel',()=>p.relative.some(r=>r[0]===16));
 await page.waitForTimeout(100);
 assert.equal(p.pointer.length,absLocked,'the wheel goes once, relatively (not again through noVNC)');
 // Esc (exitPointerLock here): the next absolute move must not carry the locking click.
 await page.evaluate(()=>document.exitPointerLock());
 await page.waitForFunction(()=>!document.pointerLockElement);
 await page.mouse.move(box.x-30,box.y-10);
 await until('absolute moves again',()=>p.pointer.length>absLocked);
 assert.deepEqual(p.pointer.slice(absLocked).filter(m=>m&1),[],'no button held after unlocking');

 // Hidden page: 'ITDB' is listed; visible again: gone.
 const n=p.encodings.length;
 await page.evaluate(()=>window.setHidden(true));
 await until('background',()=>p.encodings.length>n&&p.encodings.at(-1).includes(BACKGROUND));
 await page.evaluate(()=>window.setHidden(false));
 await until('foreground',()=>!p.encodings.at(-1).includes(BACKGROUND));
 assert.deepEqual(errors,[]);
 console.log('PASS: relative pointer only after Itda\'s announcement, lock on click while hidden, deltas batched with no absolute moves, no held button after unlock, wheel sent once, background hint follows visibility');
}catch(e){console.error(e);console.error(peers.map(p=>({encodings:p.encodings.length,pointer:p.pointer,relative:p.relative})));process.exitCode=1;}
finally{if(browser)await browser.close();for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));}
