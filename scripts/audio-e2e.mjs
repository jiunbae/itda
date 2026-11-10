// Remote sound in the web viewer, against the real bundled noVNC and a synthetic RFB server that
// speaks QEMU Audio: -259 is listed only while sound is on, the format and enable follow the
// server's confirmation, PCM split across WebSocket messages plays with the right samples, the
// choice is saved per connection, and off sends disable. A fake AudioContext stands in for the
// speakers (and for the browser's wait for a click). No real host or credentials are used.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');

// The player's scheduling on its own.
const {schedule,decode,LEAD,MAX_AHEAD}=await import('data:text/javascript;base64,'+fs.readFileSync(path.join(root,'web/viewer/novnc-src/audio.js')).toString('base64'));
assert.equal(schedule(0,10),10+LEAD,'the first chunk starts after the jitter lead');
assert.equal(schedule(10.05,10),10.05,'the next chunk follows the last');
assert.equal(schedule(9.9,10),10+LEAD,'ran dry: buffer again');
assert.equal(schedule(10+MAX_AHEAD+0.01,10),null,'a backlog is dropped');
const odd=new Uint8Array(9);new DataView(odd.buffer).setInt16(1,16384,true);new DataView(odd.buffer).setInt16(3,-32768,true);
const l=new Float32Array(2),r=new Float32Array(2);decode(odd.subarray(1),l,r);
assert.deepEqual([l[0],r[0]],[0.5,-1],'S16LE at an odd offset');
console.log('PASS: player scheduling and decoding');

const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const host={id:'audio-fixture',label:'Sound',name:'Sound',kind:'vnc',os:'windows',opts:{clipboard:true}};
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
const pcm=(frames,l,r)=>{const b=Buffer.alloc(frames*4);for(let i=0;i<frames;i++){b.writeInt16LE(l,i*4);b.writeInt16LE(r,i*4+2);}return b;};
const audioData=d=>Buffer.concat([Buffer.from([255,1,0,2]),u32(d.length),d]);
server.on('upgrade',(req,socket)=>{
 const accept=createHash('sha1').update(req.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
 socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: '+accept+'\r\nSec-WebSocket-Protocol: binary\r\n\r\n');
 const peer={encodings:[],qemu:[],requests:0,confirmed:false,send:d=>socket.write(frame(d))};peers.push(peer);
 peer.send(Buffer.from('RFB 003.008\n'));
 let wire=Buffer.alloc(0),input=Buffer.alloc(0),phase=0;
 const consume=()=>{for(;;){
  if(phase===0){if(input.length<12)return;input=input.subarray(12);phase=1;peer.send(Buffer.from([1,1]));}
  else if(phase===1){if(input.length<1)return;input=input.subarray(1);phase=2;peer.send(u32(0));}
  else if(phase===2){if(input.length<1)return;input=input.subarray(1);phase=3;const name=Buffer.from('Sound fixture');peer.send(Buffer.concat([u16(64),u16(48),Buffer.from([32,24,0,1,0,255,0,255,0,255,16,8,0,0,0,0]),u32(name.length),name]));}
  else{
   if(!input.length)return;const t=input[0];let n=({0:20,3:10,4:8,5:6,150:10,248:9})[t];
   if(t===2){if(input.length<4)return;n=4+input.readUInt16BE(2)*4;}
   if(t===6){if(input.length<8)return;n=8+Math.abs(input.readInt32BE(4));}
   if(t===255){if(input.length<4)return;n=input[1]===1&&input.readUInt16BE(2)===2?10:4;}
   if(!n)throw Error('Unexpected RFB client message '+t);
   if(input.length<n)return;const m=input.subarray(0,n);input=input.subarray(n);
   if(t===2){const e=[];for(let i=4;i<n;i+=4)e.push(m.readInt32BE(i));peer.encodings.push(e);
    // Like Itda's server: confirm -259 in the next update.
    if(e.includes(-259)&&!peer.confirmed){peer.confirmed=true;peer.send(update([rect(0,0,0,0,-259)]));}}
   if(t===3&&!peer.requests++)peer.send(update([rect(0,0,64,48,0,solid(64,48,40))]));
   if(t===255)peer.qemu.push([...m]);
  }
 }};
 socket.on('data',chunk=>{wire=Buffer.concat([wire,chunk]);while(wire.length>=2){const op=wire[0]&15;let n=wire[1]&127,at=2;if(n===126){if(wire.length<4)return;n=wire.readUInt16BE(2);at=4;}else if(n===127){if(wire.length<10)return;n=Number(wire.readBigUInt64BE(2));at=10;}const masked=!!(wire[1]&128),start=at+(masked?4:0);if(wire.length<start+n)return;const d=Buffer.from(wire.subarray(start,start+n));if(masked)for(let i=0;i<n;i++)d[i]^=wire[at+i%4];wire=wire.subarray(start+n);if(op===8)return socket.end();input=Buffer.concat([input,d]);consume();}});
 socket.on('error',()=>{});
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const port=server.address().port;
const until=async(what,ok,ms=3000)=>{const end=Date.now()+ms;while(!ok()){if(Date.now()>end)throw Error('timed out: '+what);await new Promise(r=>setTimeout(r,10));}};
const FORMAT=[255,1,0,2,3,2,0,0,0xbb,0x80],ENABLE=[255,1,0,0],DISABLE=[255,1,0,1];
let browser;
try{
 browser=await pw.chromium.launch({...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{}),headless:true});
 const context=await browser.newContext({locale:'ko-KR',viewport:{width:900,height:640}});
 // Speakers: what was scheduled, and when. Like a browser's, it resumes only in a click or key.
 await context.addInitScript(()=>{
  window.played=[];window.contexts=0;
  // The Mac app's window (?open=): options are saved through the app.
  window.ipc={postMessage:m=>(window.ipcEvents||=[]).push(JSON.parse(m))};
  window.AudioContext=class{constructor(){window.contexts++;this.state='suspended';this.currentTime=1;this.destination={};window.lastAudio=this;}
   resume(){if(navigator.userActivation.isActive)this.state='running';return Promise.resolve();}close(){this.state='closed';return Promise.resolve();}
   createBuffer(ch,frames,rate){const d=[new Float32Array(frames),new Float32Array(frames)];return{frames,rate,getChannelData:i=>d[i],d};}
   createBufferSource(){const s={connect(){},start(at){window.played.push({at,frames:s.buffer.frames,l:s.buffer.d[0][0],r:s.buffer.d[1][0]});}};return s;}};
 });
 const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto(`http://127.0.0.1:${port}/?open=audio-fixture`);
 await until('connected',()=>peers.length===1&&peers[0].requests>0);
 await page.waitForSelector('[data-t="audio"]');
 const p=peers[0];
 assert(p.encodings.length>=1&&!p.encodings.some(e=>e.includes(-259)),'-259 listed while sound is off');
 assert.equal(await page.getAttribute('[data-t="audio"]','aria-pressed'),'false');

 await page.click('[data-t="audio"]');
 await until('-259 listed',()=>p.encodings.some(e=>e.includes(-259)));
 await until('format and enable',()=>p.qemu.length>=2);
 assert.deepEqual(p.qemu.slice(0,2),[FORMAT,ENABLE],'format, then enable, after the confirmation');
 assert.equal(await page.getAttribute('[data-t="audio"]','aria-pressed'),'true');
 await page.waitForFunction(()=>window.ipcEvents?.some(e=>e.ev==='opts'&&e.opts.audio===true));

 // Begin, then 480 frames split mid-sample across three messages, then another chunk and a
 // screen update that must still be drawn after the sound.
 const chunk=Buffer.concat([Buffer.from([255,1,0,1]),audioData(pcm(480,16384,-16384))]);
 p.send(chunk.subarray(0,7));p.send(chunk.subarray(7,15));p.send(chunk.subarray(15));
 p.send(Buffer.concat([audioData(pcm(240,8192,8192)),update([rect(0,0,64,48,0,solid(64,48,220))])]));
 await page.waitForFunction(()=>window.played.length===2);
 await page.waitForFunction(()=>document.querySelector('#screen canvas')?.getContext('2d').getImageData(5,5,1,1).data[1]>200);
 const played=await page.evaluate(()=>window.played);
 assert.deepEqual(played.map(x=>[x.frames,x.l,x.r]),[[480,0.5,-0.5],[240,0.25,0.25]],'samples decoded');
 assert(Math.abs(played[1].at-(played[0].at+0.01))<1e-9,'the second chunk follows the first');

 await page.click('[data-t="audio"]');
 await until('disable',()=>p.qemu.length===3);
 assert.deepEqual(p.qemu[2],DISABLE);
 p.send(audioData(pcm(480,1000,1000)));await page.waitForTimeout(100);
 assert.equal(await page.evaluate(()=>window.played.length),2,'nothing plays once off');
 assert.equal(await page.evaluate(()=>window.ipcEvents.filter(e=>e.ev==='opts').at(-1).opts.audio),false,'saved off');

 // Remembered on: listed from the first SetEncodings; the sound waits for the first click.
 host.opts.audio=true;
 await page.reload();
 await until('reconnected',()=>peers.length===2&&peers[1].requests>0);
 const q=peers[1];
 assert(q.encodings[0].includes(-259),'remembered sound is asked for from the start');
 await until('enable',()=>q.qemu.length>=2);
 assert.deepEqual(q.qemu.slice(0,2),[FORMAT,ENABLE]);
 q.send(audioData(pcm(480,16384,16384)));await page.waitForTimeout(100);
 assert.equal(await page.evaluate(()=>window.played.length),0,'no sound before a click (the browser holds it)');
 await page.mouse.click(450,400);
 q.send(audioData(pcm(480,16384,16384)));
 await page.waitForFunction(()=>window.played.length===1);
 assert.deepEqual(errors,[]);
 console.log('PASS: sound off by default, -259 only when on, format+enable after confirmation, fragmented PCM played in order, saved per connection, disable, remembered choice waits for a click');
}catch(e){console.error(e);console.error(peers.map(p=>({encodings:p.encodings.map(e=>e.includes(-259)),qemu:p.qemu})));process.exitCode=1;}
finally{if(browser)await browser.close();for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));}
