// Layout/content checks for the static site; SITE_URL can point to the deployed Pages site.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import {pathToFileURL} from 'node:url';
const pw=await import(process.env.PLAYWRIGHT?pathToFileURL(process.env.PLAYWRIGHT).href:'playwright');
const base=(process.env.SITE_URL||'http://127.0.0.1:4186/docs/').replace(/\/?$/,'/');
const out=process.env.SCREENSHOT_DIR||path.join(os.tmpdir(),'itda-landing-qa');fs.mkdirSync(out,{recursive:true});
const browser=await pw.chromium.launch({headless:true,...(process.env.CHROMIUM?{executablePath:process.env.CHROMIUM}:{})});
const assert=(ok,msg)=>{if(!ok)throw Error(msg);};
try{
  for(const width of [360,390,768,1024,1440]) for(const colorScheme of ['light','dark']) for(const lang of ['ko','en']){
    const context=await browser.newContext({viewport:{width,height:1100},colorScheme,reducedMotion:'reduce'});
    const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
    for(const name of ['', 'start.html']){
      await page.goto(base+name+'?lang='+lang,{waitUntil:'networkidle'});
      await page.evaluate(async()=>{await document.fonts.ready;for(const image of document.images){image.loading='eager';await image.decode().catch(()=>{});}});
      assert(await page.evaluate(()=>document.documentElement.lang)===lang,'Language switch failed');
      const broken=await page.evaluate(()=>[...document.images].filter(i=>!i.complete||!i.naturalWidth).map(i=>i.currentSrc));
      assert(!broken.length,`Broken images: ${broken}`);
      const overflow=await page.evaluate(()=>[...document.querySelectorAll('.nav .wrap,.hero-copy,.device-scene,.device-phone,.device-tablet,.show-copy,.features,.paths,.phone-row,.final')].filter(e=>{const r=e.getBoundingClientRect();return r.width&& (r.left < -2||r.right>innerWidth+2);}).map(e=>e.className));
      assert(!overflow.length,`Overflow ${width}/${colorScheme}/${lang}/${name}: ${overflow}`);
      const dead=await page.evaluate(()=>[...document.querySelectorAll('a[href^="#"]')].filter(a=>a.hash.length>1&&!document.getElementById(decodeURIComponent(a.hash.slice(1)))).map(a=>a.hash));
      assert(!dead.length,`Broken anchors: ${dead}`);
      const ratios=await page.evaluate(()=>[...document.querySelectorAll('.device-laptop img,.device-tablet img,.device-phone img')].map(i=>{const r=i.getBoundingClientRect();return Math.abs(r.width/r.height-i.naturalWidth/i.naturalHeight);}));
      assert(ratios.every(d=>d<.01),`Device image distortion at${width}/${name}`);
      if(width===390||width===1440){
        await page.screenshot({path:path.join(out,`${name?'guide':'landing'}-${width}-${colorScheme}-${lang}.png`)});
        if(!name&&width===1440&&lang==='ko'){
          await page.locator('.stage').screenshot({path:path.join(out,`devices-${colorScheme}.png`)});
          await page.locator('.browser-devices').screenshot({path:path.join(out,`tablet-${colorScheme}.png`)});
        }
      }
    }
    assert(!errors.length,`Runtime errors: ${errors}`);
    await page.locator('.lang button[data-set="'+(lang==='ko'?'en':'ko')+'"]').click();
    assert(await page.evaluate(()=>document.documentElement.lang)!==lang,'Language control not interactive');
    await context.close();
  }
  console.log('PASS: 40 landing/guide layouts, 5 widths, KO/EN, light/dark, images, anchors, device ratios and language controls');
}finally{await browser.close();}
