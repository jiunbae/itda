// Actual viewer UI with deterministic transport/IPC fixtures: no remote host, native input
// or performance measurement. Session identity verifies diagnostics never reconnect.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const host={id:'studio',label:'Studio',host:'studio.local',os:'mac',kind:'vnc',hp:{on:true,native:false}};
let mode={hp:true},httpStatus=500,hpReady=false,readyRequests=0;
const requests=[];
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 if(url.pathname.startsWith('/api/')){
  res.setHeader('Content-Type','application/json');
  if(url.pathname.startsWith('/api/hp-ready/')){readyRequests++;return res.end(JSON.stringify({ready:hpReady}));}
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
  window.WebSocket=class extends EventTarget {constructor(url){super();this.url=url;window.fixtureWs=this;}close(){}send(){}};
 });
 await p.clock.install();
 await p.goto(`http://127.0.0.1:${server.address().port}/?open=studio`);
 const hp=p.locator('[data-t=hp]'),quality=p.locator('[data-t=hp-quality]');
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.title.includes('확인하지 못함'));
 assert(!await hp.evaluate(b=>b.classList.contains('on')),'HTTP500 claimed active mode');
 const session=await p.evaluate(()=>fixtureSessions),attempt=requests[0];
 assert(await p.locator('[data-t=hp-disable]').count()===1,'unknown mode lacks explicit disable action');
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
 mode=null;await p.goto(before);
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.title.includes('확인하지 못함'));
 await p.locator('[data-t=hp-disable]').click();
 assert(await p.evaluate(()=>ipcEvents.some(m=>m.ev==='hp-off')),'unknown mode cannot be disabled');
 mode={hp:true};
 await p.goto(before+'&hold=1');
 await hp.waitFor();
 assert((await hp.getAttribute('title')).includes('사용 설정됨'),'preconnect label claims unknown/active transport');
 await hp.click();
 assert(await p.evaluate(()=>ipcEvents.some(m=>m.ev==='hp-off')),'cannot disable saved HP before connect completes');
 await p.goto(before);
 await quality.waitFor();
 const dropped=await p.evaluate(()=>{fixtureRfb.dispatchEvent(new Event('disconnect'));const b=document.querySelector('[data-t=hp]'),label=b.title;b.click();return{label,off:ipcEvents.some(m=>m.ev==='hp-off')};});
 assert(dropped.label.includes('사용 설정됨')&&dropped.off,'disconnect left stale label or lost disable action');
 // The replacement that click opened connects; the dropped attempt's delayed callback must
 // then say nothing: no 'reconnecting' to the app, no reconnect overlay over the live session.
 const app=()=>p.evaluate(()=>({sessions:fixtureSessions,reconnecting:ipcEvents.filter(m=>m.ev==='reconnecting').length,errors:ipcEvents.filter(m=>m.ev==='error').length,status:document.querySelector('#status').hidden?'':document.querySelector('#status').textContent}));
 const raced=await app();
 await p.clock.runFor(200);
 {const now=await app();assert(now.reconnecting===raced.reconnecting&&now.errors===raced.errors&&!now.status,'a superseded drop reported over the live session: '+JSON.stringify(now));}
 await p.clock.runFor(12000);
 {const now=await app();assert(now.sessions===raced.sessions&&!now.status,'a superseded drop reopened: '+JSON.stringify(now));}
 // An ordinary drop still reconnects on its own.
 const base=await app();
 await p.evaluate(()=>fixtureRfb.dispatchEvent(new Event('disconnect')));
 await p.clock.runFor(100);
 {const now=await app();assert(now.reconnecting===base.reconnecting+1&&now.status.includes('다시 연결합니다'),'a drop did not start reconnecting: '+JSON.stringify(now));}
 await p.clock.runFor(2100);
 {const now=await app();assert(now.sessions===base.sessions+1&&!now.status,'the reconnect did not happen: '+JSON.stringify(now));}
 // The first drop has a retry scheduled, then a quality-change replacement connects.
 // Drop that replacement later: its own retry is due after the first timer. Counting
 // total sessions alone cannot distinguish which timer reopened the connection.
 await p.evaluate(()=>fixtureRfb.dispatchEvent(new Event('disconnect')));
 await p.clock.runFor(100);
 const pending=await app();
 await p.evaluate(()=>itdaHp({id:'studio',quality:false,ok:true}));
 await p.clock.runFor(100);
 const replaced=await app();
 assert(replaced.sessions===pending.sessions+1&&!replaced.status,'the replacement did not connect');
 await p.clock.runFor(700);
 await p.evaluate(()=>fixtureRfb.dispatchEvent(new Event('disconnect')));
 await p.clock.runFor(100);
 await p.clock.runFor(1200); // original retry due at +2050 ms; current retry at +2950 ms
 {const now=await app();assert(now.sessions===replaced.sessions,'a superseded retry opened before the current retry deadline: '+JSON.stringify(now));}
 await p.clock.runFor(900);
 {const now=await app();assert(now.sessions===replaced.sessions+1&&!now.status,'the current retry did not open once at its own deadline: '+JSON.stringify(now));}
 // Screen Sharing stood in for a locked Mac: ask until it's unlocked, then switch once.
 mode={hp:false,locked:true,reason:'화면이 잠겨 있어 고성능 모드로 보여 줄 수 없습니다'};hpReady=false;readyRequests=0;
 await p.goto(before);
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.title.includes('연결되지 않음'));
 assert(await p.evaluate(()=>[...document.querySelectorAll('.toast')].some(t=>t.textContent.includes('잠금을 풀면'))),'locked fallback did not say it will switch');
 const lockedSessions=await p.evaluate(()=>fixtureSessions);
 await p.clock.runFor(4100);await p.waitForTimeout(50);await p.clock.runFor(4100);await p.waitForTimeout(50);
 assert(readyRequests>=2&&await p.evaluate(()=>fixtureSessions)===lockedSessions,'still-locked checks reconnected or never asked: '+readyRequests);
 hpReady=true;mode={hp:true};
 await p.clock.runFor(4100);await p.waitForTimeout(100);
 assert(await p.evaluate(()=>fixtureSessions)===lockedSessions+1,'unlock did not switch to high-performance');
 // A press meant as "try again" landing just after the switch must not turn it off.
 await hp.click();
 assert(!await p.evaluate(()=>ipcEvents.some(m=>m.ev==='hp-off')),'a press during the unlock switch turned high-performance off');
 const asked=readyRequests;await p.clock.runFor(20000);await p.waitForTimeout(50);
 assert(readyRequests===asked,'kept asking after switching: '+(readyRequests-asked));
 await hp.click();
 assert(await p.evaluate(()=>ipcEvents.some(m=>m.ev==='hp-off')),'after the switch settled the button no longer turns high-performance off');
 // The Mac locks during high-performance sharing: the gateway ends it with LOCKED_CLOSE. The
 // viewer reconnects at once (no drop/retry countdown), lands on Screen Sharing with the locked
 // message, and switches back once unlocked.
 hpReady=false;mode={hp:true};
 await p.goto(before);await quality.waitFor();
 const lockDrop=()=>p.evaluate(()=>{fixtureWs.dispatchEvent(new CloseEvent('close',{code:4030,reason:'그 Mac의 화면이 잠겨 기본 화면 공유로 다시 연결합니다'}));fixtureRfb.dispatchEvent(new Event('disconnect'));});
 {
  const base=await app();
  mode={hp:false,locked:true,reason:'화면이 잠겨 있어 고성능 모드로 보여 줄 수 없습니다'};readyRequests=0;
  await lockDrop();
  await p.clock.runFor(100);await p.waitForTimeout(50);
  const now=await app();
  assert(now.sessions===base.sessions+1&&now.reconnecting===base.reconnecting&&now.errors===base.errors&&!now.status,'a lock during high-performance did not reconnect at once: '+JSON.stringify(now));
  assert(await p.evaluate(()=>[...document.querySelectorAll('.toast')].some(t=>t.textContent.includes('잠겨'))),'a lock during high-performance said nothing');
  await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.title.includes('연결되지 않음'));
  assert(await p.evaluate(()=>[...document.querySelectorAll('.toast')].some(t=>t.textContent.includes('잠금을 풀면'))),'the reconnect after a lock did not say it will switch back');
  const locked=await p.evaluate(()=>fixtureSessions);
  hpReady=true;mode={hp:true};
  await p.clock.runFor(4100);await p.waitForTimeout(100);
  assert(await p.evaluate(()=>fixtureSessions)===locked+1&&readyRequests>=1,'unlock after a lock during high-performance did not switch back');
  // A second lock close soon after is handled as an ordinary drop (no reconnect loop).
  const again=await app();
  await lockDrop();
  await p.clock.runFor(100);
  const dropped=await app();
  assert(dropped.reconnecting===again.reconnecting+1&&dropped.sessions===again.sessions,'a repeated lock close looped instead of the usual drop handling: '+JSON.stringify(dropped));
  await p.clock.runFor(2100);
 }
 // Another user takes the screen (fast user switching): the same at-once reconnect, saying so,
 // and the watch for it to be back keeps going past 15 minutes (every 30 s after that).
 hpReady=false;mode={hp:true};
 await p.clock.runFor(16000);
 await p.goto(before);await quality.waitFor();
 {
  const base=await app();
  mode={hp:false,locked:true,other_user:true,reason:'다른 사용자가 이 Mac 화면을 쓰고 있어 고성능 모드로 보여 줄 수 없습니다'};readyRequests=0;
  await p.evaluate(()=>{fixtureWs.dispatchEvent(new CloseEvent('close',{code:4030,reason:'다른 사용자가 그 Mac 화면을 쓰기 시작해 기본 화면 공유로 다시 연결합니다'}));fixtureRfb.dispatchEvent(new Event('disconnect'));});
  await p.clock.runFor(100);await p.waitForTimeout(50);
  const now=await app();
  assert(now.sessions===base.sessions+1&&now.reconnecting===base.reconnecting&&!now.status,'another user taking the screen did not reconnect at once: '+JSON.stringify(now));
  const toasts=()=>p.evaluate(()=>[...document.querySelectorAll('.toast')].map(t=>t.textContent).join('|'));
  await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.title.includes('연결되지 않음'));
  const said=await toasts();
  assert(said.includes('다른 사용자가')&&said.includes('화면이 돌아오면')&&!said.includes('잠금을 풀면'),'another user taking the screen was explained as a lock: '+said);
  const taken=await p.evaluate(()=>fixtureSessions);
  for(let i=0;i<230;i++){await p.clock.runFor(4100);await p.waitForTimeout(2);}
  const asked=readyRequests;
  assert(asked>=200,'did not keep asking for the first 15 minutes: '+asked);
  await p.clock.runFor(4100);await p.waitForTimeout(20);await p.clock.runFor(4100);await p.waitForTimeout(20);
  assert(readyRequests<=asked+1,'kept asking every 4 s after 15 minutes: '+(readyRequests-asked));
  hpReady=true;mode={hp:true};
  for(let i=0;i<3;i++){await p.clock.runFor(15000);await p.waitForTimeout(20);}
  assert(await p.evaluate(()=>fixtureSessions)===taken+1,'did not switch back once the screen came back after 15 minutes');
  assert((await toasts()).includes('화면이 돌아와'),'the switch back did not say why');
 }
 assert(errors.length===0,errors.join('\n'));
 console.log('PASS: HTTP500 and null mode unknown/retry; locked fallback switches once unlocked; a lock during high-performance reconnects through Screen Sharing and back; active capture and stale status guards; diagnostic retries preserve session, settings and per-attempt token');
}finally{await browser.close();await new Promise(r=>server.close(r));}
