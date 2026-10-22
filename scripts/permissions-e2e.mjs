// Real permission guide + shared remote panel, with deterministic native IPC responses.
// No system settings, TCC records, remote files or user input are changed.
import http from 'node:http';import fs from 'node:fs';import path from 'node:path';import os from 'node:os';import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../web'),pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const server=http.createServer((req,res)=>{const name=new URL(req.url,'http://localhost').pathname,p=name==='/icon.svg'?path.resolve(root,'../assets/itda.svg'):path.resolve(root,'.'+name);if((name!=='/icon.svg'&&!p.startsWith(root+path.sep))||!fs.existsSync(p)||!fs.statSync(p).isFile()){res.writeHead(404);return res.end();}res.setHeader('Content-Type',p.endsWith('.js')?'text/javascript':p.endsWith('.css')?'text/css':p.endsWith('.svg')?'image/svg+xml':'text/html');res.end(fs.readFileSync(p));});await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});const assert=(v,m)=>{if(!v)throw Error(m);},out=process.env.SCREENSHOT_DIR||path.join(os.tmpdir(),'itda-permission-qa');fs.mkdirSync(out,{recursive:true});
try{
 const p=await browser.newPage({viewport:{width:420,height:720}}),errors=[];p.on('pageerror',e=>errors.push(e.message));await p.clock.install();
 await p.addInitScript(()=>{window.sent=[];window.permissionState={screen:false,access:false,app_path:'/Applications/Itda.app',requested:'screen',requested_seq:1};window.ipc={postMessage:raw=>{const m=JSON.parse(raw);sent.push(m);if(m.cmd==='permission_request'){permissionState.requested=m.permission;permissionState.requested_seq++;}if(m.cmd==='permission_status'||m.cmd==='permission_request')queueMicrotask(()=>itda.recv({type:'permissions',...permissionState}));}};});
 await p.goto(`http://127.0.0.1:${server.address().port}/permissions.html`);await p.getByText('0/2 완료',{exact:false}).waitFor();
 const initialReads=await p.evaluate(()=>sent.filter(m=>m.cmd==='permission_status').length);await p.clock.fastForward(1600);assert(await p.evaluate(()=>sent.filter(m=>m.cmd==='permission_status').length)>initialReads,'Deep-link guide did not automatically read permission status');
 assert(await p.locator('img').evaluateAll(xs=>xs.every(i=>i.complete&&i.naturalWidth)),'Guide icons missing');
 // Keyboard and mouse come first: switching on screen recording reopens Itda, so it is last.
 assert(JSON.stringify(await p.locator('section.permission').evaluateAll(xs=>xs.map(x=>x.dataset.permission)))==='["access","screen"]','Permission order: screen recording should be last');
 // High-performance sharing has its own status line when it is on for this Mac.
 await p.evaluate(()=>itda.recv({type:'permissions',...permissionState,enabled:true,hp_serving:false}));
 assert((await p.locator('.hp-state').textContent()).includes('자동으로 켜집니다'),'HP status line missing');
 await p.evaluate(()=>itda.recv({type:'permissions',...permissionState}));
 assert(!await p.evaluate(()=>sent.some(m=>m.cmd==='permission_request')),'Guide requested permissions without a user action');
 // A fresh-process grant is not a usable grant in the running app until restart.
 await p.evaluate(()=>{permissionState.configured_screen=true;itda.recv({type:'permissions',...permissionState});});
 assert(await p.locator('[data-permission="screen"] .badge').textContent()==='재시작 필요','Fresh grant still shown as denied');
 assert(await p.locator('[data-permission="access"]').evaluate(e=>e.classList.contains('next')),'Granted-but-stale screen trapped the user on the wrong step');
 await p.evaluate(()=>{permissionState.configured_access=true;itda.recv({type:'permissions',...permissionState});});
 assert(await p.locator('.progress').textContent()==='설정 변경이 확인됐어요 · Itda를 다시 열어 주세요','Restart state missing');
 assert(await p.locator('[data-action="close"]').textContent()==='나중에','Cached denial was falsely completed');
 await p.setViewportSize({width:360,height:640});
 const restartBox=await p.locator('[data-action="restart"]').boundingBox(),restartFooter=await p.locator('footer').boundingBox();
 assert(restartBox.y>=0&&restartBox.y+restartBox.height<=restartFooter.y,'Restart action is hidden under fixed footer');
 await p.screenshot({path:path.join(out,'helper-restart-required.png')});
 await p.setViewportSize({width:420,height:720});

 await p.locator('[data-action="restart"]').click();assert(await p.evaluate(()=>sent.at(-1).cmd)==='permission_restart','Restart action not routed');
 await p.locator('[data-action="status"]').click();assert(await p.evaluate(()=>sent.some(m=>m.cmd==='permission_status'&&m.refresh===true)),'Manual check did not request fresh verification');
 await p.evaluate(()=>{permissionState.configured_screen=false;permissionState.configured_access=false;itda.recv({type:'permissions',...permissionState});});

 await p.locator('[data-request="screen"]').click();assert(await p.evaluate(()=>sent.filter(m=>m.cmd==='permission_request').map(m=>m.permission).join())==='screen','Requested both permissions at once');
 await p.evaluate(()=>permissionState.screen=true);await p.clock.fastForward(1600);await p.getByText('1/2 완료',{exact:false}).waitFor();assert(await p.locator('[data-request="screen"]').isDisabled(),'Granted screen permission not recognized');
 await p.locator('.instructions summary').click();await p.locator('.instructions summary').focus();await p.clock.fastForward(1600);assert(await p.locator('.instructions summary').evaluate(e=>e===document.activeElement),'Status polling stole helper focus');await p.locator('.instructions [data-action="reveal"]').click();assert(await p.evaluate(()=>sent.at(-1).cmd)==='permission_reveal','Reveal app failed');
 await p.locator('[data-action="copy"]').click();assert(await p.evaluate(()=>sent.at(-1).cmd)==='permission_copy','Copy app location failed');
 const drag=await p.locator('.app-drag').boundingBox();await p.mouse.move(drag.x+30,drag.y+20);await p.mouse.down();await p.mouse.move(drag.x+55,drag.y+20);await p.mouse.up();assert(await p.evaluate(()=>sent.some(m=>m.cmd==='permission_drag')),'Native app drag handoff missing');
 await p.locator('.instructions summary').click();await p.evaluate(()=>scrollTo(0,0));await p.screenshot({path:path.join(out,'helper-light.png')});await p.emulateMedia({colorScheme:'dark'});await p.screenshot({path:path.join(out,'helper-dark.png')});
 await p.clock.fastForward(181000);const count=await p.evaluate(()=>sent.filter(m=>m.cmd==='permission_status').length);await p.clock.fastForward(10000);assert(await p.evaluate(()=>sent.filter(m=>m.cmd==='permission_status').length)===count,'Guide polling did not stop');
 await p.locator('[data-request="access"]').click();await p.evaluate(()=>permissionState.access=true);await p.clock.fastForward(1600);await p.getByText('✓ 연결할 준비가 됐어요').waitFor();assert(await p.locator('[data-action="close"]').textContent()==='완료','Guide completion missing');
 await p.setViewportSize({width:360,height:640});const footer=await p.locator('footer').boundingBox();assert(footer.y>=0&&footer.y+footer.height<=640,'Helper actions fall below window');assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Helper overflows narrow window');
 // A guide opened without a deep link still polls, recovering a lost initial IPC reply.
 const recovery=await browser.newPage();await recovery.clock.install();
 await recovery.addInitScript(()=>{window.reads=0;window.ipc={postMessage:raw=>{if(JSON.parse(raw).cmd==='permission_status'&&++reads>1)queueMicrotask(()=>itda.recv({type:'permissions',screen:false,access:false,requested:null,requested_seq:0,app_path:'/Applications/Itda.app'}));}};});
 await recovery.goto(`http://127.0.0.1:${server.address().port}/permissions.html`);await recovery.clock.fastForward(6100);
 assert(await recovery.evaluate(()=>reads)>=2,'Lost first IPC reply permanently blocked checks');
 const recoveredReads=await recovery.evaluate(()=>reads);await recovery.clock.fastForward(1600);
 assert(await recovery.evaluate(()=>reads)>recoveredReads,'Guide without a requested permission did not poll');await recovery.close();
 // The guide beside System Settings: when a switch doesn't take, it opens its help and offers
 // to remove the earlier build's entry (the request goes to the app; nothing is changed here).
 const g=await browser.newPage({viewport:{width:300,height:470}});await g.addInitScript(()=>{window.sent=[];window.ipc={postMessage:raw=>sent.push(JSON.parse(raw))};});
 await g.goto(`http://127.0.0.1:${server.address().port}/permission-guide.html`);await g.evaluate(()=>itda.recv({type:'guide',permission:'screen'}));
 assert(!await g.locator('#stale').evaluate(d=>d.open),'Stale help open before any wait');
 await g.evaluate(()=>itda.recv({type:'guide',stale:true}));assert(await g.locator('#stale').evaluate(d=>d.open),'Stale help not opened');
 await g.locator('[data-act="reset"]').click();assert(await g.evaluate(()=>sent.at(-1).cmd)==='guide_reset','Reset not routed to the app');
 await g.evaluate(()=>itda.recv({type:'guide',stale:false}));assert(!await g.locator('#stale').evaluate(d=>d.open),'Stale help left open after reset');await g.close();
 // The remote panel must not confuse a stale report with a running server.
 await p.goto(`http://127.0.0.1:${server.address().port}/index.html`);await p.evaluate(()=>{window.remoteEvents=[];window.done=0;window.cancelled=0;const screen=document.createElement('div');screen.id='existing-session';screen.textContent='connected';document.body.append(screen);ItdaHpSetup.begin({id:'remote',name:'Studio Mac',send:(op,extra)=>remoteEvents.push({op,...extra}),done:()=>done++,cancel:()=>cancelled++});});
 await p.evaluate(()=>ItdaHpSetup.message({id:'other',ready:true,screen:true,access:true}));assert(await p.evaluate(()=>done)===0,'Other device completed setup');
 await p.evaluate(()=>ItdaHpSetup.message({id:'remote',error:'Temporary SSH failure'}));await p.getByRole('button',{name:'다시 시도',exact:true}).click();assert(await p.evaluate(()=>remoteEvents.at(-1).op)==='upgrade','Upgrade retry unavailable');
 // The remote helper opened by setup guides the steps: the panel only follows its progress.
 await p.evaluate(()=>ItdaHpSetup.message({id:'remote',ready:true,screen:false,access:false}));assert(await p.locator('[data-hp-permission="screen"]').isHidden()&&await p.locator('[data-hp-open="screen"]').isHidden()&&await p.locator('[data-hp-action="check"]').isHidden(),'Remote helper steps repeated in the local panel');assert((await p.locator('.hp-setup h2').textContent()).includes('권한 0/2'),'Compact progress missing');assert((await p.locator('[data-hp-action="helper"]').textContent()).includes('다시 열기'),'Reopen action missing');
 // When the remote helper couldn't be opened, the steps and their buttons come back.
 await p.evaluate(()=>ItdaHpSetup.message({id:'remote',ready:true,screen:false,access:false,helper_error:'원격 Mac 화면에 로그인한 계정으로 연결해 주세요'}));assert(await p.locator('[data-hp-remote]').isHidden(),'Compact mode without a remote helper');await p.locator('[data-hp-open="screen"]').click();assert(await p.evaluate(()=>remoteEvents.at(-1).op==='permissions'&&remoteEvents.at(-1).permission==='screen'),'Settings action was not routed to remote');
 await p.evaluate(()=>ItdaHpSetup.message({id:'remote',status:true,error:'SSH temporarily unavailable'}));assert(await p.locator('[data-hp-error]').isVisible(),'Status failure not shown');
 await p.evaluate(()=>ItdaHpSetup.message({id:'remote',status:true,running:true,screen:true,access:false}));assert(await p.locator('[data-hp-error]').isHidden(),'Recovered status retained stale error');
 await p.evaluate(()=>ItdaHpSetup.message({id:'remote',status:true,running:false,screen:true,access:true}));assert(await p.evaluate(()=>done)===0,'Stopped server reported complete');
 await p.screenshot({path:path.join(out,'remote-guide.png')});
 await p.locator('[data-hp-action="later"]').click();assert(await p.evaluate(()=>cancelled)===1&&await p.locator('#existing-session').count()===1,'Closing guide disrupted existing screen connection');
 await p.evaluate(()=>{ItdaHpSetup.begin({id:'remote',name:'Studio Mac',send:(op,extra)=>{remoteEvents.push({op,...extra});if(op==='status')queueMicrotask(()=>ItdaHpSetup.message({id:'remote',status:true,running:true,screen:false,access:false}));},done:()=>done++});ItdaHpSetup.message({id:'remote',ready:true,screen:false,access:false});});await p.clock.fastForward(301000);const n=await p.evaluate(()=>remoteEvents.length);await p.clock.fastForward(10000);assert(await p.evaluate(()=>remoteEvents.length)===n,'Remote polling did not stop');
 await p.evaluate(()=>{ItdaHpSetup.message({id:'remote',status:true,running:true,screen:true,access:true});ItdaHpSetup.message({id:'remote',status:true,running:true,screen:true,access:true});});assert(await p.evaluate(()=>done)===1,'Completion should occur exactly once');
 assert(!errors.length,errors.join('\n'));console.log('PASS: permission requests require clicks; one permission at a time; native drag handoff; bounded polling; remote routing/retry/recovery; stale-server guard; existing connection retained; complete exactly once');
}finally{await browser.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
