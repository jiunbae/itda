// The film's scenes ("plates"): the real Itda web UI in headless Chromium with fictional
// devices (the same fixtures as scripts/site-capture.mjs), reserved example addresses and a
// made-up remote desktop (desktop.html) that a synthetic RFB server streams to the viewer.
// No accounts, devices, native IPC or user data are touched. Each plate is a PNG at 2x.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
const here=path.dirname(fileURLToPath(import.meta.url));
const root=path.resolve(here,'../..');

const HOSTS=[
  {id:'a1',name:'Studio Mac',os:'mac',host:'studio.local',user:'alex',favorite:true,group:'Work',remote_size:'1600×1000'},
  {id:'b2',name:'Office PC',os:'windows',host:'office.local',user:'',favorite:true,group:'Work',remote_size:'1600×1000'},
  {id:'d4',name:'MacBook Air',os:'mac',host:'air.local',user:'alex',favorite:true,group:'Home',remote_size:'1600×1000'},
  {id:'e5',name:'Home PC',os:'windows',host:'home.local',user:'',favorite:false,group:'Home',remote_size:'1600×1000'},
  {id:'c3',name:'Dev Server',os:'linux',host:'dev.local',user:'alex',favorite:false,group:'Dev',remote_size:'1600×1000'},
  {id:'f6',name:'Build Server',os:'linux',host:'build.local',user:'alex',favorite:false,group:'Dev',remote_size:'1600×1000'},
].map(h=>({...h,label:h.name,kind:'vnc',has_pw:true,login_user:h.user,account:'',ssh:{on:h.os==='linux'},files:{method:'sftp',user:'alex',path:'/Users/alex/Documents'},opts:{},thumb:1}));
const ENTRIES=[['Brand assets',true,0],['Film',true,0],['Projects',true,0],['Screenshots',true,0],['Launch plan.md',false,2850],['Poster frame.png',false,1824000],['Storyboard.pdf',false,182400],['Roadmap.md',false,5460]].map(([name,dir,size])=>({name,dir,size,modified:1780000000,link:false}));
const FOUND=[{name:'Design Mac',host:'design.local',hostname:'design.local',ip:'192.0.2.21',os:'mac',itda:true,share:true},{name:'Meeting PC',host:'meeting.local',hostname:'meeting.local',ip:'192.0.2.22',os:'windows',itda:true,share:true}];
const TAILNET={running:true,me:{name:'My Mac'},peers:[{name:'Lab Mac',dns:'lab.example.ts.net',ip:'100.64.0.20',os:'macOS',online:true}]};

// RFB pieces
const u16=n=>{const b=Buffer.alloc(2);b.writeUInt16BE(n);return b;};
const u32=n=>{const b=Buffer.alloc(4);b.writeUInt32BE(n>>>0);return b;};
const rect=(x,y,w,h,e,d=Buffer.alloc(0))=>Buffer.concat([u16(x),u16(y),u16(w),u16(h),u32(e),d]);
const update=rects=>Buffer.concat([Buffer.from([0,0]),u16(rects.length),...rects]);
function wsFrame(bytes){let h;if(bytes.length<126)h=Buffer.from([0x82,bytes.length]);else if(bytes.length<65536){h=Buffer.alloc(4);h[0]=0x82;h[1]=126;h.writeUInt16BE(bytes.length,2);}else{h=Buffer.alloc(10);h[0]=0x82;h[1]=127;h.writeBigUInt64BE(BigInt(bytes.length),2);}return Buffer.concat([h,bytes]);}

/** RGBA (from a PNG decoded in the browser) → RFB Raw pixels in the format noVNC asks for with
 *  SetPixelFormat (32bpp, red shift 0, little endian: R, G, B, X). */
const raw=(rgba)=>{const b=Buffer.from(rgba);for(let i=3;i<b.length;i+=4)b[i]=0;return b;};

export async function capturePlates({pw,out,log=console.log}){
  fs.mkdirSync(out,{recursive:true});
  const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
  try{
    // 1. The fictional desktops, and their pixels for the RFB server.
    const dp=await browser.newPage({viewport:{width:1600,height:1000}});
    const desk={};
    for(const s of [1,2]){
      await dp.goto(pathToFileURL(path.join(here,'desktop.html')).href+'?screen='+s);
      await dp.evaluate(()=>document.fonts.ready);
      const png=await dp.screenshot();fs.writeFileSync(path.join(out,`desktop-${s}.png`),png);
      const rgba=await dp.evaluate(async b64=>{const img=new Image();img.src='data:image/png;base64,'+b64;await img.decode();const c=new OffscreenCanvas(1600,1000);const x=c.getContext('2d');x.drawImage(img,0,0);return Array.from(x.getImageData(0,0,1600,1000).data);},png.toString('base64'));
      desk[s]={png,pixels:raw(Uint8Array.from(rgba))};
    }
    await dp.close();
    // Two displays side by side for the multi-monitor plate.
    const dual=Buffer.alloc(3200*1000*4);
    for(let y=0;y<1000;y++){desk[1].pixels.copy(dual,y*3200*4,y*1600*4,(y+1)*1600*4);desk[2].pixels.copy(dual,(y*3200+1600)*4,y*1600*4,(y+1)*1600*4);}

    // 2. Static server for the product frontend, fixture API, thumbnails and the RFB socket.
    const sockets=new Set(),peers=[];let dualScreens=false;
    const server=http.createServer((req,res)=>{
      const url=new URL(req.url,'http://localhost');
      if(url.pathname.startsWith('/api/')){
        res.setHeader('Content-Type','application/json');
        const phone=/iPhone/.test(req.headers['user-agent']||'');
        if(url.pathname.endsWith('/me'))return res.end(JSON.stringify(phone?{}:{local:true}));
        if(url.pathname.endsWith('/hosts'))return res.end(JSON.stringify(HOSTS.map(h=>({...h,ssh:h.ssh.on}))));
        return res.end('[]');
      }
      const t=url.pathname.match(/\/thumb\/(\w+)\.png$/);
      if(t){res.setHeader('Content-Type','image/png');return res.end(desk[t[1]==='b2'||t[1]==='e5'?2:1].png);}
      let name=url.pathname;
      if(name==='/viewer/')name='/web/viewer/index.html';else if(/^\/viewer\/(i18n\.js|locales\/)/.test(name))name='/web'+name.slice(7);else if(name.startsWith('/viewer/'))name='/web'+name;else if(name==='/web/')name='/web/index.html';
      const file=name.endsWith('/icon.svg')?path.join(root,'assets/itda.svg'):name.endsWith('/icon-192.png')?path.join(root,'assets/itda-192.png'):path.resolve(root,'.'+name);
      if(!file.startsWith(root+path.sep)||!fs.existsSync(file)||!fs.statSync(file).isFile()){res.writeHead(404);return res.end();}
      res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':name.endsWith('.png')?'image/png':name.endsWith('.svg')?'image/svg+xml':name.endsWith('.webmanifest')?'application/manifest+json':'text/html');
      res.end(fs.readFileSync(file));
    });
    server.on('connection',s=>{sockets.add(s);s.on('close',()=>sockets.delete(s));});
    server.on('upgrade',(req,socket)=>{
      const id=decodeURIComponent(new URL(req.url,'http://x').pathname.split('/').pop());
      const accept=createHash('sha1').update(req.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
      socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: '+accept+'\r\nSec-WebSocket-Protocol: binary\r\n\r\n');
      const two=dualScreens&&id==='a1',W=two?3200:1600,H=1000,pixels=two?dual:desk[id==='b2'?2:1].pixels;
      const peer={id,encodings:[],send:d=>socket.write(wsFrame(d)),sent:false};peers.push(peer);
      peer.send(Buffer.from('RFB 003.008\n'));
      let wire=Buffer.alloc(0),input=Buffer.alloc(0),phase=0;
      const consume=()=>{for(;;){
        if(phase===0){if(input.length<12)return;input=input.subarray(12);phase=1;peer.send(Buffer.from([1,1]));}
        else if(phase===1){if(input.length<1)return;input=input.subarray(1);phase=2;peer.send(u32(0));}
        else if(phase===2){if(input.length<1)return;input=input.subarray(1);phase=3;const n=Buffer.from('Studio Mac');peer.send(Buffer.concat([u16(W),u16(H),Buffer.from([32,24,0,1,0,255,0,255,0,255,16,8,0,0,0,0]),u32(n.length),n]));}
        else{
          if(!input.length)return;const t=input[0];let n=({0:20,3:10,4:8,5:6,150:10,248:9,0xea:6})[t];
          if(t===2){if(input.length<4)return;n=4+input.readUInt16BE(2)*4;}
          if(t===6){if(input.length<8)return;n=8+Math.abs(input.readInt32BE(4));}
          if(t===255){if(input.length<4)return;n=input[1]===1&&input.readUInt16BE(2)===2?10:4;}
          if(!n)return socket.destroy();
          if(input.length<n)return;const m=input.subarray(0,n);input=input.subarray(n);
          if(t===2){const e=[];for(let i=4;i<n;i+=4)e.push(m.readInt32BE(i));peer.encodings.push(e);
            const rects=[];
            // Like Itda's own server: confirm sound, announce the relative pointer.
            if(e.includes(-259))rects.push(rect(0,0,0,0,-259));
            if(e.includes(-257))rects.push(rect(1,0x4954,0,0,-257));
            if(rects.length)peer.send(update(rects));}
          if(t===3&&!peer.sent){peer.sent=true;
            const rects=[];
            if(two){const s=Buffer.concat([Buffer.from([2,0,0,0]),u32(1),u16(0),u16(0),u16(1600),u16(1000),u32(0),u32(2),u16(1600),u16(0),u16(1600),u16(1000),u32(0)]);rects.push(rect(0,0,W,H,-308,s));}
            rects.push(rect(0,0,W,H,0,pixels));peer.send(update(rects));}
        }
      }};
      socket.on('data',chunk=>{wire=Buffer.concat([wire,chunk]);while(wire.length>=2){const op=wire[0]&15;let n=wire[1]&127,at=2;if(n===126){if(wire.length<4)return;n=wire.readUInt16BE(2);at=4;}else if(n===127){if(wire.length<10)return;n=Number(wire.readBigUInt64BE(2));at=10;}const masked=!!(wire[1]&128),start=at+(masked?4:0);if(wire.length<start+n)return;const d=Buffer.from(wire.subarray(start,start+n));if(masked)for(let i=0;i<n;i++)d[i]^=wire[at+i%4];wire=wire.subarray(start+n);if(op===8)return socket.end();input=Buffer.concat([input,d]);consume();}});
      socket.on('error',()=>{});
    });
    await new Promise(r=>server.listen(0,'127.0.0.1',r));
    const base=`http://127.0.0.1:${server.address().port}`;
    const shot=async(page,name,opts={})=>{await page.screenshot({path:path.join(out,name+'.png'),...opts});log('plate',name);};
    // Where things are in each plate (plate pixels = CSS px × deviceScaleFactor), for the camera and cursor.
    const layout={};
    const mark=async(page,plate,key,locator)=>{const b=await locator.first().boundingBox();const d=await page.evaluate(()=>devicePixelRatio);
      const v=page.viewportSize();(layout[plate]||={size:[v.width*d,v.height*d]})[key]=[b.x*d,b.y*d,b.width*d,b.height*d];};

    // 3. The Mac app's windows (home, nearby, files, viewer): light theme, Mac layout, 2x.
    const mac=await browser.newContext({viewport:{width:1100,height:700},deviceScaleFactor:2,reducedMotion:'reduce',locale:'en-US',timezoneId:'America/Los_Angeles',userAgent:'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'});
    await mac.clock.install({time:new Date('2026-11-10T01:24:00Z')});
    await mac.addInitScript(({entries})=>{
      window.ipc={postMessage:raw=>{
        const m=JSON.parse(raw);if(m.cmd!=='files')return;
        const dir=name=>({name,dir:true,size:0,modified:1780000000});
        const tree={'/Users/alex':[dir('Documents'),dir('Downloads'),dir('Desktop')],'/Users/alex/Documents':entries,'/Users/alex/Documents/Film':[dir('Plates'),{name:'Cut v3.mov',dir:false,size:48200000,modified:1780000000},{name:'Poster.jpg',dir:false,size:820000,modified:1780000000}],'/Users/alex/Documents/Film/Plates':[{name:'home.png',dir:false,size:420000,modified:1780000000},{name:'viewer.png',dir:false,size:610000,modified:1780000000}]};
        const result={...m,type:'files_result',ok:true};
        if(m.op==='connect')result.data={path:'/Users/alex/Documents',entries,local_path:'/Users/alex',local_entries:tree['/Users/alex'],rsync:true};
        else if(m.op==='list')result.data={path:m.path,entries:tree[m.path]||[]};
        else return;
        queueMicrotask(()=>itda.recv(result));
      }};
    },{entries:ENTRIES});
    const home=await mac.newPage();
    const prepare=async(extra)=>{
      await home.goto(base+'/web/');
      await home.evaluate(({hosts,extra})=>{
        S.hosts=hosts;S.accounts=[{id:'sample',name:'Work account',user:'alex',has_pw:true,uses:2}];S.version='';
        S.settings={...S.settings,onboarded:true,theme:'light',view:'grid'};S.online=Object.fromEntries(hosts.map(h=>[h.id,true]));S.computer='My Mac';S.sysDark=false;
        S.hosts.forEach(h=>h.last_used=Math.floor(Date.now()/1000)-120);
        if(extra)Function('S',extra)(S);
        applyTheme();render();
      },{hosts:HOSTS,extra});
      await home.evaluate(()=>document.fonts.ready);
      await home.waitForFunction(()=>[...document.querySelectorAll('img[data-thumb]')].every(i=>i.complete));
    };
    await prepare();await shot(home,'home');
    await mark(home,'home','studio',home.locator('.card',{hasText:'Studio Mac'}).locator('.thumb'));
    // Nearby devices turning up one by one.
    for(let n=0;n<=3;n++){
      await prepare(`S.found=${JSON.stringify(FOUND)}.slice(0,${Math.min(n,2)});S.tailnet=${n>=3?JSON.stringify(TAILNET):'{running:true,me:{name:"My Mac"},peers:[]}'};S.page='nearby';`);
      await shot(home,`nearby-${n}`);
    }
    await mark(home,'nearby','add',home.locator('.found').first().locator('button').last());
    await mark(home,'nearby','list',home.locator('.found').first());
    // Files window: list, then columns into Film.
    await home.setViewportSize({width:1100,height:640});
    await prepare();
    await home.evaluate(()=>ItdaFiles.launch(S.hosts[0]));
    await home.locator('.files-list [data-index]').first().waitFor();
    await shot(home,'files-list');
    await mark(home,'files','columns',home.locator('[data-mode="columns"]'));
    await mark(home,'files','film',home.locator('.files-list [data-index]').filter({hasText:'Film'}));
    await home.locator('[data-mode="columns"]').click();
    await home.locator('.files-column').last().getByText('Film',{exact:true}).click();
    await home.locator('.files-column [data-index]').filter({hasText:'Plates'}).first().waitFor();
    await shot(home,'files-columns');
    await mark(home,'files','plates',home.locator('.files-column [data-index]').filter({hasText:'Plates'}));
    // Share page: the QR and PIN for phones.
    await home.setViewportSize({width:1100,height:640});
    await prepare(`S.settings.web={on:true,port:8780};S.web={running:true,pin:'482915',urls:[{url:'http://192.0.2.10:8780',kind:'lan'}],qr:${JSON.stringify(fs.readFileSync(path.join(root,'scripts/fixtures/site-qr.svg'),'utf8'))}};S.share={local_name:'my-mac.local',user:'alex'};S.page='share';`);
    await home.locator('.web-panel').screenshot({path:path.join(out,'share-qr.png')});log('plate','share-qr');

    // Viewer windows (the app's connection window: ?open=).
    const viewer=async(id,size,name,after)=>{
      const p=await mac.newPage();await p.setViewportSize(size);
      await p.goto(`${base}/viewer/?open=${id}`);
      await p.waitForFunction(()=>{const c=document.querySelector('#screen canvas');if(!c||!c.width)return false;const d=c.getContext('2d').getImageData(c.width>>1,c.height>>1,1,1).data;return d[3]>0&&(d[0]+d[1]+d[2])>0;});
      await p.evaluate(()=>document.fonts.ready);
      if(after)await after(p);
      await shot(p,name);
      for(const [k,sel] of [['audio','[data-t="audio"]'],['lock','[data-t="mouse-lock"]'],['toast','.toast:not([hidden]),#toast:not([hidden])'],['tools','#tools']])if(await p.locator(sel).count()&&await p.locator(sel).first().isVisible())await mark(p,name,k,p.locator(sel));
      await p.close();
    };
    await viewer('a1',{width:1280,height:820},'viewer');
    await viewer('a1',{width:1280,height:820},'viewer-sound',async p=>{
      await p.locator('[data-t="audio"]').click();
      await p.waitForFunction(()=>document.querySelector('[data-t="audio"]')?.getAttribute('aria-pressed')==='true');
      await p.waitForTimeout(300);
    });
    await viewer('a1',{width:1280,height:820},'viewer-lock',async p=>{
      // The remote pointer hides (a game): Itda offers to lock the mouse.
      const peer=peers.at(-1);await p.waitForSelector('[data-t="mouse-lock"]');
      peer.send(update([rect(0,0x4954,0,0,-257)]));
      await p.waitForFunction(()=>[...document.querySelectorAll('.toast,#toast')].some(t=>/lock the mouse/i.test(t.textContent)));
    });
    // Two remote displays, each on its own (chosen through the viewer's monitor menu).
    dualScreens=true;
    for(const i of [1,2])
      await viewer('a1',{width:960,height:640},`monitor-${i}`,async p=>{await p.locator('[data-t="monitors"]').click();await p.locator(`[data-monitor="${i}"]`).click();await p.waitForTimeout(300);});
    dualScreens=false;
    await mac.close();

    // 4. The phone: the web viewer in a phone browser, list then the same Mac.
    const phone=await browser.newContext({viewport:{width:390,height:844},deviceScaleFactor:3,isMobile:true,hasTouch:true,reducedMotion:'reduce',locale:'en-US',timezoneId:'America/Los_Angeles',userAgent:'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1'});
    const pp=await phone.newPage();
    await pp.goto(base+'/viewer/');
    await pp.waitForSelector('text=Studio Mac');
    await pp.evaluate(()=>document.fonts.ready);
    await shot(pp,'phone-list');
    await mark(pp,'phone-list','studio',pp.getByText('Studio Mac',{exact:true}));
    // Turned sideways to watch the screen.
    await pp.setViewportSize({width:844,height:390});
    await pp.getByText('Studio Mac',{exact:true}).first().click();
    await pp.waitForFunction(()=>{const c=document.querySelector('#screen canvas');if(!c||!c.width)return false;const d=c.getContext('2d').getImageData(c.width>>1,c.height>>1,1,1).data;return (d[0]+d[1]+d[2])>0;});
    await pp.waitForTimeout(4500); // the 'connected' toast and the toolbar fade
    await shot(pp,'phone-view');
    await phone.close();
    fs.writeFileSync(path.join(out,'layout.json'),JSON.stringify(layout,null,1));
    for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));
  }finally{await browser.close();}
}
