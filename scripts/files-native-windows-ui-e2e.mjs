// Real Windows console + WebView2 test, driven from a machine with ssh/scp and Playwright.
// Requires an ALREADY logged-in Windows console owned by the SSH user; no unlock or elevation.
// ITDA_WINDOWS_SSH=<trusted-alias> ITDA_WINDOWS_APP=/local/itda.exe \
// ITDA_SOURCE_COMMIT=<artifact-commit> ITDA_WINDOWS_RUN_ID=<successful-CI-run> PLAYWRIGHT=/path/to/playwright/index.mjs \
// node scripts/files-native-windows-ui-e2e.mjs
// Uses an isolated profile, fixture files, temporary same-user interactive Scheduled Task,
// loopback CDP tunnel and owned-window-only resize. Stops/removes only its own resources.
// Leaves local receipt/screenshots for review. No physical OS input, clipboard or permission changes.
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
const provenance={driver_sha256:createHash('sha256').update(fs.readFileSync(fileURLToPath(import.meta.url))).digest('hex'),source_commit:source,binary_sha256:binaryHash,ci_run:Number(run),artifact:null};
const tag='itda-quality-ui-'+randomUUID(),local=fs.mkdtempSync(path.join(os.tmpdir(),tag+'-'));
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
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,()=>{
 if(finalizing){interrupted=signal;process.exitCode=signal==='SIGINT'?130:143;return;}
 let cleanupError;
 try{cleanupRemote();}catch(e){cleanupError=e;}
 fs.writeFileSync(resultFile,JSON.stringify({status:'interrupted',signal,...provenance,results,cleanup:{status:cleanupError?'failed':'passed',...cleanupReceipt,...(cleanupError?{error:cleanupError.message}:{})}},null,2));
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
 remoteTouched=true;
 remote=JSON.parse(ps(`$ErrorActionPreference='Stop';$r=Join-Path $env:TEMP '${tag}';New-Item -ItemType Directory -Path $r|Out-Null;New-Item -ItemType Directory -Path (Join-Path $r 'data\\Itda')|Out-Null;New-Item -ItemType Directory -Path (Join-Path $r 'files')|Out-Null;New-Item -ItemType Directory -Path (Join-Path $r 'files\\Projects')|Out-Null;Set-Content -LiteralPath (Join-Path $r 'files\\Notes.txt') -Value 'Native Windows preview fixture' -Encoding UTF8;$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$l.Start();$port=$l.LocalEndpoint.Port;$l.Stop();[pscustomobject]@{root=$r;port=$port;user=[Security.Principal.WindowsIdentity]::GetCurrent().Name}|ConvertTo-Json -Compress`));
 const cfg=path.join(local,'config');fs.mkdirSync(cfg);
 const fixtureId='quality-'+tag;
 fs.writeFileSync(path.join(cfg,'hosts.json'),JSON.stringify([{id:fixtureId,name:'Studio Mac',host:'studio.invalid',os:'mac',kind:'vnc',user:'fixture',opts:{}}]));fs.writeFileSync(path.join(cfg,'accounts.json'),'[]');
 fs.writeFileSync(path.join(cfg,'settings.json'),JSON.stringify({onboarded:true,theme:'light',announce:false,auto_update:false,thumbnails:false,web:{on:false},share:{on:false},hp:{on:false}}));
 const remoteRoot=remote.root.replaceAll('\\','/');
 execFileSync('scp',[candidate,`${host}:${remoteRoot}/itda.exe`],{stdio:'pipe',timeout:60000});
 assert(ps(`(Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path ${q(remote.root)} 'itda.exe')).Hash`).toLowerCase()===binaryHash,'Remote executable hash differs from supplied candidate');
 for(const name of ['hosts.json','accounts.json','settings.json'])execFileSync('scp',[path.join(cfg,name),`${host}:${remoteRoot}/data/Itda/${name}`],{stdio:'pipe',timeout:20000});
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
 await page.getByRole('button',{name:'설정',exact:true}).click();
 assert(await page.getByRole('checkbox',{name:'클립보드 공유',exact:true}).count()===1,'native checkbox lacks name');
 results.push({check:'native Windows setting checkbox accessible name',result:'PASS'});
 await page.locator('#newBtn').click();await page.getByRole('dialog',{name:'새 연결',exact:true}).waitFor();
 await page.locator('[data-key=edit] [data-kind=mac]').click();await page.locator('#f-host').fill('fixture.invalid');
 for(let i=0;i<25;i++){await page.keyboard.press(i<12?'Tab':'Shift+Tab');assert(await page.evaluate(()=>!!document.activeElement.closest('#modals')),'native renderer Tab escaped modal');}
 await page.locator('#account-name-help').click();await page.getByRole('dialog',{name:'Mac 계정 이름 찾기',exact:true}).waitFor();await page.locator('[data-key=account-name-help] [data-x=close]').click();assert(await page.locator('#f-host').inputValue()==='fixture.invalid','helper lost original form');
 await page.locator('[data-key=edit] [data-x=cancel]').click();assert(await page.locator('#newBtn').evaluate(e=>e===document.activeElement),'native modal did not restore trigger');
 results.push({check:'native renderer modal traversal, nested guide and preserved editor/focus return',result:'PASS'});
 await page.screenshot({path:path.join(local,'native-settings.png')});
 const filePromise=context.waitForEvent('page');await page.getByRole('button',{name:'파일 브라우저',exact:true}).click();const files=await filePromise;await files.locator('.files-layer').waitFor();
 assert(await files.evaluate(()=>window.ITDA_WINDOW_CHROME===true),'actual native file window chrome flag missing');
 await files.locator('[data-network-sidebar] [data-side=local]').click();await files.keyboard.press('Control+l');const address=files.getByRole('textbox',{name:'주소',exact:true});await address.fill(remote.root+'\\files');await address.press('Enter');
 const note=files.locator('.files-list [data-index]').filter({hasText:'Notes.txt'}).first();await note.waitFor();await note.click();
 await files.keyboard.press('Alt+p');await files.locator('.files-preview-content pre').waitFor();assert((await files.locator('.files-preview-content pre').textContent()).includes('Native Windows preview fixture'),'native local preview bytes missing');await files.locator('[data-action=close-preview]').click();
 results.push({check:'actual native file window with integrated tabs, local folder listing and real text preview',result:'PASS'});
 await files.evaluate(()=>{window.localCopyJobs=[];const old=ItdaFiles.message;ItdaFiles.message=function(m){if(m.type==='files_job')localCopyJobs.push({state:m.state,req:m.req,error:m.error,completed_names:m.completed_names});return old.call(this,m);};});
 const project=files.locator('.files-list [data-index]').filter({hasText:'Projects'}).first();const a=await note.boundingBox(),b=await project.boundingBox();assert(a&&b,'Owned local source/destination rows missing');
 await files.mouse.move(a.x+55,a.y+a.height/2);await files.mouse.down();await files.mouse.move(b.x+55,b.y+b.height/2,{steps:12});await files.mouse.up();
 const copy=files.getByRole('dialog',{name:'파일 복사',exact:true});await copy.waitFor();await copy.getByRole('button',{name:'복사',exact:true}).click();
 await files.waitForFunction(()=>localCopyJobs.some(j=>j.state==='completed'),null,{timeout:15000});
 const same=ps(`$r=${q(remote.root)};((Get-FileHash -LiteralPath (Join-Path $r 'files\\Notes.txt') -Algorithm SHA256).Hash-eq (Get-FileHash -LiteralPath (Join-Path $r 'files\\Projects\\Notes.txt') -Algorithm SHA256).Hash)`);assert(same==='True','Native local copy destination hash differs');await new Promise(r=>setTimeout(r,500));
 const errorVisible=await files.locator('.files-error').isVisible(),errorText=errorVisible?await files.locator('.files-error').textContent():'';results.push({check:'native managed local copy succeeds without a remote-connection error',result:errorVisible?'FAIL':'PASS',error:errorText,jobs:await files.evaluate(()=>localCopyJobs)});assert(!errorVisible,'Local copy completed but UI reported: '+errorText);

 // Delete only the generated source after confirmation opens: exercise the real
 // Rust error path, then recover through the visible Home retry control.
 const failedSource=remote.root+'\\files\\Retry.txt';
 ps(`Set-Content -LiteralPath ${q(failedSource)} -Value 'private retry fixture' -Encoding UTF8`);
 await files.keyboard.press('Control+l');await address.fill(remote.root+'\\files');await address.press('Enter');
 const retryNote=files.locator('.files-list [data-index]').filter({hasText:'Retry.txt'}).first();await retryNote.waitFor();
 const ra=await retryNote.boundingBox(),rb=await project.boundingBox();assert(ra&&rb,'Retry rows missing');
 await files.mouse.move(ra.x+55,ra.y+ra.height/2);await files.mouse.down();await files.mouse.move(rb.x+55,rb.y+rb.height/2,{steps:12});await files.mouse.up();
 await copy.waitFor();ps(`Remove-Item -LiteralPath ${q(failedSource)} -Force`);await copy.getByRole('button',{name:'복사',exact:true}).click();
 await files.waitForFunction(()=>localCopyJobs.some(j=>j.state==='failed'),null,{timeout:15000});
 const failure=await files.evaluate(()=>localCopyJobs.find(j=>j.state==='failed'));assert(failure?.error?.includes('원본 파일이 없습니다')&&Array.isArray(failure.completed_names)&&failure.completed_names.length===0,'Expected missing-source failure with no completed names');
 const queuedBeforeRetry=await files.evaluate(()=>localCopyJobs.filter(j=>j.state==='queued').length);
 const retryButton=files.locator('[data-retry-job]');await retryButton.waitFor();
 const home=files.locator('[data-network-sidebar] [data-action=explorer-home]');
 assert(await home.count()===1,'Home navigation selector changed');await home.click();await retryButton.waitFor({state:'visible'});await files.waitForTimeout(1000);assert(await files.evaluate(()=>localCopyJobs.filter(j=>j.state==='queued').length)===queuedBeforeRetry,'Home automatically retried the failed copy');
 ps(`Set-Content -LiteralPath ${q(failedSource)} -Value 'private retry fixture restored' -Encoding UTF8`);
 await retryButton.click();const retryDialog=files.getByRole('dialog',{name:'남은 파일 다시 복사',exact:true});await retryDialog.waitFor();await files.waitForTimeout(1000);assert(await files.evaluate(()=>localCopyJobs.filter(j=>j.state==='queued').length)===queuedBeforeRetry,'Retry ran before confirmation');await retryDialog.getByRole('button',{name:'다시 복사',exact:true}).click();
 await files.waitForFunction(()=>localCopyJobs.some(j=>j.state==='completed'&&j.completed_names?.includes('Retry.txt')),null,{timeout:15000});
 const retried=await files.evaluate(()=>localCopyJobs.find(j=>j.state==='completed'&&j.completed_names?.includes('Retry.txt')));assert(retried.req&&retried.req!==failure.req,'Retry must use a new request');
 assert(await retryButton.count()===0,'Recovered failure row remains retryable');
 assert(ps(`((Get-FileHash -LiteralPath ${q(failedSource)} -Algorithm SHA256).Hash-eq (Get-FileHash -LiteralPath ${q(remote.root+'\\files\\Projects\\Retry.txt')} -Algorithm SHA256).Hash)`)==='True','Retried destination hash differs');
 results.push({check:'native failed copy remains actionable on Home and retry requires confirmation then copies restored source',result:'PASS',jobs:await files.evaluate(()=>localCopyJobs),confirmation_required:true,new_request_verified:true});
 await files.locator('[data-network-sidebar] [data-side=local]').click();await files.keyboard.press('Control+l');await address.fill(remote.root+'\\files');await address.press('Enter');await note.waitFor();
 const tabsBefore=await files.getByRole('tab').count();await files.locator('.files-list').focus();await files.keyboard.press('Control+t');await files.waitForTimeout(250);const tabsAfter=await files.getByRole('tab').count();results.push({check:'local-only folder supports Ctrl+T',result:tabsAfter===tabsBefore+1?'PASS':'FAIL',tabs_before:tabsBefore,tabs_after:tabsAfter,new_tab_disabled:await files.locator('[data-action=new-tab]').isDisabled()});await files.locator('[data-action=new-tab]').click();await files.waitForTimeout(200);results.push({check:'local-only new-tab button changes tab count',result:await files.getByRole('tab').count()===tabsAfter+1?'PASS':'FAIL',tabs_after_click:await files.getByRole('tab').count(),native_host:await files.evaluate(()=>window.ITDA_FILE_HOST),tab_title:await files.locator('.files-tab.on').getAttribute('title')});
 const dpr=await files.evaluate(()=>devicePixelRatio),request=randomUUID();const cmd=JSON.stringify({op:'resize-file',width:Math.round(560*dpr),height:Math.round(540*dpr),request});
 ps(`$r=${q(remote.root)};Set-Content -LiteralPath (Join-Path $r 'command.tmp') -Value ${q(cmd)} -Encoding UTF8;Move-Item -LiteralPath (Join-Path $r 'command.tmp') -Destination (Join-Path $r 'command.json') -Force`);
 let ack;for(let i=0;i<30;i++){const raw=ps(`$p=Join-Path ${q(remote.root)} 'command-result.json';if(Test-Path $p){Get-Content $p -Raw}`);if(raw){ack=JSON.parse(raw.replace(/^\uFEFF/,''));if(ack.request===request)break;}await new Promise(r=>setTimeout(r,200));}assert(ack?.hwnd,'native file window resize did not identify exactly one owned window');await files.waitForTimeout(700);const resizedWidth=await files.evaluate(()=>innerWidth);results.push({check:'native file window can fit requested small viewport',result:resizedWidth<=600?'PASS':'FAIL',requested_css_width:560,actual_css_width:resizedWidth,dpr});
 assert(resizedWidth<=600,'native minimum width still prevents narrow overlay test');
 await note.click();await files.locator('[data-action=details]').click();await files.locator('.files-details').waitFor();const rect=await files.locator('.files-preview').boundingBox(),viewport=await files.evaluate(()=>({width:innerWidth,height:innerHeight}));assert(rect.width>200&&rect.x>=0&&rect.x+rect.width<=viewport.width+1,'native narrow details pane offscreen');await files.keyboard.press('Escape');assert(await files.locator('.files-preview').isHidden(),'native narrow preview Escape did not close');
 results.push({check:'actual-width native Windows file window retains visible details and Escape focus recovery',result:'PASS',viewport,dpr});await files.screenshot({path:path.join(local,'native-files-narrow.png')});

 fs.writeFileSync(resultFile,JSON.stringify({...provenance,method:'actual Windows console app, existing WebView2 attached through private loopback CDP/SSH tunnel; no emulated Windows UA',results,status:results.some(r=>r.result==='FAIL')?'findings':'checks-passed-cleanup-pending',screenshot:path.join(local,'native-settings.png'),files_screenshot:path.join(local,'native-files-narrow.png'),limits:['candidate build, not production rollout','loopback CDP is reachable by other local processes; use a trusted test account/machine','CDP renderer input does not prove physical keyboard/global Windows accelerators','no real Explorer OLE drop or native multi-monitor/phone verified']},null,2));
 console.log(JSON.stringify(results));if(results.some(r=>r.result==='FAIL'))process.exitCode=1;
}catch(e){fs.writeFileSync(resultFile,JSON.stringify({status:'failed',results,error:e.message,local,...provenance},null,2));throw e;}finally{
 finalizing=true;
 let cleanupError;
 try{cleanupRemote();}catch(e){cleanupError=e;}
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
