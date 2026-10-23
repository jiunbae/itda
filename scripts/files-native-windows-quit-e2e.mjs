// Real Windows console + WebView2 test, driven from a machine with ssh/scp and Playwright.
// Requires an ALREADY logged-in Windows console owned by the SSH user; no unlock or elevation.
// ITDA_WINDOWS_SSH=<trusted-alias> ITDA_WINDOWS_APP=/local/itda.exe \
// ITDA_SOURCE_COMMIT=<artifact-commit> ITDA_WINDOWS_RUN_ID=<successful-CI-run> PLAYWRIGHT=/path/to/playwright/index.mjs \
// ITDA_SFTP_PYTHON=/private/venv/bin/python node scripts/files-native-windows-quit-e2e.mjs
// Uses an isolated profile, fixture files, temporary same-user interactive Scheduled Task,
// loopback CDP tunnel and PID-verified messages to its own main HWND. No OS shutdown is requested. Stops/removes only its own resources.
// Leaves a local receipt for review. No physical OS input, clipboard or permission changes.
// Verifies source ancestry and exact executable bytes against the successful CI run's artifact before launch (requires gh + repository history containing the commit).
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import {pathToFileURL,fileURLToPath} from 'node:url';
import {execFileSync,spawn} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
const pw=await import(process.env.PLAYWRIGHT ? pathToFileURL(path.resolve(process.env.PLAYWRIGHT)).href : 'playwright');
const host=process.env.ITDA_WINDOWS_SSH,candidate=process.env.ITDA_WINDOWS_APP,source=process.env.ITDA_SOURCE_COMMIT,run=process.env.ITDA_WINDOWS_RUN_ID;
if(!host || !/^[a-zA-Z0-9][a-zA-Z0-9_.-]*$/.test(host) || !candidate || !/^[0-9a-f]{40}$/.test(source || "") || !/^[0-9]+$/.test(run || ""))throw Error('Provide trusted ITDA_WINDOWS_SSH alias, ITDA_WINDOWS_APP executable, ITDA_SOURCE_COMMIT (40 hex), and ITDA_WINDOWS_RUN_ID provenance');
if(!fs.statSync(candidate).isFile())throw Error('ITDA_WINDOWS_APP must be a local executable file');
const scenario=process.env.ITDA_QUIT_SCENARIO;if(!['wait','stop'].includes(scenario))throw Error('ITDA_QUIT_SCENARIO must be wait or stop');
const binaryHash=createHash('sha256').update(fs.readFileSync(candidate)).digest('hex');
const fixtureDir=path.join(path.dirname(fileURLToPath(import.meta.url)),'fixtures');
const python=process.env.ITDA_SFTP_PYTHON;if(!python)throw Error('ITDA_SFTP_PYTHON must identify the private Paramiko interpreter');
let serverProcess,serverConfig,serverRoot,sftpTunnel;
const provenance={scenario,sftp_fixture_sha256:createHash('sha256').update(fs.readFileSync(path.join(fixtureDir,'read-only-sftp-barrier.py'))).digest('hex'),driver_sha256:createHash('sha256').update(fs.readFileSync(fileURLToPath(import.meta.url))).digest('hex'),source_commit:source,binary_sha256:binaryHash,ci_run:Number(run),artifact:null};
const tag='itda-quality-quit-'+randomUUID(),local=fs.mkdtempSync(path.join(os.tmpdir(),tag+'-'));
const ps=s=>execFileSync('ssh',['-o','BatchMode=yes','-o','ConnectTimeout=5',host,'powershell','-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from("$ProgressPreference='SilentlyContinue';"+s,'utf16le').toString('base64')],{encoding:'utf8',timeout:60000}).trim();
const q=s=>"'"+s.replaceAll("'","''")+"'";
const assert=(v,m)=>{if(!v)throw Error(m);};
let remote,tunnel,browser,taskStarted=false,cleanupReceipt,remoteTouched=false,finalizing=false,interrupted=null;const results=[];
function readSftpAudit(){return serverRoot&&fs.existsSync(path.join(serverRoot,'audit.jsonl'))?fs.readFileSync(path.join(serverRoot,'audit.jsonl'),'utf8').trim().split('\n').filter(Boolean).map(x=>JSON.parse(x)):[];}
const resultFile=process.env.ITDA_UI_RESULT || path.join(local,'result.json');
console.log('Native UI receipt: '+resultFile);
let cleaning=false;
function cleanupRemote(){
 if(cleaning)return;
 cleaning=true;
 if(!remoteTouched){cleanupReceipt={remote:'not-created'};return;}
 tunnel?.kill();
 {const raw=ps(`$r=${remote?.root?q(remote.root):"(Join-Path $env:TEMP '"+tag+"')"};$exe=Join-Path $r 'itda.exe';Get-CimInstance Win32_Process -Filter "Name='itda.exe'"|Where-Object {$_.ExecutablePath-eq $exe}|ForEach-Object{Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue};Start-Sleep -Seconds 1;for($i=0;$i-lt 20;$i++){if((Get-ScheduledTask -TaskName '${tag}' -ErrorAction SilentlyContinue).State-ne 'Running'){break};Start-Sleep -Milliseconds 250};Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'"|Where-Object {$_.CommandLine -like ('*'+$r+'*')}|ForEach-Object{Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue};$finished=$null;$end=Join-Path $r 'finished.json';if(Test-Path -LiteralPath $end){$finished=Get-Content -LiteralPath $end -Raw|ConvertFrom-Json};Stop-ScheduledTask -TaskName '${tag}' -ErrorAction SilentlyContinue;Unregister-ScheduledTask -TaskName '${tag}' -Confirm:$false -ErrorAction SilentlyContinue;if(Test-Path -LiteralPath $r){try{Remove-Item -LiteralPath $r -Recurse -Force -ErrorAction Stop}catch{throw ('Fixture directory remains: '+$r+'; '+$_)}};[pscustomobject]@{end_receipt_present=($null-ne $finished);startup_unchanged_end=$finished.startup_unchanged_end}|ConvertTo-Json -Compress`);cleanupReceipt=JSON.parse(raw.replace(/^\uFEFF/,''));if(taskStarted&&(!cleanupReceipt.end_receipt_present||!cleanupReceipt.startup_unchanged_end))throw Error('Native startup state changed or final guard receipt missing');}
}
async function cleanupServer(){
 sftpTunnel?.kill();
 if(serverProcess&&serverProcess.exitCode===null&&serverProcess.signalCode===null){serverProcess.kill();await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{serverProcess.kill('SIGKILL');},3000);const fail=setTimeout(()=>reject(Error('Owned SFTP fixture did not stop')),6000);serverProcess.once('exit',()=>{clearTimeout(timer);clearTimeout(fail);resolve();});});}
 if(serverRoot){if(!path.basename(serverRoot).startsWith('itda-quality-sftp-')||path.resolve(path.dirname(serverRoot))!==path.resolve(os.tmpdir()))throw Error('SFTP workspace ownership mismatch during cleanup');fs.rmSync(serverRoot,{recursive:true,force:true});if(fs.existsSync(serverRoot))throw Error('SFTP fixture workspace remains after cleanup');}
}
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,async()=>{
 if(finalizing){interrupted=signal;process.exitCode=signal==='SIGINT'?130:143;return;}
 let cleanupError;
 try{cleanupRemote();}catch(e){cleanupError=e;}
 try{await cleanupServer();}catch(e){cleanupError||=e;}
 fs.writeFileSync(resultFile,JSON.stringify({status:'interrupted',signal,...provenance,results,cleanup:{status:cleanupError?'failed':'passed',...cleanupReceipt,sftp_workspace_removed:!serverRoot||!fs.existsSync(serverRoot),...(cleanupError?{error:cleanupError.message}:{})}},null,2));
 process.exit(signal==='SIGINT'?130:143);
});
try{
 // Bind the supplied executable to an actual successful Itda CI artifact before
 // launching anything on Windows. A label/ancestry check alone cannot bind bytes.
 const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
 execFileSync('git',['merge-base','--is-ancestor','5c72e50',source],{cwd:repo,stdio:'pipe'});
 const ci=JSON.parse(execFileSync('gh',['api',`repos/jiunbae/itda/actions/runs/${run}`],{encoding:'utf8',timeout:30000}));
 assert(ci.head_sha===source && ci.repository?.full_name==='jiunbae/itda' && ci.head_repository?.full_name==='jiunbae/itda' && ci.path==='.github/workflows/ci.yml' && ci.conclusion==='success','Candidate must match a successful Itda CI run');
 const artifacts=JSON.parse(execFileSync('gh',['api',`repos/jiunbae/itda/actions/runs/${run}/artifacts`],{encoding:'utf8',timeout:30000})).artifacts.filter(a=>a.name==='itda-windows-x64'&&!a.expired);
 assert(artifacts.length===1,'Need exactly one current itda-windows-x64 artifact');
 provenance.artifact={id:artifacts[0].id,name:artifacts[0].name};
 const artifact=path.join(local,'ci-artifact');fs.mkdirSync(artifact);
 execFileSync('gh',['run','download',run,'--repo','jiunbae/itda','--name','itda-windows-x64','--dir',artifact],{stdio:'pipe',timeout:60000});
 assert(fs.readdirSync(artifact,{recursive:true}).filter(n=>path.basename(n).toLowerCase()==='itda.exe').length===1,'Need exactly one executable in CI artifact');
 assert(createHash('sha256').update(fs.readFileSync(path.join(artifact,'itda.exe'))).digest('hex')===binaryHash,'Supplied candidate differs from the identified CI executable');
 provenance.ci_artifact_verified=true;
 // Establish read-only SSH reachability before starting fixtures or attempting a
 // remote mutation. An offline device is an unmet prerequisite, not a quit failure.
 provenance.phase='ssh-preflight';
 assert(ps("Write-Output 'itda-quality-reachable'")==='itda-quality-reachable','Windows SSH preflight failed');
 provenance.ssh_preflight_passed=true;
 serverRoot=fs.mkdtempSync(path.join(os.tmpdir(),'itda-quality-sftp-'));fs.chmodSync(serverRoot,0o700);
 serverProcess=spawn(python,[path.join(fixtureDir,'read-only-sftp-barrier.py'),'--workspace',serverRoot],{stdio:['ignore','pipe','ignore']});
 let serverOutput='';const manifestPath=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('SFTP fixture startup timed out')),20000);serverProcess.on('error',reject);serverProcess.on('exit',()=>reject(Error('SFTP fixture exited before startup')));serverProcess.stdout.on('data',b=>{serverOutput+=b;if(serverOutput.includes('\n')){clearTimeout(timer);resolve(serverOutput.split('\n')[0].trim());}});});
 serverConfig=JSON.parse(fs.readFileSync(manifestPath,'utf8'));assert(serverConfig.workspace===serverRoot,'SFTP fixture returned another workspace');
 assert(serverConfig.pid===serverProcess.pid && path.basename(serverRoot).startsWith('itda-quality-sftp-') && path.resolve(path.dirname(serverRoot))===path.resolve(os.tmpdir()),'SFTP fixture ownership mismatch');
 provenance.sftp_library=serverConfig.paramiko_version;

 remoteTouched=true;
 remote=JSON.parse(ps(`$ErrorActionPreference='Stop';$r=Join-Path $env:TEMP '${tag}';New-Item -ItemType Directory -Path $r|Out-Null;New-Item -ItemType Directory -Path (Join-Path $r 'data\\Itda')|Out-Null;New-Item -ItemType Directory -Path (Join-Path $r 'files')|Out-Null;New-Item -ItemType Directory -Path (Join-Path $r 'files\\Projects')|Out-Null;Set-Content -LiteralPath (Join-Path $r 'files\\Notes.txt') -Value 'Native Windows preview fixture' -Encoding UTF8;$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$l.Start();$port=$l.LocalEndpoint.Port;$f=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$f.Start();$filePort=$f.LocalEndpoint.Port;$f.Stop();$l.Stop();New-Item -ItemType Directory -Path (Join-Path $r 'drop-destination')|Out-Null;[pscustomobject]@{root=$r;port=$port;file_port=$filePort;user=[Security.Principal.WindowsIdentity]::GetCurrent().Name}|ConvertTo-Json -Compress`));
 const cfg=path.join(local,'config');fs.mkdirSync(cfg);
 const fixtureId='quality-'+tag;
 fs.writeFileSync(path.join(cfg,'hosts.json'),JSON.stringify([{id:fixtureId,name:'Fixture SFTP',host:'127.0.0.1',os:'mac',kind:'vnc',user:'',opts:{},files:{method:'sftp',host:'127.0.0.1',port:remote.file_port,user:''}}]));fs.writeFileSync(path.join(cfg,'accounts.json'),'[]');
 fs.writeFileSync(path.join(cfg,'settings.json'),JSON.stringify({onboarded:true,theme:'light',tray:false,notify:false,announce:false,auto_update:false,thumbnails:false,web:{on:false},share:{on:false},hp:{on:false}}));
 fs.writeFileSync(path.join(cfg,'known_hosts'),`[127.0.0.1]:${remote.file_port} ${serverConfig.host_key_type} ${serverConfig.host_key_base64}\n`);
 const remoteRoot=remote.root.replaceAll('\\','/');
 execFileSync('scp',[candidate,`${host}:${remoteRoot}/itda.exe`],{stdio:'pipe',timeout:60000});
 assert(ps(`(Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path ${q(remote.root)} 'itda.exe')).Hash`).toLowerCase()===binaryHash,'Remote executable hash differs from supplied candidate');
 for(const name of ['hosts.json','accounts.json','settings.json','known_hosts'])execFileSync('scp',[path.join(cfg,name),`${host}:${remoteRoot}/data/Itda/${name}`],{stdio:'pipe',timeout:20000});
 sftpTunnel=spawn('ssh',['-o','BatchMode=yes','-o','ExitOnForwardFailure=yes','-N','-R',`127.0.0.1:${remote.file_port}:127.0.0.1:${serverConfig.port}`,host],{stdio:'ignore'});
 let forward=false;for(let i=0;i<20;i++){try{forward=ps(`$c=[Net.Sockets.TcpClient]::new();try{$c.Connect('127.0.0.1',${remote.file_port});$c.GetStream().ReadTimeout=1000;$b=New-Object byte[] 64;$n=$c.GetStream().Read($b,0,64);[Text.Encoding]::ASCII.GetString($b,0,$n)}finally{$c.Close()}`).startsWith('SSH-2.0-');}catch{}if(forward)break;await new Promise(r=>setTimeout(r,200));}assert(forward,'SFTP reverse loopback tunnel not ready');
 const launch=String.raw`$ErrorActionPreference='Stop';$r=${q(remote.root)};$exe=Join-Path $r 'itda.exe';$run='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run';$before=(Get-ItemProperty -Path $run -Name Itda -ErrorAction SilentlyContinue).Itda;$p=$null;$existing=@(Get-Process itda -ErrorAction SilentlyContinue|Select-Object -ExpandProperty Id)
 try{$env:ITDA_DATA_DIR=Join-Path $r 'data';$env:ITDA_DEBUG_PORT='${remote.port}';$env:ITDA_DEVTOOLS='1';$p=Start-Process -FilePath $exe -PassThru -WindowStyle Minimized;if($existing-contains $p.Id){throw 'candidate PID was already present'}
 $isolated=$false;for($i=0;$i-lt 60;$i++){$p.Refresh();if($p.HasExited){throw 'candidate exited before isolation verified'};if((Test-Path (Join-Path $env:ITDA_DATA_DIR 'Itda\WebView2'))-and(@(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" -ErrorAction SilentlyContinue|Where-Object {$_.CommandLine -like ('*'+$r+'*')}).Count-gt 0)){$isolated=$true;break};Start-Sleep -Milliseconds 250};if(-not $isolated){Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue;throw 'private WebView2 profile not verified; stopped candidate'}
 for($i=0;$i-lt 60;$i++){$p.Refresh();if($p.HasExited){throw 'candidate exited'};if($p.MainWindowHandle-ne [IntPtr]::Zero){break};Start-Sleep -Milliseconds 500}
 Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class ItdaQualityWindow{[DllImport("user32.dll")]public static extern bool ShowWindowAsync(IntPtr h,int c);[DllImport("user32.dll")]static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll",SetLastError=true)]static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);[DllImport("user32.dll",SetLastError=true)]static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);static IntPtr Own(uint pid,long value){var h=new IntPtr(value);uint actual;if(pid==0||h==IntPtr.Zero||GetWindowThreadProcessId(h,out actual)==0||actual!=pid)throw new InvalidOperationException("Fixture HWND is not owned by its candidate PID");return h;}public static long QueryEnd(uint pid,long value){UIntPtr result;if(SendMessageTimeoutW(Own(pid,value),0x11,UIntPtr.Zero,IntPtr.Zero,2,2000,out result)==IntPtr.Zero)throw new InvalidOperationException("Owned query timed out");return checked((long)result.ToUInt64());}public static void RequestClose(uint pid,long value){if(!PostMessageW(Own(pid,value),0x10,IntPtr.Zero,IntPtr.Zero))throw new InvalidOperationException("Owned WM_CLOSE failed");}}'
 [ItdaQualityWindow]::ShowWindowAsync($p.MainWindowHandle,4)|Out-Null
 $main=$p.MainWindowHandle.ToInt64()
 $after=(Get-ItemProperty -Path $run -Name Itda -ErrorAction SilentlyContinue).Itda
 [pscustomobject]@{pid=$p.Id;hwnd=$p.MainWindowHandle.ToInt64();existing_app_processes=$existing.Count;existing_app_pids=$existing;startup_unchanged=($before-eq $after);startup_entry_existed=($null-ne $before);udf_exists=(Test-Path (Join-Path $env:ITDA_DATA_DIR 'Itda\WebView2'))}|ConvertTo-Json -Compress|Set-Content -LiteralPath (Join-Path $r 'started.json') -Encoding UTF8
 $until=[DateTime]::UtcNow.AddSeconds(300);while(-not $p.HasExited-and[DateTime]::UtcNow-lt $until){$f=Join-Path $r 'command.json';if(Test-Path -LiteralPath $f){$c=Get-Content -LiteralPath $f -Raw|ConvertFrom-Json;Remove-Item -LiteralPath $f;try{$value=$null;if($c.op-eq 'query-end'){$value=[ItdaQualityWindow]::QueryEnd($p.Id,$main)}elseif($c.op-eq 'close-main'){[ItdaQualityWindow]::RequestClose($p.Id,$main);$value=$true}else{throw 'Unknown fixture operation'};[pscustomobject]@{request=$c.request;value=$value}|ConvertTo-Json -Compress|Set-Content -LiteralPath (Join-Path $r 'command-result.json') -Encoding UTF8}catch{[pscustomobject]@{request=$c.request;error=$_.Exception.Message}|ConvertTo-Json -Compress|Set-Content -LiteralPath (Join-Path $r 'command-result.json') -Encoding UTF8}};Start-Sleep -Milliseconds 100;$p.Refresh()}
 }catch{$_|Out-String|Set-Content -LiteralPath (Join-Path $r 'error.txt') -Encoding UTF8}
 finally{if($p-and-not $p.HasExited){Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue};$current=(Get-ItemProperty -Path $run -Name Itda -ErrorAction SilentlyContinue).Itda;$unchanged=($before-eq $current);if($current-and $current.Contains($exe)){if($null-ne $before){Set-ItemProperty -Path $run -Name Itda -Value $before}else{Remove-ItemProperty -Path $run -Name Itda -ErrorAction SilentlyContinue}};[pscustomobject]@{startup_unchanged_end=$unchanged}|ConvertTo-Json -Compress|Set-Content -LiteralPath (Join-Path $r 'finished.json') -Encoding UTF8}
 `;
 fs.writeFileSync(path.join(local,'action.json'),JSON.stringify({args:'-NoProfile -NonInteractive -EncodedCommand '+Buffer.from(launch,'utf16le').toString('base64')}));
 execFileSync('scp',[path.join(local,'action.json'),`${host}:${remoteRoot}/action.json`],{stdio:'pipe',timeout:20000});
 ps(`$ErrorActionPreference='Stop';$r=${q(remote.root)};$a=Get-Content -LiteralPath (Join-Path $r 'action.json') -Raw|ConvertFrom-Json;$act=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $a.args;$principal=New-ScheduledTaskPrincipal -UserId ${q(remote.user)} -LogonType Interactive -RunLevel Limited;$settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 6);Register-ScheduledTask -TaskName '${tag}' -Action $act -Principal $principal -Settings $settings|Out-Null;Start-ScheduledTask -TaskName '${tag}'`);
 taskStarted=true;
 console.log('Started isolated native Windows candidate task');
 let start;
 for(let i=0;i<50;i++){
  const text=ps(`$r=${q(remote.root)};if(Test-Path (Join-Path $r 'error.txt')){Write-Output ('ERROR:'+(Get-Content (Join-Path $r 'error.txt') -Raw))}elseif(Test-Path (Join-Path $r 'started.json')){Get-Content (Join-Path $r 'started.json') -Raw}`);
  if(text.startsWith('ERROR:'))throw Error(text);
  if(text){start=JSON.parse(text.replace(/^\uFEFF/,''));break;}
  await new Promise(r=>setTimeout(r,500));
 }
 assert(start?.pid&&start.hwnd,'native candidate window not ready');assert(start.udf_exists,'private WebView2 UDF missing');assert(start.startup_unchanged,'isolated startup changed normal Run entry');
 const owned=ps(`$r=${q(remote.root)};@(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'"|Where-Object {$_.CommandLine -like ('*'+$r+'*')}).Count`);assert(Number(owned)>0,'no WebView2 process with private profile path');
 const l=net.createServer();await new Promise(r=>l.listen(0,'127.0.0.1',r));const port=l.address().port;await new Promise(r=>l.close(r));
 tunnel=spawn('ssh',['-o','BatchMode=yes','-o','ExitOnForwardFailure=yes','-N','-L',`127.0.0.1:${port}:127.0.0.1:${remote.port}`,host],{stdio:'ignore'});
 let ready=false;for(let i=0;i<60;i++){try{ready=(await fetch(`http://127.0.0.1:${port}/json/version`,{signal:AbortSignal.timeout(1000)})).ok;}catch{}if(ready)break;await new Promise(r=>setTimeout(r,500));}assert(ready,'native CDP endpoint unavailable');
 browser=await pw.chromium.connectOverCDP(`http://127.0.0.1:${port}`,{timeout:30000});
 const context=browser.contexts()[0],page=context.pages()[0];await page.waitForFunction(id=>typeof S!=='undefined'&&S.hosts.some(h=>h.id===id),fixtureId,{timeout:30000});
 results.push({check:'actual native Windows WebView2 with private UDF and unchanged startup entry',result:'PASS',startup_entry_existed:start.startup_entry_existed,existing_app_processes:start.existing_app_processes,existing_app_pids:start.existing_app_pids});

 const audit=readSftpAudit;
 const waitFor=async (test,message,ms=15000)=>{const end=Date.now()+ms;while(Date.now()<end){if(await test())return;await new Promise(r=>setTimeout(r,100));}throw Error(message);};
 await page.evaluate(()=>{window.quitObserved=[];const original=itda.recv;itda.recv=function(m){if(['quit_guard','quit_pending'].includes(m.type))quitObserved.push({type:m.type,reason:m.reason,count:m.count,stuck:m.stuck,waiting:m.waiting,phase:m.phase});return original.call(this,m);};});
 const command=async op=>{const request=randomUUID(),value=JSON.stringify({op,request});ps(`$r=${q(remote.root)};Set-Content -LiteralPath (Join-Path $r 'command.tmp') -Value ${q(value)} -Encoding UTF8;Move-Item -LiteralPath (Join-Path $r 'command.tmp') -Destination (Join-Path $r 'command.json') -Force`);for(let i=0;i<30;i++){const raw=ps(`$p=Join-Path ${q(remote.root)} 'command-result.json';if(Test-Path -LiteralPath $p){Get-Content -LiteralPath $p -Raw}`);if(raw){const answer=JSON.parse(raw.replace(/^\uFEFF/,''));if(answer.request===request){assert(!answer.error,answer.error);return answer.value;}}await new Promise(r=>setTimeout(r,150));}throw Error('Owned native message acknowledgement timed out');};
 assert(await command('query-end')===1,'Idle native app did not allow WM_QUERYENDSESSION');
 results.push({check:'PID-verified native main-window shutdown query allows an idle candidate',result:'PASS',scope:'WM_QUERYENDSESSION only; no real OS shutdown/logout'});

 const openSource=async()=>{
   const newPage=context.waitForEvent('page');await page.getByRole('button',{name:'파일 브라우저',exact:true}).click();const f=await newPage;await f.locator('.files-layer').waitFor();
   await f.locator(`[data-device="${fixtureId}"]`).first().click();
   const setup=f.locator('.files-setup');await setup.waitFor();await setup.locator('[name=account]').selectOption('custom');await setup.locator('[name=user]').fill(serverConfig.user);await setup.locator('.files-password > summary').click();await setup.locator('[name=password]').fill(serverConfig.password);await setup.locator('[name=save_password]').uncheck();await setup.locator('[data-connect]').click();
   const row=f.locator('[data-pane=remote] [data-index]').filter({hasText:'payload.bin'}).first();await row.waitFor({timeout:45000});
   const sourceTab=f.getByRole('tab').first();await f.locator('[data-action=new-tab]').click();const localTab=f.getByRole('tab').last();await f.locator('[data-network-sidebar] [data-side=local]').click();await f.keyboard.press('Control+l');const address=f.getByRole('textbox',{name:'주소',exact:true});await address.fill(remote.root+'\\files\\Projects');await address.press('Enter');await f.locator('[data-pane=local] .files-note').filter({hasText:'표시할 항목이 없습니다'}).waitFor();
   await sourceTab.click();await row.waitFor();
   return {f,row,localTab};
 };
 const beginCopy=async()=>{
   fs.rmSync(path.join(serverRoot,'release-read'),{force:true});fs.writeFileSync(path.join(serverRoot,'hold-read'),'hold');
   const before=audit().filter(e=>e.event==='read-blocked').length;
   const {f,row,localTab}=await openSource(),from=await row.boundingBox(),to=await localTab.boundingBox();assert(from&&to,'Generated source row/local target tab missing');
   await f.mouse.move(from.x+50,from.y+from.height/2);await f.mouse.down();await f.mouse.move(to.x+to.width/2,to.y+to.height/2,{steps:16});await f.mouse.up();
   const dialog=f.getByRole('dialog',{name:'파일 복사',exact:true});await dialog.waitFor();assert((await dialog.textContent()).includes('payload.bin'),'Copy confirmation changed source');await dialog.getByRole('button',{name:'복사',exact:true}).click();
   await waitFor(()=>audit().filter(e=>e.event==='read-blocked').length>before,'Source never reached the controlled mid-stream barrier');
   assert(audit().some(e=>e.event==='read-bytes'&&e.path==='/payload.bin'&&e.length>0),'No actual source bytes read before the barrier');
   return f;
 };
 const inspectDestination=()=>JSON.parse(ps(`$r=Join-Path ${q(remote.root)} 'files\\Projects';$a=@(Get-ChildItem -LiteralPath $r -Force -Recurse|ForEach-Object{$entry=$_;$size=$entry.Length;$hash='';if($entry.Name-eq 'payload.bin'){$hash=(Get-FileHash -LiteralPath $entry.FullName -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()}elseif(-not $entry.PSIsContainer){try{$f=[IO.File]::Open($entry.FullName,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite-bor[IO.FileShare]::Delete));try{$size=$f.Length}finally{$f.Dispose()}}catch{}};[pscustomobject]@{name=$entry.Name;size=$size;sha256=$hash}});ConvertTo-Json -InputObject $a -Compress`));

 const alive=()=>ps(`$p=Get-Process -Id ${start.pid} -ErrorAction SilentlyContinue;if($p-and $p.Path-eq (Join-Path ${q(remote.root)} 'itda.exe')){'alive'}else{'gone'}`)==='alive';
 provenance.phase='midstream-preconditions';
 const filePage=await beginCopy();await waitFor(()=>inspectDestination().some(f=>/^\.itda-.*\.partial$/.test(f.name)&&f.size>0),'No actual staging bytes before quit request');
 const held=audit().filter(e=>e.event==='read-blocked').at(-1),staged=inspectDestination().find(f=>/^\.itda-.*\.partial$/.test(f.name)&&f.size>0);
 results.push({check:'actual SFTP copy is paused after a nonzero destination stage',result:'PASS',scope:'precondition only',barrier_offset:held.offset,staging_bytes:staged.size});
 provenance.phase='busy-query-and-cancel';
 assert(await command('query-end')===0,'Busy native app allowed the shutdown query');
 const guard=page.locator('[data-key="quit-guard"]');await guard.waitFor();assert((await guard.textContent()).includes('1개'),'Native query guard did not show the active transfer count');
 await guard.locator('[data-x=cancel]').click();await guard.waitFor({state:'detached'});await new Promise(r=>setTimeout(r,1000));
 assert(alive(),'Cancelling quit ended the app');assert(!audit().some(e=>e.event==='read-aborted'),'Cancelling quit stopped the source read');assert(inspectDestination().some(f=>/^\.itda-.*\.partial$/.test(f.name)),'Cancel quit discarded the active transfer');
 results.push({check:'busy native shutdown query is refused; cancelling its guard leaves app and paused copy alive',result:'PASS',query:0,no_actual_shutdown:true});
 provenance.phase='main-close-'+scenario;
 await command('close-main');await guard.waitFor();assert(alive(),'Main WM_CLOSE bypassed the guard');
 const closed=page.waitForEvent('close',{timeout:45000});
 if(scenario==='wait'){
   await guard.locator('[data-x=wait]').click();await page.locator('#quit-pending').waitFor();await new Promise(r=>setTimeout(r,1000));assert(alive(),'Wait quit ended before the paused transfer could finish');
   provenance.guard_events=await page.evaluate(()=>quitObserved);fs.writeFileSync(path.join(serverRoot,'release-read'),'resume');await closed;
   await waitFor(()=>!alive(),'Candidate did not exit after its transfer finished',10000);
   const payload=serverConfig.files.find(f=>f.path==='/payload.bin'),copied=inspectDestination();assert(copied.length===1&&copied[0].name==='payload.bin'&&copied[0].sha256===payload.sha256,'Wait quit did not retain exactly one hash-matching completed file');
   results.push({check:'native home WM_CLOSE (tray disabled) waits for real copy completion then exits; destination SHA256 matches',result:'PASS',bytes:payload.size,sha256:payload.sha256});
 }else{
   await guard.locator('[data-x=stop]').click();await closed;await waitFor(()=>!alive(),'Candidate did not exit after cancelled-transfer cleanup',10000);
   await waitFor(()=>audit().some(e=>e.event==='read-aborted'&&e.offset===held.offset&&e.reason==='transport_closed'),'Stop quit did not abort the held SSH transport');
   assert(!audit().some(e=>e.event==='read-bytes'&&e.path==='/payload.bin'&&e.offset>=held.offset),'Stop quit advanced the paused source read');assert(inspectDestination().length===0,'Stop quit left destination or partial staging files');
   results.push({check:'native home WM_CLOSE (tray disabled) stops a real mid-stream copy and exits only after staging cleanup',result:'PASS',cancelled_before_barrier_release:true});
 }
 const payload=serverConfig.files.find(f=>f.path==='/payload.bin');assert(createHash('sha256').update(fs.readFileSync(path.join(serverRoot,'files','payload.bin'))).digest('hex')===payload.sha256,'Source fixture changed');
 provenance.phase='exited-after-'+scenario;

 fs.writeFileSync(resultFile,JSON.stringify({...provenance,method:'actual Windows console app, existing WebView2 attached through private loopback CDP/SSH tunnel; no emulated Windows UA',results,status:results.some(r=>r.result==='FAIL')?'findings':'checks-passed-cleanup-pending',limits:['candidate build, not production rollout','loopback CDP is reachable by other local processes; use a trusted test account/machine','CDP renderer input does not prove physical keyboard/global Windows accelerators','no real Explorer OLE drop, physical input, screen reader, multi-monitor or phone verified','OS notifications disabled and untested; no actual OS shutdown, logout or physical tray menu click','Only managed SFTP copy and this scenario are tested; timeout/force, update, installer, native drags and Mac quit are unverified','Shutdown query and main WM_CLOSE target only the PID-verified private app; no production instance was signalled']},null,2));
 console.log(JSON.stringify(results));if(results.some(r=>r.result==='FAIL'))process.exitCode=1;
}catch(e){fs.writeFileSync(resultFile,JSON.stringify({status:'failed',results,error:e.message,local,...provenance,sftp_audit:readSftpAudit()},null,2));throw e;}finally{
 finalizing=true;
 let cleanupError;
 try{cleanupRemote();}catch(e){cleanupError=e;}
 try{await cleanupServer();}catch(e){cleanupError||=e;}
 let timer;
 try{await Promise.race([browser?.close(),new Promise(resolve=>{timer=setTimeout(resolve,3000);})]);}catch{}finally{clearTimeout(timer);}
 const receipt=fs.existsSync(resultFile)?JSON.parse(fs.readFileSync(resultFile,'utf8')):{...provenance,results};
 receipt.cleanup={status:cleanupError?'failed':'passed',...cleanupReceipt,sftp_workspace_removed:!serverRoot||!fs.existsSync(serverRoot),...(cleanupError?{error:cleanupError.message}:{})};
 if(interrupted){receipt.status='interrupted';receipt.signal=interrupted;}
 if(cleanupError)receipt.status='cleanup-failed';else if(receipt.status==='checks-passed-cleanup-pending')receipt.status='passed';
 fs.writeFileSync(resultFile,JSON.stringify(receipt,null,2));
 if(cleanupError)throw cleanupError;
 console.log('Removed native candidate task and remote fixture');
}
