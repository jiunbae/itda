// A browser viewer (no app) on Screen Sharing because the Mac was locked switches to
// high-performance once it's unlocked, as the app window does. Real viewer UI; RFB and
// WebSocket are fixtures.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const host={id:'imac',label:'iMac',host:'imac.local',os:'mac',kind:'vnc',hp:{on:true,native:false},has_pw:true};
let mode={hp:false,setup:false,locked:true,reason:'화면이 잠겨 있어 고성능 모드로 보여 줄 수 없습니다'},ready=false,readyAsked=0,modeAsked=0;
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 if(url.pathname.startsWith('/api/')){
  res.setHeader('Content-Type','application/json');
  if(url.pathname.startsWith('/api/hp-ready/')){readyAsked++;return res.end(JSON.stringify({ready}));}
  if(url.pathname.startsWith('/api/mode/')){modeAsked++;return res.end(JSON.stringify(mode));}
  return res.end(JSON.stringify(url.pathname==='/api/hosts'?[host]:url.pathname==='/api/accounts'?[]:{ok:true}));
 }
 if(url.pathname==='/novnc.min.js'){
  res.setHeader('Content-Type','text/javascript');
  return res.end(`window.RFB=class extends EventTarget {
   constructor(){super();this._fbWidth=640;this._fbHeight=480;this._rfbConnectionState='connected';this._display={_backbuffer:document.createElement('canvas'),itdaFrame:1};setTimeout(()=>this.dispatchEvent(new Event('connect')),0);}
   focus(){} itdaViewport(){} disconnect(){this._rfbConnectionState='disconnected';this.dispatchEvent(new Event('disconnect'));}
  };`);
 }
 const name=url.pathname==='/'?'index.html':url.pathname.slice(1),file=path.resolve(root,/^(i18n\.js|locales\/[\w.-]+\.js)$/.test(name)?'web':'web/viewer',name);
 if(!file.startsWith(path.join(root,'web')+path.sep)||!fs.existsSync(file)||!fs.statSync(file).isFile()){res.writeHead(404);return res.end();}
 res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(file));
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(v,m)=>{if(!v)throw Error(m);};
try{
 const p=await browser.newPage({locale:'ko-KR'}),errors=[];p.on('pageerror',e=>errors.push(e.message));
 await p.addInitScript(()=>{
  window.wsUrls=[];
  window.WebSocket=class extends EventTarget {constructor(url){super();this.url=url;wsUrls.push(url);}close(){}send(){}};
 });
 await p.clock.install();
 await p.goto(`http://127.0.0.1:${server.address().port}/`);
 assert(!(await p.evaluate(()=>!!window.ipc)),'fixture is not a browser viewer');
 await p.locator('.host[data-id=imac] .open').click();
 await p.waitForFunction(()=>wsUrls.length===1);
 for(let i=0;i<50&&!modeAsked;i++)await p.clock.runFor(100);
 assert(modeAsked===1,'never asked what this connection got');
 // Still locked: it keeps asking, without reconnecting.
 for(let i=0;i<50&&readyAsked<1;i++)await p.clock.runFor(100);
 assert(readyAsked>=1,'never asked whether the Mac was unlocked');
 assert(await p.evaluate(()=>wsUrls.length)===1,'reconnected while still locked');
 // Unlocked there: it switches to high-performance.
 ready=true;mode={hp:true};
 for(let i=0;i<100&&(await p.evaluate(()=>wsUrls.length))<2;i++)await p.clock.runFor(200);
 assert(await p.evaluate(()=>wsUrls.length)===2,'did not switch to high-performance after unlocking');
 assert(!(await p.evaluate(()=>wsUrls[1])).includes('nohp'),'switched back without high-performance');
 assert(!errors.length,'page errors: '+errors.join('; '));
 console.log('viewer-browser-unlock e2e: ok');
}finally{await browser.close();server.close();}
