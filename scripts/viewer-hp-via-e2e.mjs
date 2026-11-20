// The same Mac saved twice, by its LAN address without high-performance sharing and by its
// Tailscale name with it (hosts_json's hp_via): opening the first connects through the second,
// under the first's name, and the viewer treats it as high-performance (mode check, keys).
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const hosts=[
 {id:'lan',label:'Studio (LAN)',host:'192.168.1.20',os:'mac',kind:'vnc',has_pw:true,hp:{on:false,native:false},hp_via:'tail'},
 {id:'tail',label:'Studio',host:'studio.tail',os:'mac',kind:'vnc',has_pw:true,hp:{on:true,native:false},hp_via:null},
];
const modes=[];
const server=http.createServer((req,res)=>{
 const url=new URL(req.url,'http://localhost');
 if(url.pathname.startsWith('/api/')){
  res.setHeader('Content-Type','application/json');
  if(url.pathname.startsWith('/api/mode/')){modes.push(url.pathname.slice(10));return res.end(JSON.stringify({hp:true}));}
  return res.end(JSON.stringify(url.pathname==='/api/hosts'?hosts:url.pathname==='/api/accounts'?[]:{local:true,ok:true}));
 }
 if(url.pathname==='/novnc.min.js'){
  res.setHeader('Content-Type','text/javascript');
  return res.end(`window.RFB=class extends EventTarget {
   constructor(){super();window.fixtureSessions=(window.fixtureSessions||0)+1;window.fixtureRfb=this;this._fbWidth=640;this._fbHeight=480;this._rfbConnectionState='connected';this._display={_backbuffer:document.createElement('canvas'),itdaFrame:1};if(!new URLSearchParams(location.search).has('hold'))setTimeout(()=>this.dispatchEvent(new Event('connect')),0);}
   focus(){} itdaViewport(){} disconnect(){this._rfbConnectionState='disconnected';this.dispatchEvent(new Event('disconnect'));}
  };`);
 }
 const name=url.pathname==='/'?'index.html':url.pathname.slice(1),file=path.resolve(root,/^(i18n\.js|locales\/[\w.-]+\.js)$/.test(name)?'web':'web/viewer',name);
 if(!file.startsWith(path.join(root,'web')+path.sep)||!fs.existsSync(file)||!fs.statSync(file).isFile()){res.writeHead(404);return res.end();}
 res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(file));
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(v,m)=>{if(!v)throw Error(m);};
try{
 const p=await browser.newPage({locale:'ko-KR'}),errors=[];p.on('pageerror',e=>errors.push(e.message));
 await p.addInitScript(()=>{
  window.ipcEvents=[];window.ipc={postMessage:m=>ipcEvents.push(JSON.parse(m))};
  window.WebSocket=class extends EventTarget {constructor(url){super();this.url=url;window.fixtureWs=this;}close(){}send(){}};
 });
 await p.goto(`http://127.0.0.1:${server.address().port}/?open=lan`);
 await p.waitForFunction(()=>window.fixtureWs);
 const ws=await p.evaluate(()=>fixtureWs.url);
 assert(ws.includes('/ws/tail?'),'did not connect through the high-performance connection: '+ws);
 assert(await p.title()==='Studio (LAN)','not under the opened connection\'s name: '+await p.title());
 await p.waitForFunction(()=>document.querySelector('[data-t=hp]')?.classList.contains('on'));
 assert(modes.length>0&&modes.every(m=>m==='tail'),'mode asked of the wrong connection: '+modes);
 assert(errors.length===0,errors.join('\n'));
 console.log('PASS: a Mac saved twice opens through its high-performance connection, under the name opened, in high-performance mode');
}finally{await browser.close();await new Promise(r=>server.close(r));}
