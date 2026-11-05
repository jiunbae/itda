// High-performance video the browser can't decode: the viewer reconnects through Screen Sharing
// at once (?nohp=1) instead of a black screen, keeps the saved choice, and the mode button tries
// high-performance again. Real viewer UI; RFB, WebSocket and VideoDecoder are fixtures.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const host={id:'imac',label:'iMac',host:'imac.local',os:'mac',kind:'vnc',hp:{on:true,native:false}};
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 if(url.pathname.startsWith('/api/')){
  res.setHeader('Content-Type','application/json');
  if(url.pathname.startsWith('/api/mode/')){
   // What the gateway records for the attempt: Screen Sharing when the viewer asked for it.
   return res.end(JSON.stringify(globalThis.nextMode||{hp:true}));
  }
  return res.end(JSON.stringify(url.pathname==='/api/hosts'?[host]:url.pathname==='/api/accounts'?[]:{local:true,ok:true}));
 }
 if(url.pathname==='/novnc.min.js'){
  res.setHeader('Content-Type','text/javascript');
  return res.end(`window.RFB=class extends EventTarget {
   constructor(){super();window.fixtureSessions=(window.fixtureSessions||0)+1;window.fixtureRfb=this;this._fbWidth=640;this._fbHeight=480;this._rfbConnectionState='connected';this._display={_backbuffer:document.createElement('canvas'),itdaFrame:1};
    // noVNC's H.264 decoder: its error callback throws, as noVNC's does.
    this.decoder=new VideoDecoder({output(){},error(e){throw Error('Failed to decode frame: '+e.message);}});
    setTimeout(()=>this.dispatchEvent(new Event('connect')),0);}
   focus(){} itdaViewport(){} disconnect(){this._rfbConnectionState='disconnected';this.dispatchEvent(new Event('disconnect'));}
  };`);
 }
 const name=url.pathname==='/'?'index.html':url.pathname.slice(1),file=path.resolve(root,'web/viewer',name);
 if(!file.startsWith(path.join(root,'web/viewer')+path.sep)||!fs.existsSync(file)||!fs.statSync(file).isFile()){res.writeHead(404);return res.end();}
 res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(file));
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(v,m)=>{if(!v)throw Error(m);};
try{
 const p=await browser.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
 await p.addInitScript(()=>{
  window.ipcEvents=[];window.ipc={postMessage:m=>ipcEvents.push(JSON.parse(m))};
  window.wsUrls=[];
  window.WebSocket=class extends EventTarget {constructor(url){super();this.url=url;wsUrls.push(url);}close(){}send(){}};
  window.VideoDecoder=class {constructor(init){this.init=init;window.fixtureDecoder=this;}fail(){this.init.error(new DOMException('Decoder failure','EncodingError'));}};
 });
 await p.goto(`http://127.0.0.1:${server.address().port}/?open=imac`);
 const hp=p.locator('[data-t=hp]');
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.classList.contains('on'));
 assert(!(await p.evaluate(()=>wsUrls[0])).includes('nohp'),'first attempt skipped high-performance');
 // The decoder fails on this Mac's video.
 globalThis.nextMode={hp:false,setup:false,locked:false,reason:'이 기기에서 그 Mac의 고성능 영상을 표시하지 못했습니다'};
 await p.evaluate(()=>{try{fixtureDecoder.fail();}catch(_){}});
 await p.waitForFunction(()=>wsUrls.length===2);
 assert((await p.evaluate(()=>wsUrls[1])).includes('nohp=1'),'did not reconnect through Screen Sharing');
 assert(!(await p.evaluate(()=>ipcEvents.some(m=>m.ev==='hp-off'))),'turned the saved high-performance choice off');
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.title.includes('다시 시도'));
 // Another decoder error now (no H.264 on Screen Sharing, but whatever it is) doesn't loop.
 await p.evaluate(()=>{try{fixtureDecoder.fail();}catch(_){}});
 await p.waitForTimeout(300);
 assert(await p.evaluate(()=>wsUrls.length)===2,'a decoder error in Screen Sharing reconnected again');
 // The mode button tries high-performance again.
 globalThis.nextMode={hp:true};
 await hp.click();
 await p.waitForFunction(()=>wsUrls.length===3);
 assert(!(await p.evaluate(()=>wsUrls[2])).includes('nohp'),'retry still skipped high-performance');
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.classList.contains('on'));
 assert(!errors.length,'page errors: '+errors.join('; '));
 console.log('hp-undecodable e2e: ok');
}finally{await browser.close();server.close();}
