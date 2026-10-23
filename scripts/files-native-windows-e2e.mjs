// Native macOS transfer engine against a real Windows OpenSSH host; no window/input.
// ITDA_WINDOWS_SSH=<trusted SSH alias> node scripts/files-native-windows-e2e.mjs
// Optional ITDA_APP chooses an installed bundle. SSH key/agent authentication must
// already work. Only unique fixture directories under each machine's temp path are
// written/removed; no system settings, native clipboard or user configuration changes.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import {spawn,execFileSync} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
const sshHost=process.env.ITDA_WINDOWS_SSH;
if(!sshHost||sshHost.startsWith('-')||/\s/.test(sshHost))throw Error('Set ITDA_WINDOWS_SSH to an existing trusted Windows OpenSSH alias');
if(process.platform!=='darwin')throw Error('This fixture drives an installed macOS native binary');
const config=execFileSync('ssh',['-G',sshHost],{encoding:'utf8',timeout:10000});
const sshPort=Number(config.match(/^port (\d+)$/m)?.[1]||22),sshUser=config.match(/^user (.+)$/m)?.[1];
const bundle=process.env.ITDA_APP||'/Applications/Itda.app';
const release=execFileSync('/usr/libexec/PlistBuddy',['-c','Print :CFBundleShortVersionString',path.join(bundle,'Contents/Info.plist')],{encoding:'utf8'}).trim();
const uuid=randomUUID(),tag='itda-quality-'+uuid,root=fs.mkdtempSync(path.join(os.tmpdir(),tag+'-'));
const powershell=script=>execFileSync('ssh',['-o','BatchMode=yes','-o','ConnectTimeout=5',sshHost,'powershell','-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from("$ProgressPreference='SilentlyContinue';"+script,'utf16le').toString('base64')],{encoding:'utf8',timeout:20000}).trim();
let remote,app,spawnError;const results=[];
const assert=(v,m)=>{if(!v)throw Error(m);};
const hash=b=>createHash('sha256').update(b).digest('hex');
const quote=s=>"'"+s.replaceAll("'","''")+"'";
try {
 remote=JSON.parse(powershell(`$ErrorActionPreference='Stop';$d=Join-Path $env:TEMP '${tag}';New-Item -ItemType Directory -Path $d|Out-Null;[pscustomobject]@{path=$d;user=$env:USERNAME}|ConvertTo-Json -Compress`));
 const source=path.join(root,'source');fs.mkdirSync(source);
 const name='정보공개 결정통지서(접수번호_17780155).pdf'.normalize('NFD'),bytes=Buffer.from('%PDF-1.4\nItda disposable Unicode fixture '+uuid+'\n%%EOF\n');fs.writeFileSync(path.join(source,name),bytes);
 const data=path.join(root,'data','Itda');fs.mkdirSync(data,{recursive:true});
 const options={method:'sftp',host:sshHost,port:sshPort,user:sshUser||remote.user,path:remote.path.replaceAll('\\','/'),engine:'sftp'};
 fs.writeFileSync(path.join(data,'hosts.json'),JSON.stringify([{id:'quality-native',name:'Disposable Windows fixture',host:sshHost,os:'windows',user:sshUser||remote.user,files:options}]));
 fs.writeFileSync(path.join(data,'settings.json'),JSON.stringify({onboarded:true,announce:false,auto_update:false,thumbnails:false,web:{on:false},share:{on:false},hp:{on:false}}));
 const server=net.createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));
 const runtime=path.join(root,'runtime');fs.mkdirSync(runtime);
 app=spawn(path.join(bundle,'Contents/MacOS/itda'),[],{env:{...process.env,TMPDIR:runtime,ITDA_DATA_DIR:path.join(root,'data'),ITDA_TEST_HTTP:String(port)},stdio:'ignore'});
 app.on('error',e=>{spawnError=e;});
 const base=`http://127.0.0.1:${port}`;let ready=false;for(let i=0;i<100;i++){if(spawnError)throw spawnError;if(app.exitCode!==null||app.signalCode!==null)throw Error('isolated native process exited before bridge ready');try{if((await fetch(base,{signal:AbortSignal.timeout(300)})).ok){ready=true;break;}}catch{}await new Promise(r=>setTimeout(r,50));}assert(ready,'isolated native bridge did not start');
 const client='quality-'+uuid;
 const send=msg=>fetch(base+'/ipc?client='+client,{method:'POST',body:JSON.stringify({cmd:'files',id:'quality-native',window:'quality-native-window',...msg}),signal:AbortSignal.timeout(3000)});
 let inbox=[],observed=[];const wait=async match=>{const deadline=Date.now()+45000;for(;;){const idx=inbox.findIndex(match);if(idx>=0)return inbox.splice(idx,1)[0];if(Date.now()>deadline)throw Error('native event timed out; message kinds '+inbox.map(m=>m.type+':'+(m.op||m.state||'')).join(','));const batch=await (await fetch(base+'/poll?client='+client,{signal:AbortSignal.timeout(12000)})).json();inbox.push(...batch);observed.push(...batch.filter(m=>m.type==='files_job'));}};
 await fetch(base+'/ipc?client='+client,{method:'POST',body:JSON.stringify({cmd:'ready'})});
 await send({op:'connect',options});const c=await wait(m=>m.type==='files_result'&&m.op==='connect');assert(c.ok,`native connect failed: ${c.error}`);const dest=c.data.path;
 results.push({check:'installed native SFTP to physical Windows fixture directory',result:'PASS'});
 const copy=async policy=>{await send({op:'transfer',from:'local',to:'remote',source,destination:dest,names:[name],policy,engine:'sftp',move:false});return await wait(m=>m.type==='files_job'&&['completed','failed','cancelled'].includes(m.state));};
 const first=await copy('keep');assert(first.state==='completed',`copy failed: ${first.error}`);
 const inspect=()=>JSON.parse(powershell(`$ErrorActionPreference='Stop';@(Get-ChildItem -LiteralPath ${quote(remote.path)} -File|ForEach-Object{[pscustomobject]@{name=$_.Name;hash=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})|ConvertTo-Json -Compress`));
 const list1=[inspect()].flat();assert(list1.length===1&&list1[0].name===name.normalize('NFC'),'destination name was not composed NFC');assert(list1[0].hash===hash(bytes),'Windows copied bytes differ');assert(fs.existsSync(path.join(source,name)),'source disappeared');results.push({check:'Mac NFD Korean filename → Windows NFC + SHA256 bytes + original preservation',result:'PASS'});
 const second=await copy('keep');assert(second.state==='completed',`keep-both failed: ${second.error}`);const list2=[inspect()].flat();assert(list2.length===2&&list2.every(x=>x.name===x.name.normalize('NFC')&&x.hash===hash(bytes)),'keep-both NFC or bytes failed');results.push({check:'existing NFC destination: keep-both creates second correct file',result:'PASS'});
 const changedBytes=Buffer.from('changed source for skip '+uuid);fs.writeFileSync(path.join(source,name),changedBytes);
 const third=await copy('skip');assert(third.state==='completed','skip failed');const afterSkip=[inspect()].flat();assert(afterSkip.length===2&&afterSkip.every(x=>x.hash===hash(bytes))&&JSON.stringify(afterSkip.map(x=>x.name).sort())===JSON.stringify(list2.map(x=>x.name).sort()),'skip changed existing files');assert(hash(fs.readFileSync(path.join(source,name)))===hash(changedBytes),'skip changed source');results.push({check:'skip preserves both existing names/hashes while source contains different bytes',result:'PASS'});
 const movedName='검증 후 이동.txt'.normalize('NFD');const movedBytes=Buffer.from('verified move '+uuid);fs.writeFileSync(path.join(source,movedName),movedBytes);
 await send({op:'transfer',from:'local',to:'remote',source,destination:dest,names:[movedName],policy:'keep',engine:'sftp',move:true});
 const moved=await wait(m=>m.type==='files_job'&&['completed','failed','cancelled'].includes(m.state));assert(moved.state==='completed',`move failed: ${moved.error}`);
 assert(!fs.existsSync(path.join(source,movedName)),'verified move left source unexpectedly');const movedFile=[inspect()].flat().find(x=>x.name===movedName.normalize('NFC'));assert(movedFile?.hash===hash(movedBytes),'verified move destination differs');results.push({check:'after move: Windows NFC and SHA256 correct; temporary source removed',result:'PASS'});
 const cancelledName='cancel.bin',cancelBytes=Buffer.alloc(8*1024*1024,43);fs.writeFileSync(path.join(source,cancelledName),cancelBytes);
 powershell(`$ErrorActionPreference='Stop';[IO.File]::WriteAllText((Join-Path ${quote(remote.path)} 'cancel.bin'),'original destination')`);
 inbox=inbox.filter(m=>m.type!=='files_job');
 await send({op:'transfer',from:'local',to:'remote',source,destination:dest,names:[cancelledName],policy:'replace',engine:'sftp',move:true});
 const queued=await wait(m=>m.type==='files_job'&&m.state==='queued');await send({op:'cancel',job:queued.job});
 const cancelled=await wait(m=>m.type==='files_job'&&m.job===queued.job&&['completed','failed','cancelled'].includes(m.state));assert(cancelled.state==='cancelled',`expected cancellation, got ${cancelled.state}: ${cancelled.error}`);
 assert(fs.existsSync(path.join(source,cancelledName))&&hash(fs.readFileSync(path.join(source,cancelledName)))===hash(cancelBytes),'cancel changed temporary source');const existing=[inspect()].flat().find(x=>x.name===cancelledName);assert(existing?.hash===hash(Buffer.from('original destination')),'cancel replaced existing destination');
 const remaining=[inspect()].flat().map(x=>x.name).sort(),expected=[...list2.map(x=>x.name),movedName.normalize('NFC'),cancelledName].sort();assert(JSON.stringify(remaining)===JSON.stringify(expected),'cancellation left partial or unexpected files');
 results.push({check:'early move cancellation (phase not controlled) preserves source and existing destination, with no leftover files',result:'PASS',observed_events:observed.filter(m=>m.job===queued.job).map(m=>({state:m.state,...Object.fromEntries(['bytes','total','files'].filter(k=>typeof m[k]==='number').map(k=>[k,m[k]]))}))});
 await send({op:'disconnect'});
 console.log(JSON.stringify({release,method:'installed native Mac binary isolated test bridge; real SFTP to Windows OpenSSH; disposable temp dirs only',results,scope_limits:['No actual Windows WebView2, Explorer drag/drop, UI selection, VoiceOver or large-file performance verified','UI and installed-binary version may differ; record the bundle version and commit separately']},null,2));
}finally{
 const cleanupErrors=[];
 try{if(app?.pid&&app.exitCode===null&&app.signalCode===null){const exited=new Promise(resolve=>app.once('exit',resolve));app.kill();const timer=setTimeout(()=>{if(app.exitCode===null)app.kill('SIGKILL');},3000);try{await exited;}finally{clearTimeout(timer);}}}catch(e){cleanupErrors.push(e);}
 try{powershell(`$ErrorActionPreference='Stop';$p=Join-Path $env:TEMP '${tag}';if((Test-Path -LiteralPath $p)-and((Split-Path $p -Leaf)-eq '${tag}')){Remove-Item -LiteralPath $p -Recurse -Force}`);}catch(e){cleanupErrors.push(e);}
 try{fs.rmSync(root,{recursive:true,force:true,maxRetries:3,retryDelay:100});}catch(e){cleanupErrors.push(e);}
 if(cleanupErrors.length)throw new AggregateError(cleanupErrors,'Fixture cleanup failed; do not treat the run as PASS');
}

