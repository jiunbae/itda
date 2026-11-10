// The small window beside System Settings: drag Itda into the list, or flip its switch.
(() => {
 'use strict';
 const send=(cmd,extra={})=>window.ipc?.postMessage(JSON.stringify({cmd,...extra}));
 const $=id=>document.getElementById(id);
 const TEXT={
  screen:{title:'permGuide.screen.title',lead:'permGuide.screen.lead'},
  access:{title:'permGuide.access.title',lead:'permGuide.access.lead'},
 };
 let drag;
 window.itda={recv(m){
  if(m.type!=='guide')return;
  if(m.permission){$('done').hidden=true;const t=TEXT[m.permission]||TEXT.screen;$('step').textContent=m.step||'';$('title').textContent=T(t.title);$('lead').textContent=T(t.lead);}
  if(m.side){const a=$('arrow');a.textContent=m.side==='left'?'←':'→';a.classList.toggle('right',m.side!=='left');}
  if(m.app_path!==undefined)$('path').textContent=m.app_path||'';
  if(m.state!==undefined){const s=$('state');s.textContent=m.state||'';s.classList.toggle('error',!!m.error);}
  if(m.done){$('done-text').textContent=m.done;$('done').hidden=false;}
  // Not granted after a while: show manual re-registration advice for this app.
  if(m.stale!==undefined)$('stale').open=!!m.stale;
 }};
 const app=$('app');
 app.addEventListener('pointerdown',e=>{if(e.button===0)drag={x:e.clientX,y:e.clientY};});
 document.addEventListener('pointermove',e=>{if(drag&&e.buttons===1&&Math.hypot(e.clientX-drag.x,e.clientY-drag.y)>4){drag=null;send('guide_drag');}});
 document.addEventListener('pointerup',()=>drag=null);document.addEventListener('pointercancel',()=>drag=null);
 app.addEventListener('dragstart',e=>e.preventDefault());
 document.addEventListener('click',e=>{const a=e.target.closest('[data-act]')?.dataset.act;if(a)send(`guide_${a}`);});
 I18N.apply();
 $('done-text').textContent=T('permGuide.done');
 send('guide_ready');
})();
