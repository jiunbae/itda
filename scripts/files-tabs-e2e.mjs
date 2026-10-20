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
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(value,msg)=>{if(!value)throw Error(msg);};
try{
 for(const platform of ['mac','windows']){
  const c=await browser.newContext({viewport:{width:1200,height:800},reducedMotion:'reduce',userAgent:platform==='mac'?'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)':'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
  await c.addInitScript(()=>{
   const entry=(name,dir=false)=>({name,dir,size:100,modified:1780000000});
   // studio and build answer; slow never finishes connecting (used for cancel).
   const homes={studio:'/Users/alex',build:'/home/dev'};
   const tree={'/Users/alex':[entry('Docs',true),entry('Alpha-A.txt')],'/Users/alex/Docs':[entry('Inside-A.txt')],'/home/dev':[entry('Beta-B.txt')],'/local':[entry('Local.txt')]};
   window.testPackets=[];
   window.ipc={postMessage:raw=>{const m=JSON.parse(raw);if(m.cmd!=='files')return;testPackets.push({id:m.id,op:m.op,side:m.side,path:m.path});
    const result={...m,type:'files_result',ok:true};
    if(m.op==='connect'){if(!homes[m.id])return;result.data={path:homes[m.id],entries:tree[homes[m.id]],local_path:'/local',local_entries:tree['/local'],rsync:true};}
    else if(m.op==='list')result.data={path:m.path,entries:tree[m.path]||[]};
    else return;
    setTimeout(()=>itda.recv(result),10);
   }};
  });
  const p=await c.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
  if(platform==='windows'&&process.platform!=='win32')await p.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));
  await p.goto(base+'/web/');
  await p.evaluate(()=>{const files={method:'sftp',user:'alex'};S.hosts=[{id:'studio',name:'Studio Mac',host:'studio.local',os:'mac',kind:'vnc',login_user:'alex',has_pw:true,files},{id:'build',name:'Build Server',host:'build.local',os:'linux',kind:'vnc',login_user:'dev',has_pw:true,files},{id:'slow',name:'Slow Box',host:'slow.local',os:'linux',kind:'vnc',login_user:'dev',has_pw:true,files}];S.settings={...S.settings,onboarded:true,theme:'light'};render();ItdaFiles.launch(S.hosts[0]);});
  const layer=p.locator('.files-layer'),row=name=>layer.locator('[data-index]').filter({hasText:name}).first();
  const packets=()=>p.evaluate(()=>testPackets),mod=platform==='mac'?'Meta':'Control';
  const count=async(id,op)=>(await packets()).filter(x=>x.id===id&&x.op===op).length;
  await row('Alpha-A.txt').waitFor();
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+t`);
  assert(await layer.locator('[data-tab]').count()===2,'New tab failed');
  // Open another device in the new tab.
  await layer.locator('[data-device="build"]').first().click();
  await row('Beta-B.txt').waitFor();
  assert(await layer.locator('[data-tab]').count()===2,'Opening a device replaced the tabs');
  assert(await count('studio','release')===0&&await count('studio','disconnect')===0,'First device was released when another tab opened a device');
  assert((await layer.locator('[data-tab]').nth(0).getAttribute('title')).startsWith('Studio Mac'),'First tab lost its device');
  assert((await layer.locator('[data-tab]').nth(1).getAttribute('title')).startsWith('Build Server'),'Second tab is not on the new device');
  // Back to the first tab: same device, same connection, no reconnect.
  await layer.locator('[data-tab]').nth(0).click();
  await row('Alpha-A.txt').waitFor();
  assert(await count('studio','connect')===1,'Switching tabs reconnected the first device');
  const last=(await packets()).filter(x=>x.op==='list').at(-1);
  assert(last.id==='studio','Tab switch listed the wrong device: '+JSON.stringify(last));
  await row('Docs').dblclick();await row('Inside-A.txt').waitFor();
  // A remote selection on one device must not paste into another device.
  await row('Inside-A.txt').click();await p.keyboard.press(`${mod}+c`);
  await layer.locator('[data-tab]').nth(1).click();await row('Beta-B.txt').waitFor();
  await layer.locator('.files-list').focus();await p.keyboard.press(`${mod}+v`);
  assert((await layer.locator('.files-error').textContent()).includes('다른 기기'),'Cross-device paste was not refused');
  assert(!(await packets()).some(x=>x.op==='transfer'),'Cross-device paste started a transfer');
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
  assert(!errors.length,errors.join('\n'));
  await c.close();
 }
 console.log(`PASS: tabs on different devices keep their connections, cancel restores, close releases unused devices (${process.platform})`);
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
