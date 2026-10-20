// Test the actual home/menu and viewer toolbar without launching an external app.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const assert=(ok,why)=>{if(!ok)throw Error(why);};
const sockets=new Set();
const fixture=(kind)=>({id:'fixture',name:'Fixture',label:'Fixture',host:'mac.local:5901',user:'june',os:kind==='missing'?'':kind==='rdp'?'mac':kind,kind:kind==='rdp'?'rdp':'vnc',has_pw:true,opts:{}});
const server=http.createServer((req,res)=>{
  const url=new URL(req.url,'http://localhost');
  const referer=new URL(req.headers.referer||'http://localhost');
  if(url.pathname.startsWith('/api/')){
    res.setHeader('Content-Type','application/json');
    return res.end(JSON.stringify(url.pathname==='/api/me'?{local:true}:url.pathname==='/api/hosts'?[fixture(referer.searchParams.get('target')||'mac')]:[]));
  }
  const name=url.pathname==='/'?'web/index.html':url.pathname==='/viewer/'?'web/viewer/index.html':`web${url.pathname}`;
  const file=path.resolve(root,name);
  if(!file.startsWith(`${root}/web/`)||!fs.existsSync(file)||!fs.statSync(file).isFile()){res.writeHead(404);return res.end();}
  res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':'text/html');
  res.end(fs.readFileSync(file));
});
server.on('connection',socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));});
server.on('upgrade',(req,socket)=>{
  const accept=createHash('sha1').update(req.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
  socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\nSec-WebSocket-Protocol: binary\r\n\r\n`);
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const base=`http://127.0.0.1:${server.address().port}`;
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
try{
  for(const localMac of [true,false]){
    const context=await browser.newContext({userAgent:localMac?'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)':'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'});
    await context.addInitScript(mac=>{Object.defineProperty(navigator,'platform',{value:mac?'MacIntel':'Win32'});window.messages=[];window.ipc={postMessage:m=>messages.push(JSON.parse(m))};},localMac);
    const home=await context.newPage();await home.goto(base);
    for(const kind of ['mac','macos','windows','linux','missing','rdp']){
      await home.evaluate(h=>{S.hosts=[h];render();showCtx(h.id,20,20);},fixture(kind));
      const option=home.locator('#ctx [data-act="apple-screen-sharing"]');
      const expected=localMac&&['mac','macos'].includes(kind);
      assert(await option.count()===(expected?1:0),`Home visibility wrong: ${localMac}/${kind}`);
      if(expected){await option.click();assert(await home.evaluate(()=>messages.at(-1).cmd==='apple_screen_sharing'&&messages.at(-1).id==='fixture'&&!('password' in messages.at(-1))), 'Home sent credentials or wrong target');}
    }
    for(const kind of ['mac','macos','windows','missing','rdp']){
      const page=await context.newPage();await page.goto(`${base}/viewer/?open=fixture&target=${kind}`);
      await page.locator('#tools [data-t="keys"]').waitFor();
      const option=page.locator('[data-t="apple-screen-sharing"]');
      const expected=localMac&&['mac','macos'].includes(kind);
      assert(await option.count()===(expected?1:0),`Viewer visibility wrong: ${localMac}/${kind}`);
      if(expected){await option.click();assert(await page.evaluate(()=>messages.at(-1).ev==='apple-screen-sharing'&&Object.keys(messages.at(-1)).length===1),'Viewer supplied an address/credentials instead of bound session');}
      await page.close();
    }
    await context.close();
  }
  const web=await browser.newPage();
  await web.goto(`${base}/viewer/?target=mac`);
  await web.locator('.host[data-id="fixture"] .open').click();
  await web.locator('#tools [data-t="keys"]').waitFor();
  assert(await web.locator('[data-t="apple-screen-sharing"]').count()===0,'Browser viewer exposed native-only action');
  await web.close();
  console.log('PASS: Apple Screen Sharing menu/toolbar visibility and credential-free IPC on Mac/Windows viewers');
}finally{await browser.close();for(const socket of sockets)socket.destroy();await new Promise(resolve=>server.close(resolve));}
