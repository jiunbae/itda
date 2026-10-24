// Real native Windows managed-SFTP quit test, in an ephemeral CI job only.
// No global input, OS shutdown, user files, Explorer OLE or production installation.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import {spawn,execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const scenario=process.env.ITDA_QUIT_SCENARIO;
if(!['wait','stop'].includes(scenario))throw Error('ITDA_QUIT_SCENARIO must be wait or stop');
const artifact=path.join(repo,'test-artifacts/native-quit-'+scenario),output=path.join(artifact,'result.json');fs.mkdirSync(artifact,{recursive:true});
const report={status:'failed',scenario,scope:'Native Windows CI managed SFTP copy and owned main-window messages only; no real shutdown, physical input, Explorer OLE, user-machine, Mac or persona-score acceptance',checks:[],cleanup:{status:'not-started'}};
let root,child,identity,browser,ended,interrupted,originalStartup,exited=false,server,serverRoot,serverConfig;
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,()=>{interrupted=signal;});
const assert=(ok,message)=>{if(!ok)throw Error(message);};
const hash=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const control=operation=>JSON.parse(execFileSync('pwsh',['-NoProfile','-NonInteractive','-File',path.join(repo,'scripts/windows-native-idle-control.ps1'),'-Operation',operation,'-Root',root,'-AppPid',String(operation==='cleanup'&&exited?0:child?.pid||0),'-CreationTicks',identity?.creation_ticks||'','-Hwnd',identity?.hwnd||'0','-DebugPort',String(report.debug_port||0)],{encoding:'utf8',timeout:30000}).replace(/^\uFEFF/,''));
const wait=async(test,message,ms=45000)=>{const end=Date.now()+ms;while(Date.now()<end){assert(!interrupted,'Interrupted: '+interrupted);if(await test())return;await new Promise(r=>setTimeout(r,100));}throw Error(message);};
function audit(){
 const file=serverRoot&&path.join(serverRoot,'audit.jsonl');if(!file||!fs.existsSync(file))return [];
 const text=fs.readFileSync(file,'utf8'),complete=text.slice(0,text.lastIndexOf('\n')+1);
 return complete.split('\n').filter(Boolean).map(line=>JSON.parse(line));
}
function destination(){
 const dir=path.join(root,'files/Projects');return fs.readdirSync(dir).map(name=>{const file=path.join(dir,name);return {name,size:fs.statSync(file).size,...(name==='payload.bin'?{sha256:hash(file)}:{})};});
}
try{
 assert(process.platform==='win32'&&process.env.GITHUB_ACTIONS==='true','Ephemeral Windows GitHub Actions only');
 const checkout=execFileSync('git',['rev-parse','HEAD'],{cwd:repo,encoding:'utf8'}).trim();assert(checkout===process.env.GITHUB_SHA,'Checkout differs from job source');
 // Original reviewed guard must be an ancestor; the busy result below, not ancestry
 // or an idle default reply, is the functional discriminator.
 execFileSync('git',['merge-base','--is-ancestor','8168276b48927c77be84c867857bcfc01fb2f415',checkout],{cwd:repo,stdio:'pipe'});
 const preflight=JSON.parse(fs.readFileSync(path.join(artifact,'preflight.json'),'utf8').replace(/^\uFEFF/,''));
 assert(preflight.status==='eligible-for-idle-pilot'&&preflight.source.checkout===checkout&&preflight.source.run_id===process.env.GITHUB_RUN_ID,'Fresh same-job eligible prerequisites required');
 const age=Date.now()-Date.parse(preflight.recorded_at);assert(age>=0&&age<120000,'Prerequisite receipt is stale');
 report.environment=preflight;report.provenance={checkout,pr_head:process.env.ITDA_PR_HEAD||null,run_id:process.env.GITHUB_RUN_ID,run_attempt:process.env.GITHUB_RUN_ATTEMPT,binary_sha256:hash(path.join(repo,'target/release/itda.exe')),driver_sha256:hash(fileURLToPath(import.meta.url)),control_sha256:hash(path.join(repo,'scripts/windows-native-idle-control.ps1')),fixture_sha256:hash(path.join(repo,'scripts/fixtures/read-only-sftp-barrier.py')),build:'Same-job build, not released/signed provenance',rustc:execFileSync('rustc',['--version'],{encoding:'utf8'}).trim()};
 serverRoot=fs.mkdtempSync(path.join(os.tmpdir(),'itda-quality-sftp-'));
 server=spawn('python',[path.join(repo,'scripts/fixtures/read-only-sftp-barrier.py'),'--workspace',serverRoot],{env:{...process.env,PYTHONUTF8:'1'},stdio:['ignore','pipe','pipe']});server.stderr.on('data',()=>{});
 const manifest=await new Promise((resolve,reject)=>{let text='';const timer=setTimeout(()=>reject(Error('SFTP fixture startup timed out')),20000);server.once('error',e=>{clearTimeout(timer);reject(e);});server.once('exit',()=>{clearTimeout(timer);reject(Error('SFTP fixture exited before readiness'));});server.stdout.on('data',b=>{text+=b;if(text.includes('\n')){clearTimeout(timer);resolve(text.split('\n')[0].trim());}});});
 assert(path.resolve(manifest)===path.join(serverRoot,'connection.json'),'Fixture manifest escaped owned workspace');
 serverConfig=JSON.parse(fs.readFileSync(manifest,'utf8'));
 assert(serverConfig.pid===server.pid&&serverConfig.workspace===serverRoot&&serverConfig.host==='127.0.0.1','SFTP ownership mismatch');
 report.sftp={library:serverConfig.paramiko_version,port:serverConfig.port};
 root=fs.mkdtempSync(path.join(os.tmpdir(),'itda-native-idle-'));report.workspace_spelling={requested:root};root=control('workspace').root;report.workspace_spelling.canonical=root;
 const data=path.join(root,'data/Itda');fs.mkdirSync(data,{recursive:true});fs.mkdirSync(path.join(root,'files/Projects'),{recursive:true});
 const fixtureId='native-quit-fixture';
 fs.writeFileSync(path.join(data,'hosts.json'),JSON.stringify([{id:fixtureId,name:'Fixture SFTP',host:'127.0.0.1',os:'mac',kind:'vnc',user:'',opts:{},files:{method:'sftp',host:'127.0.0.1',port:serverConfig.port,user:''}}]));fs.writeFileSync(path.join(data,'accounts.json'),'[]');
 fs.writeFileSync(path.join(data,'settings.json'),JSON.stringify({onboarded:true,theme:'light',tray:false,notify:false,announce:false,auto_update:false,thumbnails:false,web:{on:false},share:{on:false},hp:{on:false}}));
 fs.writeFileSync(path.join(data,'known_hosts'),`[127.0.0.1]:${serverConfig.port} ${serverConfig.host_key_type} ${serverConfig.host_key_base64}\n`);
 const candidate=path.join(root,'itda.exe');fs.copyFileSync(path.join(repo,'target/release/itda.exe'),candidate);assert(hash(candidate)===report.provenance.binary_sha256,'Copied executable bytes differ');
 originalStartup=control('startup');
 const listener=net.createServer();await new Promise((resolve,reject)=>{listener.once('error',reject);listener.listen(0,'127.0.0.1',resolve);});report.debug_port=listener.address().port;await new Promise(r=>listener.close(r));
 child=spawn(candidate,[],{env:{...process.env,ITDA_DATA_DIR:path.join(root,'data'),ITDA_DEBUG_PORT:String(report.debug_port)},stdio:['ignore','pipe','pipe']});child.stdout.on('data',()=>{});child.stderr.on('data',()=>{});
 ended=new Promise(resolve=>{child.once('error',error=>resolve({error:error.message}));child.once('exit',(code,signal)=>{exited=true;resolve({code,signal});});});
 await wait(()=>{
  assert(child.pid&&child.exitCode===null&&child.signalCode===null,'Candidate exited before readiness');
  const state=control('inspect');assert(state.alive,'Candidate disappeared');report.last_native_state=state;
  if(identity)assert(identity.creation_ticks===state.creation_ticks,'Candidate PID was reused');identity=state;
  if(state.hwnd==='0'||!state.visible||!state.private_udf||!state.webview_pids.length||!state.listeners.length)return false;
  assert(state.session_id===preflight.checks.session.id&&state.window_class==='itda_home'&&/^Itda \(profile [0-9a-f]{16}\)$/.test(state.window_title),'Wrong native session/home window');
  assert(state.listeners.every(l=>['127.0.0.1','::1'].includes(l.LocalAddress)&&[child.pid,...state.webview_pids].includes(l.OwningProcess)),'Unowned/non-loopback CDP listener');return true;
 },'Private native home did not become ready',60000);
 report.native_identity={pid:identity.pid,hwnd:identity.hwnd,creation_ticks:identity.creation_ticks,session_id:identity.session_id,window_class:identity.window_class,window_title:identity.window_title};
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${report.debug_port}`,{timeout:20000});const context=browser.contexts()[0],page=context.pages()[0];assert(page,'Native page missing');
 await page.waitForFunction(id=>typeof S!=='undefined'&&S.hosts.length===1&&S.hosts[0].id===id&&S.settings.tray===false,fixtureId,{timeout:20000});
 assert(control('query-end').response===1&&control('inspect').alive,'Idle query failed before copy');
 report.checks.push({check:'owned native WebView2 and isolated fixture profile; idle query permits exit without closing',result:'PASS'});
 await page.evaluate(()=>{window.quitObserved=[];const recv=itda.recv;itda.recv=function(m){if(['quit_guard','quit_pending'].includes(m.type))quitObserved.push({type:m.type,reason:m.reason,count:m.count,stuck:m.stuck,waiting:m.waiting,phase:m.phase});return recv.call(this,m);};});
 const newPage=context.waitForEvent('page');await page.getByRole('button',{name:'파일 브라우저',exact:true}).click();const f=await newPage;await f.locator('.files-layer').waitFor();await f.locator(`[data-device="${fixtureId}"]`).first().click();
 const setup=f.locator('.files-setup');await setup.waitFor();await setup.locator('[name=account]').selectOption('custom');await setup.locator('[name=user]').fill(serverConfig.user);await setup.locator('.files-password > summary').click();await setup.locator('[name=password]').fill(serverConfig.password);await setup.locator('[name=save_password]').uncheck();await setup.locator('[data-connect]').click();
 const row=f.locator('[data-pane=remote] [data-index]').filter({hasText:'payload.bin'}).first();await row.waitFor({timeout:45000});
 const sourceTab=f.getByRole('tab').first();await f.locator('[data-action=new-tab]').click();const localTab=f.getByRole('tab').last();await f.locator('[data-network-sidebar] [data-side=local]').click();await f.keyboard.press('Control+l');const address=f.getByRole('textbox',{name:'주소',exact:true});await address.fill(path.join(root,'files/Projects'));await address.press('Enter');await f.locator('[data-pane=local] .files-note').filter({hasText:'표시할 항목이 없습니다'}).waitFor();
 fs.writeFileSync(path.join(serverRoot,'hold-read'),'hold');await sourceTab.click();await row.click();await f.keyboard.press('Control+c');await localTab.click();await f.locator('[data-pane=local] .files-list').focus();await f.keyboard.press('Control+v');
 const copy=f.getByRole('dialog',{name:'파일 복사',exact:true});await copy.waitFor();assert((await copy.textContent()).includes('payload.bin'),'Copy source changed');await copy.getByRole('button',{name:'복사',exact:true}).click();
 await wait(()=>audit().some(e=>e.event==='read-blocked'),'Source did not reach held read');await wait(()=>destination().some(e=>/^\.itda-.*\.partial$/.test(e.name)&&e.size>0),'No actual destination staging bytes');
 const held=audit().find(e=>e.event==='read-blocked'),stage=destination().find(e=>/^\.itda-.*\.partial$/.test(e.name));assert(held.offset===2*1024*1024,'Wrong barrier offset');
 report.checks.push({check:'actual managed SFTP copy held after source bytes and nonzero destination stage',result:'PASS',barrier_offset:held.offset,staging_bytes:stage.size});
 assert(control('query-end').response===0,'Busy native query allowed exit');const guard=page.locator('[data-key=quit-guard]');await guard.waitFor();assert((await guard.textContent()).includes('1개'),'Guard active count is not one');
 await guard.locator('[data-x=cancel]').click();await guard.waitFor({state:'detached'});assert(control('inspect').alive&&!audit().some(e=>e.event==='read-aborted')&&destination().some(e=>/^\.itda-.*\.partial$/.test(e.name)),'Cancelling quit stopped the app or paused copy');
 report.checks.push({check:'busy query returns 0 and shows guard; cancel preserves app and held real copy',result:'PASS'});
 assert(control('close').posted,'Owned main close was not posted');await guard.waitFor();assert(control('inspect').alive,'Main close bypassed the guard');
 if(scenario==='wait'){
  await guard.locator('[data-x=wait]').click();await page.locator('#quit-pending').waitFor();assert(control('inspect').alive,'Wait quit exited while copy was held');report.guard_events=await page.evaluate(()=>quitObserved);fs.writeFileSync(path.join(serverRoot,'release-read'),'resume');
 }else{await guard.locator('[data-x=stop]').click();}
 let exit;await wait(async()=>{exit=await Promise.race([ended,new Promise(r=>setTimeout(()=>r(null),100))]);return !!exit;},'Native guard did not exit after '+scenario,45000);assert(exit.code===0&&!exit.signal&&!exit.error,'Abnormal native exit: '+JSON.stringify(exit));report.exit=exit;
 const payload=serverConfig.files.find(file=>file.path==='/payload.bin');
 if(scenario==='wait'){
  const copied=destination();assert(copied.length===1&&copied[0].name==='payload.bin'&&copied[0].size===payload.size&&copied[0].sha256===payload.sha256,'Wait quit did not leave exactly one hash-matching completed file');
  report.checks.push({check:'main close waits for real copy commit then exits; complete destination hash matches',result:'PASS',bytes:payload.size,sha256:payload.sha256});
 }else{
  await wait(()=>audit().some(e=>e.event==='read-aborted'&&e.offset===held.offset&&e.reason==='transport_closed'),'Stop quit did not abort the held SSH transport',10000);
  assert(!audit().some(e=>e.event==='read-bytes'&&e.path==='/payload.bin'&&e.offset>=held.offset)&&destination().length===0,'Stop quit advanced source reads or left destination/staging files');
  report.checks.push({check:'main close stops held real copy; exit observed with aborted transport, no later source bytes and empty destination/staging',result:'PASS'});
 }
 assert(hash(path.join(serverRoot,'files/payload.bin'))===payload.sha256,'Generated source changed');report.checks.push({check:'generated source preserved',result:'PASS'});report.status='passed';
}catch(error){report.error=serverConfig?.password?error.message.replaceAll(serverConfig.password,'[fixture password redacted]'):error.message;process.exitCode=1;}
finally{
 let cleanupError;
 try{if(root){report.cleanup=control('cleanup');const unchanged=originalStartup?JSON.stringify(control('startup'))===JSON.stringify(originalStartup):null;fs.rmSync(root,{recursive:true,force:true});assert(!fs.existsSync(root),'Private app workspace remains');report.cleanup={...report.cleanup,workspace_removed:true,startup_unchanged:unchanged};assert(unchanged!==false,'Startup registration changed');}}catch(error){cleanupError=error;}
 try{
  report.sftp_audit=audit();
  if(server&&server.exitCode===null&&server.signalCode===null){let timer;const stopped=new Promise(r=>server.once('exit',r));server.kill();try{await Promise.race([stopped,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Owned SFTP process did not stop')),5000);})]);}finally{clearTimeout(timer);}}
  if(serverRoot){assert(path.dirname(serverRoot)===os.tmpdir()&&path.basename(serverRoot).startsWith('itda-quality-sftp-'),'SFTP workspace ownership mismatch');fs.rmSync(serverRoot,{recursive:true,force:true});assert(!fs.existsSync(serverRoot),'Private SFTP workspace remains');report.cleanup.sftp_workspace_removed=true;}
 }catch(error){cleanupError||=error;}
 if(browser){let timer;try{await Promise.race([browser.close().catch(()=>{}),new Promise(r=>{timer=setTimeout(r,3000);})]);}finally{clearTimeout(timer);}}
 child?.stdout?.destroy();child?.stderr?.destroy();child?.unref();server?.stdout?.destroy();server?.stderr?.destroy();server?.unref();
 report.cleanup.status=cleanupError?'failed':'passed';if(cleanupError){report.cleanup.error=cleanupError.message;report.status='failed';process.exitCode=1;}
 if(interrupted){report.status='interrupted';report.signal=interrupted;process.exitCode=1;}
 fs.writeFileSync(output,JSON.stringify(report,null,2));console.log(JSON.stringify({status:report.status,scenario,checks:report.checks.length,cleanup:report.cleanup.status,receipt:output}));
}
