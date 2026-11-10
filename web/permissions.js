// The app owning these permissions also hosts this window. No separate TCC identity.
(() => {
 'use strict';
 const send=(cmd,extra={})=>window.ipc?.postMessage(JSON.stringify({cmd,...extra}));
 let timer,deadline=0,pending=false,pendingAt=0,drag,requested=null,seenRequest=0;
 const error=text=>{const el=document.querySelector('.error');el.hidden=!text;el.textContent=text||'';};
 function check(refresh=false){if((pending&&Date.now()-pendingAt<5000)||document.hidden)return;pending=true;pendingAt=Date.now();send('permission_status',{refresh});}
 function watch(){deadline=Date.now()+180000;clearInterval(timer);timer=setInterval(()=>{if(Date.now()>deadline){clearInterval(timer);document.querySelector('.progress').textContent=T('perm.retryHint');return;}check();},1500);}
 window.itda={recv(m){
  if(m.type!=='permissions')return;
  pending=false;
  if(m.copied){const b=document.querySelector('[data-action="copy"]');b.textContent=T('perm.copied');setTimeout(()=>b.textContent=T('perm.copy'),1500);}
  if(m.error){error(m.error);return;}error('');
  if(m.supported===false){
   clearInterval(timer);deadline=0;
   document.querySelector('h1').textContent=T('perm.unsupported.title');
   document.querySelector('.intro').textContent=m.support_error||T('perm.unsupported.default');
   document.body.classList.remove('complete');
   document.querySelector('.progress').textContent=T('perm.unsupported.progress');
   document.querySelector('.hp-state').hidden=true;
   for(const row of document.querySelectorAll('[data-permission]')){row.hidden=true;row.querySelector('button').disabled=true;}
   document.querySelector('.instructions').hidden=true;
   document.querySelector('.recovery').hidden=true;
   document.querySelector('.reminder').hidden=true;
   document.querySelector('footer p').hidden=true;
   document.querySelector('[data-action="status"]').hidden=true;
   document.querySelector('.app-drag').disabled=true;
   document.querySelector('.current-app').textContent=`Itda ${m.version||''} · ${m.app_path||T('perm.noAppPath')}`;
   document.querySelector('[data-action="close"]').textContent=T('common.close');
   return;
  }
  document.querySelector('.reminder').hidden=false;
  document.querySelector('footer p').hidden=false;
  document.querySelector('[data-action="status"]').hidden=false;
  const ready=m.screen&&m.access,count=Number(!!m.screen)+Number(!!m.access);
  const activeTransfers=Number(m.active_transfers)||0,restartClosing=m.restart_closing===true;
  // Keyboard and mouse first: screen recording reopens Itda, so it is the last step.
  const restartKeys=['access','screen'].filter(key=>!m[key]&&m[`configured_${key}`]===true),restart=restartKeys.length>0;
  const ungranted=['access','screen'].filter(key=>!m[key]&&!restartKeys.includes(key));
  const checkedDenied=ungranted.some(key=>m[`configured_${key}`]===false);
  requested=m.requested||requested;
  if(m.requested_seq&&m.requested_seq!==seenRequest){seenRequest=m.requested_seq;if(!ready)watch();}
  const next=ungranted.includes(requested)?requested:ungranted[0]||restartKeys[0]||'access';
  document.querySelector('h1').textContent=ready?T('perm.h1.ready'):T('perm.h1.setup');
  document.querySelector('.intro').textContent=ready?T('perm.intro.ready'):T('perm.intro');
  document.body.classList.toggle('complete',!!ready);
  document.querySelector('.progress').textContent=ready?T('perm.progress.ready'):restart?ungranted.length?T('perm.progress.restartMore'):T('perm.progress.restart'):!m.access?T('perm.progress.access',{count}):T('perm.progress.screen',{count});
  // High-performance sharing has its own line: it starts by itself once both are allowed.
  const hp=document.querySelector('.hp-state');hp.hidden=!m.enabled;
  hp.textContent=!m.enabled?'':m.hp_serving&&ready?T('perm.hp.on'):ready?T('perm.hp.starting'):T('perm.hp.waiting');
  for(const key of ['access','screen']){const row=document.querySelector(`[data-permission="${key}"]`),done=!!m[key];row.hidden=false;row.classList.toggle('done',done);row.classList.toggle('pending',restartKeys.includes(key));row.classList.toggle('next',!done&&key===next);row.querySelector('.badge').textContent=done?T('perm.badge.allowed'):restartKeys.includes(key)?T('perm.badge.restart'):T('perm.badge.needed');row.querySelector('button').disabled=done||restartKeys.includes(key);}
  document.querySelector('.recovery').hidden=ready;
  document.querySelector('.recovery h2').textContent=restart?T('perm.recovery.apply'):T('perm.recovery.title');
  document.querySelector('.recovery-message').textContent=activeTransfers>0?T('perm.recovery.transfers',{count:activeTransfers}):restartClosing?T('perm.recovery.closing'):restart?ungranted.length?T('perm.recovery.restartMore'):T('perm.recovery.restart'):checkedDenied?T('perm.recovery.denied'):T('perm.recovery.default');
  document.querySelector('.current-app').textContent=`Itda ${m.version||''} · ${m.app_path||T('perm.noAppPath')}`;
  document.querySelector('[data-action="restart"]').disabled=!m.app_path||activeTransfers>0||restartClosing;
  const instructions=document.querySelector('.instructions');instructions.hidden=ready||!requested||(restart&&!ungranted.length);
  const activeRow=document.querySelector(`[data-permission="${next}"]`);
  if(activeRow.nextElementSibling!==instructions)activeRow.after(instructions);
  document.querySelector('#next-step').textContent=next==='screen'?T('perm.next.screen'):T('perm.next.access');
  document.querySelector('.app-path').textContent=m.app_path||'';
  document.querySelector('.app-drag').disabled=!m.app_path;
  if(m.notice)document.querySelector('.progress').textContent=m.notice;
  const close=document.querySelector('[data-action="close"]');close.textContent=ready?T('perm.done'):T('perm.later');
  if(ready){clearInterval(timer);deadline=0;}
 }};
 document.addEventListener('click',e=>{const request=e.target.closest('[data-request]');if(request&&!request.disabled){requested=request.dataset.request;send('permission_request',{permission:request.dataset.request});watch();return;}const action=e.target.closest('[data-action]')?.dataset.action;if(!action)return;if(action==='status'){pending=false;check(true);watch();}else send(`permission_${action}`);});
 const app=document.querySelector('.app-drag');app.addEventListener('pointerdown',e=>{if(e.button===0&&!app.disabled)drag={x:e.clientX,y:e.clientY};});document.addEventListener('pointermove',e=>{if(drag&&e.buttons===1&&Math.hypot(e.clientX-drag.x,e.clientY-drag.y)>5){drag=null;send('permission_drag');}});document.addEventListener('pointerup',()=>drag=null);document.addEventListener('pointercancel',()=>drag=null);app.addEventListener('dragstart',e=>e.preventDefault());
 document.addEventListener('visibilitychange',()=>{if(!document.hidden)check();});window.addEventListener('focus',check);window.addEventListener('beforeunload',()=>clearInterval(timer));
 // Static text from data-i18n; the parts recv() rewrites start out here.
 I18N.apply();
 document.querySelector('h1').textContent=T('perm.h1.setup');document.querySelector('.intro').textContent=T('perm.intro');document.querySelector('.progress').textContent=T('perm.checking');
 for(const b of document.querySelectorAll('.badge'))b.textContent=T('perm.badge.checking');
 document.querySelector('.recovery h2').textContent=T('perm.recovery.title');document.querySelector('.recovery-message').textContent=T('perm.recovery.default');document.querySelector('[data-action="copy"]').textContent=T('perm.copy');document.querySelector('[data-action="close"]').textContent=T('perm.later');
 check();watch();
})();
