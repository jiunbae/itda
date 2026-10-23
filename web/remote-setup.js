// Native app provisioning flow. Only the confirmed opaque plan token starts a write.
(() => {
 'use strict';
 let current=null;
 window.ItdaRemoteSetup={open(options){
  const {id='',hosts,accounts,send,modal,close,toast,esc}=options;
  if(current&&(current.phase==='applying'||current.phase==='complete'||current.phase==='error'&&current.applied)) {current.options=options;if(!current.panel.isConnected)document.querySelector('#modals').append(current.panel);current.panel.querySelector('[data-close]').focus();return;}
  if(current)close('remote-setup');
  const request=Date.now().toString(36)+Math.random().toString(36).slice(2);
  const panel=modal('remote-setup',`<div class="dialog remote-setup"><div class="body"><h2>원격 기기에 Itda 설치</h2><p class="lead">SSH로 앱을 설치하고, 선택한 연결과 계정 설정을 함께 가져갑니다.</p>
   <div data-form>
    <label class="rs-label">설치할 기기<select class="input" data-target><option value="">주소 직접 입력</option>${hosts.filter(h=>h.os!=='linux').map(h=>`<option value="${esc(h.id)}">${esc(h.name||h.host)}</option>`).join('')}</select></label>
    <div class="rs-address"><label class="rs-label">SSH 주소<input class="input" data-address placeholder="Mac 이름 또는 IP 주소" autocomplete="off"></label><label class="rs-label">포트<input class="input" data-port type="number" min="1" max="65535" value="22"></label></div>
    <label class="rs-label">로그인 계정<select class="input" data-account><option value="">이 연결에 저장한 계정</option><option value="-">직접 입력 · 기본 SSH 키 사용</option>${accounts.map(a=>`<option value="${esc(a.id)}">${esc(a.name||a.user)}</option>`).join('')}</select></label>
    <div class="rs-address"><label class="rs-label">사용자 이름<input class="input" data-user autocomplete="off" placeholder="SSH 설정을 쓰면 비워도 됩니다"></label><label class="rs-label">SSH 암호<input class="input" data-password type="password" autocomplete="new-password" placeholder="저장된 암호 또는 SSH 키 사용"></label></div>
    <p class="note-line">Mac은 ‘원격 로그인’, Windows는 OpenSSH 서버가 켜져 있어야 합니다. 처음 연결할 때는 먼저 파일 열기나 SSH 화면 연결로 기기를 확인해 주세요.</p><details><summary>처음 SSH 연결을 확인하는 방법</summary><p class="note-line">파일 창에서 아래 설치 대상과 같은 SSH 주소·포트·로그인 계정을 사용해 서버 키를 확인하세요. SMB 연결은 SSH 확인이 아닙니다. 파일 연결이 LAN 주소로 전환되었다면 설치 대상도 같은 주소인지 확인하세요. 확인 후 이 창의 ‘연결 확인’을 다시 누르면 됩니다. 입력한 설치 대상과 공유 선택은 유지됩니다. 주소를 직접 입력했다면 먼저 그 기기를 연결 목록에 추가해 주세요.</p><p data-trust-target class="note-line"></p><button class="btn" type="button" data-verify>이 기기의 파일 창 열기</button></details>
    <h3>공유할 연결 <span data-count></span></h3><div class="rs-profiles">${hosts.length?hosts.map(h=>`<label><input type="checkbox" data-host="${esc(h.id)}" checked><span>${esc(h.name||h.host)}<small>${esc(h.host)}</small></span></label>`).join(''):'<p>저장된 연결이 없습니다. 앱 설치와 일반 설정 공유는 가능합니다.</p>'}</div>
    <label class="rs-check"><input type="checkbox" data-settings checked><span>모양·입력·새 연결 기본값 공유</span></label>
    <label class="rs-check"><input type="checkbox" data-passwords><span>선택한 연결의 저장된 비밀번호 공유<small>계정·화면·파일 로그인 정보를 원격 기기의 보안 저장소에 넣습니다.</small></span></label>
    <label class="rs-check"><input type="checkbox" data-keys><span>계정에 지정한 SSH 개인키도 공유<small>직접 지정한 키와 그 키의 암호만 전달합니다. 기본 ~/.ssh 폴더와 SSH 에이전트는 복사하지 않습니다.</small></span></label>
    <label class="rs-check"><input type="checkbox" data-launch checked><span>완료 후 원격 Itda 열기</span></label>
    <label class="rs-check"><input type="checkbox" data-stop><span>원격 Itda가 실행 중이면 종료 후 적용<small>그 기기의 Itda 연결 창과 진행 중인 파일 전송이 종료됩니다. 이전 버전은 직접 종료해야 할 수 있습니다.</small></span></label>
    <p class="note-line">기존 원격 프로필은 유지하고 이 기기에서 공유한 항목을 추가·갱신합니다. 화면 공유 권한, 웹 공개 설정, SSH 신뢰 목록은 복사하지 않습니다.</p>
   </div>
   <section class="rs-review" data-review hidden></section><p class="rs-stage" data-stage role="status" aria-live="polite"></p><p class="rs-error" data-error role="alert" hidden></p>
  </div><div class="foot"><button class="btn" data-close>취소</button><button class="btn" data-new hidden>새 설치</button><button class="btn" data-back hidden>선택 변경</button><button class="btn primary" data-probe>연결 확인</button><button class="btn primary" data-apply hidden>설치하고 설정 공유</button></div></div>`,{sticky:true});
  const q=s=>panel.querySelector(s),c=current={panel,request,send,toast,close,esc,phase:'form',token:null,options};
  q('[data-new]').onclick=()=>{if(c.token)send('remote_setup_discard',{token:c.token});current=null;close('remote-setup');window.ItdaRemoteSetup.open(c.options);};
  const trustTarget=()=>{
   const h=hosts.find(h=>h.id===q('[data-target]').value),f=h?.files||{};
   const address=q('[data-address]').value.trim(),port=Number(q('[data-port]').value),user=q('[data-user]').value.trim();
   const fileAddress=f.host||address,filePort=f.port||h?.ssh?.port||22;
   const fileUser=f.user||accounts.find(a=>a.id===(f.account||h?.account))?.user||h?.user||'';
   const reason=!h?'저장된 기기를 선택해야 파일 창을 열 수 있습니다.':f.method==='smb'||((!f.method||f.method==='auto')&&f.share)?'이 기기의 파일 연결은 SMB입니다. 파일 창에서 SSH 연결로 바꿔 같은 대상을 확인하세요.':fileAddress!==address||filePort!==port||fileUser!==user?'파일 연결의 SSH 주소·포트·사용자가 설치 대상과 다릅니다. 같은 대상으로 설정한 뒤 확인하세요.':'';
   q('[data-trust-target]').textContent=`확인할 SSH 대상: ${user||'(SSH 기본 사용자)'}@${address||'(주소 입력 필요)'} · 포트 ${q('[data-port]').value}${reason?' — '+reason:''}`;
   q('[data-verify]').disabled=!!reason;
   return !reason;
  };
  panel.addEventListener('input',trustTarget);panel.addEventListener('change',trustTarget);
  q('[data-verify]').onclick=()=>{const target=q('[data-target]').value;if(target&&trustTarget())send('files_window',{id:target});};
  const selected=()=>[...panel.querySelectorAll('[data-host]:checked')].map(e=>e.dataset.host);
  const count=()=>q('[data-count]').textContent=`${selected().length}개`;
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
  q('[data-back]').onclick=()=>{if(c.token)send('remote_setup_discard',{token:c.token});c.token=null;c.applied=false;c.target=null;c.phase='form';q('[data-close]').textContent='취소';q('[data-new]').hidden=true;q('[data-form]').hidden=false;q('[data-review]').hidden=true;q('[data-apply]').hidden=true;q('[data-back]').hidden=true;q('[data-probe]').hidden=false;q('[data-probe]').disabled=false;stage('');failure('');};
  q('[data-probe]').onclick=()=>{
   if(!q('[data-address]').value.trim()){failure('설치할 기기의 SSH 주소를 입력하세요.');return;}
   const port=Number(q('[data-port]').value);if(!Number.isInteger(port)||port<1||port>65535){failure('SSH 포트를 확인하세요.');return;}
   c.phase='checking';failure('');stage('SSH 연결을 확인하고 있어요…');q('[data-probe]').disabled=true;q('[data-new]').hidden=true;
   send('remote_setup_probe',{request,id:q('[data-target]').value,address:q('[data-address]').value.trim(),port,user:q('[data-user]').value.trim(),login_account:q('[data-account]').value,password:q('[data-password]').value,selection:{host_ids:selected(),settings:q('[data-settings]').checked,passwords:q('[data-passwords]').checked,private_keys:q('[data-keys]').checked,close_running:q('[data-stop]').checked,launch:q('[data-launch]').checked}});
   q('[data-password]').value='';
  };
  q('[data-apply]').onclick=()=>{if(c.phase!=='prepared'||!c.token)return;c.phase='applying';c.applied=true;q('[data-apply]').disabled=true;q('[data-back]').disabled=true;q('[data-close]').textContent='백그라운드에서 계속';stage('설치를 시작하고 있어요…');send('remote_setup_apply',{request,token:c.token});};
 },message(m){
  if(m.type!=='remote_setup')return false;
  const c=current;
  if(!c||m.request!==c.request){if(m.prepared&&m.token)window.ipc?.postMessage(JSON.stringify({cmd:'remote_setup_discard',token:m.token}));return true;}
  const {q,stage,failure}=c.ui;
  if(m.stage)stage(m.stage);
  if(m.error){if(c.target){q('[data-review]').hidden=false;q('[data-review]').innerHTML=`<h3>설치 대상: ${c.esc(c.target)}</h3>`;}const applying=c.phase==='applying';c.phase='error';failure(m.error);stage('');q('[data-probe]').disabled=false;q('[data-apply]').hidden=true;q('[data-back]').hidden=false;q('[data-back]').disabled=false;q('[data-close]').textContent='닫기';q('[data-new]').hidden=false;if(applying&&!c.panel.isConnected){c.toast(`원격 설치·설정 공유에 실패했습니다 — ${m.error} · 다시 열어 결과를 확인하세요.`,'E7BA',true);}return true;}
  if(m.prepared){c.target=`${m.user?m.user+'@':''}${m.address} · SSH ${m.port}`;c.token=m.token;c.phase='prepared';q('[data-form]').hidden=true;q('[data-review]').hidden=false;stage('아래 내용을 확인한 뒤 진행하세요.');
   const identities=m.identities||{hosts:[],accounts:[]};
   const scope=`<h3>공유할 연결·계정</h3><ul>${identities.hosts.map(h=>`<li>${c.esc(h.name)} · ${c.esc(h.address)}${h.user?` · 연결 사용자 ${c.esc(h.user)}`:''}${h.account?` · 연결 계정 ${c.esc(h.account)}`:''}${h.filesAddress?` · 파일 주소 ${c.esc(h.filesAddress)}`:''}${h.filesAccount?` · 파일 계정 ${c.esc(h.filesAccount)}`:''}${h.filesUser?` · 파일 사용자 ${c.esc(h.filesUser)}`:''}</li>`).join('')||'<li>연결 없음</li>'}</ul><ul>${identities.accounts.map(a=>`<li>${c.esc(a.name)} · ${c.esc(a.user)} · 비밀번호 ${m.passwords?(a.password?'공유':'저장된 비밀번호 없음'):'보내지 않음'} · SSH 개인키 ${m.private_keys&&a.key?'공유 (저장된 키 암호 포함)':'보내지 않음'}</li>`).join('')||'<li>저장 계정 없음</li>'}</ul><p>비밀번호·개인키는 공유를 선택하고 이 기기에 저장된 항목만 전송합니다. 위 연결의 화면·파일·고성능 로그인 비밀번호도 같은 선택을 따릅니다.</p>`;
   q('[data-review]').innerHTML=`<h3>${c.esc(m.user?m.user+'@'+m.address:m.address)} <small>SSH ${c.esc(m.port)}</small></h3><p>${m.platform==='mac'?'Mac':'Windows'} · 현재 버전 ${c.esc(m.version||'확인된 버전 없음')}${m.running?' · Itda 실행 중':''}</p><dl><dt>SSH 서버 키</dt><dd><small>${(m.fingerprints||[]).map(c.esc).join('<br>')}<br>기존 SSH 신뢰 목록의 키</small></dd><dt>연결·계정</dt><dd>${c.esc(m.hosts)}개 연결 · ${c.esc(m.accounts)}개 계정</dd><dt>일반 설정</dt><dd>${m.settings?'공유':'유지'}</dd><dt>계정·연결 비밀번호</dt><dd>${m.passwords?'포함':'보내지 않음'}</dd><dt>SSH 개인키</dt><dd>${m.private_keys?c.esc(m.key_count)+'개와 저장된 키 암호 포함':'보내지 않음'}</dd></dl><p>기존 원격 프로필을 유지하며 가져온 항목을 병합합니다. ${m.launch?'완료 후 원격 앱을 엽니다.':'설치 후 앱은 직접 열 수 있습니다.'}</p>${m.running?`<p class="rs-warning">${m.close_running?'원격 Itda를 종료한 뒤 적용합니다. 열린 연결과 전송이 종료됩니다.':'원격 Itda가 실행 중입니다. 선택 변경에서 종료 후 적용을 고르거나 원격 앱을 먼저 종료해 주세요.'}</p>`:''}`;
   q('[data-review]').insertAdjacentHTML('beforeend',scope);
   const mismatch=!m.identities||identities.hosts.length!==m.hosts||identities.accounts.length!==m.accounts;
   if(mismatch)failure('공유할 연결·계정 목록이 변경됐습니다. 이 창을 닫고 다시 열어 목록을 확인해 주세요.');
   q('[data-back]').hidden=false;q('[data-probe]').hidden=true;q('[data-apply]').hidden=false;q('[data-apply]').disabled=mismatch||m.running&&!m.close_running;
  }
  if(m.complete){c.phase='complete';q('[data-review]').hidden=false;q('[data-review]').innerHTML=`<h3>설치와 설정 공유를 마쳤어요</h3><p>${c.esc(c.target||'')}</p><p>Itda ${c.esc(m.version)} · 연결 ${c.esc(m.hosts)}개 · 계정 ${c.esc(m.accounts)}개</p><p>원격 기기의 기존 프로필도 유지됩니다. 화면 공유 권한은 그 기기에서 직접 허용해 주세요.</p>`;q('[data-form]').hidden=true;q('[data-apply]').hidden=true;q('[data-back]').hidden=true;q('[data-probe]').hidden=true;q('[data-close]').textContent='완료';q('[data-new]').hidden=false;stage('');if(m.launch_error)failure(m.launch_error);if(!c.panel.isConnected){c.toast(m.launch_error||'원격 설치와 설정 공유를 완료했습니다. 다시 열어 결과를 확인할 수 있습니다.',m.launch_error?'E7BA':'E73E');}}
  return true;
 }};
})();
