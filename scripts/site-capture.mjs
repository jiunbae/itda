// Capture the real product frontend with fictional examples. No accounts, remote hosts or native IPC.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const out=path.join(root,'docs/img');
const hosts=[
  {id:'a1',name:'Studio Mac',os:'mac',host:'studio.local',user:'alex',favorite:true,group:'작업 공간',last_used:1780000000,remote_size:'2560×1600'},
  {id:'b2',name:'Office PC',os:'windows',host:'office.local',user:'',favorite:true,group:'작업 공간',last_used:1779999800,remote_size:'1920×1080'},
  {id:'d4',name:'MacBook Air',os:'mac',host:'air.local',user:'alex',favorite:true,group:'개인',last_used:1779998000,remote_size:'2560×1664'},
  {id:'e5',name:'Home PC',os:'windows',host:'home.local',user:'',favorite:false,group:'개인',last_used:1779997000,remote_size:'1920×1080'},
  {id:'f6',name:'Build Server',os:'linux',host:'build.local',user:'alex',favorite:false,group:'개발',last_used:1779996000,remote_size:'1920×1080'},
  {id:'c3',name:'Dev Server',os:'linux',host:'dev.local',user:'alex',favorite:false,group:'개발',last_used:1779999000,remote_size:'1920×1080'},
].map(h=>({...h,label:h.name,kind:'vnc',has_pw:true,login_user:h.user,account:'',ssh:{on:h.os==='linux'},files:{method:'sftp',user:'alex',path:'/Users/alex/Documents'},opts:{}}));
const entries=[['Brand assets',true,0],['Projects',true,0],['Screenshots',true,0],['Design notes.md',false,2850],['Project brief.pdf',false,182400],['Release checklist.txt',false,1260],['Roadmap.md',false,5460]].map(([name,dir,size])=>({name,dir,size,modified:1780000000,link:false}));
const sockets=new Set();
const server=http.createServer((req,res)=>{
  if(req.url.startsWith('/api/')){res.setHeader('Content-Type','application/json');return res.end(JSON.stringify(req.url.endsWith('/me')?{local:true}:req.url.endsWith('/hosts')?hosts.map(h=>({...h,ssh:h.ssh.on})):[]));}
  let name=new URL(req.url,'http://localhost').pathname;
  if(name==='/viewer/')name='/web/viewer/index.html';
  else if(name.startsWith('/viewer/'))name='/web'+name;
  else if(name==='/web/')name='/web/index.html';
  const file=name.endsWith('/icon.svg')?path.join(root,'assets/itda.svg'):name.endsWith('/icon-192.png')?path.join(root,'assets/itda-192.png'):path.resolve(root,'.'+name);
  if(!file.startsWith(root+path.sep)||!fs.existsSync(file)||!fs.statSync(file).isFile()){res.writeHead(404);return res.end();}
  res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':name.endsWith('.png')?'image/png':name.endsWith('.svg')?'image/svg+xml':'text/html');res.end(fs.readFileSync(file));
});
server.on('connection',s=>{sockets.add(s);s.on('close',()=>sockets.delete(s));});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}`;
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
try{
  const context=await browser.newContext({viewport:{width:1440,height:900},deviceScaleFactor:1,locale:'ko-KR',timezoneId:'Asia/Seoul',userAgent:'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'});
  await context.clock.install({time:new Date('2026-10-20T06:00:00Z')});
  await context.addInitScript(({hosts,entries})=>{
    window.fixtureHosts=hosts;window.fixtureEntries=entries;
    window.ipc={postMessage:raw=>{
      const m=JSON.parse(raw);
      if(m.cmd==='files'&&m.op==='connect')queueMicrotask(()=>itda.recv({type:'files_result',id:m.id,window:m.window,op:'connect',ok:true,data:{path:'/Users/alex/Documents',entries,local_path:'/Users/alex',local_entries:entries,rsync:true}}));
    }};
  },{hosts,entries});
  const page=await context.newPage();
  const prepare=async(theme='light')=>{
    await page.goto(base+'/web/');
    await page.evaluate(({hosts,theme})=>{
      S.hosts=hosts;S.accounts=[{id:'sample',name:'작업 계정',user:'alex',has_pw:true,uses:2}];S.version='';
      S.settings={...S.settings,onboarded:true,theme,view:'grid'};S.online={a1:true,b2:true,c3:true,d4:true,e5:true,f6:true};S.computer='My Mac';S.sysDark=theme==='dark';
      S.hosts.forEach(h=>h.last_used=Math.floor(Date.now()/1000)-120);applyTheme();render();
    },{hosts,theme});
    await page.evaluate(()=>document.fonts.ready);
  };
  for(const theme of ['light','dark']){
    await prepare(theme);await page.screenshot({path:path.join(out,`home-${theme}.png`)});
    await page.evaluate(()=>ItdaFiles.launch(S.hosts[0]));await page.locator('.files-list [data-index]').first().waitFor();
    await page.screenshot({path:path.join(out,`files-${theme}.png`)});
  }
  await prepare('light');
  await page.evaluate(()=>{S.found=[{name:'Design Mac',host:'design.local',hostname:'design.local',ip:'192.0.2.21',os:'mac',itda:true,share:true},{name:'Meeting PC',host:'meeting.local',hostname:'meeting.local',ip:'192.0.2.22',os:'windows',itda:true,share:true}];S.tailnet={running:true,me:{name:'My Mac'},peers:[{name:'Lab Mac',dns:'lab.example.ts.net',ip:'100.64.0.20',os:'macOS',online:true}]};S.page='nearby';render();});
  if(await page.locator('.found').count()!==3)throw Error('Nearby fixture must show 3 addable devices');
  await page.screenshot({path:path.join(out,'nearby.png')});
  await prepare('light');await page.evaluate(()=>openEdit(S.hosts[0]));await page.locator('#modals [data-key="edit"]').waitFor();await page.screenshot({path:path.join(out,'editor.png')});
  await prepare('light');
  await page.evaluate(qr=>{S.settings.web={on:true,port:8780};S.web={running:true,pin:'123456',urls:[{url:'http://192.0.2.10:8780',kind:'lan'}],qr};S.share={local_name:'my-mac.local',user:'alex'};S.page='share';render();},fs.readFileSync(path.join(root,'scripts/fixtures/site-qr.svg'),'utf8'));
  await page.screenshot({path:path.join(out,'share.png')});
  // Landing detail views: actual app zoom, shorter windows, and explicit mobile crops.
  await page.setViewportSize({width:1100,height:580});
  for(const theme of ['light','dark']){
    await page.setViewportSize({width:1440,height:700});
    await prepare(theme);await page.evaluate(()=>{document.documentElement.style.zoom='1.25';});
    const grid=await page.locator('.grid').boundingBox(),card=await page.locator('.grid .card').first().boundingBox();
    await page.screenshot({path:path.join(out,`home-focus-${theme}.png`),clip:{x:grid.x,y:grid.y,width:grid.width,height:card.height}});
    await page.screenshot({path:path.join(out,`home-mobile-${theme}.png`),clip:card});
    await page.screenshot({path:path.join(out,`home-medium-${theme}.png`),clip:{x:grid.x,y:grid.y,width:card.width*2+20,height:card.height}});
    await page.setViewportSize({width:1100,height:580});
    await page.evaluate(()=>ItdaFiles.launch(S.hosts[0]));await page.locator('.files-list [data-index]').first().waitFor();
    await page.screenshot({path:path.join(out,`files-focus-${theme}.png`)});
    const pane=await page.locator('.files-panes').boundingBox();
    await page.screenshot({path:path.join(out,`files-medium-${theme}.png`),clip:{x:pane.x,y:0,width:pane.width,height:580}});
    await page.screenshot({path:path.join(out,`files-mobile-${theme}.png`),clip:{x:pane.x,y:0,width:400,height:450}});
  }
  await prepare('light');await page.evaluate(()=>{document.documentElement.style.zoom='1.25';S.found=[{name:'Design Mac',host:'design.local',hostname:'design.local',ip:'192.0.2.21',os:'mac',itda:true,share:true},{name:'Meeting PC',host:'meeting.local',hostname:'meeting.local',ip:'192.0.2.22',os:'windows',itda:true,share:true}];S.tailnet={running:true,me:{name:'My Mac'},peers:[{name:'Lab Mac',dns:'lab.example.ts.net',ip:'100.64.0.20',os:'macOS',online:true}]};S.page='nearby';render();});
  await page.locator('.page').screenshot({path:path.join(out,'nearby-focus.png')});
  await page.setViewportSize({width:530,height:720});
  await page.locator('.page').screenshot({path:path.join(out,'nearby-mobile.png')});
  await page.setViewportSize({width:1100,height:580});await prepare('light');
  await page.evaluate(qr=>{document.documentElement.style.zoom='1.25';S.settings.web={on:true,port:8780};S.web={running:true,pin:'123456',urls:[{url:'http://192.0.2.10:8780',kind:'lan'}],qr};S.share={local_name:'my-mac.local',user:'alex'};S.page='share';render();},fs.readFileSync(path.join(root,'scripts/fixtures/site-qr.svg'),'utf8'));
  await page.locator('.web-panel').screenshot({path:path.join(out,'share-focus.png')});
  await page.setViewportSize({width:530,height:720});
  await page.locator('.web-panel').screenshot({path:path.join(out,'share-mobile.png')});
  await context.close();
  for(const [name,width,height]of [['phone-list',390,720]]){
    const p=await browser.newPage({viewport:{width,height},deviceScaleFactor:2,colorScheme:'light',locale:'ko-KR',timezoneId:'Asia/Seoul',isMobile:true,hasTouch:true});
    await p.goto(base+'/viewer/');await p.locator('.host[data-id="a1"]').waitFor();await p.evaluate(()=>document.fonts.ready);
    await p.screenshot({path:path.join(out,name+'.png')});await p.close();
  }
  const og=await browser.newPage({viewport:{width:1200,height:630},colorScheme:'dark'});
  await og.goto(base+'/docs/index.html?lang=ko');
  const stage=await og.locator('.stage').innerHTML();
  await og.setContent(`<!doctype html><html lang="ko" data-lang="ko"><head><base href="${base}/docs/"><link rel="stylesheet" href="style.css"><style>body{width:1200px;height:630px;background:radial-gradient(ellipse at 70% 55%,#32305c,#100f1d 70%);display:grid;grid-template-columns:390px 1fr;gap:30px;align-items:center;padding:48px;overflow:hidden}.brand{font-size:28px;margin-bottom:36px}.brand img{width:44px;height:44px}h1{font-size:52px;line-height:1.18;letter-spacing:-.04em;margin:0 0 20px}.og-copy p{font-size:19px;color:#b7b4d3;line-height:1.7}.og-devices{width:640px}.og-devices .device-scene{padding-bottom:24px}.og-devices figcaption{display:none}.og-copy small{color:#a4a1ff;font-size:16px}.device-scene>.device-phone{width:22%}</style></head><body><div class="og-copy"><div class="brand"><img src="favicon.svg" alt="">Itda</div><h1>내 기기를,<br><span class="grad">잇다.</span></h1><p>Windows · Mac · 휴대폰<br>화면 공유와 파일 브라우저</p><small>무료 · 오픈 소스</small></div><div class="og-devices">${stage}</div></body></html>`);
  await og.evaluate(async()=>{await document.fonts.ready;await Promise.all([...document.images].map(i=>i.decode()));});
  await og.screenshot({path:path.join(out,'og.png')});await og.close();
  console.log('Captured product frontend with fictional data: desktop, files, nearby, editor, phone and readable details');
}finally{await browser.close();for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));}
