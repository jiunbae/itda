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
      if(m.op==='local_home')r.data={path:home,entries:tree[home]};
      else if(m.op==='list')r.data={path:m.path,entries:tree[m.path]||[]};
      else return;
      setTimeout(()=>itda.recv(r),10);
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
  assert(!errors.length,errors.join('\n'));
  await c.close();
  console.log('PASS: device-less file window: places, This PC drives, Up to This PC, Back to Network');
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
