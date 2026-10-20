// The app owning these permissions also hosts this window. No separate TCC identity.
(() => {
 'use strict';
 const send=(cmd,extra={})=>window.ipc?.postMessage(JSON.stringify({cmd,...extra}));
 let timer,deadline=0,pending=false,drag,requested=null,seenRequest=0;
 const error=text=>{const el=document.querySelector('.error');el.hidden=!text;el.textContent=text||'';};
 function check(){if(pending||document.hidden)return;pending=true;send('permission_status');}
 function watch(){deadline=Date.now()+180000;clearInterval(timer);timer=setInterval(()=>{if(Date.now()>deadline){clearInterval(timer);document.querySelector('.progress').textContent='설정을 마치면 ‘다시 확인’을 눌러 주세요.';return;}check();},1500);}
 window.itda={recv(m){
  if(m.type!=='permissions')return;
  pending=false;
  if(m.copied){const b=document.querySelector('[data-action="copy"]');b.textContent='복사했어요';setTimeout(()=>b.textContent='앱 위치 복사',1500);}
  if(m.error){error(m.error);return;}error('');
  const ready=m.screen&&m.access,count=Number(!!m.screen)+Number(!!m.access);
  requested=m.requested||requested;
  if(m.requested_seq&&m.requested_seq!==seenRequest){seenRequest=m.requested_seq;if(!ready)watch();}
  const next=requested&&!m[requested]?requested:!m.screen?'screen':'access';
  document.querySelector('.progress').textContent=ready?'✓ 연결할 준비가 됐어요':`${count}/2 완료 · ${!m.screen?'화면 보기':'키보드·마우스 조작'}를 허용해 주세요`;
  for(const key of ['screen','access']){const row=document.querySelector(`[data-permission="${key}"]`),done=!!m[key];row.classList.toggle('done',done);row.classList.toggle('next',!done&&key===next);row.querySelector('.badge').textContent=done?'✓ 허용됨':'허용 필요';row.querySelector('button').disabled=done;}
  const instructions=document.querySelector('.instructions');instructions.hidden=ready||!requested;
  const activeRow=document.querySelector(`[data-permission="${next}"]`);
  if(activeRow.nextElementSibling!==instructions)activeRow.after(instructions);
  document.querySelector('#next-step').textContent=next==='screen'?'1. 화면 및 시스템 오디오 기록 설정':'2. 키보드·마우스 조작 설정';
  document.querySelector('.app-path').textContent=m.app_path||'';
  document.querySelector('.app-drag').disabled=!m.app_path;
  const close=document.querySelector('[data-action="close"]');close.textContent=ready?'완료':'나중에';
  if(ready){clearInterval(timer);deadline=0;}
 }};
 document.addEventListener('click',e=>{const request=e.target.closest('[data-request]');if(request&&!request.disabled){requested=request.dataset.request;send('permission_request',{permission:request.dataset.request});watch();return;}const action=e.target.closest('[data-action]')?.dataset.action;if(!action)return;if(action==='status'){pending=false;check();watch();}else send(`permission_${action}`);});
 const app=document.querySelector('.app-drag');app.addEventListener('pointerdown',e=>{if(e.button===0&&!app.disabled)drag={x:e.clientX,y:e.clientY};});document.addEventListener('pointermove',e=>{if(drag&&e.buttons===1&&Math.hypot(e.clientX-drag.x,e.clientY-drag.y)>5){drag=null;send('permission_drag');}});document.addEventListener('pointerup',()=>drag=null);document.addEventListener('pointercancel',()=>drag=null);app.addEventListener('dragstart',e=>e.preventDefault());
 document.addEventListener('visibilitychange',()=>{if(!document.hidden)check();});window.addEventListener('focus',check);window.addEventListener('beforeunload',()=>clearInterval(timer));
 check();
})();
