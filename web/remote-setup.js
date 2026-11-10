// Native app provisioning flow. Only the confirmed opaque plan token starts a write.
(() => {
 'use strict';
 let current=null;
 window.ItdaRemoteSetup={open(options){
  const {id='',hosts,accounts,send,modal,close,toast,esc}=options;
  if(current&&(current.phase==='applying'||current.phase==='complete'||current.phase==='error'&&current.applied)) {current.options=options;if(!current.panel.isConnected)document.querySelector('#modals').append(current.panel);current.panel.querySelector('[data-close]').focus();return;}
  if(current)close('remote-setup');
  const request=Date.now().toString(36)+Math.random().toString(36).slice(2);
  const panel=modal('remote-setup',`<div class="dialog remote-setup"><div class="body"><h2>${T('remoteSetup.title')}</h2><p class="lead">${T('remoteSetup.lead')}</p>
   <div data-form>
    <label class="rs-label">${T('remoteSetup.target')}<select class="input" data-target><option value="">${T('remoteSetup.target.manual')}</option>${hosts.filter(h=>h.os!=='linux').map(h=>`<option value="${esc(h.id)}">${esc(h.name||h.host)}</option>`).join('')}</select></label>
    <div class="rs-address"><label class="rs-label">${T('remoteSetup.address')}<input class="input" data-address placeholder="${T('remoteSetup.address.ph')}" autocomplete="off"></label><label class="rs-label">${T('share.port')}<input class="input" data-port type="number" min="1" max="65535" value="22"></label></div>
    <label class="rs-label">${T('remoteSetup.account')}<select class="input" data-account><option value="">${T('remoteSetup.account.saved')}</option><option value="-">${T('remoteSetup.account.manual')}</option>${accounts.map(a=>`<option value="${esc(a.id)}">${esc(a.name||a.user)}</option>`).join('')}</select></label>
    <div class="rs-address"><label class="rs-label">${T('remoteSetup.user')}<input class="input" data-user autocomplete="off" placeholder="${T('remoteSetup.user.ph')}"></label><label class="rs-label">${T('remoteSetup.password')}<input class="input" data-password type="password" autocomplete="new-password" placeholder="${T('remoteSetup.password.ph')}"></label></div>
    <p class="note-line">${T('remoteSetup.sshNote')}</p><details><summary>${T('remoteSetup.trust.summary')}</summary><p class="note-line">${T('remoteSetup.trust.text')}</p><p data-trust-target class="note-line"></p><button class="btn" type="button" data-verify>${T('remoteSetup.trust.open')}</button></details>
    <h3>${T('remoteSetup.hosts')} <span data-count></span></h3><div class="rs-profiles">${hosts.length?hosts.map(h=>`<label><input type="checkbox" data-host="${esc(h.id)}" checked><span>${esc(h.name||h.host)}<small>${esc(h.host)}</small></span></label>`).join(''):`<p>${T('remoteSetup.hosts.none')}</p>`}</div>
    <label class="rs-check"><input type="checkbox" data-settings checked><span>${T('remoteSetup.settings')}</span></label>
    <label class="rs-check"><input type="checkbox" data-passwords><span>${T('remoteSetup.passwords')}<small>${T('remoteSetup.passwords.desc')}</small></span></label>
    <label class="rs-check"><input type="checkbox" data-keys><span>${T('remoteSetup.keys')}<small>${T('remoteSetup.keys.desc')}</small></span></label>
    <label class="rs-check"><input type="checkbox" data-launch checked><span>${T('remoteSetup.launch')}</span></label>
    <label class="rs-check"><input type="checkbox" data-stop><span>${T('remoteSetup.stop')}<small>${T('remoteSetup.stop.desc')}</small></span></label>
    <p class="note-line">${T('remoteSetup.mergeNote')}</p>
   </div>
   <section class="rs-review" data-review hidden></section><p class="rs-stage" data-stage role="status" aria-live="polite"></p><p class="rs-error" data-error role="alert" hidden></p>
  </div><div class="foot"><button class="btn" data-close>${T('common.cancel')}</button><button class="btn" data-new hidden>${T('remoteSetup.new')}</button><button class="btn" data-back hidden>${T('remoteSetup.back')}</button><button class="btn primary" data-probe>${T('remoteSetup.probe')}</button><button class="btn primary" data-apply hidden>${T('remoteSetup.apply')}</button></div></div>`,{sticky:true});
  const q=s=>panel.querySelector(s),c=current={panel,request,send,toast,close,esc,phase:'form',token:null,options};
  q('[data-new]').onclick=()=>{if(c.token)send('remote_setup_discard',{token:c.token});current=null;close('remote-setup');window.ItdaRemoteSetup.open(c.options);};
  const trustTarget=()=>{
   const h=hosts.find(h=>h.id===q('[data-target]').value),f=h?.files||{};
   const address=q('[data-address]').value.trim(),port=Number(q('[data-port]').value),user=q('[data-user]').value.trim();
   const fileAddress=f.host||address,filePort=f.port||h?.ssh?.port||22;
   const fileUser=f.user||accounts.find(a=>a.id===(f.account||h?.account))?.user||h?.user||'';
   const reason=!h?T('remoteSetup.trust.noDevice'):f.method==='smb'||((!f.method||f.method==='auto')&&f.share)?T('remoteSetup.trust.smb'):fileAddress!==address||filePort!==port||fileUser!==user?T('remoteSetup.trust.mismatch'):'';
   q('[data-trust-target]').textContent=`${T('remoteSetup.trust.target',{user:user||T('remoteSetup.trust.defaultUser'),address:address||T('remoteSetup.trust.needAddress'),port:q('[data-port]').value})}${reason?' — '+reason:''}`;
   q('[data-verify]').disabled=!!reason;
   return !reason;
  };
  panel.addEventListener('input',trustTarget);panel.addEventListener('change',trustTarget);
  q('[data-verify]').onclick=()=>{const target=q('[data-target]').value;if(target&&trustTarget())send('files_window',{id:target});};
  const selected=()=>[...panel.querySelectorAll('[data-host]:checked')].map(e=>e.dataset.host);
  const count=()=>q('[data-count]').textContent=T('remoteSetup.count',{count:selected().length});
  panel.querySelectorAll('[data-host]').forEach(e=>e.onchange=count);count();
  const userFor=()=>{const a=accounts.find(a=>a.id===q('[data-account]').value),h=hosts.find(h=>h.id===q('[data-target]').value);q('[data-user]').value=a?.user|| (q('[data-account]').value==='-'?'':h?.ssh?.user||accounts.find(a=>a.id===h?.account)?.user||h?.user||'');};
  q('[data-account]').onchange=userFor;
  q('[data-address]').oninput=()=>{q('[data-target]').value='';q('[data-account]').value='-';q('[data-password]').value='';};
  q('[data-target]').onchange=()=>{const h=hosts.find(h=>h.id===q('[data-target]').value);let address=h?.host||'';address=address.replace(/^\[([^\]]+)\](?::\d+)?$/,'$1').replace(/^([^:]+):\d+$/,'$1');q('[data-address]').value=address;q('[data-port]').value=h?.ssh?.port||22;q('[data-account]').value=h?.account||'';q('[data-password]').value='';userFor();};
  q('[data-target]').value=id;q('[data-target]').onchange();trustTarget();
  q('[data-close]').onclick=()=>close('remote-setup');
  panel.addEventListener('itda-modal-close',()=>{q('[data-password]').value='';if(c.phase==='applying'||c.phase==='complete'||c.phase==='error'&&c.applied)return;if(c.token)send('remote_setup_discard',{token:c.token});if(current===c)current=null;});
  const stage=t=>q('[data-stage]').textContent=t;
  const failure=t=>{q('[data-error]').textContent=t;q('[data-error]').hidden=!t;};
  c.ui={q,stage,failure};
  q('[data-back]').onclick=()=>{if(c.token)send('remote_setup_discard',{token:c.token});c.token=null;c.applied=false;c.target=null;c.phase='form';q('[data-close]').textContent=T('common.cancel');q('[data-new]').hidden=true;q('[data-form]').hidden=false;q('[data-review]').hidden=true;q('[data-apply]').hidden=true;q('[data-back]').hidden=true;q('[data-probe]').hidden=false;q('[data-probe]').disabled=false;stage('');failure('');};
  q('[data-probe]').onclick=()=>{
   if(!q('[data-address]').value.trim()){failure(T('remoteSetup.needAddress'));return;}
   const port=Number(q('[data-port]').value);if(!Number.isInteger(port)||port<1||port>65535){failure(T('remoteSetup.badPort'));return;}
   c.phase='checking';failure('');stage(T('remoteSetup.checking'));q('[data-probe]').disabled=true;q('[data-new]').hidden=true;
   send('remote_setup_probe',{request,id:q('[data-target]').value,address:q('[data-address]').value.trim(),port,user:q('[data-user]').value.trim(),login_account:q('[data-account]').value,password:q('[data-password]').value,selection:{host_ids:selected(),settings:q('[data-settings]').checked,passwords:q('[data-passwords]').checked,private_keys:q('[data-keys]').checked,close_running:q('[data-stop]').checked,launch:q('[data-launch]').checked}});
   q('[data-password]').value='';
  };
  q('[data-apply]').onclick=()=>{if(c.phase!=='prepared'||!c.token)return;c.phase='applying';c.applied=true;q('[data-apply]').disabled=true;q('[data-back]').disabled=true;q('[data-close]').textContent=T('remoteSetup.background');stage(T('remoteSetup.starting'));send('remote_setup_apply',{request,token:c.token});};
 },message(m){
  if(m.type!=='remote_setup')return false;
  const c=current;
  if(!c||m.request!==c.request){if(m.prepared&&m.token)window.ipc?.postMessage(JSON.stringify({cmd:'remote_setup_discard',token:m.token}));return true;}
  const {q,stage,failure}=c.ui;
  if(m.stage)stage(m.stage);
  if(m.error){if(c.target){q('[data-review]').hidden=false;q('[data-review]').innerHTML=`<h3>${T('remoteSetup.targetIs',{target:c.esc(c.target)})}</h3>`;}const applying=c.phase==='applying';c.phase='error';failure(m.error);stage('');q('[data-probe]').disabled=false;q('[data-apply]').hidden=true;q('[data-back]').hidden=false;q('[data-back]').disabled=false;q('[data-close]').textContent=T('common.close');q('[data-new]').hidden=false;if(applying&&!c.panel.isConnected){c.toast(T('remoteSetup.failedToast',{error:m.error}),'E7BA',true);}return true;}
  if(m.prepared){c.target=`${m.user?m.user+'@':''}${m.address} · SSH ${m.port}`;c.token=m.token;c.phase='prepared';q('[data-form]').hidden=true;q('[data-review]').hidden=false;stage(T('remoteSetup.review'));
   const identities=m.identities||{hosts:[],accounts:[]};
   const scope=`<h3>${T('remoteSetup.scope.title')}</h3><ul>${identities.hosts.map(h=>`<li>${c.esc(h.name)} · ${c.esc(h.address)}${h.user?` · ${T('remoteSetup.scope.user',{name:c.esc(h.user)})}`:''}${h.account?` · ${T('remoteSetup.scope.account',{name:c.esc(h.account)})}`:''}${h.filesAddress?` · ${T('remoteSetup.scope.filesAddress',{address:c.esc(h.filesAddress)})}`:''}${h.filesAccount?` · ${T('remoteSetup.scope.filesAccount',{name:c.esc(h.filesAccount)})}`:''}${h.filesUser?` · ${T('remoteSetup.scope.filesUser',{name:c.esc(h.filesUser)})}`:''}</li>`).join('')||`<li>${T('remoteSetup.scope.noHosts')}</li>`}</ul><ul>${identities.accounts.map(a=>`<li>${c.esc(a.name)} · ${c.esc(a.user)} · ${m.passwords?(a.password?T('remoteSetup.scope.pw.shared'):T('remoteSetup.scope.pw.none')):T('remoteSetup.scope.pw.notSent')} · ${m.private_keys&&a.key?T('remoteSetup.scope.key.shared'):T('remoteSetup.scope.key.notSent')}</li>`).join('')||`<li>${T('remoteSetup.scope.noAccounts')}</li>`}</ul><p>${T('remoteSetup.scope.note')}</p>`;
   q('[data-review]').innerHTML=`<h3>${c.esc(m.user?m.user+'@'+m.address:m.address)} <small>SSH ${c.esc(m.port)}</small></h3><p>${m.platform==='mac'?'Mac':'Windows'} · ${T('remoteSetup.review.version',{version:c.esc(m.version||T('remoteSetup.review.noVersion'))})}${m.running?` · ${T('remoteSetup.review.running')}`:''}</p><dl><dt>${T('remoteSetup.review.hostKey')}</dt><dd><small>${(m.fingerprints||[]).map(c.esc).join('<br>')}<br>${T('remoteSetup.review.hostKeyKnown')}</small></dd><dt>${T('remoteSetup.review.profiles')}</dt><dd>${T('remoteSetup.review.hosts',{count:c.esc(m.hosts)})} · ${T('remoteSetup.review.accounts',{count:c.esc(m.accounts)})}</dd><dt>${T('remoteSetup.review.settings')}</dt><dd>${m.settings?T('remoteSetup.review.shared'):T('remoteSetup.review.kept')}</dd><dt>${T('remoteSetup.review.passwords')}</dt><dd>${m.passwords?T('remoteSetup.review.included'):T('remoteSetup.review.notSent')}</dd><dt>${T('remoteSetup.review.keys')}</dt><dd>${m.private_keys?T('remoteSetup.review.keyCount',{count:c.esc(m.key_count)}):T('remoteSetup.review.notSent')}</dd></dl><p>${T('remoteSetup.review.merge')} ${m.launch?T('remoteSetup.review.launch'):T('remoteSetup.review.noLaunch')}</p>${m.running?`<p class="rs-warning">${m.close_running?T('remoteSetup.review.willStop'):T('remoteSetup.review.isRunning')}</p>`:''}`;
   q('[data-review]').insertAdjacentHTML('beforeend',scope);
   const mismatch=!m.identities||identities.hosts.length!==m.hosts||identities.accounts.length!==m.accounts;
   if(mismatch)failure(T('remoteSetup.mismatch'));
   q('[data-back]').hidden=false;q('[data-probe]').hidden=true;q('[data-apply]').hidden=false;q('[data-apply]').disabled=mismatch||m.running&&!m.close_running;
  }
  if(m.complete){c.phase='complete';q('[data-review]').hidden=false;q('[data-review]').innerHTML=`<h3>${T('remoteSetup.done.title')}</h3><p>${c.esc(c.target||'')}</p><p>Itda ${c.esc(m.version)} · ${T('remoteSetup.done.hosts',{count:c.esc(m.hosts)})} · ${T('remoteSetup.done.accounts',{count:c.esc(m.accounts)})}</p><p>${T('remoteSetup.done.text')}</p>`;q('[data-form]').hidden=true;q('[data-apply]').hidden=true;q('[data-back]').hidden=true;q('[data-probe]').hidden=true;q('[data-close]').textContent=T('remoteSetup.done');q('[data-new]').hidden=false;stage('');if(m.launch_error)failure(m.launch_error);if(!c.panel.isConnected){c.toast(m.launch_error||T('remoteSetup.doneToast'),m.launch_error?'E7BA':'E73E');}}
  return true;
 }};
})();
