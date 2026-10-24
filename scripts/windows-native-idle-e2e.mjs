// Native Windows CI idle pilot. No transfers, physical input, OS shutdown or install.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import {spawn,execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const output=path.join(repo,'test-artifacts/native-idle/result.json');
fs.mkdirSync(path.dirname(output),{recursive:true});
const report={status:'failed',scope:'Baseline main without PR102 quit guard: validates the native CI harness only. No transfer/guard, physical-input, shutdown/logout or persona-score acceptance',checks:[],cleanup:{status:'not-started'}};
let root,child,identity,browser,ended,interrupted,originalStartup,exited=false;
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,()=>{interrupted=signal;});
const assert=(ok,message)=>{if(!ok)throw Error(message);};
const hash=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const control=(operation,pin=identity)=>JSON.parse(execFileSync('pwsh',['-NoProfile','-NonInteractive','-File',path.join(repo,'scripts/windows-native-idle-control.ps1'),'-Operation',operation,'-Root',root,'-AppPid',String(operation==='cleanup'&&exited?0:child?.pid||0),'-CreationTicks',pin?.creation_ticks||'', '-Hwnd',pin?.hwnd||'0','-DebugPort',String(report.debug_port||0)],{encoding:'utf8',timeout:30000}).replace(/^\uFEFF/,''));
const wait=async(test,message,ms=60000)=>{const end=Date.now()+ms;while(Date.now()<end){assert(!interrupted,'Interrupted: '+interrupted);if(await test())return;await new Promise(r=>setTimeout(r,200));}throw Error(message);};
try{
 assert(process.platform==='win32'&&process.env.GITHUB_ACTIONS==='true','This pilot runs only in ephemeral Windows GitHub Actions');
 const checkout=execFileSync('git',['rev-parse','HEAD'],{cwd:repo,encoding:'utf8'}).trim();
 assert(checkout===process.env.GITHUB_SHA,'Checkout differs from the current job source');
 const preflight=JSON.parse(fs.readFileSync(path.join(repo,'test-artifacts/native-idle/preflight.json'),'utf8').replace(/^\uFEFF/,''));
 assert(preflight.status==='eligible-for-idle-pilot'&&preflight.source.checkout===checkout&&preflight.source.run_id===process.env.GITHUB_RUN_ID,'Fresh same-job eligible environment receipt required');
 const age=Date.now()-Date.parse(preflight.recorded_at);assert(age>=0&&age<120000,'Environment receipt is stale');
 report.provenance={checkout,pr_head:process.env.ITDA_PR_HEAD||null,run_id:process.env.GITHUB_RUN_ID,run_attempt:process.env.GITHUB_RUN_ATTEMPT,build:'Same-job scripts/windows-release.ps1; copied executable byte comparison, not released/signed provenance',binary_sha256:hash(path.join(repo,'target/release/itda.exe')),driver_sha256:hash(fileURLToPath(import.meta.url)),control_sha256:hash(path.join(repo,'scripts/windows-native-idle-control.ps1'))};
 report.provenance.rustc=execFileSync('rustc',['--version'],{encoding:'utf8'}).trim();
 report.environment=preflight;
 root=fs.mkdtempSync(path.join(os.tmpdir(),'itda-native-idle-'));
 report.workspace_spelling={requested:root};root=control('workspace').root;report.workspace_spelling.canonical=root;
 const data=path.join(root,'data/Itda');fs.mkdirSync(data,{recursive:true});
 for(const name of ['hosts.json','accounts.json'])fs.writeFileSync(path.join(data,name),'[]');
 fs.writeFileSync(path.join(data,'settings.json'),JSON.stringify({onboarded:true,theme:'light',tray:false,notify:false,announce:false,auto_update:false,thumbnails:false,web:{on:false},share:{on:false},hp:{on:false}}));
 const candidate=path.join(root,'itda.exe');fs.copyFileSync(path.join(repo,'target/release/itda.exe'),candidate);
 assert(hash(candidate)===report.provenance.binary_sha256,'Copied candidate bytes differ');
 originalStartup=control('startup');
 const listener=net.createServer();await new Promise((resolve,reject)=>{listener.once('error',reject);listener.listen(0,'127.0.0.1',resolve);});report.debug_port=listener.address().port;await new Promise(r=>listener.close(r));
 child=spawn(candidate,[],{env:{...process.env,ITDA_DATA_DIR:path.join(root,'data'),ITDA_DEBUG_PORT:String(report.debug_port)},stdio:['ignore','pipe','pipe']});
 ended=new Promise(resolve=>{child.once('error',error=>resolve({error:error.message}));child.once('exit',(code,signal)=>{exited=true;resolve({code,signal});});});
 let stderr='';child.stderr.on('data',b=>{stderr=(stderr+b).slice(-4000);});child.stdout.on('data',()=>{});
 await wait(()=>{
  assert(child.pid&&child.exitCode===null&&child.signalCode===null,'Candidate exited before readiness');
  const state=control('inspect');assert(state.alive,'Candidate process disappeared');
  report.last_native_state=state;
  if(identity)assert(identity.creation_ticks===state.creation_ticks,'Candidate PID was reused');
  identity=state;
  if(state.hwnd==='0'||!state.visible||!state.private_udf||!state.webview_pids.length||!state.listeners.length)return false;
  assert(state.session_id===preflight.checks.session.id,'Candidate launched in another session');
  assert(state.window_class==='itda_home'&&/^Itda \(profile [0-9a-f]{16}\)$/.test(state.window_title),'Pinned HWND is not the isolated home window');
  assert(state.listeners.every(l=>['127.0.0.1','::1'].includes(l.LocalAddress)&&[child.pid,...state.webview_pids].includes(l.OwningProcess)),'CDP listener is not loopback/owned by this private candidate');
  identity=state;return true;
 },'Owned visible native window/private WebView2/CDP did not become ready');
 report.checks.push({check:'native window and private WebView2 profile, same active session, owned loopback CDP listener',result:'PASS'});
 report.native_identity={pid:identity.pid,hwnd:identity.hwnd,creation_ticks:identity.creation_ticks,session_id:identity.session_id,window_class:identity.window_class,window_title:identity.window_title,private_webview_count:identity.webview_pids.length};
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${report.debug_port}`,{timeout:20000});
 const page=browser.contexts()[0]?.pages()[0];assert(page,'Native page missing');
 await page.waitForFunction(()=>typeof S!=='undefined'&&S.settings.onboarded===true,{},{timeout:20000});
 const state=await page.evaluate(()=>({hosts:S.hosts.length,tray:S.settings.tray,onboarded:S.settings.onboarded}));
 assert(state.hosts===0&&state.tray===false&&state.onboarded===true,'Private application configuration was not loaded');
 assert(JSON.stringify(control('startup'))===JSON.stringify(originalStartup),'Isolated candidate changed startup registration');
 report.checks.push({check:'real native page loads isolated empty address book, tray off and unchanged startup registration',result:'PASS'});
 let wrongIdentityRejected=false;try{control('query-end',{...identity,creation_ticks:'1'});}catch(error){wrongIdentityRejected=error.message.includes('Candidate PID creation time changed');}
 assert(wrongIdentityRejected,'A mismatched process identity was not rejected before messaging');
 report.checks.push({check:'incorrect creation-time identity rejected before querying the owned window',result:'PASS'});
 assert(control('query-end').response===1,'Idle WM_QUERYENDSESSION did not permit exit');
 assert(control('inspect').alive,'A shutdown query unexpectedly terminated the app');
 report.checks.push({check:'PID/HWND-bound idle WM_QUERYENDSESSION returns 1 without terminating app',result:'PASS',scope:'query only; no WM_ENDSESSION or OS shutdown/logout'});
 assert(control('close').posted===true,'Owned WM_CLOSE not posted');
 let exit;await wait(async()=>{exit=await Promise.race([ended,new Promise(r=>setTimeout(()=>r(null),100))]);return !!exit;},'Owned WM_CLOSE did not exit cleanly',20000);
 assert(exit.code===0&&!exit.signal&&!exit.error,'Candidate exited abnormally: '+JSON.stringify(exit));
 report.checks.push({check:'owned native main-window close exits cleanly with tray disabled',result:'PASS',exit});
 report.status='passed';
}catch(error){report.error=error.message;process.exitCode=1;}
finally{
 let cleanupError;
 try{
  if(root){
   report.cleanup=control('cleanup');
   const startupUnchanged=originalStartup?JSON.stringify(control('startup'))===JSON.stringify(originalStartup):null;
   fs.rmSync(root,{recursive:true,force:true});assert(!fs.existsSync(root),'Private workspace remains');
   report.cleanup={...report.cleanup,status:'passed',workspace_removed:true,startup_unchanged:startupUnchanged};
   assert(startupUnchanged!==false,'Startup registration changed during pilot');
  }
 }catch(error){cleanupError=error;report.cleanup={...report.cleanup,status:'failed',error:error.message};process.exitCode=1;}
 if(browser){let timer;try{await Promise.race([browser.close().catch(()=>{}),new Promise(r=>{timer=setTimeout(r,3000);})]);}finally{clearTimeout(timer);}}
 // Failed ownership checks must not keep this driver alive on inherited pipe handles.
 // An unverified child is not killed: failed cleanup remains explicit in the receipt.
 child?.stdout?.destroy();child?.stderr?.destroy();child?.unref();
 if(interrupted){report.status='interrupted';report.signal=interrupted;process.exitCode=1;}
 if(cleanupError)report.status='failed';
 fs.writeFileSync(output,JSON.stringify(report,null,2));console.log(JSON.stringify({status:report.status,checks:report.checks.length,cleanup:report.cleanup.status,receipt:output}));
}
