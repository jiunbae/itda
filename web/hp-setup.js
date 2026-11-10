// Shared remote setup guide. The existing screen connection stays usable beneath it.
(() => {
 'use strict';
 let current,timer;
 // Text kept as a function follows a language change (web/i18n.js).
 const tx=v=>typeof v==='function'?v():v;
 const clear=()=>{clearTimeout(timer);const c=current;if(!c)return;const focused=c.panel.contains(document.activeElement);c.panel.remove();current=null;if(focused&&c.previous?.isConnected)c.previous.focus();};
 function render(){
  const c=current;if(!c)return;
  // The helper on the remote Mac guides each step: here only a one-line progress, not the
  // same steps again. (Without it, e.g. it couldn't be opened, the steps and their buttons stay.)
  const remote=!!c.remoteHelper&&!c.error;
  const allowed=['access','screen'].filter(k=>c[k]===true).length;
  c.panel.querySelector('h2').textContent=remote?T('hpSetup.titleProgress',{name:c.name,allowed}):T('hpSetup.title',{name:c.name});
  // (After the bounded wait, say so and offer the check again.)
  const waiting=remote&&Date.now()<=c.deadline;
  c.panel.querySelector('[data-hp-stage]').textContent=waiting?T('hpSetup.followHelper'):tx(c.stage);
  for(const key of ['access','screen']){const row=c.panel.querySelector(`[data-hp-permission="${key}"]`),done=c[key]===true;row.querySelector('.hp-result').textContent=T(done?'hpSetup.allowed':c[key]===false?'hpSetup.needed':'hpSetup.checking');row.classList.toggle('done',done);row.querySelector('button').disabled=!c.ready||done;}
  c.panel.querySelector('[data-hp-action="helper"]').disabled=!c.ready;
  c.panel.classList.toggle('hp-compact',remote);
  c.panel.querySelector('[data-hp-remote]').hidden=!remote;
  c.panel.querySelectorAll('[data-hp-manual]').forEach(p=>p.hidden=remote);
  c.panel.querySelector('[data-hp-action="helper"]').textContent=T(remote?'hpSetup.reopenHelper':'hpSetup.openHelper');
  c.panel.querySelector('[data-hp-action="check"]').hidden=waiting;
  for(const key of ['access','screen'])c.panel.querySelector(`[data-hp-open="${key}"]`).hidden=remote;
  const check=c.panel.querySelector('[data-hp-action="check"]');check.disabled=!c.ready&&!c.failed;check.textContent=T(c.failed?'hpSetup.retry':'hpSetup.check');
  const error=c.panel.querySelector('[data-hp-error]');error.hidden=!c.error;error.textContent=tx(c.error)||'';
 }
 function poll(){clearTimeout(timer);const c=current;if(!c||!c.ready)return;if(Date.now()>c.deadline){c.stage=()=>T('hpSetup.checkWhenDone');render();return;}timer=setTimeout(()=>{if(current===c)c.send('status');},4000);}
 function begin(options){
  clear();const panel=document.createElement('aside');panel.className='hp-setup';panel.setAttribute('data-i18n-attr','aria-label:hpSetup.label');
  // Fixed text by message ID (I18N.apply, again on a language change); render() sets the rest.
  panel.innerHTML='<header><span>⚡</span><h2></h2></header><p data-hp-stage role="status" aria-live="polite"></p><p class="hp-explain" data-hp-remote hidden data-i18n-html="hpSetup.remoteExplain"></p><p class="hp-explain" data-hp-manual data-i18n-html="hpSetup.manualExplain"></p><div class="hp-permission" data-hp-permission="access"><div><strong data-i18n="hpSetup.access"></strong><span class="hp-result"></span></div><button data-hp-open="access" data-i18n="hpSetup.openAccess"></button></div><div class="hp-permission" data-hp-permission="screen"><div><strong data-i18n="hpSetup.screen"></strong><span class="hp-result"></span></div><button data-hp-open="screen" data-i18n="hpSetup.openScreen"></button></div><p class="hp-explain" data-hp-manual data-i18n="hpSetup.manualSwitch"></p><p data-hp-error role="alert" hidden></p><footer><button data-hp-action="helper"></button><button data-hp-action="check"></button><button data-hp-action="later" data-i18n="hpSetup.later"></button></footer>';
  I18N.apply(panel);
  current={...options,panel,previous:document.activeElement,stage:()=>T('hpSetup.preparing'),ready:false,screen:null,access:null,deadline:Date.now()+300000};
  panel.addEventListener('click',e=>{const c=current;if(!c)return;const permission=e.target.closest('[data-hp-open]')?.dataset.hpOpen;if(permission){c.error='';c.send('permissions',{permission});render();return;}const action=e.target.closest('[data-hp-action]')?.dataset.hpAction;if(action==='helper'){c.deadline=Date.now()+300000;c.send('permissions');poll();}if(action==='check'){c.deadline=Date.now()+300000;c.error='';const failed=c.failed;c.failed=false;c.stage=failed?()=>T('hpSetup.preparingAgain'):c.stage;c.send(failed?'upgrade':'status');render();}if(action==='later'){const cancel=c.cancel;clear();cancel?.();}});
  (options.container||document.body).append(panel);
  // From the connection editor, keep helper controls inside its modal focus scope.
  // Closing that editor must leave the ongoing setup available as a standalone guide.
  options.container?.closest('.scrim')?.addEventListener('itda-modal-close',()=>{if(panel.isConnected)document.body.append(panel);},{once:true});
  render();
 }
 function message(m){
  const c=current;if(!c||m.id!==c.id)return false;
  if(m.helper){c.error=m.error||'';c.remoteHelper=!m.error;if(!m.error)c.stage=()=>T('hpSetup.helperOpened');render();return true;}
  if(m.error){c.error=m.error;c.failed=!m.status;c.stage=m.status?()=>T('hpSetup.statusFailed'):()=>T('hpSetup.prepareFailed');render();if(m.status)poll();return true;}
  if(m.stage){c.stage=m.stage;render();return true;}
  if(m.ready||m.status){
   c.ready=true;c.failed=false;c.error=m.helper_error||'';c.screen=!!m.screen;c.access=!!m.access;
   // Setup opens the remote helper itself when something is missing (helper_error says if not).
   if(m.ready&&!(c.screen&&c.access))c.remoteHelper=!m.helper_error;
   if(m.running===false){c.error=()=>T('hpSetup.notRunning');c.stage=()=>T('hpSetup.checkItda');render();poll();return true;}
   if(c.screen&&c.access){const done=c.done;clear();done?.();return true;}
   c.stage=()=>T('hpSetup.allowBoth');render();poll();return true;
  }
  return true;
 }
 window.I18N?.onChange(()=>render());
 window.ItdaHpSetup={begin,message,close:clear,activeFor:id=>current?.id===id};
})();
