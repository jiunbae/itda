// Shared remote setup guide. The existing screen connection stays usable beneath it.
(() => {
 'use strict';
 let current,timer;
 const clear=()=>{clearTimeout(timer);current?.panel.remove();current=null;};
 function render(){
  const c=current;if(!c)return;
  c.panel.querySelector('h2').textContent=`${c.name} 고성능 모드`;
  c.panel.querySelector('[data-hp-stage]').textContent=c.stage;
  for(const key of ['screen','access']){const row=c.panel.querySelector(`[data-hp-permission="${key}"]`),done=c[key]===true;row.querySelector('.hp-result').textContent=done?'✓ 허용됨':c[key]===false?'허용 필요':'확인 중';row.classList.toggle('done',done);row.querySelector('button').disabled=!c.ready||done;}
  c.panel.querySelector('[data-hp-action="helper"]').disabled=!c.ready;
  const check=c.panel.querySelector('[data-hp-action="check"]');check.disabled=!c.ready&&!c.failed;check.textContent=c.failed?'다시 시도':'다시 확인';
  const error=c.panel.querySelector('[data-hp-error]');error.hidden=!c.error;error.textContent=c.error||'';
 }
 function poll(){clearTimeout(timer);const c=current;if(!c||!c.ready)return;if(Date.now()>c.deadline){c.stage='설정을 마치면 ‘다시 확인’을 눌러 주세요.';render();return;}timer=setTimeout(()=>{if(current===c)c.send('status');},4000);}
 function begin(options){
  clear();const panel=document.createElement('aside');panel.className='hp-setup';panel.setAttribute('aria-label','고성능 모드 설정');
  panel.innerHTML='<header><span>⚡</span><h2></h2></header><p data-hp-stage role="status" aria-live="polite"></p><p class="hp-explain">설정은 <b>원격 Mac</b>에서 진행합니다. 이 창 아래의 기존 화면 공유로 설정을 조작할 수 있어요.</p><div class="hp-permission" data-hp-permission="screen"><div><strong>1. 화면 보기</strong><span class="hp-result"></span></div><button data-hp-open="screen">원격 화면 권한 설정 열기 ↗</button></div><div class="hp-permission" data-hp-permission="access"><div><strong>2. 키보드·마우스 조작</strong><span class="hp-result"></span></div><button data-hp-open="access">원격 조작 권한 설정 열기 ↗</button></div><p class="hp-explain">Itda 스위치를 켜세요. ‘종료 후 다시 열기’가 나오면 선택하세요. 준비가 끝나면 자동으로 이어집니다.</p><p data-hp-error role="alert" hidden></p><footer><button data-hp-action="helper">원격 Mac의 도우미 열기</button><button data-hp-action="check">다시 확인</button><button data-hp-action="later">나중에</button></footer>';
  current={...options,panel,stage:'원격 Mac을 준비하고 있어요…',ready:false,screen:null,access:null,deadline:Date.now()+300000};
  panel.addEventListener('click',e=>{const c=current;if(!c)return;const permission=e.target.closest('[data-hp-open]')?.dataset.hpOpen;if(permission){c.error='';c.send('permissions',{permission});render();return;}const action=e.target.closest('[data-hp-action]')?.dataset.hpAction;if(action==='helper')c.send('permissions');if(action==='check'){c.deadline=Date.now()+300000;c.error='';const failed=c.failed;c.failed=false;c.stage=failed?'다시 준비하고 있어요…':c.stage;c.send(failed?'upgrade':'status');render();}if(action==='later'){const cancel=c.cancel;clear();cancel?.();}});
  document.body.append(panel);render();
 }
 function message(m){
  const c=current;if(!c||m.id!==c.id)return false;
  if(m.helper){c.error=m.error||'';if(!m.error)c.stage='원격 Mac에 권한 도우미를 열었어요.';render();return true;}
  if(m.error){c.error=m.error;c.failed=!m.status;c.stage=m.status?'원격 Mac의 상태를 확인하지 못했어요.':'고성능 모드를 준비하지 못했어요.';render();if(m.status)poll();return true;}
  if(m.stage){c.stage=m.stage;render();return true;}
  if(m.ready||m.status){
   c.ready=true;c.failed=false;c.error=m.helper_error||'';c.screen=!!m.screen;c.access=!!m.access;
   if(m.running===false){c.error='원격 Mac의 Itda가 응답하지 않습니다. 도우미를 열어 앱을 다시 실행한 뒤 확인해 주세요.';c.stage='원격 Mac의 Itda를 확인해 주세요.';render();poll();return true;}
   if(c.screen&&c.access){const done=c.done;clear();done?.();return true;}
   c.stage='원격 Mac에서 두 가지 권한을 허용해 주세요.';render();poll();return true;
  }
  return true;
 }
 window.ItdaHpSetup={begin,message,close:clear,activeFor:id=>current?.id===id};
})();
