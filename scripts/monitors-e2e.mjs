// Synthetic two-monitor RFB server: test actual bundled noVNC, popup views and input.
// No real remote host or credentials are used.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const assert=(ok,why)=>{if(!ok)throw Error(why);};
const sockets=new Set(), peers=[];let connections=0;
const host={id:'monitors-fixture',label:'Two monitors',name:'Two monitors',kind:'rdp',os:'windows',opts:{resize:true,monitor_mode:0,monitor_layout:0,clipboard:true}};
if(process.env.MAC_FIXTURE){host.kind='vnc';host.os='mac';}
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 if(url.pathname==='/fixture-stats'){res.setHeader('Content-Type','application/json');return res.end(JSON.stringify({connections,peers:peers.map(p=>({pointer:p.pointer,keys:p.keys,resizes:p.resizes,closed:!!p.closed}))}));}
 if(url.pathname==='/fixture-fragment'){const data=update([rect(0,0,640,480,0,pixels(180,70,130))]);for(const p of peers){let at=0;const send=()=>{const end=Math.min(data.length,at+640*4*17+13);p.send(data.subarray(at,end));at=end;if(at<data.length)setTimeout(send,0);};send();}return res.end('fragment');}
 if(url.pathname==='/fixture-burst'){const frames=Array.from({length:12},(_,i)=>update([rect(0,0,640,480,0,pixels(20,30+i*10,20)),rect(640,0,640,480,0,pixels(20,20,230))]));for(const p of peers)p.send(Buffer.concat(frames));return res.end('burst');}
 if(url.pathname==='/fixture-change'){for(const p of peers)p.send(update([rect(0,0,640,480,0,pixels(20,230,20))]));return res.end('changed');}
 if(url.pathname==='/api/me'){res.setHeader('Content-Type','application/json');return res.end(JSON.stringify({local:true,scroll:{mac:'fast'}}));}
 if(url.pathname==='/api/hosts'){res.setHeader('Content-Type','application/json');return res.end(JSON.stringify([host]));}
 if(url.pathname==='/api/edit'){res.setHeader('Content-Type','application/json');return res.end('{"ok":true}');}
 const name=url.pathname==='/'?'index.html':url.pathname.slice(1);
 if(!['index.html','viewer.js','viewer.css','monitors.js','interaction.js','novnc.min.js','icon.svg','icon-192.png','manifest.webmanifest','sw.js'].includes(name)){res.writeHead(404);return res.end();}
 let file=path.join(root,'web/viewer',name);if(name==='icon.svg')file=path.join(root,'assets/itda.svg');if(name==='icon-192.png')file=path.join(root,'assets/itda-192.png');
 res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':name.endsWith('.svg')?'image/svg+xml':'text/html');res.end(fs.readFileSync(file));
});
server.on('connection',s=>{sockets.add(s);s.on('close',()=>sockets.delete(s));});
function frame(bytes){let header;if(bytes.length<126)header=Buffer.from([0x82,bytes.length]);else if(bytes.length<65536){header=Buffer.alloc(4);header[0]=0x82;header[1]=126;header.writeUInt16BE(bytes.length,2);}else{header=Buffer.alloc(10);header[0]=0x82;header[1]=127;header.writeBigUInt64BE(BigInt(bytes.length),2);}return Buffer.concat([header,bytes]);}
const u16=n=>{const b=Buffer.alloc(2);b.writeUInt16BE(n);return b;};
const u32=n=>{const b=Buffer.alloc(4);b.writeUInt32BE(n>>>0);return b;};
function rect(x,y,w,h,encoding,data){return Buffer.concat([u16(x),u16(y),u16(w),u16(h),u32(encoding),data]);}
function update(rects){return Buffer.concat([Buffer.from([0,0]),u16(rects.length),...rects]);}
const topology=()=>rect(0,0,1280,480,-308,Buffer.concat([Buffer.from([2,0,0,0]),...[[101,0,0],[202,640,0]].map(([id,x,y])=>Buffer.concat([u32(id),u16(x),u16(y),u16(640),u16(480),u32(0)]))]));
function pixels(r,g,b){const data=Buffer.alloc(640*480*4);for(let i=0;i<data.length;i+=4){data[i]=r;data[i+1]=g;data[i+2]=b;}return data;}
server.on('upgrade',(req,socket)=>{
 connections++;const key=req.headers['sec-websocket-key'];const accept=createHash('sha1').update(key+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
 socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: '+accept+'\r\nSec-WebSocket-Protocol: binary\r\n\r\n');
 const peer={pointer:[],keys:[],resizes:0,send:data=>socket.write(frame(data))};peers.push(peer);peer.send(Buffer.from('RFB 003.008\n'));
 let wire=Buffer.alloc(0),input=Buffer.alloc(0),phase=0,sent=false;
 function consume(){
  while(true){
   if(phase===0){if(input.length<12)return;input=input.subarray(12);phase=1;peer.send(Buffer.from([1,1]));}
   else if(phase===1){if(input.length<1)return;input=input.subarray(1);phase=2;peer.send(u32(0));}
   else if(phase===2){if(input.length<1)return;input=input.subarray(1);phase=3;const name=Buffer.from('Two-monitor fixture');peer.send(Buffer.concat([u16(1280),u16(480),Buffer.from([32,24,0,1,0,255,0,255,0,255,16,8,0,0,0,0]),u32(name.length),name]));}
   else{
    if(!input.length)return;const type=input[0];let n=({0:20,3:10,4:8,5:6,150:10,248:9})[type];
    if(type===2){if(input.length<4)return;n=4+input.readUInt16BE(2)*4;}
    if(type===6){if(input.length<8)return;n=8+input.readUInt32BE(4);}
    if(type===251){if(input.length<8)return;n=8+input[6]*16;peer.resizes++;}
    if(!n)throw Error('Unexpected RFB client message '+type);
    if(input.length<n)return;const m=input.subarray(0,n);input=input.subarray(n);
    if(type===3&&!sent){sent=true;peer.send(update([topology(),rect(0,0,640,480,0,pixels(230,20,20)),rect(640,0,640,480,0,pixels(20,20,230))]));}
    if(type===5)peer.pointer.push({mask:m[1],x:m.readUInt16BE(2),y:m.readUInt16BE(4)});
    if(type===4)peer.keys.push({down:!!m[1],key:m.readUInt32BE(4)});
   }
  }
 }
 socket.on('data',chunk=>{
  wire=Buffer.concat([wire,chunk]);while(wire.length>=2){const op=wire[0]&15;let n=wire[1]&127,at=2;if(n===126){if(wire.length<4)return;n=wire.readUInt16BE(2);at=4;}else if(n===127){if(wire.length<10)return;n=Number(wire.readBigUInt64BE(2));at=10;}const masked=!!(wire[1]&128),start=at+(masked?4:0);if(wire.length<start+n)return;const data=Buffer.from(wire.subarray(start,start+n));if(masked)for(let i=0;i<n;i++)data[i]^=wire[at+i%4];wire=wire.subarray(start+n);if(op===8)return socket.end();if(op===9)continue;input=Buffer.concat([input,data]);consume();}
 });socket.on('error',()=>{});socket.on('close',()=>peer.closed=true);
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const port=server.address().port;
if(process.env.FIXTURE_ONLY){console.log(`Fixture: http://127.0.0.1:${port}/?open=monitors-fixture`);await new Promise(()=>{});}
let browser, page;
try{
 browser=await pw.chromium.launch({...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{}),headless:true});
 const context=await browser.newContext({viewport:{width:1000,height:720}});
 await context.addInitScript(()=>{const send=WebSocket.prototype.send;window.wirePackets=[];WebSocket.prototype.send=function(data){window.wirePackets.push(data.byteLength);return send.call(this,data);};let RFB;Object.defineProperty(window,'RFB',{get:()=>RFB,set:Base=>{RFB=class extends Base{constructor(...a){super(...a);window.fixtureRfb=this;}};}});window.ipc={postMessage:msg=>(window.ipcEvents||=[]).push(JSON.parse(msg))};});
 page=await context.newPage();const errors=[];context.on('page',p=>p.on('pageerror',e=>errors.push(e.message)));page.on('pageerror',e=>{errors.push(e.message);console.error('PAGE ERROR',e.message);});page.on('console',m=>{if(m.type()==='error')console.error('BROWSER',m.text());});
 await page.goto(`http://127.0.0.1:${port}/?open=monitors-fixture`);
 await page.waitForFunction(()=>window.ItdaMonitorOwner?.state().screens.length===2);
 await page.waitForFunction(()=>window.ItdaMonitorOwner?.state().canvas?.getContext('2d').getImageData(950,200,1,1).data[2]>200);
 const layout=await page.evaluate(async()=>{const {screenLayout}=await import('./monitors.js');return{manual:screenLayout(1281,801,[],4),invalid:screenLayout(100,100,[{id:1,x:-1,y:0,w:100,h:100}])};});
 assert(layout.manual.reduce((a,s)=>a+s.w*s.h,0)===1281*801,'Manual layout loses odd pixels');assert(layout.invalid[0].unknown,'Invalid server geometry was accepted');
 const canvas=page.locator('#screen canvas');
 await page.locator('[data-t="monitors"]').click();await page.locator('[data-monitor="202"]').click();
 await canvas.click({position:{x:200,y:150}});
 await page.waitForTimeout(80);assert(peers[0].pointer.some(p=>p.mask===1&&p.x>=640),'Selected monitor input lost framebuffer offset');
 await page.locator('[data-t="monitors"]').click();
 if(process.env.MAC_FIXTURE){const beforeKeys=peers[0].keys.length;await page.locator('[data-t=monitors]').click();await page.locator('[data-t=mac-actions]').click();await page.locator('[data-mac-action=expose]').click();await page.waitForTimeout(40);const keys=peers[0].keys.slice(beforeKeys);assert(JSON.stringify(keys)===JSON.stringify([{down:true,key:65507},{down:true,key:65364},{down:false,key:65364},{down:false,key:65507}]),'Exposé chord or modifier release incorrect');await page.locator('[data-t=monitors]').click();}
 const mainWheel=await page.evaluate(()=>{const c=document.querySelector('#screen canvas'),r=c.getBoundingClientRect(),before=wirePackets.length;c.dispatchEvent(new WheelEvent('wheel',{bubbles:true,cancelable:true,deltaY:2000,clientX:r.left+100,clientY:r.top+100}));return wirePackets.length-before;});
 await page.waitForTimeout(40);assert(mainWheel===1,'Wheel was not batched into one packet');assert(peers[0].pointer.slice(-80).every((p,i)=>p.mask===(i%2?0:16)&&p.x>=640),'Wheel click count, release or cropped coordinates incorrect');
 console.log('PASS: main wheel 80 pointer messages in one WebSocket packet (previously 80 packets)');
 const fractions=await page.evaluate(mac=>{const c=document.querySelector('#screen canvas'),r=c.getBoundingClientRect(),before=wirePackets.length,step=mac?10:50;for(const deltaY of [1,-1,step-1,1])c.dispatchEvent(new WheelEvent('wheel',{bubbles:true,cancelable:true,deltaY,clientX:r.left+100,clientY:r.top+100}));return wirePackets.length-before;},!!process.env.MAC_FIXTURE);assert(fractions===1,'Fractional wheel motion, reversal or capped-event remainder is incorrect');

 const next=context.waitForEvent('page');await page.locator('[data-monitor-mode="2"]').click();const child=await next;
 await child.locator('.monitor-canvas').waitFor();await child.waitForFunction(()=>document.querySelector('.monitor-canvas')?.getContext('2d').getImageData(50,50,1,1).data[0]>200);
 assert(connections===1,'Splitting windows opened another RFB session');
 await child.locator('.monitor-canvas').click();await child.keyboard.press('a');await child.waitForTimeout(80);
 assert(peers[0].pointer.some(p=>p.mask===1&&p.x<640),'Mirror input did not use its own monitor offset');
 assert(peers[0].keys.some(k=>k.key===97&&k.down),'Mirror keyboard was not forwarded');
 const mirrorWheel=await child.evaluate(()=>{const c=document.querySelector('.monitor-canvas'),r=c.getBoundingClientRect();c.dispatchEvent(new WheelEvent('wheel',{cancelable:true,deltaY:2000,clientX:r.left+50,clientY:r.top+50}));});await child.waitForTimeout(40);assert(peers[0].pointer.slice(-80).every((p,i)=>p.mask===(i%2?0:16)&&p.x<640),'Mirror wheel differs from owner or lost its monitor coordinates');
 if(process.env.MAC_FIXTURE){const beforeKeys=peers[0].keys.length;await child.locator('[data-mirror=mac-actions]').click();await child.locator('[data-mac-action=mission]').click();await child.waitForTimeout(40);assert(peers[0].keys.slice(beforeKeys).map(k=>k.key).join(',')==='65507,65362,65362,65507','Mirror Mission Control keys not forwarded');}
 const cdp=await context.newCDPSession(page);await cdp.send('Emulation.setCPUThrottlingRate',{rate:4});
 const timing=await page.evaluate(async()=>{const d=window.fixtureRfb._display,start=d.itdaFrame;let checkpoints=[];const at=performance.now();const timer=setInterval(()=>checkpoints.push(d.itdaFrame-start),0);await fetch('/fixture-burst');await new Promise((resolve,reject)=>{const poll=()=>{if(d.itdaFrame-start>=12)return resolve();if(performance.now()-at>5000)return reject(Error('Burst stalled'));setTimeout(poll,1);};poll();});clearInterval(timer);return {ms:Math.round(performance.now()-at),updates:d.itdaFrame-start,intermediate:checkpoints.filter(n=>n>0&&n<12)};});await cdp.send('Emulation.setCPUThrottlingRate',{rate:1});assert(timing.updates>=12,'Burst lost a completed update');console.log('Burst decode checkpoints',timing);
 await page.evaluate(()=>fetch('/fixture-fragment'));await child.waitForFunction(()=>{const d=document.querySelector('.monitor-canvas').getContext('2d').getImageData(50,50,1,1).data;return d[0]===180&&d[1]===70&&d[2]===130&&d[3]===255;});
 peers[0].send(update([rect(0,0,640,480,0,pixels(20,230,20))]));
 await child.waitForFunction(()=>document.querySelector('.monitor-canvas').getContext('2d').getImageData(50,50,1,1).data[1]>200);
 await child.locator('.monitor-canvas').focus();await child.keyboard.down('b');await child.waitForTimeout(40);
 await page.evaluate(()=>window.ItdaMonitorOwner.option('view_only',true));
 await child.waitForTimeout(80);assert(peers[0].keys.some(k=>k.key===98&&!k.down),'View-only left a mirror key held');
 const count=peers[0].pointer.length;await child.locator('.monitor-canvas').click();await child.waitForTimeout(80);assert(peers[0].pointer.length===count,'View-only mirror sent input');
 if(process.env.MAC_FIXTURE){const keys=peers[0].keys.length;await page.locator('[data-t=mac-actions]').click();await page.locator('[data-mac-action=desktop]').click();await page.waitForTimeout(40);assert(peers[0].keys.length===keys,'View-only sent a Mac action');}
 assert(peers[0].resizes===0,'Monitor view requested a destructive remote resize');
 if(process.env.SCREENSHOT_DIR){fs.mkdirSync(process.env.SCREENSHOT_DIR,{recursive:true});await page.screenshot({path:path.join(process.env.SCREENSHOT_DIR,'monitor-owner.png')});await child.screenshot({path:path.join(process.env.SCREENSHOT_DIR,'monitor-child.png')});}
 await page.evaluate(()=>window.ItdaMonitorOwner.mode(1));await child.waitForEvent('close');
 assert(connections===1,'Collecting windows reconnected the remote session');assert(errors.length===0,errors.join('\n'));
 console.log('PASS: two reported monitors, exact manual splits, cropped pointer offsets, one RFB session, sibling pixels/keyboard, update propagation, view-only release and no remote resize');
}catch(e){console.error(e);console.error('fixture',connections,peers.map(p=>({pointer:p.pointer,keys:p.keys,resizes:p.resizes,closed:!!p.closed})));if(page)console.error(await page.evaluate(()=>({body:document.body.innerText,owner:!!window.ItdaMonitorOwner,state:window.ItdaMonitorOwner?.state()?.screens,events:window.ipcEvents})).catch(()=>({})));process.exitCode=1;}finally{if(browser)await browser.close();for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));}
