// Quitting (or updating) while transfers run, on the real main-window frontend with an
// in-memory IPC fixture: the dialog's choices and wording, the waiting banner that follows
// the count and can be cancelled. The decision itself is home.rs (Rust tests).
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/quit-guard-e2e.mjs
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const server=http.createServer((req,res)=>{
  const name=new URL(req.url,'http://localhost').pathname;
  const file=path.resolve(root,'.'+(name==='/web/'?'/web/index.html':name));
  if(!file.startsWith(root+path.sep)||!fs.existsSync(file)){res.writeHead(404);return res.end();}
  res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':name.endsWith('.svg')?'image/svg+xml':'text/html');res.end(fs.readFileSync(file));
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}`;
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(value,msg)=>{if(!value)throw Error(msg);};
try{
  const c=await browser.newContext({viewport:{width:1200,height:800},reducedMotion:'reduce'});
  await c.addInitScript(()=>{window.testPackets=[];window.ipc={postMessage:raw=>testPackets.push(JSON.parse(raw))};});
  const p=await c.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
  await p.goto(base+'/web/');
  await p.evaluate(()=>{S.hosts=[];S.settings={...S.settings,onboarded:true,theme:'light'};render();});
  const recv=m=>p.evaluate(m=>itda.recv(m),m);
  const last=()=>p.evaluate(()=>testPackets.filter(x=>x.cmd==='quit_choice').at(-1));
  // Quit with two transfers running: asked, waiting is the default and nothing is sent until answered.
  await recv({type:'quit_guard',reason:'quit',count:2,waiting:false});
  const dialog=p.getByRole('dialog',{name:'전송 2개가 진행 중입니다',exact:true});
  await dialog.waitFor();
  assert((await dialog.textContent()).includes('원본은 그대로 남고'),'The dialog does not say what stopping does');
  await p.waitForFunction(()=>document.activeElement?.dataset.x==='wait');
  assert(!await last(),'A choice was sent before the person answered');
  await p.mouse.click(5,5);
  assert(await dialog.isVisible(),'Clicking outside dismissed the quit question');
  await dialog.getByRole('button',{name:'전송이 끝나면 종료',exact:true}).click();
  {const t=await last();assert(t.choice==='wait'&&t.reason==='quit','Wait packet: '+JSON.stringify(t));}
  // Waiting: a banner follows the count and can cancel the quit.
  await recv({type:'quit_pending',reason:'quit',count:2});
  const bar=p.locator('#quit-pending');
  assert((await bar.getByRole('status').textContent()).includes('2개 남음'),'Banner count');
  await recv({type:'quit_pending',reason:'quit',count:1});
  assert((await bar.getByRole('status').textContent()).includes('1개 남음'),'Banner did not follow the count');
  await bar.getByRole('button',{name:'종료 취소'}).click();
  {const t=await last();assert(t.choice==='cancel','Cancel packet: '+JSON.stringify(t));}
  await recv({type:'quit_pending',count:0,cancelled:true});
  await bar.waitFor({state:'detached'});
  await p.locator('#toasts .toast').filter({hasText:'종료를 취소했습니다'}).waitFor();
  // An update: its own wording, and stopping is explicit.
  await recv({type:'quit_guard',reason:'update',count:1,waiting:false});
  const upd=p.getByRole('dialog',{name:'전송 1개가 진행 중입니다',exact:true});
  assert((await upd.textContent()).includes('업데이트하려면 Itda를 다시 시작합니다'),'Update wording');
  await upd.getByRole('button',{name:'중단하고 업데이트',exact:true}).click();
  {const t=await last();assert(t.choice==='stop'&&t.reason==='update','Stop packet: '+JSON.stringify(t));}
  // Asked again while already waiting: says so, one dialog.
  await recv({type:'quit_guard',reason:'quit',count:3,waiting:true});
  await recv({type:'quit_guard',reason:'quit',count:3,waiting:true});
  assert(await p.locator('#modals .scrim[data-key="quit-guard"]').count()===1,'Two quit dialogs');
  assert((await p.getByRole('dialog',{name:'전송 3개가 진행 중입니다'}).textContent()).includes('이미 전송이 끝나면'),'Pending wait not mentioned');
  await p.getByRole('button',{name:'취소',exact:true}).click();
  {const t=await last();assert(t.choice==='cancel'&&t.reason==='quit','Dialog cancel: '+JSON.stringify(t));}
  // A stop that didn't finish: Itda stays open; waiting again is the default, forcing says
  // what it leaves, and an update can't be forced.
  await recv({type:'quit_guard',reason:'quit',count:1,stuck:true});
  const stuck=p.getByRole('dialog',{name:'전송 1개가 아직 멈추지 않았습니다',exact:true});
  assert((await stuck.textContent()).includes('임시 파일'),'Force does not say what it leaves');
  await p.waitForFunction(()=>document.activeElement?.dataset.x==='stop');
  await stuck.getByRole('button',{name:'강제로 종료',exact:true}).click();
  {const t=await last();assert(t.choice==='force'&&t.reason==='quit','Force packet: '+JSON.stringify(t));}
  await recv({type:'quit_guard',reason:'update',count:2,stuck:true});
  const stuckUpdate=p.getByRole('dialog',{name:'전송 2개가 아직 멈추지 않았습니다',exact:true});
  assert(!await stuckUpdate.getByRole('button',{name:'강제로 종료'}).count(),'An update can be forced');
  assert((await stuckUpdate.textContent()).includes('업데이트를 설치하지 않습니다'),'Update stuck wording');
  await stuckUpdate.getByRole('button',{name:'다시 기다리기',exact:true}).click();
  {const t=await last();assert(t.choice==='stop'&&t.reason==='update','Retry packet: '+JSON.stringify(t));}
  assert(!errors.length,errors.join('\n'));
  console.log('PASS: quit guard dialog (wait/stop/cancel, update wording), waiting banner and cancellation');
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
