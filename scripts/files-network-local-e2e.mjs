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
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(value,msg)=>{if(!value)throw Error(msg);};
try{
  const c=await browser.newContext({viewport:{width:1200,height:800},reducedMotion:'reduce',userAgent:'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
  await c.addInitScript(()=>{
    const entry=(name,dir=false)=>({name,dir,size:100,modified:1780000000});
    const home='C:\\Users\\alex';
    const tree={[home]:[entry('Desktop',true),entry('Downloads',true),entry('Documents',true),entry('note.txt')],[home+'\\Desktop']:[entry('on-desktop.txt')],[home+'\\Downloads']:[entry('setup.exe')],'':[entry('C:',true),entry('D:',true)],'C:\\':[entry('Users',true),entry('Windows',true)],'C:\\Users':[entry('alex',true)],'D:\\':[entry('Games',true)]};
    window.testPackets=[];
    window.ipc={postMessage:raw=>{const m=JSON.parse(raw);if(m.cmd!=='files')return;testPackets.push(m);
      const r={...m,type:'files_result',ok:true};
      if(m.op==='connect')r.data={path:'/remote',entries:[entry('remote.txt')],local_path:home,local_entries:tree[home]};
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
  // Copying without a device is explained instead of silently dropped.
  await row('C:').dblclick();await row('Users').waitFor();await row('Users').click();await p.keyboard.press('Control+c');
  await layer.locator('[data-network-sidebar] [data-bookmark="Downloads"]').click();await row('setup.exe').waitFor();
  await layer.locator('.files-list').focus();await p.keyboard.press('Control+v');
  assert((await layer.locator('.files-error').textContent()).includes('기기를 고르지 않은 창'),'Device-less copy not explained');
  assert(!(await p.evaluate(()=>testPackets)).some(x=>x.op==='transfer'),'Device-less copy sent a transfer');
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
  console.log('PASS: device-less file window: places, This PC drives (read-only), Up/Back, slow home answer, per-tab Back, device-less copy explained');
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
