// Actual viewer UI with deterministic transport/IPC fixtures: no remote host, native input
// or performance measurement. Session identity verifies diagnostics never reconnect.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const host={id:'studio',label:'Studio',host:'studio.local',os:'mac',kind:'vnc',hp:{on:true,native:false}};
let mode={hp:true},httpStatus=500;
const requests=[];
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 if(url.pathname.startsWith('/api/')){
  res.setHeader('Content-Type','application/json');
  if(url.pathname.startsWith('/api/mode/')){requests.push(url.searchParams.get('attempt'));res.writeHead(httpStatus);return res.end(JSON.stringify(mode));}
  return res.end(JSON.stringify(url.pathname==='/api/hosts'?[host]:url.pathname==='/api/accounts'?[]:{local:true,ok:true}));
 }
 if(url.pathname==='/novnc.min.js'){
  res.setHeader('Content-Type','text/javascript');
  return res.end(`window.RFB=class extends EventTarget {
   constructor(){super();window.fixtureSessions=(window.fixtureSessions||0)+1;window.fixtureRfb=this;this._fbWidth=640;this._fbHeight=480;this._rfbConnectionState='connected';this._display={_backbuffer:document.createElement('canvas'),itdaFrame:1};if(!new URLSearchParams(location.search).has('hold'))setTimeout(()=>this.dispatchEvent(new Event('connect')),0);}
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
  window.WebSocket=class extends EventTarget {constructor(url){super();this.url=url;}close(){}send(){}};
 });
 await p.clock.install();
 await p.goto(`http://127.0.0.1:${server.address().port}/?open=studio`);
 const hp=p.locator('[data-t=hp]'),quality=p.locator('[data-t=hp-quality]');
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.title.includes('확인하지 못함'));
 assert(!await hp.evaluate(b=>b.classList.contains('on')),'HTTP500 claimed active mode');
 const session=await p.evaluate(()=>fixtureSessions),attempt=requests[0];
 httpStatus=200;mode=null;await hp.click();
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.title.includes('확인하지 못함'));
 mode={hp:true};await hp.click();await quality.waitFor();
 assert((await quality.getAttribute('title')).includes('확인 중'),'unknown capture mislabeled as fast/native');
 await p.clock.fastForward(13010);
 assert((await quality.getAttribute('title')).includes('확인하지 못함'),'missing IPC response never leaves checking state');
 await quality.click();
 let request=await p.evaluate(()=>ipcEvents.filter(m=>m.ev==='hp-status').at(-1).req);
 const sendStatus=(req,extra)=>p.evaluate(({req,extra})=>itdaHp({id:'studio',req,status:true,quality:'points',quality_active:true,capture_width:1440,capture_height:900,...extra}),{req,extra});
 await sendStatus('obsolete',{quality:'native'});
 assert((await quality.getAttribute('title')).includes('확인 중'),'stale status changed quality');
 for(let i=0;i<5;i++)await sendStatus(request,{quality_active:false,capture_width:0,capture_height:0});
 assert((await quality.getAttribute('title')).includes('확인하지 못함'),'exhausted capture status lacks recovery');
 await quality.click();
 const pendingCount=await p.evaluate(()=>ipcEvents.filter(m=>m.ev==='hp-status').length);
 await quality.click();
 assert(await p.evaluate(()=>ipcEvents.filter(m=>m.ev==='hp-status').length)===pendingCount,'checking clicks fan out status requests');
 const retry=await p.evaluate(()=>ipcEvents.filter(m=>m.ev==='hp-status').at(-1).req);
 assert(retry!==request,'quality retry did not invalidate old status replies');
 await sendStatus(request,{quality:'native'});
 assert((await quality.getAttribute('title')).includes('확인 중'),'old retry response was accepted');
 await p.evaluate(req=>itdaHp({id:'studio',req,status:true,error:'fixture SSH unavailable'}),retry);
 assert((await quality.getAttribute('title')).includes('확인하지 못함'),'SSH error did not immediately offer retry');
 await quality.click();
 const recovered=await p.evaluate(()=>ipcEvents.filter(m=>m.ev==='hp-status').at(-1).req);
 await sendStatus(recovered,{quality:'native',capture_width:2880,capture_height:1800});
 assert(await quality.evaluate(b=>b.classList.contains('on')),'verified native capture not active');
 assert(await p.evaluate(()=>fixtureSessions)===session&&requests.every(r=>r===attempt),'diagnostic retries reconnected session or changed attempt token');
 assert(!await p.evaluate(()=>ipcEvents.some(m=>m.ev==='hp-quality'||m.ev==='hp-off')),'unknown status retry modified saved settings');
 // Reconnect creates a new attempt; a delayed answer from the old one stays ignored.
 await hp.click();await p.waitForFunction(()=>fixtureSessions===2);
 await sendStatus(retry,{quality:'native'});
 assert(await quality.count()===0,'old connection status restored Retina control');
 const before=p.url();
 await p.goto(before+'&hold=1');
 await hp.waitFor();
 assert((await hp.getAttribute('title')).includes('사용 설정됨'),'preconnect label claims unknown/active transport');
 await hp.click();
 assert(await p.evaluate(()=>ipcEvents.some(m=>m.ev==='hp-off')),'cannot disable saved HP before connect completes');
 await p.goto(before);
 await quality.waitFor();
 const dropped=await p.evaluate(()=>{fixtureRfb.dispatchEvent(new Event('disconnect'));const b=document.querySelector('[data-t=hp]'),label=b.title;b.click();return{label,off:ipcEvents.some(m=>m.ev==='hp-off')};});
 assert(dropped.label.includes('사용 설정됨')&&dropped.off,'disconnect left stale label or lost disable action');
 assert(errors.length===0,errors.join('\n'));
 console.log('PASS: HTTP500 and null mode unknown/retry; active capture and stale status guards; diagnostic retries preserve session, settings and per-attempt token');
}finally{await browser.close();await new Promise(r=>server.close(r));}
