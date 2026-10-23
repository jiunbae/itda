// Transfers that keep running after their file window closed, on the real frontend with an
// in-memory IPC fixture (no SSH, no user files): the main window lists them, follows their
// progress and outcome, cancels and dismisses them as the app's; a file window that opens
// later picks them up from the state snapshot and offers the retry for what is left.
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/files-background-e2e.mjs
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
const hosts=[{id:'studio',name:'Studio Mac',host:'studio.local',os:'mac',kind:'vnc',login_user:'alex',has_pw:true,files:{method:'sftp',user:'alex'}}];
// What the backend sends for a transfer whose window closed (src/files/background.rs).
const request={id:'__network',from:'local',to:'local',source:'C:\\Users\\alex',destination:'D:\\Backup',names:['Photos','report.txt'],count:2,move:true,policy:'keep',engine:'auto',from_id:null};
let seq=10;
const app=(m)=>({type:'files_job',window:'app',orphan:true,request,seq:++seq,...m});
try{
  const c=await browser.newContext({viewport:{width:1200,height:800},reducedMotion:'reduce',userAgent:'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
  await c.addInitScript(()=>{
    const entry=(name,dir=false)=>({name,dir,size:100,modified:1780000000});
    const tree={'C:\\Users\\alex':[entry('Photos',true),entry('report.txt')],'D:\\Backup':[],'':[entry('C:',true),entry('D:',true)]};
    window.testPackets=[];
    window.ipc={postMessage:raw=>{const m=JSON.parse(raw);testPackets.push(m);if(m.cmd!=='files')return;
      const r={...m,type:'files_result',ok:true};
      if(m.op==='local_home')r.data={path:'C:\\Users\\alex',entries:tree['C:\\Users\\alex']};
      else if(m.op==='list')r.data={path:m.path,entries:tree[m.path]||[]};
      else return;
      setTimeout(()=>itda.recv(r),10);
    }};
  });
  const errors=[];
  const icons=async p=>{if(process.platform!=='win32')await p.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));};

  // ---- the main window
  const main=await c.newPage();main.on('pageerror',e=>errors.push(e.message));
  await icons(main);await main.goto(base+'/web/');
  await main.evaluate(h=>{S.hosts=h;S.settings={...S.settings,onboarded:true,theme:'light'};render();},hosts);
  const packets=()=>main.evaluate(()=>testPackets);
  const recv=(page,m)=>page.evaluate(m=>itda.recv(m),m);
  const nav=main.locator('[data-transfers]');
  assert(!await nav.count(),'Transfers entry shown with nothing to show');
  // A window closed with a running move: the app says it continues and lists it.
  await recv(main,{type:'files_background',window:'app',count:1});
  await main.locator('#toasts .toast').filter({hasText:'전송 1개는 백그라운드에서 계속됩니다'}).waitFor();
  await recv(main,app({id:'__network',job:'bg-1',state:'running',file:'Photos/a.jpg',bytes:40,total:100,moving:true}));
  await nav.waitFor();
  assert((await nav.getAttribute('aria-label')).includes('1개 진행 중'),'Sidebar entry lacks the running count');
  await nav.click();
  const dialog=main.getByRole('dialog',{name:'전송',exact:true});
  await dialog.waitFor();
  const row=dialog.locator('[data-id="transfer-bg-1"]');
  assert((await row.textContent()).includes('이 컴퓨터 · 이동'),'Row does not name the device and kind: '+await row.textContent());
  assert(await row.getByRole('progressbar').count()===1,'No progress for a running transfer');
  // Progress updates the open list in place.
  await recv(main,app({id:'__network',job:'bg-1',state:'running',file:'Photos/b.jpg',bytes:70,total:100,moving:true}));
  await main.waitForFunction(()=>document.querySelector('[data-id="transfer-bg-1"] progress')?.value===70);
  // Cancel goes to the app's transfer, not a page's.
  await row.getByRole('button',{name:'취소',exact:true}).click();
  {const t=(await packets()).filter(x=>x.op?.endsWith('_background')).at(-1);assert(t?.op==='cancel_background'&&t.job==='bg-1'&&t.id==='__network','Cancel packet: '+JSON.stringify(t));}
  await recv(main,app({id:'__network',job:'bg-1',state:'cancelled',error:'전송을 취소했습니다',completed_names:['Photos'],moving:true}));
  await main.locator('#toasts .toast').filter({hasText:'파일 이동 취소됨'}).waitFor();
  await row.getByRole('button',{name:'파일 창에서 다시 시도'}).waitFor();
  assert((await row.textContent()).includes('2개 중 1개 끝남'),'Ended row lacks what finished');
  // The same end again (another event or a snapshot) says nothing more.
  await recv(main,app({id:'__network',job:'bg-1',state:'cancelled',completed_names:['Photos'],moving:true}));
  assert(await main.locator('#toasts .toast').filter({hasText:'파일 이동 취소됨'}).count()===1,'Outcome announced twice');
  // An older event arriving late (a racing snapshot) doesn't bring a finished one back.
  await recv(main,app({id:'__network',job:'bg-1',state:'running',seq:1,moving:true}));
  assert(!(await row.getByRole('button',{name:'취소',exact:true}).count()),'A stale event revived an ended transfer');
  // Dismiss: told to the app, gone from the list and the sidebar.
  await row.getByRole('button',{name:/목록에서 지우기/}).click();
  {const t=(await packets()).filter(x=>x.op?.endsWith('_background')).at(-1);assert(t?.op==='dismiss_background'&&t.job==='bg-1','Dismiss packet: '+JSON.stringify(t));}
  await row.waitFor({state:'detached'});
  assert(!await nav.count(),'Sidebar entry stayed after the last transfer was dismissed');
  await recv(main,app({id:'__network',job:'bg-1',state:'failed',seq:99}));
  assert(!await nav.count()&&!await dialog.locator('[data-id="transfer-bg-1"]').count(),'An event after dismissal brought the transfer back');
  // A batch too big to retry from here says only what it can do.
  await recv(main,app({id:'__network',job:'bg-big',state:'failed',seq:5,error:'x',request:{...request,names:null,count:5000}}));
  assert(await dialog.locator('[data-id="transfer-bg-big"]').getByRole('button',{name:'파일 창 열기',exact:true}).count()===1,'Big batch offers a retry it can not do');
  assert((await dialog.locator('.transfer-status').textContent()).includes('1개 실패'),'Status line: '+await dialog.locator('.transfer-status').textContent());
  await dialog.locator('[data-id="transfer-bg-big"]').getByRole('button',{name:/목록에서 지우기/}).click();
  // A failure elsewhere: the entry asks for attention; its retry lives in the file window.
  await recv(main,app({id:'studio',job:'bg-2',state:'failed',error:'SFTP 연결이 끊겼습니다',completed_names:[],request:{...request,id:'studio',move:false}}));
  await main.locator('#toasts .toast').filter({hasText:'Studio Mac · 파일 복사 실패 — SFTP 연결이 끊겼습니다'}).waitFor();
  assert((await nav.getAttribute('aria-label')).includes('확인 필요'),'Failed transfer does not ask for attention');
  await dialog.locator('[data-id="transfer-bg-2"]').getByRole('button',{name:'파일 창에서 다시 시도'}).click();
  {const t=(await packets()).filter(x=>x.cmd==='files_window').at(-1);assert(t?.id==='studio','Retry did not open the device file window: '+JSON.stringify(t));}
  await main.keyboard.press('Escape');

  // ---- a file window opened later: the state snapshot brings the app's transfer in.
  const win=await c.newPage();win.on('pageerror',e=>errors.push(e.message));
  await icons(win);await win.goto(base+'/web/?files=__network');
  const failed=app({id:'__network',job:'bg-3',state:'failed',error:'디스크가 가득 찼습니다',completed_names:['Photos'],partial:null,moving:true});
  const running=app({id:'__network',job:'bg-4',state:'running',file:'big.iso',bytes:1,total:9});
  await win.evaluate(({h,jobs})=>{itda.recv({type:'state',hosts:h,accounts:[],settings:{onboarded:true,theme:'light'},sessions:{},files_jobs:jobs});},{h:hosts,jobs:[failed,running]});
  const layer=win.locator('.files-layer');
  await layer.locator('[data-id="file-job-bg-3"]').waitFor();
  assert(!await win.locator('#toasts .toast').count(),'A snapshot was announced');
  await layer.locator('[data-cancel-job="bg-4"]').click();
  {const t=(await win.evaluate(()=>testPackets)).filter(x=>x.op?.endsWith('_background')).at(-1);assert(t?.op==='cancel_background'&&t.job==='bg-4','File window cancel: '+JSON.stringify(t));}
  // Retry what is left: a new confirmation, then the app's record is dismissed.
  await layer.locator('[data-retry-job="bg-3"]').click();
  const confirm=win.getByRole('dialog',{name:'남은 파일 다시 이동',exact:true});
  assert((await confirm.textContent()).includes('남은 1개'),'Retry is not for what is left: '+await confirm.textContent());
  await confirm.getByRole('button',{name:'다시 이동',exact:true}).click();
  {const all=await win.evaluate(()=>testPackets),t=all.filter(x=>x.op==='transfer').at(-1),d=all.filter(x=>x.op==='dismiss_background').at(-1);
   assert(t&&JSON.stringify(t.names)==='["report.txt"]'&&t.move&&t.from==='local'&&t.to==='local'&&t.destination==='D:\\Backup'&&!t.from_window,'Retry request: '+JSON.stringify(t));
   assert(d?.job==='bg-3','Retried transfer stays in the app list');}
  // Dismissed elsewhere: gone here too.
  await recv(win,{type:'files_job',window:'app',orphan:true,job:'bg-4',id:'__network',dismissed:true});
  await layer.locator('[data-id="file-job-bg-4"]').waitFor({state:'detached'});
  assert(!errors.length,errors.join('\n'));
  console.log('PASS: background transfers — main-window list/progress/cancel/dismiss/outcome, file-window snapshot, cancel and retry');
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
