// The app owning these permissions also hosts this window. No separate TCC identity.
(() => {
 'use strict';
 const send=(cmd,extra={})=>window.ipc?.postMessage(JSON.stringify({cmd,...extra}));
 let timer,deadline=0,pending=false,pendingAt=0,drag,requested=null,seenRequest=0;
 const error=text=>{const el=document.querySelector('.error');el.hidden=!text;el.textContent=text||'';};
 function check(refresh=false){if((pending&&Date.now()-pendingAt<5000)||document.hidden)return;pending=true;pendingAt=Date.now();send('permission_status',{refresh});}
 function watch(){deadline=Date.now()+180000;clearInterval(timer);timer=setInterval(()=>{if(Date.now()>deadline){clearInterval(timer);document.querySelector('.progress').textContent='설정을 마치면 ‘다시 확인’을 눌러 주세요.';return;}check();},1500);}
 window.itda={recv(m){
  if(m.type!=='permissions')return;
  pending=false;
  if(m.copied){const b=document.querySelector('[data-action="copy"]');b.textContent='복사했어요';setTimeout(()=>b.textContent='앱 위치 복사',1500);}
  if(m.error){error(m.error);return;}error('');
  const ready=m.screen&&m.access,count=Number(!!m.screen)+Number(!!m.access);
  const restartKeys=['screen','access'].filter(key=>!m[key]&&m[`configured_${key}`]===true),restart=restartKeys.length>0;
  const ungranted=['screen','access'].filter(key=>!m[key]&&!restartKeys.includes(key));
  requested=m.requested||requested;
  if(m.requested_seq&&m.requested_seq!==seenRequest){seenRequest=m.requested_seq;if(!ready)watch();}
  const next=ungranted.includes(requested)?requested:ungranted[0]||restartKeys[0]||'access';
  document.querySelector('.progress').textContent=ready?'✓ 연결할 준비가 됐어요':restart?ungranted.length?'권한 변경을 확인했어요 · 남은 권한을 허용해 주세요':'설정 변경이 확인됐어요 · Itda를 다시 열어 주세요':`${count}/2 완료 · ${!m.screen?'화면 보기':'키보드·마우스 조작'}를 허용해 주세요`;
  for(const key of ['screen','access']){const row=document.querySelector(`[data-permission="${key}"]`),done=!!m[key];row.classList.toggle('done',done);row.classList.toggle('pending',restartKeys.includes(key));row.classList.toggle('next',!done&&key===next);row.querySelector('.badge').textContent=done?'✓ 허용됨':restartKeys.includes(key)?'재시작 필요':'허용 필요';row.querySelector('button').disabled=done||restartKeys.includes(key);}
  document.querySelector('.recovery').hidden=ready;
  document.querySelector('.recovery h2').textContent=restart?'변경한 권한 적용':'설정에서 이미 허용했나요?';
  document.querySelector('.recovery-message').textContent=restart?ungranted.length?'허용한 권한이 실행 중인 Itda에 아직 반영되지 않았어요. 남은 권한을 설정한 후 Itda를 다시 열어 주세요.':'권한 변경을 확인했어요. 현재 실행 중인 Itda에 반영하려면 다시 열어 주세요.':'권한을 켰어도 실행 중인 Itda에 바로 반영되지 않을 수 있어요. Itda를 다시 열어 확인해 주세요.';
  document.querySelector('.current-app').textContent=`Itda ${m.version||''} · ${m.app_path||'앱 위치를 확인할 수 없습니다'}`;
  document.querySelector('[data-action="restart"]').disabled=!m.app_path;
  const instructions=document.querySelector('.instructions');instructions.hidden=ready||!requested||(restart&&!ungranted.length);
  const activeRow=document.querySelector(`[data-permission="${next}"]`);
  if(activeRow.nextElementSibling!==instructions)activeRow.after(instructions);
  document.querySelector('#next-step').textContent=next==='screen'?'1. 화면 및 시스템 오디오 기록 설정':'2. 키보드·마우스 조작 설정';
  document.querySelector('.app-path').textContent=m.app_path||'';
  document.querySelector('.app-drag').disabled=!m.app_path;
  const close=document.querySelector('[data-action="close"]');close.textContent=ready?'완료':'나중에';
  if(ready){clearInterval(timer);deadline=0;}
 }};
 document.addEventListener('click',e=>{const request=e.target.closest('[data-request]');if(request&&!request.disabled){requested=request.dataset.request;send('permission_request',{permission:request.dataset.request});watch();return;}const action=e.target.closest('[data-action]')?.dataset.action;if(!action)return;if(action==='status'){pending=false;check(true);watch();}else send(`permission_${action}`);});
 const app=document.querySelector('.app-drag');app.addEventListener('pointerdown',e=>{if(e.button===0&&!app.disabled)drag={x:e.clientX,y:e.clientY};});document.addEventListener('pointermove',e=>{if(drag&&e.buttons===1&&Math.hypot(e.clientX-drag.x,e.clientY-drag.y)>5){drag=null;send('permission_drag');}});document.addEventListener('pointerup',()=>drag=null);document.addEventListener('pointercancel',()=>drag=null);app.addEventListener('dragstart',e=>e.preventDefault());
 document.addEventListener('visibilitychange',()=>{if(!document.hidden)check();});window.addEventListener('focus',check);window.addEventListener('beforeunload',()=>clearInterval(timer));
 check();watch();
})();
