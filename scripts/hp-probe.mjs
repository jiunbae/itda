// Read-only SSH/RFB probe: one screenshot update, no remote input or permission changes.
// The sharing password stays in memory and is never printed or put in argv.
import net from 'node:net';
import {spawn,spawnSync} from 'node:child_process';
import {createCipheriv} from 'node:crypto';
const target=process.argv[2];
if(!target||target.startsWith('-')||!/^[-a-zA-Z0-9_.@]+$/.test(target))throw Error('Usage: node scripts/hp-probe.mjs ssh-host');
const ssh=['-o','BatchMode=yes','-o','ConnectTimeout=8'];
const secret=spawnSync('ssh',[...ssh,target,'cat "$HOME/Library/Application Support/Itda/hp-password"'],{encoding:null,timeout:12000});
if(secret.status!==0)throw Error('Cannot read sharing credential over SSH');
const key=Buffer.alloc(8);secret.stdout.copy(key,0,0,8);secret.stdout.fill(0);
for(let i=0;i<8;i++){let n=key[i],r=0;for(let b=0;b<8;b++){r=(r<<1)|(n&1);n>>=1;}key[i]=r;}
const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
const tunnel=spawn('ssh',[...ssh,'-o','ExitOnForwardFailure=yes','-N','-L',`127.0.0.1:${port}:127.0.0.1:5910`,target],{stdio:['ignore','ignore','ignore']});
let socket,buf=Buffer.alloc(0),waiter,ended=false;
async function read(n){const end=Date.now()+12000;while(buf.length<n){if(ended)throw Error('Server closed before response');if(Date.now()>end)throw Error(`Timeout waiting for ${n} bytes; buffered ${buf.length}`);await new Promise(r=>{waiter=r;setTimeout(r,50);});}const out=buf.subarray(0,n);buf=buf.subarray(n);return out;}
try{
 for(let i=0;i<100;i++){if(tunnel.exitCode!==null)throw Error('SSH forward failed');try{socket=await new Promise((r,j)=>{const s=net.connect(port,'127.0.0.1');s.once('connect',()=>r(s));s.once('error',j);});break;}catch{await new Promise(r=>setTimeout(r,50));}}
 if(!socket)throw Error('SSH forward unavailable');
 socket.on('data',b=>{buf=Buffer.concat([buf,b]);waiter?.();});socket.on('error',()=>{ended=true;waiter?.();});socket.on('close',()=>{ended=true;waiter?.();});
 const started=Date.now();const greeting=(await read(12)).toString();if(!greeting.startsWith('RFB '))throw Error('Not an RFB server');socket.write('RFB 003.008\n');
 const count=(await read(1))[0],types=await read(count);if(!types.includes(2))throw Error('VNC authentication not offered');socket.write(Buffer.from([2]));
 const challenge=await read(16),triple=Buffer.concat([key,key,key]);const cipher=createCipheriv('des-ede3',triple,null);cipher.setAutoPadding(false);socket.write(Buffer.concat([cipher.update(challenge),cipher.final()]));key.fill(0);triple.fill(0);
 const status=(await read(4)).readUInt32BE();if(status){const length=(await read(4)).readUInt32BE();throw Error((await read(length)).toString());}socket.write(Buffer.from([1]));
 const init=await read(24),width=init.readUInt16BE(0),height=init.readUInt16BE(2),name=(await read(init.readUInt32BE(20))).toString();console.log(JSON.stringify({stage:'connected',target,width,height,name,elapsed_ms:Date.now()-started}));
 const enc=Buffer.alloc(12);enc[0]=2;enc.writeUInt16BE(2,2);enc.writeInt32BE(50,4);enc.writeInt32BE(16,8);socket.write(enc);
 const req=Buffer.alloc(10);req[0]=3;req.writeUInt16BE(width,6);req.writeUInt16BE(height,8);socket.write(req);
 const update=await read(4);if(update[0]!==0)throw Error(`Unexpected server message ${update[0]}`);const rect=await read(12),encoding=rect.readInt32BE(8);let payload;
 if(encoding===50){const h=await read(8),size=h.readUInt32BE(0);payload=await read(size);console.log(JSON.stringify({stage:'frame',encoding,bytes:payload.length,flags:h.readUInt32BE(4),elapsed_ms:Date.now()-started}));}
 else if(encoding===16){const size=(await read(4)).readUInt32BE();payload=await read(size);console.log(JSON.stringify({stage:'frame',encoding,bytes:payload.length,elapsed_ms:Date.now()-started}));}
 else throw Error(`Unexpected encoding ${encoding}`);
}catch(e){console.error(JSON.stringify({stage:'failed',error:e.message}));process.exitCode=1;}finally{key.fill(0);socket?.destroy();tunnel.kill('SIGTERM');}
