// Explorer UI regression on the real frontend. In-memory IPC fixture; no user files or SSH access.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const out=process.env.SCREENSHOT_DIR||path.join(os.tmpdir(),'itda-explorer-qa');fs.mkdirSync(out,{recursive:true});
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
 for(const platform of ['windows','mac']){
  const c=await browser.newContext({viewport:{width:1200,height:800},reducedMotion:'reduce',userAgent:platform==='mac'?'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)':'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
  await c.addInitScript(()=>{
   const entry=(name,dir=false,size=100)=>({name,dir,size,modified:1780000000});
   const home='/Users/alex',tree={ [home]:[entry('Documents',true),entry('Downloads',true),entry('Notes.txt'),entry('Zebra.pdf',false,2000)], [home+'/Documents']:[entry('Alpha.txt'),entry('Beta.md')], [home+'/Downloads']:[entry('Download.txt')] };
   window.testPackets=[];
   window.ipc={postMessage:raw=>{const m=JSON.parse(raw);testPackets.push(m);if(m.cmd!=='files')return;
    const result={...m,type:'files_result',ok:true};
    if(m.op==='connect')result.data={path:home,entries:tree[home],local_path:home,local_entries:tree[home],rsync:true};
    else if(m.op==='list')result.data={path:m.path,entries:tree[m.path]||[]};
    else if(m.op==='preview')result.data={mime:'text/plain',text:'Preview fixture'};
    else return;
    queueMicrotask(()=>itda.recv(result));
   }};
  });
  const p=await c.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
  if(platform==='windows'&&process.platform!=='win32')await p.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));
  await p.goto(base+'/web/');
  await p.evaluate(()=>{S.hosts=[{id:'test',name:'Studio Mac',host:'studio.local',os:'mac',kind:'vnc',login_user:'alex',has_pw:true,files:{method:'sftp',user:'alex'}}];S.settings={...S.settings,onboarded:true,theme:'light'};render();ItdaFiles.launch(S.hosts[0]);});
  const layer=p.locator('.files-layer');await layer.locator('[data-index]').first().waitFor();
  if(platform==='windows'){
   const boxes=await p.evaluate(()=>['.files-tabs','.files-title','.files-tools','.files-panes'].map(s=>document.querySelector(s).getBoundingClientRect().top));
   assert(boxes.every((y,i)=>!i||y>boxes[i-1]),'Explorer row order is wrong');
   assert((await layer.locator('th').allTextContents()).join('|').includes('이름 ▴|수정한 날짜|종류|크기'),'Explorer column order wrong');
   await layer.locator('[data-action="view"]').click();await p.getByRole('menuitemradio',{name:'큰 아이콘',exact:true}).click();assert(await layer.locator('.files-grid').count()===1,'Icon view failed');
   await layer.locator('[data-action="view"]').click();await p.getByRole('menuitemradio',{name:'자세히',exact:true}).click();
   await layer.locator('[data-action="sort"]').click();await p.getByRole('menuitemradio',{name:'크기',exact:true}).click();
   await layer.locator('[data-action="sort"]').click();await p.getByRole('menuitemradio',{name:'내림차순',exact:true}).click();
   assert((await layer.locator('tbody tr .files-name').allTextContents()).map(x=>x.trim()).at(-2).endsWith('Zebra.pdf'),'Sort by size descending failed: '+JSON.stringify(await layer.locator('tbody tr .files-name').allTextContents()));
   await layer.locator('tbody tr').filter({hasText:'Notes.txt'}).click();assert(!await layer.locator('[data-action="copy"]').isDisabled(),'Copy did not enable');
   await layer.locator('[data-action="copy"]').click();assert(!await layer.locator('[data-action="paste"]').isDisabled(),'Paste did not enable');
   await layer.locator('[data-action="details"]').click();assert((await layer.locator('.files-details').textContent()).includes('Notes.txt'),'Details missing selected file');
   await layer.locator('[data-action="close-preview"]').click();
   await layer.locator('.files-list').focus();await p.keyboard.press('Alt+p');await layer.locator('.files-preview-content pre').waitFor();assert(await layer.locator('.files-preview-content pre').textContent()==='Preview fixture','Preview failed');
   await layer.locator('[data-action="close-preview"]').click();
   await p.keyboard.press('Control+l');assert(await layer.locator('.files-explorer-address').evaluate(e=>e.classList.contains('editing')),'Ctrl+L did not edit address');
   await layer.locator('input[data-path]').fill('/Users/alex/Documents');await p.keyboard.press('Enter');await layer.locator('tbody tr').filter({hasText:'Alpha.txt'}).waitFor();
   await p.keyboard.press('Alt+ArrowLeft');await layer.locator('tbody tr').filter({hasText:'Notes.txt'}).waitFor();
   await p.keyboard.press('Alt+ArrowRight');await layer.locator('tbody tr').filter({hasText:'Alpha.txt'}).waitFor();
   await p.keyboard.press('Alt+ArrowUp');await layer.locator('tbody tr').filter({hasText:'Notes.txt'}).waitFor();
   await p.evaluate(()=>document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'ㅣ',code:'KeyL',ctrlKey:true,bubbles:true})));assert(await layer.locator('.files-explorer-address').evaluate(e=>e.classList.contains('editing')),'Hangul Ctrl+L failed');
   await p.keyboard.press('F4');await p.keyboard.press('Escape');assert(!await layer.locator('.files-explorer-address').evaluate(e=>e.classList.contains('editing')),'Escape did not cancel address editing');
   await p.keyboard.press('Control+t');assert(await layer.locator('[data-tab]').count()===2,'New tab failed');await p.keyboard.press('Control+w');assert(await layer.locator('[data-tab]').count()===1,'Close tab failed');
   await layer.locator('[data-action="new"]').click();await p.getByRole('menuitem',{name:'폴더',exact:true}).click();
   for(const shortcut of ['F4','Control+l','Alt+p']){await p.keyboard.press(shortcut);assert(await p.locator('.files-prompt').evaluate(e=>e.contains(document.activeElement)),`${shortcut} stole prompt focus`);assert(!await layer.locator('.files-explorer-address').evaluate(e=>e.classList.contains('editing')),'Address edited behind prompt');}
   await p.getByRole('button',{name:'취소',exact:true}).click();
   await p.keyboard.press('Control+t');await layer.locator('[data-action="explorer-home"]').click();await p.keyboard.press('Control+w');assert(await layer.locator('[data-tab]').count()===1,'Home Ctrl+W closed workspace');
   await layer.locator('[data-action="explorer-home"]').click();assert(await layer.locator('.files-network h2').textContent()==='홈','Home failed');
   await layer.locator('[data-network-sidebar] [data-bookmark="Documents"]').click();await layer.locator('tbody tr').filter({hasText:'Alpha.txt'}).waitFor();
   await layer.locator('[data-action="explorer-home"]').click();await layer.locator('[data-network-sidebar] [data-side="local"]').click();assert(await layer.locator('.files-list').count()===1,'Home to This PC failed');
   await layer.locator('[data-action="network"]').click();assert(await layer.locator('.files-network h2').textContent()==='네트워크','Network failed');
   await layer.locator('[data-device="test"]').first().click();await layer.locator('tbody tr').filter({hasText:'Notes.txt'}).waitFor();
   for(const width of [800,1200]){await p.setViewportSize({width,height:800});const bounds=await layer.locator('.files-tools').evaluate(e=>({scroll:e.scrollWidth,width:e.clientWidth}));assert(bounds.scroll<=bounds.width+2,`Command bar overflows at ${width}`);await p.screenshot({path:path.join(out,`explorer-${width}.png`)});}
   await p.emulateMedia({colorScheme:'dark'});await p.evaluate(()=>{S.settings.theme='dark';applyTheme();});await p.screenshot({path:path.join(out,'explorer-dark.png')});
  }else{
   assert(!await layer.locator('.files-explorer-address').count(),'Mac layout was replaced');
   await layer.locator('[data-mode="columns"]').click();assert(await layer.locator('.files-columns').count()===1,'Mac columns regressed');
   await layer.locator('[data-mode="list"]').click();await layer.locator('tbody tr').filter({hasText:'Notes.txt'}).click();await p.keyboard.press('Meta+c');assert(!await layer.locator('[data-action="paste"]').isDisabled(),'Mac copy regressed');
   await p.screenshot({path:path.join(out,'finder.png')});
  }
  assert(!errors.length,errors.join('\n'));await c.close();
 }
 // Windows file windows draw their own title bar: tabs, then minimize/maximize/close; the
 // empty tab row moves the window. Narrow windows keep every command reachable.
 {
  const c=await browser.newContext({viewport:{width:1000,height:700},reducedMotion:'reduce',userAgent:'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
  await c.addInitScript(()=>{window.ITDA_WINDOW_CHROME=true;window.ITDA_FILE_WINDOW='native-1';
   const entry=(name,dir=false)=>({name,dir,size:100,modified:1780000000});const home='/Users/alex',tree={[home]:[entry('Documents',true),entry('Notes.txt')]};
   window.testPackets=[];window.ipc={postMessage:raw=>{const m=JSON.parse(raw);testPackets.push(m);if(m.cmd!=='files')return;const result={...m,type:'files_result',ok:true};
    if(m.op==='connect')result.data={path:home,entries:tree[home],local_path:home,local_entries:tree[home],rsync:true};else if(m.op==='list')result.data={path:m.path,entries:tree[m.path]||[]};else return;queueMicrotask(()=>itda.recv(result));}};});
  const p=await c.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
  if(process.platform!=='win32')await p.route('**/icons.js',route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync(path.join(root,'web/icons.js'),'utf8').replace('if (/Windows/.test(navigator.userAgent)) return;','')}));
  await p.goto(base+'/web/');
  await p.evaluate(()=>{S.hosts=[{id:'test',name:'Studio Mac',host:'studio.local',os:'mac',kind:'vnc',login_user:'alex',has_pw:true,files:{method:'sftp',user:'alex'}}];S.settings={...S.settings,onboarded:true,theme:'light'};render();ItdaFiles.launch(S.hosts[0]);});
  const layer=p.locator('.files-layer');await layer.locator('[data-index]').first().waitFor();
  const controls=()=>p.evaluate(()=>testPackets.filter(m=>m.cmd==='file_window_control').map(m=>`${m.window}:${m.op}`));
  const caption=layer.locator('.files-tabs .files-caption');
  assert(await caption.count()===1,'No title-bar buttons in the tab row');
  const [tabs,cap]=await Promise.all([layer.locator('.files-tabs').boundingBox(),caption.boundingBox()]);
  assert(Math.abs(cap.x+cap.width-1000)<=1&&cap.y<=tabs.y+1,'Title-bar buttons not at the top right');
  for(const op of ['minimize','maximize','close'])await caption.locator(`[data-window="${op}"]`).click();
  const bar=await layer.locator('.files-tabs').boundingBox();
  await p.mouse.move(bar.x+bar.width-200,bar.y+10);await p.mouse.down();await p.mouse.up();
  await p.mouse.dblclick(bar.x+bar.width-200,bar.y+10);
  const ops=await controls();
  assert(JSON.stringify(ops.slice(0,4))===JSON.stringify(['native-1:minimize','native-1:maximize','native-1:close','native-1:drag'])&&ops.includes('native-1:maximize'),'Title-bar actions: '+JSON.stringify(ops));
  const before=ops.length;await layer.locator('.files-tab').first().click();
  assert((await controls()).filter(x=>x.endsWith(':drag')).length===ops.filter(x=>x.endsWith(':drag')).length,'Clicking a tab moved the window');
  await p.evaluate(()=>itdaWindowState({maximized:true}));
  assert(await caption.locator('[data-window="maximize"]').getAttribute('aria-label')==='이전 크기로','Maximized state not shown');
  // Narrow: every command still inside the window, the navigation pane gives way.
  for(const w of [740,560,440]){
   await p.setViewportSize({width:w,height:520});await p.waitForTimeout(50);
   const out=await p.evaluate(()=>[...document.querySelectorAll('.files-tools .btn, .files-title .btn, .files-caption button')].filter(b=>b.offsetParent&&b.getBoundingClientRect().right>innerWidth+1).map(b=>b.dataset.action||b.dataset.window));
   assert(!out.length,`${w}px: controls outside the window ${JSON.stringify(out)}`);
   if(w<=600)assert(await layer.locator('.files-sidebar').isHidden(),`${w}px: navigation pane still squeezing the list`);
  }
  await p.screenshot({path:path.join(out,'windows-title-bar-narrow.png')});
  assert(!errors.length,errors.join('\n'));await c.close();
 }
 console.log(`PASS: Explorer layout, address/history, tabs, sort/view, copy state, preview/details, home/network, Mac regression, title-bar tabs and narrow windows (${process.platform})`);
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
