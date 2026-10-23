// Real Windows console + WebView2 test, driven from a machine with ssh/scp and Playwright.
// Requires an ALREADY logged-in Windows console owned by the SSH user; no unlock or elevation.
// ITDA_WINDOWS_SSH=<trusted-alias> ITDA_WINDOWS_APP=/local/itda.exe \
// ITDA_SOURCE_COMMIT=<artifact-commit> ITDA_WINDOWS_RUN_ID=<successful-CI-run> PLAYWRIGHT=/path/to/playwright/index.mjs \
// ITDA_SFTP_PYTHON=/private/venv/bin/python node scripts/files-native-windows-background-e2e.mjs
// Uses an isolated profile, fixture files, temporary same-user interactive Scheduled Task,
// loopback CDP tunnel and actual native caption close. Stops/removes only its own resources.
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
const binaryHash=createHash('sha256').update(fs.readFileSync(candidate)).digest('hex');
const fixtureDir=path.join(path.dirname(fileURLToPath(import.meta.url)),'fixtures');
const python=process.env.ITDA_SFTP_PYTHON;if(!python)throw Error('ITDA_SFTP_PYTHON must identify the private Paramiko interpreter');
let serverProcess,serverConfig,serverRoot,sftpTunnel;
const provenance={sftp_fixture_sha256:createHash('sha256').update(fs.readFileSync(path.join(fixtureDir,'read-only-sftp-barrier.py'))).digest('hex'),driver_sha256:createHash('sha256').update(fs.readFileSync(fileURLToPath(import.meta.url))).digest('hex'),source_commit:source,binary_sha256:binaryHash,ci_run:Number(run),artifact:null};
const tag='itda-quality-bg-'+randomUUID(),local=fs.mkdtempSync(path.join(os.tmpdir(),tag+'-'));
const ps=s=>execFileSync('ssh',['-o','BatchMode=yes','-o','ConnectTimeout=5',host,'powershell','-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from("$ProgressPreference='SilentlyContinue';"+s,'utf16le').toString('base64')],{encoding:'utf8',timeout:60000}).trim();
const q=s=>"'"+s.replaceAll("'","''")+"'";
const assert=(v,m)=>{if(!v)throw Error(m);};
let remote,tunnel,browser,taskStarted=false,cleanupReceipt,remoteTouched=false,finalizing=false,interrupted=null;const results=[];
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
 if(serverRoot&&path.basename(serverRoot).startsWith('itda-quality-sftp-')&&path.resolve(path.dirname(serverRoot))===path.resolve(os.tmpdir()))fs.rmSync(serverRoot,{recursive:true,force:true});
}
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,async()=>{
 if(finalizing){interrupted=signal;process.exitCode=signal==='SIGINT'?130:143;return;}
 let cleanupError;
 try{cleanupRemote();}catch(e){cleanupError=e;}
 fs.writeFileSync(resultFile,JSON.stringify({status:'interrupted',signal,...provenance,results,cleanup:{status:cleanupError?'failed':'passed',...cleanupReceipt,...(cleanupError?{error:cleanupError.message}:{})}},null,2));
 try{await cleanupServer();}catch{}
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
 fs.writeFileSync(path.join(cfg,'settings.json'),JSON.stringify({onboarded:true,theme:'light',notify:false,announce:false,auto_update:false,thumbnails:false,web:{on:false},share:{on:false},hp:{on:false}}));
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
 Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class ItdaQualityWindow{public delegate bool CB(IntPtr h,IntPtr p);[DllImport("user32.dll")]public static extern bool ShowWindowAsync(IntPtr h,int c);[DllImport("user32.dll")]static extern bool EnumWindows(CB cb,IntPtr p);[DllImport("user32.dll")]static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll")]static extern bool IsWindowVisible(IntPtr h);[DllImport("user32.dll",CharSet=CharSet.Unicode)]static extern int GetWindowTextLength(IntPtr h);[DllImport("user32.dll")]static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);[DllImport("user32.dll")]static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int hh,uint flags);public static long ResizeFile(uint pid,long main,int w,int hh){var a=new System.Collections.Generic.List<IntPtr>();EnumWindows((h,p)=>{uint id;GetWindowThreadProcessId(h,out id);if(id==pid&&h.ToInt64()!=main&&IsWindowVisible(h)&&GetWindowTextLength(h)>0)a.Add(h);return true;},IntPtr.Zero);if(a.Count!=1||w<380||w>3000||hh<300||hh>3000)return 0;var old=SetThreadDpiAwarenessContext(new IntPtr(-4));if(old==IntPtr.Zero)return 0;try{return SetWindowPos(a[0],IntPtr.Zero,50,50,w,hh,0x14)?a[0].ToInt64():0;}finally{SetThreadDpiAwarenessContext(old);}}}'
 [ItdaQualityWindow]::ShowWindowAsync($p.MainWindowHandle,4)|Out-Null
 $after=(Get-ItemProperty -Path $run -Name Itda -ErrorAction SilentlyContinue).Itda
 [pscustomobject]@{pid=$p.Id;hwnd=$p.MainWindowHandle.ToInt64();existing_app_processes=$existing.Count;existing_app_pids=$existing;startup_unchanged=($before-eq $after);startup_entry_existed=($null-ne $before);udf_exists=(Test-Path (Join-Path $env:ITDA_DATA_DIR 'Itda\WebView2'))}|ConvertTo-Json -Compress|Set-Content -LiteralPath (Join-Path $r 'started.json') -Encoding UTF8
 $main=$p.MainWindowHandle.ToInt64();$until=[DateTime]::UtcNow.AddSeconds(300);while(-not $p.HasExited-and[DateTime]::UtcNow-lt $until){$f=Join-Path $r 'command.json';if(Test-Path $f){$c=Get-Content $f -Raw|ConvertFrom-Json;Remove-Item $f;if($c.op-eq 'resize-file'){$hw=[ItdaQualityWindow]::ResizeFile($p.Id,$main,$c.width,$c.height);[pscustomobject]@{request=$c.request;hwnd=$hw}|ConvertTo-Json -Compress|Set-Content -LiteralPath (Join-Path $r 'command-result.json') -Encoding UTF8}};Start-Sleep -Milliseconds 100;$p.Refresh()}
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

 const audit=()=>fs.existsSync(path.join(serverRoot,'audit.jsonl'))?fs.readFileSync(path.join(serverRoot,'audit.jsonl'),'utf8').trim().split('\n').filter(Boolean).map(x=>JSON.parse(x)):[];
 const waitFor=async (test,message,ms=15000)=>{const end=Date.now()+ms;while(Date.now()<end){if(await test())return;await new Promise(r=>setTimeout(r,100));}throw Error(message);};
 await page.evaluate(()=>{window.bgObserved=[];const original=ItdaFiles.message;ItdaFiles.message=function(m){if(m.type==='files_job'||m.type==='files_background')bgObserved.push({type:m.type,count:m.count,window:m.window,orphan:m.orphan,state:m.state,job:m.job,error:m.error});return original.call(this,m);};});
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
 const nav=page.locator('[data-transfers]');
 // Cancel a genuine mid-stream transfer after the actual native file-window close.
 const first=await beginCopy();await waitFor(()=>inspectDestination().some(f=>/^\.itda-.*\.partial$/.test(f.name)&&f.size>0),'No actual destination staging bytes before native window close');const closed=first.waitForEvent('close');await first.locator('[data-window=close]').click();await closed;
 await nav.waitFor();await nav.click();const dialog=page.getByRole('dialog',{name:'전송',exact:true});await dialog.waitFor();
 const liveRow=dialog.locator('.transfer-row').filter({has:page.getByRole('button',{name:'취소',exact:true})});await liveRow.waitFor();
 const firstJob=(await liveRow.getAttribute('data-id')).slice('transfer-'.length);
 assert(await page.evaluate(job=>bgObserved.some(m=>m.job===job&&m.window==='app'&&m.orphan===true)&&bgObserved.some(m=>m.type==='files_background'&&m.count>=1),firstJob),'Closed native window did not publish adopted job and background notice');
 assert(!inspectDestination().some(f=>f.name==='payload.bin'),'Destination committed while source barrier held');
 const cancelStart=audit().length,barrier=audit().filter(e=>e.event==='read-blocked').at(-1);
 await liveRow.getByRole('button',{name:'취소',exact:true}).click();await new Promise(r=>setTimeout(r,1000));
 assert(!inspectDestination().some(f=>f.name==='payload.bin'),'Cancelled transfer committed while source remained blocked');
 await dialog.locator('.transfer-row.cancelled').waitFor({timeout:10000});
 assert(await page.evaluate(job=>bgObserved.some(m=>m.job===job&&m.state==='cancelled'&&m.window==='app'&&m.orphan===true),firstJob),'Cancellation outcome was not app-scoped');
 assert(!audit().slice(cancelStart).some(e=>e.event==='read-bytes'&&e.path==='/payload.bin'&&e.offset>=barrier.offset),'Source read advanced past held offset after cancellation');
 fs.writeFileSync(path.join(serverRoot,'release-read'),'resume');await new Promise(r=>setTimeout(r,1500));
 assert(audit().slice(cancelStart).some(e=>e.event==='read-aborted'&&e.path==='/payload.bin'&&e.offset===barrier.offset),'Source barrier was not aborted by cancellation');
 assert(!audit().slice(cancelStart).some(e=>e.event==='read-bytes'&&e.path==='/payload.bin'&&e.offset>=barrier.offset),'Source read resumed after cancellation and barrier release');
 await waitFor(()=>inspectDestination().length===0,'Cancelled mid-stream transfer left destination or partial staging files');
 const payload=serverConfig.files.find(f=>f.path==='/payload.bin');assert(createHash('sha256').update(fs.readFileSync(path.join(serverRoot,'files','payload.bin'))).digest('hex')===payload.sha256,'Source fixture changed');
 results.push({check:'actual native file-window close adopts a mid-stream SFTP copy; main-window cancellation cleans staging and preserves source',result:'PASS',cancelled_before_release:true,job:firstJob});
 await dialog.getByRole('button',{name:/목록에서 지우기/}).click();await dialog.getByRole('button',{name:'닫기',exact:true}).click();
 // A second transfer finishes after close and remains visible with its verified outcome.
 const second=await beginCopy();const closedAgain=second.waitForEvent('close');await second.locator('[data-window=close]').click();await closedAgain;await nav.waitFor();
 fs.writeFileSync(path.join(serverRoot,'release-read'),'resume');
 await waitFor(()=>inspectDestination().some(f=>f.name==='payload.bin'&&f.sha256===payload.sha256),'Background copy did not commit matching bytes',20000);
 await nav.click();const completed=dialog.locator('.transfer-row.completed');await completed.waitFor();const secondJob=(await completed.getAttribute('data-id')).slice('transfer-'.length);assert(await page.evaluate(job=>bgObserved.some(m=>m.job===job&&m.state==='completed'&&m.window==='app'&&m.orphan===true),secondJob),'Completion outcome was not app-scoped');assert(inspectDestination().length===1,'Extra files or partial staging remain after successful background copy');assert(await completed.getByRole('button',{name:'파일 창 열기',exact:true}).count()===1,'Completed transfer outcome/action missing');
 results.push({check:'actual background SFTP copy completes after native window close and main window retains outcome with destination SHA256 match',result:'PASS',bytes:payload.size,sha256:payload.sha256,events:await page.evaluate(()=>bgObserved)});

 fs.writeFileSync(resultFile,JSON.stringify({...provenance,method:'actual Windows console app, existing WebView2 attached through private loopback CDP/SSH tunnel; no emulated Windows UA',results,status:results.some(r=>r.result==='FAIL')?'findings':'checks-passed-cleanup-pending',limits:['candidate build, not production rollout','loopback CDP is reachable by other local processes; use a trusted test account/machine','CDP renderer input does not prove physical keyboard/global Windows accelerators','no real Explorer OLE drop, physical input, screen reader, multi-monitor or phone verified','OS notifications disabled and untested; quit during transfer untested','main-window cancellation only; cancellation from a second file window is untested']},null,2));
 console.log(JSON.stringify(results));if(results.some(r=>r.result==='FAIL'))process.exitCode=1;
}catch(e){fs.writeFileSync(resultFile,JSON.stringify({status:'failed',results,error:e.message,local,...provenance},null,2));throw e;}finally{
 finalizing=true;
 let cleanupError;
 try{cleanupRemote();}catch(e){cleanupError=e;}
 try{await cleanupServer();}catch(e){cleanupError||=e;}
 let timer;
 try{await Promise.race([browser?.close(),new Promise(resolve=>{timer=setTimeout(resolve,3000);})]);}catch{}finally{clearTimeout(timer);}
 const receipt=fs.existsSync(resultFile)?JSON.parse(fs.readFileSync(resultFile,'utf8')):{...provenance,results};
 receipt.cleanup={status:cleanupError?'failed':'passed',...cleanupReceipt,...(cleanupError?{error:cleanupError.message}:{})};
 if(interrupted){receipt.status='interrupted';receipt.signal=interrupted;}
 if(cleanupError)receipt.status='cleanup-failed';else if(receipt.status==='checks-passed-cleanup-pending')receipt.status='passed';
 fs.writeFileSync(resultFile,JSON.stringify(receipt,null,2));
 if(cleanupError)throw cleanupError;
 console.log('Removed native candidate task and remote fixture');
}
