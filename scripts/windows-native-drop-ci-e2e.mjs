// Explicitly gated, single functional OLE proof on a disposable Windows CI desktop.
// CDP prepares/observes fixture state only. All drag OS input is driver-tagged SendInput.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import {spawn,execFileSync} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
import {observeExit,explorerCleanupComplete} from './fixtures/windows-drop-lifecycle.mjs';
const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const artifact=path.join(repo,'test-artifacts/native-drop');fs.mkdirSync(artifact,{recursive:true});
const report={status:'failed',scope:'One hosted-CI Explorer IDataObject/IStream functional proof; no physical mouse, user-PC, 150% DPI, AT, multi-monitor, Mac or persona-9 acceptance',checks:[],limits:['Explorer writes normal ShellBag/view state only on disposable VM; explicitly allowed','No released/signed build provenance','No no-payload-before-release assumption: target may legitimately request FileContents before release','Negative B (non-target release) not implemented: Itda source window itself registers a DropTarget; this proof covers Escape cancel and Explorer positive only'],cleanup:{}};
const assert=(v,m)=>{if(!v)throw Error(m);};
const hash=p=>createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
let root,child,identity,exited=false,browser,filePage,server,serverExit,serverRoot,config,controller,controllerExit,controllerState,startup,interrupted,childExit,launchError,controllerQuiesced=true;
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,()=>{interrupted=signal;});
const ps=(file,args=[])=>execFileSync('pwsh',['-NoProfile','-NonInteractive','-File',path.join(repo,'scripts',file),...args],{encoding:'utf8',timeout:30000});
const read=p=>{const bytes=fs.readFileSync(p);try{return JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/,''));}catch{throw Error(`Invalid JSON in ${path.basename(p)} (${bytes.length} bytes)`);}};
const control=op=>JSON.parse(ps('windows-native-idle-control.ps1',['-Operation',op,'-Root',root,'-AppPid',String(exited?0:child?.pid||0),'-CreationTicks',identity?.creation_ticks||'','-Hwnd',identity?.hwnd||'0','-DebugPort',String(report.debug_port||0)]).replace(/^\uFEFF/,''));
async function wait(test,message,ms=30000,observeInterrupt=true){const end=Date.now()+ms;while(Date.now()<end){assert(!observeInterrupt||!interrupted,'Interrupted '+interrupted);if(await test())return;await sleep(100);}throw Error(message);}
function audit(){const file=serverRoot&&path.join(serverRoot,'audit.jsonl');if(!file||!fs.existsSync(file))return [];const t=fs.readFileSync(file,'utf8');return t.slice(0,t.lastIndexOf('\n')+1).split('\n').filter(Boolean).map(JSON.parse);}
function destination(){const dir=path.join(root,'drop-destination');return fs.readdirSync(dir,{recursive:true}).filter(n=>fs.statSync(path.join(dir,n)).isFile()).map(n=>({path:n.replaceAll('\\','/'),sha256:hash(path.join(dir,n))}));}
function fresh(name){const file=path.join(artifact,`preflight-${name}.json`);try{ps('windows-native-drop-preflight.ps1',['-Receipt',file]);}catch(error){if(!fs.existsSync(file))throw error;}const p=read(file);assert(p.status!=='probe-error','Prerequisite probe error');assert(p.source.checkout===report.provenance.checkout&&p.source.run_id===process.env.GITHUB_RUN_ID&&p.source.run_attempt===process.env.GITHUB_RUN_ATTEMPT,'Prerequisite source/run mismatch');const age=Date.now()-Date.parse(p.recorded_at);assert(age>=0&&age<120000,'Stale prerequisites');return p;}
try{
 assert(process.platform==='win32'&&process.env.GITHUB_ACTIONS==='true'&&process.env.ITDA_ENABLE_OWNED_INPUT==='true','Explicit disposable Windows CI input gate required');
 const checkout=execFileSync('git',['rev-parse','HEAD'],{cwd:repo,encoding:'utf8'}).trim();assert(/^[a-f0-9]{40}$/.test(process.env.ITDA_EXPECTED_SOURCE_SHA||'')&&checkout===process.env.ITDA_EXPECTED_SOURCE_SHA&&checkout===process.env.GITHUB_SHA,'Expected source SHA must equal same-job checkout and event SHA');
 report.provenance={checkout,run_id:process.env.GITHUB_RUN_ID,run_attempt:process.env.GITHUB_RUN_ATTEMPT,binary_sha256:hash(path.join(repo,'target/release/itda.exe')),driver_sha256:hash(fileURLToPath(import.meta.url)),native_driver_sha256:hash(path.join(repo,'scripts/fixtures/windows-drop-driver.cs')),sftp_fixture_sha256:hash(path.join(repo,'scripts/fixtures/read-only-sftp-drop.py')),build:'same-job windows-release.ps1 non-release candidate'};
 const initial=fresh('initial');report.environment=initial;
 if(initial.status==='prerequisite-unavailable'){report.status='prerequisite-unavailable';report.reason='prerequisite-unavailable; not verified';report.unmet=initial.unmet;process.exitCode=2;}
 else {
 assert(initial.status==='eligible-for-owned-drop','Unexpected preflight status');
 root=fs.mkdtempSync(path.join(os.tmpdir(),'itda-native-idle-'));root=control('workspace').root;startup=control('startup');
 serverRoot=fs.mkdtempSync(path.join(os.tmpdir(),'itda-quality-sftp-'));
 server=spawn('python',[path.join(repo,'scripts/fixtures/read-only-sftp-drop.py'),'--workspace',serverRoot],{env:{...process.env,PYTHONUTF8:'1'},stdio:['ignore','pipe','pipe']});server.stderr.on('data',()=>{});
 serverExit=observeExit(server);
 const manifest=await new Promise((resolve,reject)=>{let text='';const t=setTimeout(()=>reject(Error('SFTP startup timed out')),20000);server.once('error',e=>{clearTimeout(t);reject(e);});server.once('exit',()=>{clearTimeout(t);reject(Error('SFTP exited before readiness'));});server.stdout.on('data',b=>{text+=b;if(text.includes('\n')){clearTimeout(t);resolve(text.split('\n')[0].trim());}});});
 assert(path.resolve(manifest)===path.join(serverRoot,'connection.json'),'Manifest escaped workspace');config=read(manifest);assert(config.pid===server.pid&&config.workspace===serverRoot&&config.host==='127.0.0.1','Fixture ownership mismatch');
 report.fixture={host:'127.0.0.1',pinned_key_sha256:createHash('sha256').update(Buffer.from(config.host_key_base64,'base64')).digest('hex'),password_saved:false,port:config.port,paramiko_version:config.paramiko_version,files:config.files.map(({path,size,sha256})=>({path,size,sha256}))};
 const data=path.join(root,'data/Itda');fs.mkdirSync(data,{recursive:true});fs.mkdirSync(path.join(root,'drop-destination'));
 const fixtureId='native-drop-fixture';
 fs.writeFileSync(path.join(data,'hosts.json'),JSON.stringify([{id:fixtureId,name:'Owned SFTP fixture',host:'127.0.0.1',os:'mac',user:'',opts:{},files:{method:'sftp',host:'127.0.0.1',port:config.port,user:''}}]));fs.writeFileSync(path.join(data,'accounts.json'),'[]');
 fs.writeFileSync(path.join(data,'settings.json'),JSON.stringify({onboarded:true,theme:'light',tray:false,notify:false,announce:false,auto_update:false,thumbnails:false,web:{on:false},share:{on:false}}));
 fs.writeFileSync(path.join(data,'known_hosts'),`[127.0.0.1]:${config.port} ${config.host_key_type} ${config.host_key_base64}\n`);
 const candidate=path.join(root,'itda.exe');fs.copyFileSync(path.join(repo,'target/release/itda.exe'),candidate);assert(hash(candidate)===report.provenance.binary_sha256,'Copied exe differs');
 const l=net.createServer();await new Promise(r=>l.listen(0,'127.0.0.1',r));report.debug_port=l.address().port;await new Promise(r=>l.close(r));
 child=spawn(candidate,[],{env:{...process.env,ITDA_DATA_DIR:path.join(root,'data'),ITDA_DEBUG_PORT:String(report.debug_port)},stdio:['ignore','pipe','pipe']});child.stdout.on('data',()=>{});child.stderr.on('data',()=>{});child.once('error',e=>{launchError=e.message;exited=true;childExit={error:e.message};});child.once('exit',(code,signal)=>{exited=true;childExit={code,signal};});
 await wait(()=>{assert(!exited,'Candidate exited: '+(launchError||JSON.stringify(childExit)));const s=control('inspect');if(identity)assert(identity.creation_ticks===s.creation_ticks,'Candidate PID reused');identity=s;if(!s.alive||s.hwnd==='0'||!s.visible||!s.private_udf||!s.webview_pids.length||!s.listeners.length)return false;assert(s.session_id===initial.checks.session.id&&s.window_class==='itda_home'&&/^Itda \(profile [0-9a-f]{16}\)$/.test(s.window_title),'Wrong candidate window/session');assert(s.listeners.every(v=>['127.0.0.1','::1'].includes(v.LocalAddress)&&[child.pid,...s.webview_pids].includes(v.OwningProcess)),'CDP not owned/loopback');return true;},'Private WebView2 not ready',60000);
 report.native_identity={pid:identity.pid,creation_ticks:identity.creation_ticks,hwnd:identity.hwnd,session_id:identity.session_id};
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${report.debug_port}`);const context=browser.contexts()[0],page=context.pages()[0];
 await page.waitForFunction(id=>typeof S!=='undefined'&&S.hosts.length===1&&S.hosts[0].id===id,fixtureId);
 const newPage=context.waitForEvent('page');await page.evaluate(()=>document.querySelector('[data-page=files]').click());const f=await newPage;filePage=f;await f.locator('.files-layer').waitFor();
 await f.evaluate(id=>document.querySelector(`[data-device="${id}"]`).click(),fixtureId);await f.locator('.files-setup').waitFor();
 await f.evaluate(c=>{const q=s=>document.querySelector('.files-setup '+s);q('[name=account]').value='custom';q('[name=account]').dispatchEvent(new Event('change',{bubbles:true}));q('[name=user]').value=c.user;q('[name=password]').value=c.password;q('[name=save_password]').checked=false;q('[data-connect]').click();},{user:config.user,password:config.password});
 await f.waitForFunction(()=>document.querySelectorAll('[data-pane=remote] [data-index]').length===3,undefined,{timeout:45000});
 await f.evaluate(()=>{window.nativeDragEvents=[];window.nativeDragRequests=[];const recv=ItdaFiles.message;ItdaFiles.message=function(m){if(m.type==='files_native_drag_end')nativeDragEvents.push({dropped:m.dropped,error:m.error,unix_ms:Date.now()});return recv.call(this,m);};const originalSend=window.send;window.send=function(cmd,data){if(cmd==='files_native_drag')nativeDragRequests.push({unix_ms:Date.now(),count:data.items?.length});return originalSend(cmd,data);};});
 controller=spawn('pwsh',['-NoProfile','-NonInteractive','-File',path.join(repo,'scripts/windows-native-drop-control.ps1'),'-Root',root,'-AppPid',String(child.pid),'-CreationTicks',identity.creation_ticks,'-MainHwnd',identity.hwnd],{env:process.env,stdio:['ignore','pipe','pipe']});controller.stdout.on('data',()=>{});controller.stderr.on('data',()=>{});controllerQuiesced=false;controllerState=observeExit(controller);controllerExit=new Promise(resolve=>{controller.once('error',e=>resolve({error:e.message}));controller.once('exit',(code,signal)=>resolve({code,signal}));});
 const controllerError=()=>{const p=path.join(root,'controller-error.json');if(fs.existsSync(p))throw Error(read(p).error);};
 await wait(()=>{controllerError();return fs.existsSync(path.join(root,'layout.json'));},'Owned Explorer layout not ready');report.layout=read(path.join(root,'layout.json'));const layout=report.layout;
 report.viewport=await f.evaluate(()=>({width:innerWidth,height:innerHeight,dpr:devicePixelRatio}));assert(report.viewport.width>=440&&report.viewport.height>=400,'Measured file CSS viewport below minimum');
 assert(layout.SourcePid===child.pid&&layout.Session===identity.session_id&&layout.SourceIntegrity===layout.ExplorerIntegrity,'Candidate/Explorer IL or ownership mismatch');
 const command=async value=>{const request=randomUUID();fs.writeFileSync(path.join(root,'command.tmp'),JSON.stringify({...value,request}));fs.renameSync(path.join(root,'command.tmp'),path.join(root,'command.json'));let reply;await wait(()=>{controllerError();const p=path.join(root,'reply.json');if(!fs.existsSync(p))return false;reply=read(p);return reply.request===request;},'Native command timed out',20000);assert(!reply.error,reply.error);return reply.result;};
 for(const scenario of ['cancel','positive']){
  // Only selection/geometry preparation through CDP; no CDP pointer drag/button state.
  await f.evaluate(()=>{nativeDragEvents=[];nativeDragRequests=[];const list=document.querySelector('[data-pane=remote] .files-list');const row=list.querySelector('[data-index]');row.dispatchEvent(new MouseEvent('click',{bubbles:true}));list.focus();list.dispatchEvent(new KeyboardEvent('keydown',{key:'a',ctrlKey:true,bubbles:true,cancelable:true}));});
  assert(await f.locator('[data-pane=remote] [data-index].selected').count()===3,'Fixture multi-selection not established');
  const b=await f.locator('[data-pane=remote] [data-index]').first().boundingBox(),dpr=await f.evaluate(()=>devicePixelRatio);assert(b,'Source row missing');
  const point={x:Math.round(layout.OriginX+(b.x+Math.min(60,b.width/2))*dpr),y:Math.round(layout.OriginY+(b.y+b.height/2)*dpr)};
  // This is a new distinct scenario, not a retry after failed guards/input.
  await sleep(5500);const prereq=fresh(scenario);report.checks.push({check:`${scenario} fresh prerequisites`,preflight:prereq.status});assert(prereq.status==='eligible-for-owned-drop',`prerequisite-unavailable: ${scenario}; no retry`);
  assert(!audit().some(e=>e.event==='read-open'&&e.path==='/payload.bin')||scenario!=='cancel','Payload pre-staged before first native drag');
  const before=audit().length,input=await command({op:'drag',scenario,...point});
  report.checks.push({check:`${scenario} input attribution`,input});assert(!input.Error&&!input.Interfered&&input.ReleaseUnixMs&&input.ReleasePoint===({cancel:'cancel',positive:'target'}[scenario]),'Input aborted/interfered; no retry');
  await f.waitForFunction(()=>nativeDragEvents.length>0,undefined,{timeout:15000});const observation=await f.evaluate(()=>({requests:nativeDragRequests,events:nativeDragEvents}));
  // Preserve what happened before judging it. A wrong outcome must not erase the
  // native result, destination or read timing that distinguishes its cause.
  (report.observations??=[]).push({scenario,observation,destination:destination(),reads:audit().slice(before)});
  assert(observation.requests.length===1&&observation.requests[0].count===3,'Did not observe exactly one real files_native_drag');assert(observation.events.every(e=>!e.error),'Native OLE reported error');assert(observation.events.some(e=>e.dropped===(scenario==='positive')),'Wrong DoDragDrop outcome');
  if(scenario!=='positive'){
   const end=Date.now()+3000;do{assert(destination().length===0,'Negative control created files');await sleep(100);}while(Date.now()<end);
  }else{
   const expected=config.files.map(v=>({path:v.path.slice(1).normalize('NFC'),sha256:v.sha256}));let copied;
   await wait(()=>{copied=destination();return copied.length===expected.length&&expected.every(e=>copied.some(c=>c.path===e.path&&c.sha256===e.sha256));},'Explorer filenames/NFC/hashes mismatch',45000);report.destination=copied;
   for(const file of config.files)assert(audit().slice(before).some(e=>e.event==='read-open'&&e.path===file.path)&&audit().slice(before).some(e=>e.event==='read-bytes'&&e.path===file.path&&e.bytes>0),'Positive target did not read every file');
  }
  const reads=audit().slice(before).filter(e=>e.event==='read-open'||e.event==='read-bytes');
  report.checks.push({check:`${scenario} native result`,result:'PASS',observation,reads_before_explorer:reads.filter(e=>e.unix_ms<input.FirstExplorerUnixMs),reads_before_release:reads.filter(e=>e.unix_ms<input.ReleaseUnixMs),reads_after_release:reads.filter(e=>e.unix_ms>=input.ReleaseUnixMs),interpretation:'Early target reads may be legitimate IDataObject consumption; no claim that release timing proves laziness'});
 }
 for(const file of config.files)assert(hash(path.join(serverRoot,'files',file.path.slice(1)))===file.sha256,'Generated source modified');
 report.read_timing_review=report.checks.filter(c=>c.reads_before_release?.length).map(c=>({scenario:c.check,before_release:c.reads_before_release,before_explorer:c.reads_before_explorer}));
 report.status=report.read_timing_review.length?'functional-passed-read-timing-review':'passed';
 }
}catch(error){report.error=config?.password?error.message.replaceAll(config.password,'[redacted]'):error.message;report.status=report.error.includes('prerequisite-unavailable:')?'prerequisite-unavailable':'failed';if(report.status==='prerequisite-unavailable'){report.reason='prerequisite-unavailable; not verified';process.exitCode=2;}else process.exitCode=1;}
finally{
 const errors=[];
 const originalOutcome={status:report.status,error:report.error,reason:report.reason};
 try{
  if(controller){if(!controllerState.stopped){fs.writeFileSync(path.join(root,'command.tmp'),JSON.stringify({op:'stop',request:randomUUID()}));fs.renameSync(path.join(root,'command.tmp'),path.join(root,'command.json'));}const result=await Promise.race([controllerExit,sleep(15000).then(()=>null)]);assert(result,'Controller cleanup timed out; not terminating live native-input driver');report.cleanup.controller=result;assert(controllerState.stopped,'Controller exit unconfirmed; refusing to close live native-input resources');controllerQuiesced=true;
   // An exited controller is quiescent even if the scenario failed. Preserve
   // that failure, while judging cleanup from the separate window receipt.
   if(result.code!==0||result.signal||result.error){report.status='failed';report.error??='Controller exited abnormally';process.exitCode=1;}
   const p=path.join(root,'controller-cleanup.json');assert(fs.existsSync(p),'Controller cleanup receipt absent');report.cleanup.explorer=read(p);assert(explorerCleanupComplete(report.cleanup.explorer),'Owned Explorer absence was not confirmed');}
 }catch(e){errors.push(e.message);}
 // Observation only, after native input has quiesced and before closing the app
 // or removing owned fixtures. Never approve a prompt or initiate a transfer.
 if(controllerQuiesced&&root){
  try{report.final_destination=destination();}catch(e){report.final_destination_error=e.message;}
  if(filePage&&!filePage.isClosed()){
   try{report.final_native_observation=await Promise.race([filePage.evaluate(()=>({requests:window.nativeDragRequests??[],events:window.nativeDragEvents??[]})),sleep(1500).then(()=>{throw Error('Final native observation timed out');})]);}catch(e){report.final_native_observation_error=e.message;}
  }
  if(config&&serverRoot){
   try{report.generated_sources=config.files.map(file=>{const base=path.join(serverRoot,'files'),source=path.resolve(base,file.path.slice(1));assert(source.startsWith(base+path.sep),'Generated source escaped fixture');return {path:file.path,expected_sha256:file.sha256,actual_sha256:hash(source)};});}catch(e){report.generated_sources_error=e.message;}
  }
 }
 // A source-window emergency release can be accepted by Itda's own DropTarget.
 // Observe any resulting confirmation; never click/approve it. Closing the owned
 // app through WM_CLOSE disposes unapproved DOM prompts without starting the copy.
 if(browser&&controllerQuiesced){
  report.cleanup.emergency_source_releases=report.checks.filter(c=>c.input?.ReleasePoint==='emergency-source').map(c=>({may_have_offered_source:c.input.EmergencyMayHaveOfferedSource,escape_sent:c.input.EmergencyEscaped}));
  if(report.cleanup.emergency_source_releases.length)await sleep(1000); // Observe queued native-paste UI only; no input or approval.
  report.cleanup.unapproved_confirmations=[];
  for(const context of browser.contexts())for(const page of context.pages()){
   try{const count=await page.locator('.files-prompt').count();if(count)report.cleanup.unapproved_confirmations.push({count,action:'observed-only; not approved; owned app will be closed'});}catch{}
  }
  if(report.cleanup.unapproved_confirmations.length)errors.push('Unexpected unapproved file confirmation after native drag; never auto-approved');
 }
 try{
  if(child&&!exited&&identity?.hwnd&&identity.hwnd!=='0'&&controllerQuiesced){control('close');await wait(()=>exited,'Candidate did not quit normally',15000,false);}if(child&&controllerQuiesced){report.cleanup.exit=childExit;assert(childExit?.code===0&&!childExit.signal,'Candidate exit was not normal');}
 }catch(e){errors.push(e.message);}
 try{if(root&&controllerQuiesced){report.cleanup.app=control('cleanup');assert(!report.cleanup.app.forced_candidate,'Candidate required forced termination');assert(!startup||JSON.stringify(control('startup'))===JSON.stringify(startup),'Startup registration changed');report.cleanup.startup_unchanged=true;}}catch(e){errors.push(e.message);}
 if(!controllerQuiesced)errors.push('Controller still live: refusing to close its source/target or delete its workspace');
 if(browser&&controllerQuiesced)await Promise.race([browser.close().catch(()=>{}),sleep(3000)]);
 try{report.sftp_audit=audit();if(server){report.cleanup.sftp_termination_requested=false;if(!serverExit.stopped){report.cleanup.sftp_termination_requested=server.kill();await wait(()=>serverExit.stopped,'Owned SFTP did not stop',5000,false);}report.cleanup.sftp_exit=serverExit.result;}report.sftp_audit=audit();if(serverRoot){assert(path.dirname(serverRoot)===os.tmpdir()&&path.basename(serverRoot).startsWith('itda-quality-sftp-'),'SFTP cleanup ownership mismatch');fs.rmSync(serverRoot,{recursive:true,force:true});assert(!fs.existsSync(serverRoot),'SFTP workspace remains');}if(root&&!errors.length){fs.rmSync(root,{recursive:true,force:true});assert(!fs.existsSync(root),'Private workspace remains');report.cleanup.workspaces_removed=true;}}catch(e){errors.push(e.message);}
 for(const p of [child,server,controller]){p?.stdout?.destroy();p?.stderr?.destroy();p?.unref();}
 report.cleanup.status=errors.length?'failed':'passed';if(errors.length){report.cleanup.errors=errors;report.original_outcome=originalOutcome;report.status='failed';process.exitCode=1;}if(interrupted){report.status='interrupted';process.exitCode=1;}
 fs.writeFileSync(path.join(artifact,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({status:report.status,cleanup:report.cleanup.status,receipt:path.join(artifact,'result.json')}));
}
