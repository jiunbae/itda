// The device-less file window (opened first) on the real frontend with an in-memory IPC
// fixture: this computer's folders, Windows' This PC drive list, Up from a drive and Back to
// the page it came from. No SSH, no user files.
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/files-network-local-e2e.mjs
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
const engine=process.env.BROWSER||'chromium';
const executable=engine==='webkit'?process.env.WEBKIT:process.env.CHROMIUM;
const browser=await pw[engine].launch({headless:true,...(executable?{executablePath:executable}:{})});
const assert=(value,msg)=>{if(!value)throw Error(msg);};
try{
  // A new Mac profile must reach its own folders before adding or connecting a device.
  {
    const c=await browser.newContext({viewport:{width:1200,height:800},reducedMotion:'reduce',userAgent:'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'});
    try{
      await c.addInitScript(()=>{
        const entry=(name,dir=false)=>({name,dir,size:100,modified:1780000000});
        const tree={'/Users/alex':[entry('Documents',true),entry('note.txt')],'/Users/alex/Documents':[entry('report.txt')]};
        window.testPackets=[];
        window.ipc={postMessage:raw=>{
          const m=JSON.parse(raw);testPackets.push(m);if(m.cmd!=='files')return;
          if(m.op!=='local_home'&&m.op!=='list')return;
          const folder=m.op==='local_home'?'/Users/alex':m.path;
          const reply=()=>itda.recv({...m,type:'files_result',ok:true,data:{path:folder,entries:tree[folder]||[]}});
          if(m.op==='local_home')window.answerHome=reply;else setTimeout(reply,10);
        }};
      });
      const p=await c.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
      await p.goto(base+'/web/');
      await p.evaluate(()=>{S.hosts=[];S.settings={...S.settings,onboarded:true,theme:'light'};render();ItdaFiles.network();});
      const l=p.locator('.files-layer'),local=l.getByRole('button',{name:'이 Mac',exact:true}),row=n=>l.locator('[data-index]').filter({hasText:n}).first();
      await local.waitFor({timeout:1500});
      assert(await local.isDisabled(),'Local folders must wait for the home reply');
      await p.evaluate(()=>answerHome());
      await l.locator('[data-network-sidebar] [data-side="local"]:not([disabled])').waitFor();
      await local.click();await row('note.txt').waitFor();
      assert(!await l.locator('.files-network').isVisible(),'Local entry stayed on the network landing');
      await row('Documents').dblclick();await row('report.txt').waitFor();
      await l.locator('.files-list').focus();await p.keyboard.press('Meta+t');
      await row('report.txt').waitFor();
      assert((await l.locator('[role="tab"][aria-selected="true"]').textContent()).includes('Documents'),'Local tab has a network title');
      await l.getByRole('button',{name:'홈 폴더',exact:true}).click();await row('note.txt').waitFor();
      await l.locator('[data-tab]').first().click();await row('report.txt').waitFor();
      await l.getByRole('button',{name:'모든 기기',exact:true}).click();
      await l.getByRole('button',{name:'이 Mac',exact:true}).click();await row('report.txt').waitFor();
      assert(!await l.locator('.files-current-places [data-side="remote"]').isVisible(),'Device-less local browsing exposes an inert remote place');
      await l.locator('.files-list').focus();await p.keyboard.press('Meta+ArrowUp');await row('note.txt').waitFor();
      await l.getByRole('button',{name:'뒤로',exact:true}).click();await row('report.txt').waitFor();
      await l.getByRole('button',{name:'동작',exact:true}).click();
      assert(await p.getByRole('menuitem',{name:'분할 보기',exact:true}).isDisabled(),'Device-less split would expose an empty remote pane');
      assert(!await p.getByRole('menuitem',{name:'연결 설정…',exact:true}).count(),'Device-less folders offer remote connection settings');
      await p.getByRole('menuitem',{name:'Finder에서 보기',exact:true}).click();
      const opened=(await p.evaluate(()=>testPackets)).filter(m=>m.op==='reveal_folder').at(-1);
      assert(opened?.side==='local'&&opened.path==='/Users/alex/Documents','Finder action did not open the current local folder: '+JSON.stringify(opened));
      assert(!(await p.evaluate(()=>testPackets)).some(m=>m.op==='connect'),'Local browsing tried to connect to a remote device');
      assert(!errors.length,errors.join('\n'));
      console.log('PASS: Mac fresh profile enters local folders without a device, waits for home, preserves local folders across tabs/network');
    }finally{await c.close();}
  }
  const c=await browser.newContext({viewport:{width:1200,height:800},reducedMotion:'reduce',userAgent:'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
  await c.addInitScript(()=>{
    const entry=(name,dir=false)=>({name,dir,size:100,modified:1780000000});
    const home='C:\\Users\\alex';
    const tree={'/remote':[entry('remote.txt')],[home]:[entry('Desktop',true),entry('Downloads',true),entry('Documents',true),entry('note.txt')],[home+'\\Desktop']:[entry('on-desktop.txt')],[home+'\\Downloads']:[entry('setup.exe')],'':[entry('C:',true),entry('D:',true)],'C:\\':[entry('Users',true),entry('Windows',true)],'C:\\Users':[entry('alex',true)],'D:\\':[entry('Games',true)]};
    window.testPackets=[];
    window.ipc={postMessage:raw=>{const m=JSON.parse(raw);testPackets.push(m);if(m.cmd!=='files')return;
      const r={...m,type:'files_result',ok:true};
      if(m.op==='connect'&&m.options?.method==='smb'){r.op='native';r.data={};}
      else if(m.op==='connect')r.data={path:'/remote',entries:[entry('remote.txt')],local_path:home,local_entries:tree[home]};
      else if(m.op==='local_home')r.data={path:home,entries:tree[home]};
      else if(m.op==='list')r.data={path:m.path,entries:tree[m.path]||[]};
      else return;
      setTimeout(()=>itda.recv(r),m.op==='local_home'?(window.slowHome||10):10);
    }};
  });
  const p=await c.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
  if(process.platform!=='win32')await p.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));
  await p.goto(base+'/web/');
  await p.evaluate(()=>{S.hosts=[{id:'studio',name:'Studio Mac',host:'studio.local',os:'mac',kind:'vnc',login_user:'alex',has_pw:true,files:{method:'sftp',user:'alex'}}];S.settings={...S.settings,onboarded:true,theme:'light'};render();ItdaFiles.network();});
  const layer=p.locator('.files-layer'),row=n=>layer.locator('[data-index]').filter({hasText:n}).first();
  // Home/Network and local-only folders are real tabs before any remote connection.
  const tabs=layer.locator('[data-tab]'), selected=()=>layer.getByRole('tab',{selected:true});
  await layer.locator('[data-network-sidebar] [data-bookmark="Desktop"]:not([disabled])').waitFor();
  await layer.getByRole('button',{name:'새 탭',exact:true}).click();
  assert(await tabs.count()===2,'Network new-tab button did nothing');
  await layer.locator('[data-action="explorer-home"]').click();
  await selected().focus();await p.keyboard.press('Control+t');
  assert(await tabs.count()===3,'Home Ctrl+T did nothing');
  assert((await selected().textContent()).includes('홈'),'Duplicating Home lost its page');
  await layer.locator('[data-network-sidebar] [data-bookmark="Desktop"]').click();await row('on-desktop.txt').waitFor();
  await layer.locator('.files-list').focus();await p.keyboard.press('Control+t');
  assert(await tabs.count()===4,'Device-less folder Ctrl+T did nothing');
  await row('on-desktop.txt').waitFor();
  await layer.locator('[data-network-sidebar] [data-bookmark="Downloads"]').click();await row('setup.exe').waitFor();
  await tabs.nth(2).click();await row('on-desktop.txt').waitFor();
  await layer.locator('[data-action="back"]').click();
  assert((await layer.locator('.files-network h2').textContent())==='홈','Local tab lost its own Home back target');
  await tabs.nth(0).click();
  assert((await layer.locator('.files-network h2').textContent())==='네트워크',"Network tab changed into another tab's page");
  await tabs.nth(1).click();
  assert((await layer.locator('.files-network h2').textContent())==='홈','Inactive Home tab state lost');
  // Native SMB handoff must preserve this window and restore its prior context.
  await p.evaluate(()=>{window.ITDA_FILE_HOST='__network';window.ITDA_FILE_WINDOW='fixture-window';S.hosts.push({id:'share',name:'Shared Files',host:'share.local',files:{method:'smb',share:'smb://share.local/files'}});ItdaFiles.network('home');});
  await layer.locator('[data-device="share"]').first().click();
  await p.locator('.files-setup').waitFor({state:'hidden'});
  assert(!(await p.evaluate(()=>testPackets)).some(x=>x.cmd==='files_window'&&x.close),'SMB handoff closed the entire file window');
  assert(await tabs.count()===4,'SMB handoff destroyed other tabs');
  await layer.locator('[data-network-sidebar] [data-bookmark="Desktop"]').click();await row('on-desktop.txt').waitFor();
  assert((await p.evaluate(()=>testPackets)).filter(x=>x.op==='list').at(-1)?.id==='__network','SMB handoff left an unconnected device as this tab context');
  await layer.locator('[data-action="back"]').click();
  // Adopt a remote device in this tab without destroying the other local/start tabs.
  await layer.locator('[data-device="studio"]').first().click();await row('remote.txt').waitFor();
  assert(await tabs.count()===4,'Opening a device destroyed device-less tabs');
  await tabs.nth(3).click();await row('setup.exe').waitFor();
  assert((await p.evaluate(()=>testPackets)).filter(x=>x.op==='list').at(-1)?.id==='__network','Local tab switched to the remote device');
  await tabs.nth(1).click();await row('remote.txt').waitFor();
  assert((await p.evaluate(()=>testPackets)).filter(x=>x.op==='connect'&&x.id==='studio').length===1,'Switching back reconnected remote device');
  // Remove remote tab: remaining local tabs survive and unused connection is released.
  await tabs.nth(1).locator('[data-close-tab]').click();
  assert((await p.evaluate(()=>testPackets)).some(x=>x.op==='release'&&x.id==='studio'),'Last remote tab did not release connection');
  while(await tabs.count()>1)await tabs.last().locator('[data-close-tab]').click();
  await layer.locator('.files-network h2').waitFor();
  // This computer's places work before any device is connected.
  await layer.locator('[data-network-sidebar] [data-bookmark="Desktop"]:not([disabled])').waitFor();
  assert(!await layer.locator('[data-network-sidebar] [data-side="local"]').isDisabled(),'This PC disabled without a device');
  await layer.locator('[data-network-sidebar] [data-bookmark="Desktop"]').click();
  await row('on-desktop.txt').waitFor();
  // Back returns to the Network page it came from.
  assert(!await layer.locator('[data-action="back"]').isDisabled(),'Back disabled after leaving Network');
  await layer.locator('[data-action="back"]').click();
  await layer.locator('.files-network h2').filter({hasText:'네트워크'}).waitFor();
  await layer.locator('[data-network-sidebar] [data-bookmark="Downloads"]').click();
  await row('setup.exe').waitFor();
  // This PC lists the drives; Up from a drive returns to them.
  await layer.locator('[data-network-sidebar] [data-side="local"]').click();
  await row('C:').waitFor();await row('D:').waitFor();
  await layer.locator('[data-action="more"]').click();
  assert(await p.getByRole('menuitem',{name:'탐색기에서 열기',exact:true}).isDisabled(),'Virtual This PC passed an empty path to Explorer');
  await p.keyboard.press('Escape');
  assert((await row('C:').textContent()).includes('로컬 디스크'),'Drive not labelled');
  assert(await layer.locator('[data-action="up"]').isDisabled(),'Up enabled on This PC');
  await row('C:').dblclick();await row('Users').waitFor();
  const listed=(await p.evaluate(()=>testPackets)).filter(x=>x.op==='list').at(-1);
  assert(listed.path==='C:\\'&&listed.side==='local'&&listed.id==='__network','Drive opened at '+JSON.stringify(listed));
  await row('Users').dblclick();await row('alex').waitFor();
  await layer.locator('[data-action="up"]').click();await row('Windows').waitFor();
  await layer.locator('[data-action="up"]').click();await row('D:').waitFor();
  // Back through the history: C:\ → C:\Users → … then the page before.
  await layer.locator('[data-action="back"]').click();await row('Windows').waitFor();
  // This PC is a list of drives, not a folder: nothing can be created, pasted or moved there.
  await layer.locator('[data-action="up"]').click();await row('C:').waitFor();
  for(const a of ['new','paste','cut','copy','rename','trash'])assert(await layer.locator(`[data-action="${a}"]`).isDisabled(),`${a} enabled on This PC`);
  await layer.locator('.files-list').focus();await p.keyboard.press('Control+Shift+n');
  assert(!(await p.evaluate(()=>testPackets)).some(x=>x.op==='mkdir'),'A folder was created on This PC');
  // Copying on this computer works without a device.
  await row('C:').dblclick();await row('Users').waitFor();await row('Users').click();await p.keyboard.press('Control+c');
  await layer.locator('[data-network-sidebar] [data-bookmark="Downloads"]').click();await row('setup.exe').waitFor();
  await layer.locator('.files-list').focus();await p.keyboard.press('Control+v');
  await p.locator('.files-prompt [data-choice="ok"]').click();
  {const t=(await p.evaluate(()=>testPackets)).filter(x=>x.op==='transfer').at(-1);
   assert(t&&t.id==='__network'&&t.from==='local'&&t.to==='local'&&t.source==='C:\\'&&t.destination==='C:\\Users\\alex\\Downloads'&&!t.from_id,'Device-less local copy: '+JSON.stringify(t));}
  // That copy fails: retried on this computer for what is left, after a new confirmation.
  {const t=(await p.evaluate(()=>testPackets)).filter(x=>x.op==='transfer').at(-1);
   await p.evaluate(req=>{const w=ItdaFiles.windowToken;itda.recv({type:'files_job',window:w,id:'__network',job:'job-L',req,state:'queued'});itda.recv({type:'files_job',window:w,id:'__network',job:'job-L',req,state:'failed',error:'디스크가 가득 찼습니다',completed_names:[],partial:{name:'Users',dir:true},source_closed:false});},t.req);
   await layer.getByRole('button',{name:'다시 시도…'}).click();
   const d=p.getByRole('dialog',{name:'남은 파일 다시 복사',exact:true});
   assert((await d.textContent()).includes('‘Users’ 폴더는 일부만'),'Local retry lacks the partial folder note');
   await d.getByRole('button',{name:'다시 복사',exact:true}).click();
   const r=(await p.evaluate(()=>testPackets)).filter(x=>x.op==='transfer').at(-1);
   assert(r.id==='__network'&&r.from==='local'&&r.to==='local'&&JSON.stringify(r.names)==='["Users"]'&&r.policy==='replace'&&r.req!==t.req,'Local retry: '+JSON.stringify(r));}
  // A long path keeps the current folder in view in a narrow window (leading crumbs are cut).
  await p.setViewportSize({width:480,height:700});
  {const box=await layer.locator('.files-explorer-address').boundingBox(),last=await layer.locator('.files-explorer-address .files-breadcrumbs button').last().boundingBox();
   assert((await layer.locator('.files-explorer-address .files-breadcrumbs button').last().textContent()).includes('Downloads')&&last.x>=box.x&&last.x+last.width<=box.x+box.width+1,'Current folder hidden in the address bar: '+JSON.stringify({box,last}));}
  // A click on a crumb of that long path opens that folder (the crumbs must not shift under the pointer).
  await layer.locator('.files-explorer-address .files-breadcrumbs button').filter({hasText:'alex'}).click();
  await row('Downloads').waitFor();
  {const l=(await p.evaluate(()=>testPackets)).filter(x=>x.op==='list').at(-1);assert(l.path==='C:\\Users\\alex','Crumb click opened '+JSON.stringify(l));}
  await p.setViewportSize({width:1200,height:800});
  assert(!errors.length,errors.join('\n'));
  // A slow home answer must not pull an already opened This PC back to the home folder.
  {
    const q=await c.newPage();
    if(process.platform!=='win32')await q.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));
    await q.goto(base+'/web/');
    await q.evaluate(()=>{window.slowHome=700;S.hosts=[];S.settings={...S.settings,onboarded:true,theme:'light'};render();ItdaFiles.network();});
    const l=q.locator('.files-layer');
    await l.locator('[data-network-sidebar] [data-side="local"]').click();
    await l.locator('[data-index]').filter({hasText:'C:'}).first().waitFor();
    await q.waitForTimeout(1000);
    assert(await l.locator('[data-index]').filter({hasText:'D:'}).count()===1&&await l.locator('[data-index]').filter({hasText:'note.txt'}).count()===0,'Late home answer replaced This PC');
    await l.locator('[data-network-sidebar] [data-bookmark="Desktop"]:not([disabled])').waitFor();
    await q.close();
  }
  // Home discovery can finish after a different tab adopted a remote device.
  {
    const q=await c.newPage();
    if(process.platform!=='win32')await q.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));
    await q.goto(base+'/web/');
    await q.evaluate(()=>{window.slowHome=700;S.hosts=[{id:'studio',name:'Studio Mac',host:'studio.local',os:'mac',login_user:'alex',has_pw:true,files:{method:'sftp',user:'alex'}}];S.settings={...S.settings,onboarded:true};render();ItdaFiles.network();});
    const l=q.locator('.files-layer');
    await l.getByRole('button',{name:'새 탭',exact:true}).click();
    await l.locator('[data-device="studio"]').first().click();
    await l.locator('[data-index]').filter({hasText:'remote.txt'}).waitFor();
    await q.waitForTimeout(900);
    await l.locator('[data-tab]').first().click();
    await l.locator('[data-network-sidebar] [data-bookmark="Desktop"]:not([disabled])').waitFor({timeout:2000});
    await q.close();
  }
  // Each tab returns to its own page.
  {
    const q=await c.newPage();
    if(process.platform!=='win32')await q.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));
    await q.goto(base+'/web/');
    await q.evaluate(()=>{S.hosts=[{id:'studio',name:'Studio Mac',host:'studio.local',os:'mac',kind:'vnc',login_user:'alex',has_pw:true,files:{method:'sftp',user:'alex'}}];S.settings={...S.settings,onboarded:true,theme:'light'};render();ItdaFiles.launch(S.hosts[0]);});
    const l=q.locator('.files-layer'),r=n=>l.locator('[data-index]').filter({hasText:n}).first();
    await r('remote.txt').waitFor();
    await l.locator('[data-action="network"]').click();
    await l.locator('[data-network-sidebar] [data-bookmark="Desktop"]').click();await r('on-desktop.txt').waitFor();
    await l.locator('.files-list').focus();await q.keyboard.press('Control+t');
    await l.locator('[data-action="explorer-home"]').click();
    await l.locator('[data-network-sidebar] [data-bookmark="Downloads"]').click();await r('setup.exe').waitFor();
    await l.locator('[data-tab]').first().click();await r('on-desktop.txt').waitFor();
    await l.locator('[data-action="back"]').click();
    await l.locator('.files-network h2').waitFor();
    assert((await l.locator('.files-network h2').textContent())==='네트워크','Another tab changed this tab\'s Back');
    await q.close();
  }
  await c.close();
  console.log('PASS: device-less file window: places, This PC drives (read-only), Up/Back, slow home answer, per-tab Back, device-less copy, start/local tabs, SMB handoff and delayed home across devices');
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
