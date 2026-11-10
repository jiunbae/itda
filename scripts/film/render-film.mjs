// Itda's landing film: capture the plates (plates.mjs), then lay them out frame by frame in
// stage.html and encode with ffmpeg. Every frame is a pure function of its time.
//   node scripts/film/render-film.mjs                 plates + film → docs/media/
//   node scripts/film/render-film.mjs --plates        plates only
//   node scripts/film/render-film.mjs --preview 1,4.5 stills at those seconds + a contact sheet
// PLAYWRIGHT / CHROMIUM as in site-capture.mjs; FILM_WORK overrides the scratch directory.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {capturePlates} from './plates.mjs';
const here=path.dirname(fileURLToPath(import.meta.url));
const root=path.resolve(here,'../..');
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const args=process.argv.slice(2);
const work=process.env.FILM_WORK||path.join(os.tmpdir(),'itda-film');
const plates=path.join(work,'plates');
const media=path.join(root,'docs/media');
const FPS=30,W=1920,H=1080;

if(!args.includes('--reuse-plates'))await capturePlates({pw,out:plates});
if(args.includes('--plates'))process.exit(0);

const run=(cmd,a,input)=>new Promise((ok,fail)=>{const p=spawn(cmd,a,{stdio:[input?'pipe':'ignore','ignore','inherit']});p.on('error',fail);p.on('close',c=>c===0?ok():fail(Error(`${cmd} exited ${c}`)));if(input)input(p.stdin);});
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
try{
  const page=await browser.newPage({viewport:{width:W,height:H},deviceScaleFactor:1});
  const errors=[];page.on('pageerror',e=>{errors.push(e.message);console.error('stage:',e.message);});
  await page.addInitScript(l=>{window.LAYOUT=l;},JSON.parse(fs.readFileSync(path.join(plates,'layout.json'),'utf8')));
  await page.goto(pathToFileURL(path.join(here,'stage.html')).href+'?plates='+encodeURIComponent(pathToFileURL(plates).href));
  await page.waitForFunction(()=>window.stageReady===true,null,{timeout:20000});
  const duration=await page.evaluate(()=>window.DURATION);
  const frameAt=async t=>{await page.evaluate(t=>window.render(t),t);return page.screenshot({type:'png'});};
  const pi=args.indexOf('--preview');
  if(pi>=0){
    const times=(args[pi+1]||'').split(',').filter(Boolean).map(Number);
    const at=times.length?times:Array.from({length:Math.ceil(duration/2)},(_,i)=>i*2+1);
    const dir=path.join(work,'preview');fs.rmSync(dir,{recursive:true,force:true});fs.mkdirSync(dir,{recursive:true});
    for(const [i,t] of at.entries())fs.writeFileSync(path.join(dir,`${String(i).padStart(3,'0')}.png`),await frameAt(t));
    const cols=Math.min(4,at.length),rows=Math.ceil(at.length/cols);
    await run('ffmpeg',['-y','-loglevel','error','-framerate','1','-i',path.join(dir,'%03d.png'),'-vf',`scale=640:-1,tile=${cols}x${rows}:padding=6:color=white`,'-frames:v','1',path.join(work,'contact.png')]);
    console.log('preview',dir,'contact',path.join(work,'contact.png'));
  }else{
    fs.mkdirSync(media,{recursive:true});
    const master=path.join(work,'master.mp4');
    const frames=Math.round(duration*FPS);
    await run('ffmpeg',['-y','-loglevel','error','-f','image2pipe','-framerate',String(FPS),'-c:v','png','-i','-','-c:v','libx264','-preset','veryfast','-crf','10','-pix_fmt','yuv444p',master],async stdin=>{
      for(let f=0;f<frames;f++){const png=await frameAt(f/FPS);if(!stdin.write(png))await new Promise(r=>stdin.once('drain',r));if(f%60===0)process.stdout.write(`\rframe ${f}/${frames}`);}
      stdin.end();
    });
    console.log('\nencoding');
    const vf='format=yuv420p';
    await Promise.all([
      run('ffmpeg',['-y','-loglevel','error','-i',master,'-vf',vf,'-c:v','libx264','-preset','slow','-crf','22','-tune','animation','-movflags','+faststart','-an',path.join(media,'itda-film.mp4')]),
      run('ffmpeg',['-y','-loglevel','error','-i',master,'-vf',vf,'-c:v','libvpx-vp9','-crf','38','-b:v','0','-row-mt','1','-an',path.join(media,'itda-film.webm')]),
    ]);
    const posterAt=await page.evaluate(()=>window.POSTER);
    fs.writeFileSync(path.join(work,'poster.png'),await frameAt(posterAt));
    await run('ffmpeg',['-y','-loglevel','error','-i',path.join(work,'poster.png'),'-q:v','3',path.join(media,'itda-film-poster.jpg')]);
    for(const f of ['itda-film.mp4','itda-film.webm','itda-film-poster.jpg'])console.log(f,(fs.statSync(path.join(media,f)).size/1e6).toFixed(2),'MB');
  }
  if(errors.length)throw Error('stage errors: '+errors.join('; '));
}finally{await browser.close();}
