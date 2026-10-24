// One future explicitly gated disposable CI run of the ACTUAL public 1.0.6 exe.
// CDP prepares/observes only. OLE drag and the one bound fixture Copy button use
// tagged guarded SendInput. Never inject files_drop/native_drag/transfer messages.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import {spawn,execFileSync} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright';
import {observeExit} from './fixtures/windows-drop-lifecycle.mjs';
const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const out=path.join(repo,'test-artifacts/native-crosshost');
const SHA='4b2e3bbe5639e7984348ab002e58d155c131293663a6d594892e04cd4acd33b4';
const SOURCE='26264572c3163fb63ee1bcfc64508d0f187bf72d',TAG='v1.0.6';
const SUMSHA='72b141466a396d3a671ebbbb4aa1b25989430d3ebbb5ce18932908100ec4ffb6';
const assert=(v,m)=>{if(!v)throw Error(m);}, sleep=ms=>new Promise(r=>setTimeout(r,ms));
const hash=p=>createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const samePath=(a,b)=>a.toLowerCase()===b.toLowerCase();
function ownedTemp(dir,prefix){assert(!fs.lstatSync(dir).isSymbolicLink()&&samePath(fs.realpathSync(dir),dir)&&samePath(path.dirname(dir),fs.realpathSync(os.tmpdir()))&&path.basename(dir).startsWith(prefix),'Temporary workspace ownership changed');}
const read=p=>JSON.parse(fs.readFileSync(p,'utf8').replace(/^\uFEFF/,''));
const report={status:'failed',scope:'Actual published 1.0.6 binary: disposable Windows tagged native Itda file-window A to different-host file-window B copy; not physical PC, 150% DPI, AT, multi-monitor, Mac or persona-9 acceptance',checks:[],cleanup:{}};
let root,app,identity,appExit,browser,controller,controllerExit,controllerQuiescent=true,startup,interrupted;
const servers=[];let binding,controllerSpawnError;
for(const s of ['SIGINT','SIGTERM'])process.once(s,()=>{interrupted=s;});
const ps=(file,args=[])=>execFileSync('pwsh',['-NoProfile','-NonInteractive','-File',path.join(repo,'scripts',file),...args],{encoding:'utf8',timeout:30000});
const control=op=>JSON.parse(ps('windows-native-idle-control.ps1',['-Operation',op,'-Root',root,'-AppPid',String(appExit?.stopped?0:app?.pid||0),'-CreationTicks',identity?.creation_ticks||'','-Hwnd',identity?.hwnd||'0','-DebugPort',String(report.debug_port||0)]).replace(/^\uFEFF/,''));
async function wait(test,message,ms=30000,interrupt=true){const end=Date.now()+ms;while(Date.now()<end){assert(!interrupt||!interrupted,'Interrupted');if(await test())return;await sleep(100);}throw Error(message);}
function audit(s){const p=path.join(s.root,'audit.jsonl');if(!fs.existsSync(p))return [];const t=fs.readFileSync(p,'utf8');return t.slice(0,t.lastIndexOf('\n')+1).split('\n').filter(Boolean).map(JSON.parse);}
function tree(dir){return fs.readdirSync(dir,{recursive:true}).map(n=>{const p=path.join(dir,n),s=fs.lstatSync(p);assert(!s.isSymbolicLink(),'Fixture symlink appeared');return {path:n.replaceAll('\\','/'),dir:s.isDirectory(),...(s.isFile()?{sha256:hash(p)}:{})};}).sort((a,b)=>a.path.localeCompare(b.path));}
function fresh(name){const p=path.join(out,`preflight-${name}.json`);try{ps('windows-native-drop-preflight.ps1',['-Receipt',p]);}catch(e){if(!fs.existsSync(p))throw e;}const r=read(p),age=Date.now()-Date.parse(r.recorded_at);assert(r.status!=='probe-error','Prerequisite probe error');assert(r.source.checkout===report.provenance.harness_checkout&&r.source.run_id===process.env.GITHUB_RUN_ID&&r.source.run_attempt===process.env.GITHUB_RUN_ATTEMPT&&age>=0&&age<120000,'Stale/foreign prerequisites');assert(r.status==='eligible-for-owned-drop','prerequisite-unavailable: '+name);return r;}
async function fixture(script){
 const s={root:fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'itda-quality-sftp-')))};servers.push(s);
 s.child=spawn('python',[path.join(repo,'scripts/fixtures',script),'--workspace',s.root],{env:{...process.env,PYTHONUTF8:'1'},stdio:['ignore','pipe','pipe']});s.child.stderr.on('data',()=>{});s.exit=observeExit(s.child);
 const manifest=await new Promise((resolve,reject)=>{let t='';const timer=setTimeout(()=>reject(Error('Fixture readiness timeout')),20000);s.child.once('error',e=>{clearTimeout(timer);reject(e);});s.child.once('exit',()=>{clearTimeout(timer);reject(Error('Fixture exited before readiness'));});s.child.stdout.on('data',b=>{t+=b;if(t.includes('\n')){clearTimeout(timer);resolve(t.split('\n')[0].trim());}});});
 assert(path.resolve(manifest)===path.join(s.root,'connection.json'),'Manifest escaped owned workspace');s.config=read(manifest);
 assert(s.config.pid===s.child.pid&&s.config.workspace===s.root&&s.config.host==='127.0.0.1'&&Number.isInteger(s.config.port)&&s.config.port>0&&s.config.port<=65535,'Fixture ownership mismatch');return s;
}
async function publicBinary(){
 const api='https://api.github.com/repos/jiunbae/itda';
 const get=async(url,json=false)=>{const r=await fetch(url,{headers:{Accept:'application/vnd.github+json','User-Agent':'itda-public-native-qa'},signal:AbortSignal.timeout(30000)});assert(r.ok,'Public release request failed');return json?await r.json():Buffer.from(await r.arrayBuffer());};
 const release=await get(api+'/releases/tags/'+TAG,true);assert(release.tag_name===TAG&&!release.draft&&!release.prerelease&&release.published_at==='2026-10-24T08:13:58Z','Wrong/nonpublic release');
 const ref=await get(api+'/git/ref/tags/'+TAG,true);let object=ref.object;
 if(object.type==='tag')object=(await get(api+'/git/tags/'+object.sha,true)).object;
 assert(object.type==='commit'&&object.sha===SOURCE,'Public tag source mismatch');
 const asset=name=>{const matches=release.assets.filter(a=>a.name===name);assert(matches.length===1,'Ambiguous release asset');return matches[0];};
 const exe=asset('itda.exe'),sums=asset('SHA256SUMS.txt');assert(exe.digest==='sha256:'+SHA&&sums.digest==='sha256:'+SUMSHA,'Public API digest mismatch');
 for(const a of [exe,sums])assert(a.browser_download_url===`https://github.com/jiunbae/itda/releases/download/${TAG}/${a.name}`,'Unexpected asset origin');
 const sumbytes=await get(sums.browser_download_url),bytes=await get(exe.browser_download_url);
 assert(createHash('sha256').update(sumbytes).digest('hex')===SUMSHA&&createHash('sha256').update(bytes).digest('hex')===SHA,'Unauthenticated public asset hash mismatch');
 assert(sumbytes.toString('utf8').split(/\r?\n/).some(l=>new RegExp('^'+SHA+'\\s+\\*?itda\\.exe$').test(l)),'SHA256SUMS exe entry mismatch');
 assert(bytes.length===7553536&&bytes.length===exe.size,'Public exe size mismatch');
 fs.writeFileSync(path.join(root,'itda.exe'),bytes,{flag:'wx'});
 return {tag:TAG,source:SOURCE,published_at:release.published_at,binary_sha256:SHA,sha256sums_sha256:SUMSHA,download:'unauthenticated HTTPS; API digest + public sums + actual bytes',build:'actual published unsigned Windows binary; no same-job rebuild'};
}
try{
 assert(process.platform==='win32'&&process.env.GITHUB_ACTIONS==='true'&&process.env.ITDA_ENABLE_OWNED_INPUT==='true','Disposable Windows CI explicit input gate required');
 fs.mkdirSync(out,{recursive:true});const checkout=execFileSync('git',['rev-parse','HEAD'],{cwd:repo,encoding:'utf8'}).trim();assert(/^[a-f0-9]{40}$/.test(process.env.ITDA_EXPECTED_SOURCE_SHA||'')&&checkout===process.env.ITDA_EXPECTED_SOURCE_SHA&&checkout===process.env.GITHUB_SHA,'Reviewed HARNESS checkout/event SHA mismatch');
 report.provenance={harness_checkout:checkout,run_id:process.env.GITHUB_RUN_ID,run_attempt:process.env.GITHUB_RUN_ATTEMPT,driver_sha256:hash(fileURLToPath(import.meta.url)),controller_sha256:hash(path.join(repo,'scripts/windows-native-crosshost-control.ps1')),workflow_sha256:hash(path.join(repo,'.github/workflows/windows-native-crosshost.yml')),requirements_sha256:hash(path.join(repo,'scripts/fixtures/requirements-sftp.txt')),native_driver_sha256:hash(path.join(repo,'scripts/fixtures/windows-crosshost-driver.cs')),destination_fixture_sha256:hash(path.join(repo,'scripts/fixtures/confined-sftp-crosshost.py')),source_fixture_sha256:hash(path.join(repo,'scripts/fixtures/read-only-sftp-drop.py'))};
 report.environment=fresh('initial');
 root=fs.mkdtempSync(path.join(os.tmpdir(),'itda-native-idle-'));root=control('workspace').root;startup=control('startup');
 report.provenance.release=await publicBinary();
 const a=await fixture('read-only-sftp-drop.py'),b=await fixture('confined-sftp-crosshost.py');assert(a.config.port!==b.config.port&&a.root!==b.root,'Source/destination must differ');
 const sourceBefore=tree(path.join(a.root,'files'));assert(a.config.files.length===3&&sourceBefore.filter(v=>v.dir).length===1,'Expected three generated files and one folder');assert(tree(path.join(b.root,'files')).length===0,'Destination not empty');
 const hosts=[{id:'crosshost-source',name:'QA source',os:'mac',config:a.config},{id:'crosshost-destination',name:'QA destination',os:'windows',config:b.config}];
 const data=path.join(root,'data/Itda');fs.mkdirSync(data,{recursive:true});
 fs.writeFileSync(path.join(data,'hosts.json'),JSON.stringify(hosts.map(h=>({id:h.id,name:h.name,os:h.os,host:'127.0.0.1',user:'',opts:{},files:{method:'sftp',host:'127.0.0.1',port:h.config.port,user:''}}))));fs.writeFileSync(path.join(data,'accounts.json'),'[]');
 fs.writeFileSync(path.join(data,'settings.json'),JSON.stringify({onboarded:true,theme:'light',tray:false,notify:false,announce:false,auto_update:false,thumbnails:false,web:{on:false},share:{on:false}}));
 fs.writeFileSync(path.join(data,'known_hosts'),hosts.map(h=>`[127.0.0.1]:${h.config.port} ${h.config.host_key_type} ${h.config.host_key_base64}\n`).join(''));
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));report.debug_port=listener.address().port;await new Promise(r=>listener.close(r));
 app=spawn(path.join(root,'itda.exe'),[],{env:{...process.env,ITDA_DATA_DIR:path.join(root,'data'),ITDA_DEBUG_PORT:String(report.debug_port)},stdio:['ignore','pipe','pipe']});app.stdout.on('data',()=>{});app.stderr.on('data',()=>{});appExit=observeExit(app);let appError;app.once('error',e=>{appError=e;});
 await wait(()=>{assert(!appError&&!appExit.stopped,'Published candidate failed/exited');const s=control('inspect');if(identity)assert(identity.creation_ticks===s.creation_ticks,'Candidate PID reused');identity=s;if(!s.alive||s.hwnd==='0'||!s.visible||!s.private_udf||!s.webview_pids.length||!s.listeners.length)return false;assert(s.session_id===report.environment.checks.session.id&&s.window_class==='itda_home'&&/^Itda \(profile [a-f0-9]{16}\)$/.test(s.window_title),'Wrong isolated main window');assert(s.listeners.every(v=>['127.0.0.1','::1'].includes(v.LocalAddress)&&[app.pid,...s.webview_pids].includes(v.OwningProcess)),'CDP not owned/loopback');return true;},'Private WebView2 readiness',60000);
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${report.debug_port}`);const context=browser.contexts()[0],main=context.pages()[0];assert(context.pages().length===1,'Unexpected initial browser pages');await main.waitForFunction(()=>typeof S!=='undefined'&&S.hosts.length===2);
 const pages=[];
 for(const h of hosts){const next=context.waitForEvent('page');await main.evaluate(id=>ItdaFiles.launch(S.hosts.find(h=>h.id===id),true),h.id);const p=await next;pages.push(p);await p.locator('.files-setup').waitFor();
  await p.evaluate(()=>{window.qa={connected:false,remote_path:null,begins:[],ends:[],drops:[],transfers:[],jobs:[],releases:[]};const recv=ItdaFiles.message;ItdaFiles.message=function(m){if(m.type==='files_result'&&m.op==='connect'&&m.window===ItdaFiles.windowToken&&m.ok){qa.connected=true;qa.remote_path=m.data.path;}if(m.type==='files_native_drag_end'&&m.window===ItdaFiles.windowToken)qa.ends.push(m);if(m.type==='files_drop'&&m.window===ITDA_FILE_WINDOW)qa.drops.push(m);if(m.type==='files_job'&&m.window===ItdaFiles.windowToken)qa.jobs.push(m);return recv.call(this,m);};const original=window.send;window.send=function(cmd,data){if(cmd==='files_native_drag')qa.begins.push(data);if(cmd==='files'&&data.op==='transfer')qa.transfers.push(data);if(cmd==='files_drag_release')qa.releases.push(data);return original(cmd,data);};});
  await p.evaluate(c=>{const q=s=>document.querySelector('.files-setup '+s);q('[name=account]').value='custom';q('[name=account]').dispatchEvent(new Event('change',{bubbles:true}));q('[name=user]').value=c.user;q('[name=password]').value=c.password;q('[name=save_password]').checked=false;q('[data-connect]').click();},{user:h.config.user,password:h.config.password});
  await p.waitForFunction(()=>qa.connected&&!document.querySelector('.files-setup'),undefined,{timeout:45000});
 }
 const [source,target]=pages;assert(context.pages().length===3,'Unexpected extra app page');assert((await Promise.all(pages.map(p=>p.evaluate(()=>qa.remote_path)))).every(p=>p==='/'),'Unexpected fixture remote root');
 const mapping=await Promise.all(pages.map(p=>p.evaluate(()=>({host:ITDA_FILE_HOST,window:ITDA_FILE_WINDOW,page:ItdaFiles.windowToken,title:document.title}))));
 assert(mapping[0].host===hosts[0].id&&mapping[1].host===hosts[1].id&&mapping[0].title==='QA source — 파일'&&mapping[1].title==='QA destination — 파일'&&mapping.every(v=>v.window&&v.page)&&mapping[0].window!==mapping[1].window&&mapping[0].page!==mapping[1].page,'Frontend window identity mapping failed');
 binding={source_host:mapping[0].host,target_host:mapping[1].host,source_window:mapping[0].window,target_window:mapping[1].window,source_page:mapping[0].page,target_page:mapping[1].page};
 controller=spawn('pwsh',['-NoProfile','-NonInteractive','-File',path.join(repo,'scripts/windows-native-crosshost-control.ps1'),'-Root',root,'-AppPid',String(app.pid),'-CreationTicks',identity.creation_ticks,'-MainHwnd',identity.hwnd],{env:process.env,stdio:['ignore','pipe','pipe']});controller.stdout.on('data',()=>{});controller.stderr.on('data',()=>{});controller.once('error',e=>{controllerSpawnError=e.message;});controllerExit=observeExit(controller);controllerQuiescent=false;
 const controllerError=()=>{assert(!controllerSpawnError,'Controller start failed: '+controllerSpawnError);const p=path.join(root,'controller-error.json');if(fs.existsSync(p)){const r=path.join(root,'reply.json');if(fs.existsSync(r))report.failed_native_reply=read(r);throw Error(read(p).error);}};
 await wait(()=>{controllerError();return fs.existsSync(path.join(root,'layout.json'));},'Two-window layout missing');const layout=read(path.join(root,'layout.json'));report.layout=layout;report.binding=mapping.map((m,i)=>({...m,hwnd:i===0?layout.File:layout.Target}));
 assert(layout.SourceTitle===mapping[0].title&&layout.TargetTitle===mapping[1].title&&layout.SourcePid===app.pid&&layout.TargetPid===app.pid&&layout.File!==layout.Target&&layout.SourceIntegrity===layout.TargetIntegrity&&layout.Session===identity.session_id,'Native mapping/IL/session mismatch');
 const command=async value=>{const request=randomUUID();fs.writeFileSync(path.join(root,'command.tmp'),JSON.stringify({...value,binding,request}));fs.renameSync(path.join(root,'command.tmp'),path.join(root,'command.json'));let reply;await wait(()=>{controllerError();const p=path.join(root,'reply.json');if(!fs.existsSync(p))return false;reply=read(p);return reply.request===request;},'Native command timed out',20000);assert(!reply.error,reply.error);return reply.result;};
 await source.evaluate(()=>{const l=document.querySelector('[data-pane=remote] .files-list');l.querySelector('[data-index]').dispatchEvent(new MouseEvent('click',{bubbles:true}));l.focus();l.dispatchEvent(new KeyboardEvent('keydown',{key:'a',ctrlKey:true,bubbles:true,cancelable:true}));});
 assert(await source.locator('[data-pane=remote] [data-index].selected').count()===3,'Exactly three selected top-level items required');
 const from=await source.locator('[data-pane=remote] [data-index]').first().boundingBox(),to=await target.locator('[data-pane=remote] .files-list').boundingBox();assert(from&&to,'Source/target list geometry absent');
 const dpr=await source.evaluate(()=>devicePixelRatio),tdpr=await target.evaluate(()=>devicePixelRatio);report.viewport=await Promise.all(pages.map(p=>p.evaluate(()=>({width:innerWidth,height:innerHeight,dpr:devicePixelRatio}))));assert(report.viewport.every(v=>v.width>=440&&v.height>=400),'Insufficient CSS viewport');
 const point={x:Math.round(layout.OriginX+(from.x+Math.min(60,from.width/2))*dpr),y:Math.round(layout.OriginY+(from.y+from.height/2)*dpr),target_x:Math.round(layout.TargetOriginX+(to.x+to.width/2)*tdpr),target_y:Math.round(layout.TargetOriginY+(to.y+Math.min(100,to.height/2))*tdpr)};
 await sleep(5500);fresh('drag');report.checks.push({check:'native drag attribution',input:await command({op:'drag',...point})});
 await target.locator('.files-prompt [data-choice=ok]').waitFor({timeout:15000});
 const state=await Promise.all(pages.map(p=>p.evaluate(()=>qa))),begin=state[0].begins[0],drop=state[1].drops[0];
 assert(state[0].begins.length===1&&begin.items.length===3&&state[0].ends.length===1&&state[0].ends[0].dropped&&!state[0].ends[0].error&&begin.source_window===binding.source_window&&begin.window===binding.source_page&&typeof begin.gesture==='string'&&Number.isSafeInteger(begin.sequence)&&state[0].ends[0].gesture===begin.gesture&&state[0].ends[0].trace?.entered_ole===true&&state[0].ends[0].trace?.ole_drop_committed===true&&state[0].ends[0].trace?.query_count>0,'Native begin/commit/terminal not proven');
 assert(state[0].drops.length===0&&state[1].drops.length===1&&drop.window===binding.target_window&&/^[a-f0-9]{32}$/.test(drop.origin?.drag||'')&&drop.origin.id===binding.source_host&&drop.origin.window===binding.source_page&&drop.origin.side==='remote'&&drop.origin.path==='/'&&drop.origin.names.length===3&&JSON.stringify([...drop.origin.names].sort())===JSON.stringify(begin.items.map(i=>i.name).sort())&&!drop.paths,'Not a genuine private marker drop on target B');
 assert((await Promise.all(context.pages().map(p=>p.locator('.files-prompt').count()))).reduce((x,y)=>x+y,0)===1,'Unexpected/emergency-source prompt; never approve');
 assert(state.every(v=>v.transfers.length===0)&&tree(path.join(b.root,'files')).length===0,'Transfer/write started before explicit confirmation');
 const prompt=await target.evaluate(()=>{const p=document.querySelector('.files-prompt');return {count:document.querySelectorAll('.files-prompt').length,title:p?.querySelector('h2')?.textContent,button:p?.querySelector('[data-choice=ok]')?.textContent,policy:p?.querySelector('[name=policy]')?.value,text:p?.textContent};});
 assert(prompt.title==='파일 복사'&&prompt.button==='복사'&&prompt.policy==='keep'&&prompt.text.includes('QA source')&&prompt.text.includes('QA destination')&&prompt.text.includes('3개 항목'),'Wrong target copy prompt');
 const button=await target.locator('.files-prompt [data-choice=ok]').boundingBox();assert(button,'Confirmation geometry absent');
 await sleep(5500);fresh('confirm');
 // Recheck the identical bound prompt immediately before allowing one native click.
 assert(await target.locator('.files-prompt').textContent()===prompt.text&&await source.locator('.files-prompt').count()===0&&await main.locator('.files-prompt').count()===0&&(await target.evaluate(()=>qa.remote_path))==='/','Prompt changed before confirmation');
 const currentButton=await target.locator('.files-prompt [data-choice=ok]').boundingBox();assert(JSON.stringify(currentButton)===JSON.stringify(button),'Confirmation geometry changed');
 report.checks.push({check:'one expected target Copy confirmation attribution',input:await command({op:'confirm',x:Math.round(layout.TargetOriginX+(button.x+button.width/2)*tdpr),y:Math.round(layout.TargetOriginY+(button.y+button.height/2)*tdpr),prompt:{count:1,other_prompts:0,title:prompt.title,button:prompt.button,policy:prompt.policy,names_count:3,source:'/',destination:'/',origin:{id:drop.origin.id,window:drop.origin.window,side:drop.origin.side,path:drop.origin.path,names:drop.origin.names,token_sha256:createHash('sha256').update(drop.origin.drag).digest('hex')}}})});
 const expected=sourceBefore.map(v=>({...v,path:v.path.normalize('NFC')})).sort((a,b)=>a.path.localeCompare(b.path));
 await wait(async()=>{const q=await target.evaluate(()=>qa);assert(!q.jobs.some(j=>j.state==='failed'||j.state==='cancelled'),'Transfer failed/cancelled');return q.jobs.some(j=>j.state==='completed')&&JSON.stringify(tree(path.join(b.root,'files')))===JSON.stringify(expected);},'Completed job/NFC/exact destination tree/hash mismatch',60000);
 const final=await target.evaluate(()=>qa),request=final.transfers[0];assert(final.transfers.length===1&&request.id===binding.target_host&&request.window===binding.target_page&&request.from_id===binding.source_host&&request.from_window===binding.source_page&&request.from_drag===drop.origin.drag&&request.from==='remote'&&request.to==='remote'&&request.source==='/'&&request.destination==='/'&&request.move===false&&request.policy==='keep','Copy request origin/destination routing mismatch');
 const jobs=new Set(final.jobs.map(j=>j.job).filter(Boolean));assert(jobs.size===1&&final.releases.length===1&&final.releases[0].drag===drop.origin.drag,'Extra jobs or missing lease release');
 assert(JSON.stringify(tree(path.join(a.root,'files')))===JSON.stringify(sourceBefore),'Source changed');
 const reads=audit(a),writes=audit(b);assert(!writes.some(e=>e.event==='denied'),'Destination denied operation');
 for(const file of a.config.files){assert(reads.some(e=>e.event==='read-open'&&e.path===file.path)&&reads.some(e=>e.event==='read-bytes'&&e.path===file.path&&e.bytes>0),'Missing source read evidence');assert(writes.some(e=>e.event==='rename'&&e.destination===file.path.normalize('NFC')),'Missing destination staged rename');}
 assert(writes.filter(e=>e.event==='create').length===3&&writes.filter(e=>e.event==='rename').length===3&&writes.some(e=>e.event==='write'&&e.bytes>0),'Unexpected staging/write contract');
 assert(await source.locator('.files-prompt').count()===0&&await target.locator('.files-prompt').count()===0,'Unexpected remaining prompt');
 report.destination=tree(path.join(b.root,'files'));report.checks.push({check:'actual marker route, one remote-to-remote job, NFC filenames, three hashes, exact tree and original source preserved',result:'PASS'});report.status='passed';
}catch(e){report.error=e.message;for(const s of servers)if(s.config?.password)report.error=report.error.replaceAll(s.config.password,'[redacted]');report.status=report.error.includes('prerequisite-unavailable:')?'prerequisite-unavailable':'failed';process.exitCode=1;}
finally{
 const errors=[];const original={status:report.status,error:report.error};
 if(controller){try{if(!controllerExit.stopped){const request=randomUUID();fs.writeFileSync(path.join(root,'command.tmp'),JSON.stringify({op:'stop',request}));fs.renameSync(path.join(root,'command.tmp'),path.join(root,'command.json'));}await wait(()=>controllerExit.stopped,'Native controller did not quiesce; no resource teardown',15000,false);report.cleanup.controller=controllerExit.result;report.cleanup.controller_receipt=read(path.join(root,'controller-cleanup.json'));assert(report.cleanup.controller_receipt.quiescent===true&&report.cleanup.controller_receipt.buttons_up===true,'Controller cleanup/button release unconfirmed');controllerQuiescent=true;assert(controllerExit.result.code===0&&!controllerExit.result.signal,'Controller exit abnormal');}catch(e){errors.push(e.message);}}
 if(controllerQuiescent){
  try{if(browser){const prompts=await Promise.all(browser.contexts().flatMap(c=>c.pages()).map(async p=>{try{return await p.locator('.files-prompt').count();}catch{return 0;}}));report.cleanup.unapproved_prompts=prompts.reduce((a,b)=>a+b,0);if(report.cleanup.unapproved_prompts)errors.push('Unapproved confirmation present; observed only, never clicked');}
   if(app&&!appExit.stopped&&identity?.hwnd&&identity.hwnd!=='0'){control('close');await wait(()=>appExit.stopped,'App normal close timeout; no force termination',15000,false);}if(app){report.cleanup.app_exit=appExit.result;assert(appExit.stopped&&appExit.result.code===0&&!appExit.result.signal,'App normal exit not proven');}
   report.cleanup.no_forced_app_termination=true;
   if(startup){assert(JSON.stringify(control('startup'))===JSON.stringify(startup),'Startup registration changed');report.cleanup.startup_unchanged=true;}
   // Read-only absence test: never call the legacy cleanup helper that can force-kill.
   if(root){const raw=execFileSync('pwsh',['-NoProfile','-NonInteractive','-Command',"$r=$env:ITDA_QA_OWNED_ROOT; @(Get-CimInstance Win32_Process | Where-Object {($_.ExecutablePath -eq (Join-Path $r 'itda.exe')) -or ($_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -and $_.CommandLine.Contains($r+'\\',[StringComparison]::OrdinalIgnoreCase))}).Count"],{encoding:'utf8',env:{...process.env,ITDA_QA_OWNED_ROOT:root},timeout:15000});assert(raw.trim()==='0','Owned app/WebView process remains; no forced cleanup');report.cleanup.no_owned_app_processes=true;}
  }catch(e){errors.push(e.message);}
  if(browser)await Promise.race([browser.close().catch(()=>{}),sleep(3000)]);
  report.cleanup.sftp_fixtures=[];for(const s of servers){try{report[s===servers[0]?'source_audit':'destination_audit']=audit(s);const requested=!s.exit.stopped;if(requested)s.child.kill();await wait(()=>s.exit.stopped,'Owned fixture termination timeout',5000,false);s.child.stdout.destroy();s.child.stderr.destroy();s.child.unref();ownedTemp(s.root,'itda-quality-sftp-');fs.rmSync(s.root,{recursive:true,force:true});assert(!fs.existsSync(s.root),'Fixture workspace remains');report.cleanup.sftp_fixtures.push({role:s===servers[0]?'source':'destination',termination_requested:requested,exit:s.exit.result,workspace_removed:true});}catch(e){errors.push(e.message);}}
  if(root&&!errors.length){ownedTemp(root,'itda-native-idle-');fs.rmSync(root,{recursive:true,force:true});assert(!fs.existsSync(root),'Private profile remains');report.cleanup.workspaces_removed=true;}
 }else errors.push('Controller quiescence unconfirmed: resources deliberately retained, never force-closed');
 for(const child of [app,controller]){child?.stdout?.destroy();child?.stderr?.destroy();child?.unref();}
 report.cleanup.status=errors.length?'failed':'passed';if(errors.length){report.original_outcome=original;report.cleanup.errors=errors;report.status='failed';process.exitCode=1;}if(interrupted){report.status='interrupted';process.exitCode=1;}
 fs.mkdirSync(out,{recursive:true});fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({status:report.status,cleanup:report.cleanup.status}));
}
