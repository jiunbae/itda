// Native setup dialog, exercised without SSH or reading any real credentials.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const server=http.createServer((req,res)=>{
 const name=new URL(req.url,'http://localhost').pathname;
 const file=path.resolve(root,'.'+(name==='/web/'?'/web/index.html':name));
 if(!file.startsWith(root+path.sep)||!fs.existsSync(file)){res.writeHead(404);return res.end();}
 res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(file));
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(v,m)=>{if(!v)throw Error(m);};
try {
 for(const platform of ['mac','windows']) {
  const c=await browser.newContext({viewport:{width:1100,height:900},userAgent:platform==='mac'?'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)':'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
  await c.addInitScript(()=>{window.packets=[];window.ipc={postMessage:raw=>packets.push(JSON.parse(raw))};});
  const p=await c.newPage(),errors=[];p.on('pageerror',e=>errors.push(e.message));
  await p.goto(`http://127.0.0.1:${server.address().port}/web/`);
  await p.evaluate(()=>{
   S.hosts=[{id:'studio',name:'Studio <Mac>',host:'studio.local:5900',user:'alex',ssh:{port:2222},os:'mac',kind:'vnc',account:'work'}];
   S.accounts=[{id:'work',name:'Work',user:'alex'},{id:'other',name:'Other',user:'sam'}];
   S.settings={...S.settings,onboarded:true};render();
   window.openSetup=()=>ItdaRemoteSetup.open({id:'studio',hosts:S.hosts,accounts:S.accounts,send:(cmd,data)=>ipc.postMessage(JSON.stringify({cmd,...data})),modal:openModal,close:closeModal,toast,esc});
   openSetup();
  });
  const dialog=p.locator('.remote-setup'),q=s=>dialog.locator(s);
  assert(await q('[data-address]').inputValue()==='studio.local','target port must be stripped');
  assert(await q('[data-port]').inputValue()==='2222','SSH port not reused');
  assert(await q('[data-user]').inputValue()==='alex','saved account not reused');
  assert(!await q('[data-passwords]').isChecked()&&!await q('[data-keys]').isChecked(),'secrets must be opt-in');
  await q('[data-account]').selectOption('other');assert(await q('[data-user]').inputValue()==='sam','account change did not update username');
  await q('[data-password]').fill('fixture-only-password');await q('[data-probe]').click();
  assert(await q('[data-password]').inputValue()==='','password input was retained');
  const probe=await p.evaluate(()=>packets.find(m=>m.cmd==='remote_setup_probe'));
  assert(probe.password==='fixture-only-password'&&probe.login_account==='other','probe authentication options lost');
  assert(!probe.selection.passwords&&!probe.selection.private_keys,'secrets sent by default');
  assert(!(await p.evaluate(()=>packets)).some(m=>m.cmd==='remote_setup_apply'),'probe must not install');
  await p.evaluate(({request,platform})=>ItdaRemoteSetup.message({type:'remote_setup',request,prepared:true,token:'opaque-test-token',platform,version:'0.9.99',running:false,address:'studio.local',user:'sam',port:2222,hosts:1,accounts:1,settings:true,passwords:false,private_keys:false,launch:true}),{request:probe.request,platform});
  await q('[data-apply]').click();
  const apply=await p.evaluate(()=>packets.find(m=>m.cmd==='remote_setup_apply'));
  assert(apply.token==='opaque-test-token'&&Object.keys(apply).sort().join(',')==='cmd,request,token','apply must contain only confirmed opaque token');
  await q('[data-close]').click();assert(await dialog.count()===0,'background close failed');
  await p.evaluate(()=>openSetup());assert(await dialog.count()===1,'running setup did not reattach');
  assert(await q('[data-apply]').isDisabled(),'reattached flow can start duplicate install');
  await p.evaluate(request=>ItdaRemoteSetup.message({type:'remote_setup',request,complete:true,version:'0.9.99',hosts:1,accounts:1}),probe.request);
  await q('[data-close]').click();await p.evaluate(()=>openSetup());await q('[data-probe]').click();
  const next=await p.evaluate(()=>packets.filter(m=>m.cmd==='remote_setup_probe').at(-1));
  await p.evaluate(request=>ItdaRemoteSetup.message({type:'remote_setup',request,prepared:true,token:'discard-me',platform:'mac',address:'host',port:22,hosts:1,accounts:1}),next.request);
  await q('[data-close]').click();assert((await p.evaluate(()=>packets)).some(m=>m.cmd==='remote_setup_discard'&&m.token==='discard-me'),'closing preview did not discard credentials');
  assert(errors.length===0,errors.join('\n'));console.log(`${platform}: remote setup confirmed plan, opt-in secrets, reattach and discard PASS`);
  await c.close();
 }
}finally{await browser.close();await new Promise(r=>server.close(r));}
