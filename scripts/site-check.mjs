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
    page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
    page.on('requestfailed',r=>errors.push(r.url()+': '+r.failure()?.errorText));
    for(const name of ['', 'start.html']){
      await page.goto(base+name+'?lang='+lang,{waitUntil:'networkidle'});
      await page.evaluate(async()=>{await document.fonts.ready;for(const image of document.images){image.loading='eager';await image.decode().catch(()=>{});}});
      assert(await page.evaluate(()=>document.documentElement.lang)===lang,'Language switch failed');
      assert((await page.locator('meta[name="description"]').getAttribute('content')).includes(lang==='ko'?'파일 브라우저':'file browser'),'Description language stale');
      const broken=await page.evaluate(()=>[...document.images].filter(i=>!i.complete||!i.naturalWidth).map(i=>i.currentSrc));
      assert(!broken.length,`Broken images: ${broken}`);
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Document overflow');
      const overflow=await page.evaluate(()=>[...document.querySelectorAll('.nav .wrap,.hero-copy,.device-scene,.device-phone,.device-tablet,.show-copy,.features,.paths,.phone-row,.final,.product-visual,.discovery-detail,.capture-heading')].filter(e=>{const r=e.getBoundingClientRect();return r.width&& (r.left < -2||r.right>innerWidth+2);}).map(e=>e.className));
      assert(!overflow.length,`Overflow ${width}/${colorScheme}/${lang}/${name}: ${overflow}`);
      const readable=await page.evaluate(()=>[...document.querySelectorAll('img[data-ui-font]')].map(i=>({src:i.currentSrc,size:Number(i.dataset.uiFont)*i.getBoundingClientRect().width/i.naturalWidth})).filter(i=>i.size>0&&i.size<11.5));
      assert(!readable.length,`Product detail text too small: ${JSON.stringify(readable)}`);
      const overview=await page.evaluate(()=>[...document.querySelectorAll('img[data-overview]')].map(i=>{const r=i.getBoundingClientRect(),ratio=i.currentSrc.includes('nearby')?1100/680:1100/800;return i.naturalWidth>=r.width*2-2&&Math.abs(i.naturalWidth/i.naturalHeight-ratio)<.01;}));
      assert(overview.every(Boolean),'Overview cropped or upscaled beyond 2x resolution');
      const gallery=page.locator('[data-platform-gallery]');
      if(await gallery.count()){
        await gallery.locator('[data-platform="windows"]').click();
        const win=gallery.locator('[data-platform-panel="windows"]');
        assert(await win.isVisible(),'Windows file browser did not appear');
        const winReadable=await win.locator('img[data-ui-font]').evaluateAll(images=>images.every(i=>Number(i.dataset.uiFont)*i.getBoundingClientRect().width/i.naturalWidth>=11.5));
        assert(winReadable,'Windows file detail text too small');
        assert(!await gallery.locator('[data-platform-panel="mac"]').isVisible(),'Mac panel did not hide');
        await win.locator('[data-preview]').click();
        await page.locator('.image-dialog img').evaluate(i=>i.decode());
        assert((await page.locator('.image-dialog img').getAttribute('src')).includes('files-windows'),'Windows enlargement opens another platform');
        await page.keyboard.press('Escape');
        if((width===390||width===1440)&&lang==='ko'&&colorScheme==='light')await gallery.screenshot({path:path.join(out,`files-windows-${width}.png`)});
        await gallery.locator('[data-platform="windows"]').focus();await page.keyboard.press('ArrowLeft');
        assert(await gallery.locator('[data-platform="mac"]').getAttribute('aria-selected')==='true','Platform keyboard selection failed');
        await gallery.locator('[data-platform="mac"]').evaluate(e=>e.blur());
      }
      const wrongPreview=await page.evaluate(()=>[...document.querySelectorAll("[data-preview]")].filter(a=>{const img=a.querySelector("img");if(!img)return false;const family=s=>new URL(s,location.href).pathname.split("/").pop().split(/[.-]/)[0];const target=a.dataset.full||a.dataset.fullLight||a.href;return family(img.currentSrc)!==family(target);}).map(a=>a.href));
      assert(!wrongPreview.length,`Screenshot opens unrelated image: ${wrongPreview}`);
      const preview=page.locator('[data-preview]').first();
      if(await preview.count()){
        await preview.click();
        const dialog=page.locator('.image-dialog');
        assert(await dialog.evaluate(d=>d.open),'Screenshot viewer did not open');
        await dialog.locator('img').evaluate(i=>i.decode());
        await dialog.locator('[data-image-size]').click();
        assert(await dialog.evaluate(d=>d.classList.contains('actual')),'Actual-size toggle failed');
        await page.keyboard.press('Escape');
        assert(await preview.evaluate(a=>document.activeElement===a),'Viewer did not restore focus');
        await preview.evaluate(a=>a.blur());
      }
      const dead=await page.evaluate(()=>[...document.querySelectorAll('a[href^="#"]')].filter(a=>a.hash.length>1&&!document.getElementById(decodeURIComponent(a.hash.slice(1)))).map(a=>a.hash));
      assert(!dead.length,`Broken anchors: ${dead}`);
      const ratios=await page.evaluate(()=>[...document.querySelectorAll('.device-laptop img,.device-tablet img,.device-phone img')].map(i=>{const r=i.getBoundingClientRect();return Math.abs(r.width/r.height-i.naturalWidth/i.naturalHeight);}));
      assert(ratios.every(d=>d<.01),`Device image distortion at${width}/${name}`);
      if(width===390||width===1440){
        await page.screenshot({path:path.join(out,`${name?'guide':'landing'}-${width}-${colorScheme}-${lang}.png`)});
        if(!name&&lang==='ko'&&colorScheme==='light'){
          const stories=page.locator('.show');
          for(let i=0;i<await stories.count();i++)await stories.nth(i).screenshot({path:path.join(out,`story-${i}-${width}.png`)});
        }
        if(!name&&width===1440&&lang==='ko'){
          await page.locator('.stage').screenshot({path:path.join(out,`devices-${colorScheme}.png`)});
          await page.locator('.browser-devices').screenshot({path:path.join(out,`phone-feature-${colorScheme}.png`)});
        }
      }
    }
    assert(!errors.length,`Runtime errors: ${errors}`);
    await page.locator('.lang button[data-set="'+(lang==='ko'?'en':'ko')+'"]').click();
    assert(await page.evaluate(()=>document.documentElement.lang)!==lang,'Language control not interactive');
    await context.close();
  }
  console.log('PASS: 40 landing/guide layouts, 5 widths, KO/EN, light/dark, images, anchors, device ratios, readable file details, full-window overviews, platform tabs, image viewer and language controls');
}finally{await browser.close();}
