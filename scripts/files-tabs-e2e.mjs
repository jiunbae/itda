// Tabs that browse different devices in one file window, on the real frontend with an
// in-memory IPC fixture (no SSH, no user files). Opening another device in a new tab must
// keep the first tab's connection; closing a tab releases only a device no tab still uses.
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/files-tabs-e2e.mjs
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
const browser=await pw[process.env.BROWSER || "chromium"].launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(value,msg)=>{if(!value)throw Error(msg);};
try{
 for(const platform of ['mac','windows']){
  const c=await browser.newContext({viewport:{width:1200,height:800},reducedMotion:'reduce',userAgent:platform==='mac'?'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)':'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
  await c.addInitScript(()=>{
   const entry=(name,dir=false)=>({name,dir,size:100,modified:1780000000});
   // studio and build answer; slow never finishes connecting (used for cancel).
   const homes={studio:'/Users/alex',build:'/home/dev',fresh:'/fresh'};
   const tree={'/Users/alex':[entry('Docs',true),entry('Alpha-A.txt')],'/Users/alex/Docs':[entry('Inside-A.txt')],'/home/dev':[entry('Beta-B.txt')],'/fresh':[entry('Fresh.txt')],'/local':[entry('Local.txt')]};
   window.testPackets=[];
   window.ipc={postMessage:raw=>{const m=JSON.parse(raw);if(['files_clipboard','files_sort','files_drag_release'].includes(m.cmd)){testPackets.push(m);return;}if(m.cmd!=='files')return;testPackets.push(m);
    const result={...m,type:'files_result',ok:true};
    if(m.op==='connect'){if(!homes[m.id])return;result.data={path:homes[m.id],entries:tree[homes[m.id]],local_path:'/local',local_entries:tree['/local'],rsync:true};}
    else if(m.op==='list')result.data={path:m.path,entries:window.sortTesting&&m.path==='/Users/alex' ? [entry('Folder2',true),{...entry('file10.txt'),size:20,modified:300},{...entry('Zebra.pdf'),size:5,modified:100},entry('Docs',true),{...entry('file2.txt'),size:20,modified:200}] : tree[m.path]||[]};
    else return;
    if(m.op==='list'&&window.holdListReplies){(window.heldListReplies||=[]).push(result);return;}
    setTimeout(()=>itda.recv(result),10);
   }};
  });
  const p=await c.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
  if(platform==='windows'&&process.platform!=='win32')await p.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));
  await p.goto(base+'/web/');
  const launch=()=>p.evaluate(()=>{const files={method:'sftp',user:'alex'};S.hosts=[{id:'studio',name:'Studio Mac',host:'studio.local',os:'mac',kind:'vnc',login_user:'alex',has_pw:true,files},{id:'build',name:'Build Server',host:'build.local',os:'linux',kind:'vnc',login_user:'dev',has_pw:true,files},{id:'fresh',name:'Fresh Server',host:'fresh.local',os:'linux',login_user:'dev',files},{id:'slow',name:'Slow Box',host:'slow.local',os:'linux',kind:'vnc',login_user:'dev',has_pw:true,files}];S.settings={...S.settings,onboarded:true,theme:'light'};render();ItdaFiles.message({type:'state',files_clipboard:{cid:'before-window',host:'build',window:'prior-page',side:'remote',path:'/home/dev',names:['Beta-B.txt'],move:false}});ItdaFiles.launch(S.hosts[0]);});
  await launch();
  const layer=p.locator('.files-layer'),row=name=>layer.locator('[data-index]').filter({hasText:name}).first();
  const packets=()=>p.evaluate(()=>testPackets),mod=platform==='mac'?'Meta':'Control';
  const count=async(id,op)=>(await packets()).filter(x=>x.id===id&&x.op===op).length;
  await row('Alpha-A.txt').waitFor();
  // Native file-manager actions follow the displayed side, including connected hosts.
  await layer.locator('[data-action="more"]').click();
  await p.getByRole('menuitem',{name:platform==='mac'?'Finder에서 열기':'탐색기에서 열기',exact:true}).click();
  assert((await packets()).filter(x=>x.op==='finder').at(-1)?.id==='studio','Remote native action changed its device');
  await layer.locator(platform==='mac'?'.files-current-places [data-side="local"]':'[data-network-sidebar] [data-bookmark="Documents"]').click();
  await layer.locator('[data-action="more"]').click();
  await p.getByRole('menuitem',{name:platform==='mac'?'Finder에서 보기':'탐색기에서 열기',exact:true}).click();
  {const r=(await packets()).filter(x=>x.op==='reveal_folder').at(-1);assert(r?.id==='studio'&&r.side==='local'&&r.path===(platform==='mac'?'/local':'/local/Documents'),'Connected local action opened a remote location: '+JSON.stringify(r));}
  await layer.locator('.files-sidebar [data-device="studio"]').first().click();await row('Alpha-A.txt').waitFor();
  // Copy happened before this window was created: state hydration survives opening its device.
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);assert(t?.from_id==='build'&&t.from_window==='prior-page'&&t.names[0]==='Beta-B.txt','New window lost the earlier copy: '+JSON.stringify(t));}

  // Another device's clipboard can use delta when the target is this computer.
  await layer.locator(platform==='mac' ? '.files-current-places [data-side="local"]' : '[data-network-sidebar] [data-bookmark="Documents"]').click();
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  const deltaDialog=p.getByRole('dialog',{name:'파일 복사',exact:true});await deltaDialog.locator('summary').click();
  await deltaDialog.getByLabel('같은 이름의 파일이 있으면',{exact:true}).selectOption('replace');
  assert(!await deltaDialog.getByLabel('전송 방식',{exact:true}).locator('[value="rsync"]').isDisabled(),'Other-device to local copy unnecessarily blocked delta');
  await deltaDialog.getByRole('button',{name:'취소',exact:true}).click();
  await layer.locator('.files-sidebar [data-device="studio"]').first().click();await row('Alpha-A.txt').waitFor();

  // One sorting preference drives every layout, including cached ancestor columns.
  const sort=async name=>{await layer.locator('[data-action="sort"]').click();await p.getByRole('menuitemradio',{name,exact:true}).click();};
  const names=()=>layer.locator('[data-pane="remote"] [data-index]').evaluateAll(es=>es.map(e=>e.querySelector('td .files-name > span, :scope > span').textContent.trim()));
  const mode=async name=>{if(platform==='mac')await layer.locator(`[data-mode="${name}"]`).click();else{await layer.locator('[data-action="view"]').click();await p.getByRole('menuitemradio',{name:name==='grid'?'큰 아이콘':'자세히',exact:true}).click();}};
  await p.evaluate(()=>window.sortTesting=true);
  await layer.locator('[data-action="refresh"]').click();await row('file10.txt').waitFor();
  const orders=[['이름',['Docs','Folder2','file2.txt','file10.txt','Zebra.pdf']],['수정한 날짜',['Docs','Folder2','Zebra.pdf','file2.txt','file10.txt']],['크기',['Docs','Folder2','Zebra.pdf','file2.txt','file10.txt']],['종류',['Docs','Folder2','Zebra.pdf','file2.txt','file10.txt']]];
  for(const [key,expected]of orders){
   await sort(key);await sort('오름차순');
   for(const v of platform==='mac'?['list','grid','columns','gallery']:['list','grid']){await mode(v);assert(JSON.stringify(await names())===JSON.stringify(expected),`${platform}/${key}/${v}: ${JSON.stringify(await names())}`);}
   await sort('내림차순');const descending=[...expected.slice(0,2).reverse(),...expected.slice(2).reverse()];
   for(const v of platform==='mac'?['gallery','columns','grid','list']:['grid','list']){await mode(v);assert(JSON.stringify(await names())===JSON.stringify(descending),`${platform}/${key}/descending/${v}: ${JSON.stringify(await names())}`);}
  }
  await sort('이름');
  if(platform==='mac'){
   await mode('columns');await row('Docs').click();await row('Inside-A.txt').waitFor();
   const ancestor=await layer.locator('.files-column[data-column="/Users/alex"] .files-column-entry > span').allTextContents();
   assert(JSON.stringify(ancestor)===JSON.stringify(['Folder2','Docs','Zebra.pdf','file10.txt','file2.txt']),`Cached ancestor ignored shared sort: ${ancestor}`);
   await layer.locator('[data-action="back"]').click();await row('file10.txt').waitFor();await mode('list');
  }
  // A new page/window restores the setting; a header click changes the same menu preference.
  await p.reload();await launch();await row('Alpha-A.txt').waitFor();
  await layer.locator('[data-action="sort"]').click();assert(await p.getByRole('menuitemradio',{name:'내림차순',exact:true}).getAttribute('aria-checked')==='true','Sort preference did not survive reload');await p.keyboard.press('Escape');
  await layer.locator('th[data-sort="name"]').click();
  await layer.locator('[data-action="sort"]').click();assert(await p.getByRole('menuitemradio',{name:'오름차순',exact:true}).getAttribute('aria-checked')==='true','Header and menu sort diverged');await p.keyboard.press('Escape');
  // A sort chosen here is announced to every open file window, and one from another window applies here.
  assert(await p.evaluate(()=>testPackets.some(m=>m.cmd==='files_sort'&&m.sort?.key==='name')),'Sort change not announced to other windows');
  await p.keyboard.press('Escape');
  const announced=await p.evaluate(()=>testPackets.filter(m=>m.cmd==='files_sort').length);
  await p.evaluate(()=>itda.recv({type:'files_sort',sort:{key:'size',desc:true}}));
  await layer.locator('[data-action="sort"]').click();
  assert(await p.getByRole('menuitemradio',{name:'크기',exact:true}).getAttribute('aria-checked')==='true'&&await p.getByRole('menuitemradio',{name:'내림차순',exact:true}).getAttribute('aria-checked')==='true','Sort from another window not applied');
  await p.keyboard.press('Escape');
  assert(await p.evaluate(()=>testPackets.filter(m=>m.cmd==='files_sort').length)===announced,'Sort from another window echoed back');
  await p.evaluate(()=>itda.recv({type:'files_sort',sort:{key:'name',desc:false}}));

  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+t`);
  assert(await layer.locator('[data-tab]').count()===2,'New tab failed');
  // The tab's label and close button are separate keyboard controls. Arrow navigation
  // retains focus on the active tab; Space on close must close rather than preview a file.
  await p.evaluate(()=>window.holdListReplies=true);
  await layer.getByRole('tab',{selected:true}).focus();await p.keyboard.press('ArrowLeft');
  await p.waitForFunction(()=>document.activeElement.matches('.files-tab [role="tab"][aria-selected="true"]'));
  await p.keyboard.press('ArrowRight');
  const closeTabButton=layer.locator('[data-tab]').nth(1).locator('[data-close-tab]');
  await closeTabButton.focus();await p.keyboard.down('Space');
  // A real folder response may arrive while the person is holding Space. Keeping
  // focus on a replacement button is insufficient: the pending native click is
  // attached to the original control.
  await p.evaluate(()=>{const reply=heldListReplies.pop();if(!reply)throw Error('No pending list reply');window.holdListReplies=false;heldListReplies.length=0;itda.recv(reply);});
  assert(await closeTabButton.evaluate(e=>e===document.activeElement),'Folder reply lost the close button focus');
  await p.keyboard.up('Space');
  await p.waitForFunction(()=>document.querySelectorAll('.files-tab[data-tab]').length===1,null,{timeout:5000});
  assert(await layer.locator('[data-tab]').count()===1,'Keyboard close failed');
  await p.keyboard.press(`${mod}+t`);assert(await layer.locator('[data-tab]').count()===2,'Tab could not reopen after keyboard close');

  // Open another device in the new tab.
  await layer.locator('[data-device="build"]').first().click();
  await row('Beta-B.txt').waitFor();
  assert(await layer.locator('[data-tab]').count()===2,'Opening a device replaced the tabs');
  assert(await count('studio','release')===0&&await count('studio','disconnect')===0,'First device was released when another tab opened a device');
  assert((await layer.locator('[data-tab]').nth(0).getAttribute('title')).startsWith('Studio Mac'),'First tab lost its device');
  assert((await layer.locator('[data-tab]').nth(1).getAttribute('title')).startsWith('Build Server'),'Second tab is not on the new device');
  // A remote file dropped on another device's tab goes device to device: the job belongs to the
  // destination, and the source device is named.
  const src=await row('Beta-B.txt').boundingBox(),dst=await layer.locator('[data-tab]').nth(0).boundingBox();
  await p.mouse.move(src.x+60,src.y+src.height/2);await p.mouse.down();await p.mouse.move(dst.x+40,dst.y+dst.height/2,{steps:12});await p.mouse.up();
  assert((await p.locator('.files-prompt').textContent()).includes('Build Server: /home/dev'),'Transfer dialog does not name the source device');
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);
   assert(t&&t.id==='studio'&&t.from_id==='build'&&t.from==='remote'&&t.to==='remote'&&t.source==='/home/dev'&&t.destination==='/Users/alex','Device-to-device drop: '+JSON.stringify(t));}
  // Back to the first tab: same device, same connection, no reconnect.
  await layer.locator('[data-tab]').nth(0).click();
  await row('Alpha-A.txt').waitFor();
  assert(await count('studio','connect')===1,'Switching tabs reconnected the first device');
  const last=(await packets()).filter(x=>x.op==='list').at(-1);
  assert(last.id==='studio','Tab switch listed the wrong device: '+JSON.stringify(last));
  await row('Docs').dblclick();await row('Inside-A.txt').waitFor();
  // A cancelled paste keeps the cut for later (and in every window).
  await row('Inside-A.txt').click();await p.keyboard.press(`${mod}+x`);
  await layer.locator('[data-tab]').nth(1).click();await row('Beta-B.txt').waitFor();
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  await p.locator('.files-prompt [data-choice="cancel"]').click();
  assert(!(await packets()).some(x=>x.cmd==='files_clipboard'&&x.clip?.clear),'Cancel consumed the cut clipboard');
  await layer.locator('[data-tab]').nth(0).click();await row('Inside-A.txt').waitFor();
  // Copied on one device, pasted on another: device to device.
  await row('Inside-A.txt').click();await p.keyboard.press(`${mod}+c`);
  assert((await packets()).some(x=>x.cmd==='files_clipboard'&&x.clip?.names?.includes('Inside-A.txt')),'Copy was not shared with other windows');
  await layer.locator('[data-tab]').nth(1).click();await row('Beta-B.txt').waitFor();
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);
   assert(t.id==='build'&&t.from_id==='studio'&&t.source==='/Users/alex/Docs'&&t.destination==='/home/dev'&&t.names[0]==='Inside-A.txt','Device-to-device paste: '+JSON.stringify(t));}
  // Copied in another file window: the paste names that window's connection.
  await p.evaluate(()=>itda.recv({type:'files_clipboard',clip:{cid:'w2',host:'studio',window:'other-window',side:'remote',path:'/Users/alex',names:['Alpha-A.txt'],move:false}}));
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);
   assert(t.id==='build'&&t.from_id==='studio'&&t.from_window==='other-window'&&t.source==='/Users/alex','Cross-window paste: '+JSON.stringify(t));}
  // Native drop provenance goes through the same transfer confirmation and relay.
  const nativeOrigin={id:'studio',window:'source-window',side:'remote',path:'/Users/alex',names:['Alpha-A.txt'],drag:'native-live-token'};
  await p.evaluate(origin=>itda.recv({type:'files_drop',window:window.ITDA_FILE_WINDOW||window.ITDA_FILE_HOST||null,origin}),nativeOrigin);
  await p.locator('.files-prompt [data-choice="ok"]').click();
  await p.waitForFunction(()=>testPackets.some(x=>x.cmd==='files_drag_release'&&x.drag==='native-live-token'));
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);assert(t.id==='build'&&t.from_id==='studio'&&t.from_window==='source-window'&&t.from_drag==='native-live-token'&&t.names[0]==='Alpha-A.txt','Native cross-window origin lost: '+JSON.stringify(t));}
  const beforeCancel=(await packets()).filter(x=>x.op==='transfer').length;
  await p.evaluate(origin=>itda.recv({type:'files_drop',window:window.ITDA_FILE_WINDOW||window.ITDA_FILE_HOST||null,origin:{...origin,drag:'native-cancel-token'}}),nativeOrigin);
  await p.locator('.files-prompt [data-choice="cancel"]').click();
  await p.waitForFunction(()=>testPackets.some(x=>x.cmd==='files_drag_release'&&x.drag==='native-cancel-token'));
  assert((await packets()).filter(x=>x.op==='transfer').length===beforeCancel,'Cancelled native drop started a transfer');
  await layer.locator('[data-tab]').nth(0).click();await row('Inside-A.txt').waitFor();
  // Transfers show on their own device's tabs and cancel on that device.
  await p.evaluate(()=>itda.recv({type:'files_job',window:ItdaFiles.windowToken,id:'studio',job:'job-A',state:'running',file:'A-copy'}));
  await layer.locator('.files-jobs').filter({hasText:'A-copy'}).waitFor();
  await layer.locator('[data-tab]').nth(1).click();await row('Beta-B.txt').waitFor();
  assert(!(await layer.locator('.files-jobs').textContent()).includes('A-copy'),"Another device's transfer is shown in this tab");
  await layer.locator('[data-tab]').nth(0).click();await row('Inside-A.txt').waitFor();
  const cancelJobButton=layer.locator('[data-cancel-job="job-A"]');
  await cancelJobButton.focus();await p.keyboard.down('Space');
  await p.evaluate(()=>itda.recv({type:'files_job',window:ItdaFiles.windowToken,id:'studio',job:'job-A',state:'running',file:'A-copy',bytes:20,total:100}));
  assert(await cancelJobButton.evaluate(e=>e===document.activeElement),'Progress lost the cancel button focus');
  await p.keyboard.up('Space');
  await p.waitForFunction(()=>testPackets.some(x=>x.op==='cancel'&&x.job==='job-A'),null,{timeout:5000});
  assert((await packets()).filter(x=>x.op==='cancel'&&x.job==='job-A').length===1,'Space did not cancel the transfer exactly once');
  assert((await packets()).filter(x=>x.op==='cancel').at(-1)?.id==='studio','Cancel went to the wrong device');
  await p.evaluate(()=>itda.recv({type:'files_job',window:ItdaFiles.windowToken,id:'studio',job:'job-A',state:'cancelled',file:'A-copy'}));
  // If the armed job finishes, its row must not become a different job's cancel
  // button before key-up (which would cancel work the person never selected).
  await p.evaluate(()=>{for(const job of ['keyboard-A','keyboard-B'])itda.recv({type:'files_job',window:ItdaFiles.windowToken,id:'studio',job,state:'running',file:job});});
  await layer.locator('[data-cancel-job="keyboard-A"]').focus();await p.keyboard.down('Space');
  // Hold the completion's listing reply so both sides of the DOM replacement are
  // observed deterministically. A Locator resolves an ElementHandle before evaluate;
  // that handle can be detached by a reply before its callback runs.
  await p.evaluate(()=>{window.holdListReplies=true;itda.recv({type:'files_job',window:ItdaFiles.windowToken,id:'studio',job:'keyboard-A',state:'completed'});});
  const focusSnapshot=()=>p.evaluate(()=>{
   const e=document.querySelector('.files-layer [data-pane="remote"] .files-list');
   const a=document.activeElement;
   const listVisible=!!(e?.offsetWidth||e?.offsetHeight);
   return {expected:!!e&&e===a&&e.isConnected&&listVisible,focused:{tag:a?.tagName,id:a?.id,role:a?.getAttribute('role'),classes:a?.className,pane:a?.closest('[data-pane]')?.dataset.pane,cancelJob:a?.getAttribute('data-cancel-job'),dismissJob:a?.getAttribute('data-dismiss-job')},listVisible,documentFocused:document.hasFocus()};
  });
  const completionFocus=await focusSnapshot();
  assert(completionFocus.expected,'Completed job moved focus onto another transfer: '+JSON.stringify(completionFocus));
  await p.waitForFunction(()=>window.heldListReplies?.length>0);
  const priorList=await layer.locator('[data-pane="remote"] .files-list').elementHandle();
  // (The refreshed listing has a new file, as after a transfer: an unchanged one isn't repainted.)
  await p.evaluate(()=>{window.holdListReplies=false;const replies=window.heldListReplies.splice(0);for(const reply of replies){if(reply.op==='list'&&reply.data?.entries)reply.data.entries=[...reply.data.entries,{name:'zz-refreshed.txt',dir:false,link:false,size:1,modified:1}];itda.recv(reply);}});
  const refreshedFocus=await focusSnapshot();
  assert(refreshedFocus.expected,'Listing refresh lost completed-job focus: '+JSON.stringify(refreshedFocus));
  assert(await priorList.evaluate(e=>!e.isConnected&&e!==document.activeElement),'Listing refresh did not replace the old focus target');
  await priorList.dispose();
  await p.keyboard.up('Space');
  assert(!(await packets()).some(x=>x.op==='cancel'&&x.job==='keyboard-B'),'A disappearing row redirected Space to another transfer');
  await p.evaluate(()=>itda.recv({type:'files_job',window:ItdaFiles.windowToken,id:'studio',job:'keyboard-B',state:'completed'}));
  // A transfer that fails on another tab is announced once (device and reason) without moving
  // this tab, stays reachable in the list here, and is retried for what is left only after
  // a new confirmation, on its own devices, without the original clipboard or drag token.
  const job=(m)=>p.evaluate(m=>itda.recv({type:'files_job',window:ItdaFiles.windowToken,...m}),m);
  const toasts=(text)=>p.locator('#toasts .toast').filter({hasText:text}).count();
  const activeTab=()=>layer.locator('[data-tab].on').getAttribute('title');
  await layer.locator('[data-tab]').nth(1).click();await row('Beta-B.txt').waitFor();
  await p.evaluate(()=>itda.recv({type:'files_clipboard',clip:{cid:'move-clip',host:'studio',window:'other-window',side:'remote',path:'/Users/alex',names:['Alpha-A.txt','Docs','Gamma.txt'],move:true}}));
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  await p.locator('.files-prompt [data-choice="ok"]').click();
  const moved=(await packets()).filter(x=>x.op==='transfer').at(-1);
  assert(moved.id==='build'&&moved.move&&moved.req&&moved.from_clip==='move-clip','Move request: '+JSON.stringify(moved));
  await job({id:'build',job:'job-M',req:moved.req,state:'queued',moving:true});
  await layer.locator('[data-tab]').nth(0).click();await row('Inside-A.txt').waitFor();
  const failed={id:'build',job:'job-M',req:moved.req,state:'failed',error:'SFTP 연결이 끊겼습니다',completed_names:['Alpha-A.txt'],partial:{name:'Docs',dir:true},source_closed:false};
  await job(failed);await job(failed);
  assert((await activeTab()).startsWith('Studio Mac'),'A background failure moved the tab');
  assert(await toasts('Build Server · 파일 이동 실패 — SFTP 연결이 끊겼습니다')===1,'Background failure not announced exactly once');
  const failedRow=layer.locator('.files-job').filter({hasText:'Build Server ·'});
  await failedRow.waitFor();
  assert((await failedRow.textContent()).includes('3개 중 1개 끝남'),'Failed row lacks progress: '+await failedRow.textContent());
  await failedRow.getByRole('button',{name:'Build Server 탭 보기'}).click();
  assert((await activeTab()).startsWith('Build Server'),'Job row did not lead to its tab');
  await layer.locator('[data-tab]').nth(0).click();await row('Inside-A.txt').waitFor();
  const sentBefore=(await packets()).filter(x=>x.op==='transfer').length;
  await failedRow.getByRole('button',{name:'다시 시도…'}).click();
  const retryDialog=p.getByRole('dialog',{name:'남은 파일 다시 이동',exact:true});
  const note=await retryDialog.textContent();
  assert(note.includes('남은 2개')&&note.includes('이미 끝난 1개')&&note.includes('‘Docs’ 폴더는 일부만'),'Retry dialog: '+note);
  // Merging is only suggested: the other remaining name keeps the original 'keep both'.
  assert(await retryDialog.getByLabel('같은 이름의 파일이 있으면',{exact:true}).inputValue()==='keep'&&note.includes('남은 다른 항목에도 적용'),'Partial folder retry escalated the policy for other names');
  assert(note.includes('남은 파일만 새 이름의 폴더로 옮깁니다'),'Move retry keep-both text is not move-aware');
  assert((await packets()).filter(x=>x.op==='transfer').length===sentBefore,'Retry ran before it was confirmed');
  await retryDialog.getByRole('button',{name:'다시 이동',exact:true}).click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);
   const token=await p.evaluate(()=>ItdaFiles.windowToken);
   assert(t.id==='build'&&t.move&&JSON.stringify(t.names)==='["Docs","Gamma.txt"]'&&t.from_id==='studio'&&t.from_window===token&&!t.from_clip&&!t.from_drag&&t.source==='/Users/alex'&&t.policy==='keep'&&t.req!==moved.req,'Retry request: '+JSON.stringify(t));
   assert(!(await layer.locator('[data-retry-job="job-M"]').count()),'The retried job stays in the list');
   // The retry, cancelled in the background: announced as cancelled, retry offered again.
   await job({id:'build',job:'job-R',req:t.req,state:'queued',moving:true});
   await job({id:'build',job:'job-R',req:t.req,state:'cancelled',error:'전송을 취소했습니다',completed_names:[],partial:null,source_closed:false});
   assert(await toasts('Build Server · 파일 이동 취소됨')===1,'Background cancel not announced');
   assert(await layer.locator('[data-retry-job="job-R"]').count()===1,'Cancelled retry has no retry');
   await layer.locator('[data-dismiss-job="job-R"]').click();}
  // Another device's failure stays in view, with its retry, on Home/Network and through a
  // Network refresh; opening them doesn't jump to that device's tab.
  await layer.locator('[data-tab]').nth(1).click();await row('Beta-B.txt').waitFor();
  await p.evaluate(()=>itda.recv({type:'files_clipboard',clip:{cid:'home-clip',host:'studio',window:'other-window',side:'remote',path:'/Users/alex',names:['Alpha-A.txt'],move:false}}));
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);
   await job({id:'build',job:'job-H',req:t.req,state:'queued'});
   await layer.locator('[data-tab]').nth(0).click();await row('Inside-A.txt').waitFor();
   await job({id:'build',job:'job-H',req:t.req,state:'failed',error:'홈 확인용 실패',completed_names:[],partial:null,source_closed:false});}
  const homeRow=layer.locator('.files-job').filter({hasText:'홈 확인용 실패'});
  await homeRow.waitFor();
  for(const page of platform==='mac'?['network']:['explorer-home','network']){
    await layer.locator(`.files-sidebar [data-action="${page}"]`).first().click();
    await p.waitForFunction(()=>document.querySelector('.files-network'));
    for(const step of ['open','refresh']){
      if(step==='refresh')await p.evaluate(()=>ItdaFiles.refreshNetwork());
      assert(await homeRow.isVisible()&&await homeRow.getByRole('button',{name:'다시 시도…'}).isVisible(),`Failed job or its retry hidden on ${page} (${step})`);
    }
    assert(!(await activeTab()).startsWith('Build Server')&&await layer.locator('[data-tab]').nth(0).evaluate(e=>e.classList.contains('on')),'Home/Network jumped to the job\'s tab');
  }
  await layer.locator('[data-dismiss-job="job-H"]').click();
  await layer.locator('.files-sidebar [data-device="studio"]').first().click();await row('Inside-A.txt').waitFor();
  // The source device's connection is gone (no tab here, its window closed): guidance to open
  // it, no transfer.
  await layer.locator('[data-tab]').nth(1).click();await row('Beta-B.txt').waitFor();
  await p.evaluate(()=>itda.recv({type:'files_clipboard',clip:{cid:'fresh-clip',host:'fresh',window:'closed-window',side:'remote',path:'/fresh',names:['Fresh.txt'],move:false}}));
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);
   await job({id:'build',job:'job-F',req:t.req,state:'queued'});
   await job({id:'build',job:'job-F',req:t.req,state:'failed',error:'원본 기기의 파일 연결이 닫혔습니다. 그 기기의 탭이나 창을 다시 열어 주세요',completed_names:[],partial:null,source_closed:true});
   const before=(await packets()).filter(x=>x.op==='transfer').length;
   await layer.getByRole('button',{name:'원본 기기 열기…'}).click();
   const guide=p.getByRole('dialog',{name:'원본 기기의 연결이 닫혔습니다',exact:true});
   assert((await guide.textContent()).includes('Fresh Server'),'Closed source guidance does not name the device');
   assert(await guide.getByRole('button',{name:'Fresh Server 열기',exact:true}).count()===1,'No way to reopen the source');
   await guide.getByRole('button',{name:'취소',exact:true}).click();
   assert((await packets()).filter(x=>x.op==='transfer').length===before,'Closed source retried blindly');
   await layer.locator('[data-dismiss-job="job-F"]').click();}
  // An end event for a job this window never saw start says nothing.
  await job({id:'build',job:'job-stray',state:'failed',error:'stray'});
  assert(await toasts('stray')===0,'Stray end event announced');
  await layer.locator('[data-dismiss-job="job-stray"]').click().catch(()=>{});
  await layer.locator('[data-tab]').nth(0).click();await row('Inside-A.txt').waitFor();
  // A preview answered after the tab moved to another device must not block that device's previews.
  await row('Inside-A.txt').click();
  if(platform==='mac')await layer.locator('[data-action="preview"]').click();else{await layer.locator('.files-list').focus();await p.keyboard.press('Alt+p');}
  await p.waitForFunction(()=>testPackets.some(x=>x.op==='preview'&&x.id==='studio'));
  const old=(await packets()).filter(x=>x.op==='preview').at(-1);
  await layer.locator('[data-tab]').nth(1).click();await row('Beta-B.txt').waitFor();await row('Beta-B.txt').click();
  await p.evaluate(m=>itda.recv({...m,type:'files_result',ok:true,data:{mime:'text/plain',text:'A-preview'}}),old);
  await p.waitForFunction(()=>testPackets.some(x=>x.op==='preview'&&x.id==='build'),null,{timeout:3000}).catch(()=>{throw Error('Preview stayed blocked after a device change');});
  await layer.locator('[data-action="close-preview"]').click();
  await layer.locator('[data-tab]').nth(0).click();await row('Inside-A.txt').waitFor();
  // Cancel while another device is connecting: the tab returns to its device.
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+t`);
  await layer.locator('[data-device="slow"]').first().click();
  await p.locator('.files-setup [data-close]').click();
  await p.locator('.files-setup').waitFor({state:'detached'});
  assert(await layer.locator('[data-tab]').count()===3,'Cancelling closed a tab');
  await row('Inside-A.txt').waitFor();
  assert(await count('slow','disconnect')===1,'Cancelled connection was not stopped');
  assert(await count('studio','release')===0,'Cancelling released the tab device');
  // Closing tabs releases a device only when no tab still uses it.
  await layer.locator('[data-tab]').nth(2).locator('[data-close-tab]').click();
  assert(await count('studio','release')===0,'Closing one of two tabs on a device released it');
  await layer.locator('[data-tab]').nth(1).locator('[data-close-tab]').click();
  assert(await count('build','release')===1,'Closing the last tab on a device did not release it');
  await row('Inside-A.txt').waitFor();
  assert(await layer.locator('[data-tab]').count()===1,'Tab count after closing');
  // A same-device drop after the active tab moved to another device still runs on the dragged file's device.
  await layer.locator('.files-list').focus();await p.keyboard.press(platform==='mac'?'Meta+ArrowUp':'Alt+ArrowUp');await row('Alpha-A.txt').waitFor();
  await p.keyboard.press(`${mod}+t`);await layer.locator('[data-device="build"]').first().click();await row('Beta-B.txt').waitFor();
  await layer.locator('[data-tab]').nth(0).click();await row('Alpha-A.txt').waitFor();
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+t`);await row('Docs').dblclick();await row('Inside-A.txt').waitFor();
  const from=await row('Inside-A.txt').boundingBox();
  await p.mouse.move(from.x+60,from.y+from.height/2);await p.mouse.down();await p.mouse.move(from.x+90,from.y+from.height/2,{steps:4});
  await p.keyboard.press('Control+Tab');await p.keyboard.press('Control+Tab');await row('Beta-B.txt').waitFor();
  const home=await layer.locator('[data-tab]').nth(0).boundingBox();
  await p.mouse.move(home.x+40,home.y+home.height/2,{steps:12});await p.mouse.up();
  await p.locator('.files-prompt [data-choice="ok"]').click();
  const sent=(await packets()).filter(x=>x.op==='transfer').at(-1);
  assert(sent?.id==='studio'&&sent.source==='/Users/alex/Docs'&&sent.destination==='/Users/alex','Same-device tab drop ran on the active device: '+JSON.stringify(sent));
  // Sidebar device drop: an existing tab is reused, including Shift-to-move.
  const deviceDrop=async(name,id,move=false)=>{
    const a=await row(name).boundingBox(),b=await layer.locator(`[data-device="${id}"]`).first().boundingBox();
    await p.mouse.move(a.x+60,a.y+a.height/2);await p.mouse.down();
    if(move)await p.keyboard.down('Shift');
    await p.mouse.move(b.x+b.width/2,b.y+b.height/2,{steps:12});
    assert(await layer.locator(`[data-device="${id}"].files-drop-target`).count()===1,'Device drop target has no feedback');
    await p.mouse.up();if(move)await p.keyboard.up('Shift');
  };
  await deviceDrop('Beta-B.txt','studio',true);
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);assert(t.id==='studio'&&t.from_id==='build'&&t.move&&t.destination==='/Users/alex','Sidebar existing-device move: '+JSON.stringify(t));}
  // A device without a tab connects in a new tab without releasing the source.
  const releases=await count('studio','release');
  await deviceDrop('Alpha-A.txt','fresh');await row('Fresh.txt').waitFor();
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await packets()).filter(x=>x.op==='transfer').at(-1);assert(t.id==='fresh'&&t.from_id==='studio'&&t.source==='/Users/alex'&&t.destination==='/fresh'&&!t.move,'Sidebar new-device copy: '+JSON.stringify(t));}
  assert(await count('studio','release')===releases,'Device drop released its source');
  // A cancelled connection must never leave a deferred transfer for a later open.
  const transfers=(await packets()).filter(x=>x.op==='transfer').length;
  await deviceDrop('Fresh.txt','slow');await p.locator('.files-setup [data-close]').click();
  await row('Fresh.txt').waitFor();
  await layer.locator('[data-device="studio"]').first().click();await row('Alpha-A.txt').waitFor();
  assert((await packets()).filter(x=>x.op==='transfer').length===transfers,'Cancelled device drop later transferred files');
  assert(!errors.length,errors.join('\n'));
  await c.close();
 }
 console.log(`PASS: tabs on different devices keep their connections, cancel restores, close releases unused devices (${process.platform})`);
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
