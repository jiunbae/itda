import { WheelInput, macHost, macActions, macMenu, sendChord } from './interaction.js';
// A sibling window is a view of the owner's canvas. It never authenticates or opens RFB.
const esc = s => String(s ?? '').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export function screenLayout(width, height, reported = [], layout = 0) {
    const w = Math.floor(width), h = Math.floor(height);
    if (!(w > 0 && h > 0)) return [];
    const shape = {1:[2,1],2:[1,2],3:[3,1],4:[2,2]}[layout];
    if (shape) {
        const [cols,rows] = shape;
        if (w < cols || h < rows) return [];
        return Array.from({length:cols*rows},(_,i)=>{
            const col=i%cols,row=Math.floor(i/cols),x=Math.floor(col*w/cols),y=Math.floor(row*h/rows);
            return {id:i,x,y,w:Math.floor((col+1)*w/cols)-x,h:Math.floor((row+1)*h/rows)-y,manual:true};
        });
    }
    const used = new Set();
    const screens = reported.filter(r=>Number.isInteger(r.id) && !used.has(r.id) && used.add(r.id) &&
        [r.x,r.y,r.w,r.h].every(Number.isInteger) && r.x>=0 && r.y>=0 && r.w>0 && r.h>0 && r.x+r.w<=w && r.y+r.h<=h);
    return screens.length ? screens.slice(0,16) : [{id:0,x:0,y:0,w,h,unknown:true}];
}
export class MonitorController {
    constructor(env) {
        this.e=env; this.screens=[]; this.children=new Map(); this.connection=null;
        this.last=''; this.selected=null; this.openTimer=null; this.held=new Map(); this.pointers=new Map();
        window.ItdaMonitorOwner={
            state:()=>this.state(),
            closed:id=>{this.release(id);this.children.delete(id);},
            pointer:(x,y,mask,id)=>this.pointer(x,y,mask,id),
            key:(...args)=>this.key(...args),
            wheel:(x,y,held,clicks)=>this.e.S.rfb?.itdaWheel(x,y,held,clicks,true),
            option:(key,value)=>this.option(key,value),
            mode:mode=>this.mode(mode),
            windowAction:(action,id)=>this.e.post({ev:"monitor-window",action,screen:id}),
            focus:()=>window.focus(),
            close:()=>this.e.close(),
            shutdown:()=>{this.disconnect(false);const r=this.e.S.rfb;this.e.S.rfb=null;r?.disconnect();},
        };
    }
    attach(rfb) {
        this.connection=rfb; this.last=''; this.selected=null;
        rfb._itdaDisableResize=!!this.opts.monitor_layout || this.opts.monitor_mode===2;
        rfb.addEventListener('screens',()=>this.refresh());
        rfb.addEventListener('desktopresize',()=>queueMicrotask(()=>this.refresh()));
        rfb.addEventListener('connect',()=>this.refresh());
    }
    get opts() { return this.e.S.host?.opts || {}; }
    state() {
        const r=this.e.S.rfb;
        return {host:this.e.S.host,canvas:r?._display?._backbuffer,frame:r?._display?.itdaFrame || 0,
            screens:this.screens,connected:r?._rfbConnectionState==='connected',viewOnly:this.e.S.viewOnly,
            cursor:(r?._canvas?.style.cursor && r._canvas.style.cursor!=='none') ? r._canvas.style.cursor : 'default',scroll:this.e.S.macScroll};
    }
    pointer(x,y,mask,id) {
        if(this.e.S.viewOnly)return;
        if(mask&7)this.pointers.set(id,{x,y});else this.pointers.delete(id);
        this.e.S.rfb?.itdaPointer(x,y,mask);
    }
    key(key,code,down,id) {
        if(this.e.S.viewOnly)return;
        if(down===true){if(!this.held.has(id))this.held.set(id,new Map());this.held.get(id).set(code,key);}
        if(down===false)this.held.get(id)?.delete(code);
        this.e.S.rfb?.sendKey(key,code,down);
    }
    release(id) {
        for(const [code,key]of this.held.get(id)||[])this.e.S.rfb?.sendKey(key,code,false);
        this.held.delete(id);
        const p=this.pointers.get(id);if(p)this.e.S.rfb?.itdaPointer(p.x,p.y,0);
        this.pointers.delete(id);
    }
    option(key,value) {
        if (key==='view_only') { if(value) for(const id of new Set([...this.held.keys(),...this.pointers.keys()]))this.release(id); this.e.S.viewOnly=!!value; if (this.e.S.rfb) this.e.S.rfb.viewOnly=!!value; }
        this.persist({[key]:value}); this.e.render();
    }
    persist(values) {
        const h=this.e.S.host; if (!h) return;
        h.opts={...h.opts,...values};
        if(this.e.app) this.e.post({ev:'opts',opts:h.opts});
        else this.e.api('/api/edit',{cmd:'viewer_opts',id:h.id,opts:h.opts});
    }
    refresh() {
        const r=this.e.S.rfb; if(!r || r!==this.connection) return;
        this.screens=screenLayout(r._fbWidth,r._fbHeight,r._itdaScreens,this.opts.monitor_layout||0);
        const signature=JSON.stringify([this.screens,this.opts.monitor_mode,this.opts.monitor_id]);
        if(signature===this.last) return;
        this.last=signature;
        if(!this.screens.length) return;
        const preferred=this.screens.find(s=>s.id===this.opts.monitor_id) || this.screens[0];
        this.selected=this.opts.monitor_mode===1 ? null : preferred;
        r._itdaDisableResize=this.screens.length>1 || !!this.opts.monitor_layout || this.opts.monitor_mode===2;
        if(r._itdaDisableResize) r.resizeSession=false;
        this.apply(); this.e.render();
        for (const [id,child] of this.children) if(!this.screens.some(s=>s.id===id)) { try {child.ItdaMirrorStop?.();child.close();} catch{} this.children.delete(id); if(this.e.app)this.e.post({ev:"monitor-remove",screen:id}); }
        if(this.opts.monitor_mode===2 && this.screens.length>1) {
            clearTimeout(this.openTimer);
            this.openTimer=setTimeout(()=>this.openWindows(false),100);
        }
    }
    apply() {
        this.e.S.rfb?.itdaViewport(this.screens.length>1 ? this.selected : null,this.e.S.fit);
        this.e.post({ev:'monitor-state',screens:this.screens,selected:this.selected?.id??null,mode:this.opts.monitor_mode||0,full:!!this.opts.monitor_fullscreen});
    }
    select(id) {
        this.mode(0,false); this.selected=this.screens.find(s=>s.id===id)||this.screens[0];
        this.persist({monitor_mode:0,monitor_id:this.selected?.id??null}); this.last=''; this.apply(); this.e.render();
    }
    mode(mode,save=true) {
        if(mode!==2) this.closeWindows();
        if(save) this.persist({monitor_mode:mode});
        else if(this.e.S.host) this.e.S.host.opts={...this.opts,monitor_mode:mode};
        this.last=''; this.refresh();
        if(mode===2) this.openWindows(true);
    }
    closeWindows(notifyNative=true) {
        clearTimeout(this.openTimer);
        for (const [id,child] of this.children) {this.release(id);try{child.ItdaMirrorStop?.();child.close();}catch{}}
        this.children.clear();
        if(notifyNative&&this.e.app) this.e.post({ev:'monitor-close'});
    }
    disconnect(notifyNative=true) { this.closeWindows(notifyNative); this.connection=null; this.screens=[]; this.last=''; }
    openWindows(userGesture) {
        if(this.screens.length<2) { if(userGesture) this.e.toast('모니터 정보가 하나뿐입니다. 수동 화면 분할을 선택할 수 있습니다.'); return; }
        const main=this.selected||this.screens[0];
        let blocked=false;
        for(const screen of this.screens) {
            if(screen.id===main.id) continue;
            const existing=this.children.get(screen.id);
            if(existing && !existing.closed) { if(userGesture) existing.focus(); continue; }
            const url=new URL(location.href); url.searchParams.set('mirror',String(screen.id));
            if(this.e.S.host?.id) url.searchParams.set('open',this.e.S.host.id);
            url.searchParams.set('mirror_index',String(this.screens.indexOf(screen)));
            url.searchParams.set('monitor_w',String(screen.w));url.searchParams.set('monitor_h',String(screen.h));
            url.searchParams.set('monitor_full',this.opts.monitor_fullscreen?'1':'0');
            const child=window.open(url.href,`itda-monitor-${this.e.S.host.id}-${screen.id}`,'popup,width=1100,height=740');
            if(child) this.children.set(screen.id,child); else blocked=true;
        }
        if(blocked) this.e.toast('별도 창이 차단되었습니다. 모니터 메뉴의 창 열기를 다시 누르거나 팝업을 허용하세요.');
    }
    menu() {
        const n=this.screens.length;
        const unknown=this.screens.some(s=>s.unknown);
        const width=Math.max(1,...this.screens.map(s=>s.x+s.w)),height=Math.max(1,...this.screens.map(s=>s.y+s.h));
        const diagramHeight=Math.max(90,Math.min(180,height/width*420));
        return `<h3>모니터</h3><div class="monitor-layout" aria-label="원격 화면 배치" style="height:${diagramHeight}px">${this.screens.map((s,i)=>`<button data-monitor="${s.id}" style="left:${s.x/width*100}%;top:${s.y/height*100}%;width:${s.w/width*100}%;height:${s.h/height*100}%" class="${this.selected?.id===s.id?'on':''}" title="${s.w}×${s.h}">${s.manual?'영역':'모니터'} ${i+1}<small>${s.w} × ${s.h}</small></button>`).join('')}</div>
          <div class="monitor-options"><button data-monitor-mode="1" ${this.opts.monitor_mode===1?'class="on"':''}>전체 모니터 한눈에 보기</button><button data-monitor-mode="2" ${n<2?'disabled':''}>모니터마다 창 열기${this.children.size?' · 다시 표시':''}</button>
          <label><input type="checkbox" data-monitor-full ${this.opts.monitor_fullscreen?'checked':''}> 분리한 창을 로컬 모니터에 전체 화면으로 배치</label>
          <label>화면 구성 <select data-monitor-layout>${[[0,'서버가 알려준 모니터'],[1,'수동: 가로 2영역'],[2,'수동: 세로 2영역'],[3,'수동: 가로 3영역'],[4,'수동: 2 × 2영역']].map(([v,t])=>`<option value="${v}" ${v===(this.opts.monitor_layout||0)?'selected':''}>${t}</option>`).join('')}</select></label></div>
          <p class="note">${unknown?'서버가 개별 모니터 배치를 제공하지 않아 하나의 화면으로 표시합니다. 필요하면 수동 분할을 선택하세요. ':''}${this.e.S.host?.kind==='rdp'?'RDP는 원격 PC의 물리 모니터가 아닌 로그인 세션의 가상 화면을 보여줍니다. ':''}분리한 창도 같은 연결을 공유합니다. 여러 영역을 표시할 때 해상도 자동 변경은 중지됩니다.</p>`;
    }
    bindMenu(sheet) {
        sheet.querySelectorAll('[data-monitor]').forEach(b=>b.onclick=()=>{this.select(Number(b.dataset.monitor));sheet.hidden=true;});
        sheet.querySelectorAll('[data-monitor-mode]').forEach(b=>b.onclick=()=>{this.mode(Number(b.dataset.monitorMode));sheet.hidden=true;});
        sheet.querySelector('[data-monitor-full]').onchange=e=>this.persist({monitor_fullscreen:e.target.checked});
        sheet.querySelector('[data-monitor-layout]').onchange=e=>{this.closeWindows();this.persist({monitor_layout:Number(e.target.value)});this.last='';this.refresh();sheet.innerHTML=this.menu();this.bindMenu(sheet);};
    }
}
export function startMirror(env,id) {
    const owner=window.opener?.ItdaMonitorOwner;
    const status=document.querySelector('#status'), tools=document.querySelector('#tools'), holder=document.querySelector('#screen');
    document.querySelector('#view').hidden=false;
    if(!owner) {status.hidden=false;status.textContent='원래 연결 창을 찾을 수 없습니다. Itda에서 연결을 다시 여세요.';return;}
    env.S.host=owner.state().host; env.S.fit=true;
    document.title=`${env.S.host?.label||'Itda'} — 모니터`;
    const wheel=new WheelInput();
    const canvas=document.createElement('canvas'); canvas.tabIndex=0; canvas.className='monitor-canvas'; holder.append(canvas);
    let keyboard=new window.ItdaKeyboard(canvas), last=-1, geometry='', bounds=null, held=new Set(), buttons=0, point=null, stopped=false;
    const sendKey=(keysym,code,down)=>{ down?held.add([keysym,code]):held.forEach(k=>{if(k[1]===code)held.delete(k);});owner.key(keysym,code,down,id);};
    keyboard.onkeyevent=sendKey; keyboard.grab();
    function release(){wheel.reset();for(const [key,code]of held)owner.key(key,code,false,id);held.clear();if(buttons&&point)owner.pointer(point.x,point.y,0,id);buttons=0;}
    window.ItdaMirrorRelease=release;
    window.ItdaMirrorStop=()=>{stopped=true;release();keyboard.ungrab();canvas.width=canvas.height=1;env.S.rfb=null;};
    window.addEventListener('blur',release); window.addEventListener('beforeunload',release);
    env.S.rfb={sendKey:(k,c,d)=>owner.key(k,c,d,id),focus:()=>canvas.focus(),clipboardPasteFrom:()=>{},disconnect:release};
    tools.innerHTML='<button data-mirror="fit" title="맞춤 / 실제 크기">맞춤 / 1:1</button><button data-mirror="full">전체 화면</button><button data-mirror="single" title="한 창으로 돌아가기">한 창으로</button><button data-mirror="close" title="이 창 닫기">닫기</button>';
    if(macHost(env.S.host))tools.insertAdjacentHTML('afterbegin','<button data-mirror=mac-actions title="Mission Control · Exposé">Mac 화면 동작</button>');
    const sheet=document.querySelector('#sheet');
    sheet.onclick=e=>{const action=macActions.find(a=>a[0]===e.target.closest('[data-mac-action]')?.dataset.macAction);if(!action)return;if(!owner.state().viewOnly)sendChord(env.S.rfb,action[3]);sheet.hidden=true;canvas.focus();};
    tools.onclick=e=>{const a=e.target.closest('[data-mirror]')?.dataset.mirror;if(a==='mac-actions'){sheet.innerHTML=macMenu();sheet.hidden=!sheet.hidden;}if(a==='fit'){env.S.fit=!env.S.fit;last=-1;}if(a==='full'){if(env.app)owner.windowAction('full',id);else if(document.fullscreenElement)document.exitFullscreen();else document.documentElement.requestFullscreen?.();}if(a==='single')owner.mode(0);if(a==='close'){if(env.app)owner.windowAction('close',id);else window.close();}};
    function coords(e) {const r=canvas.getBoundingClientRect();return{x:bounds.x+Math.max(0,Math.min(bounds.w-1,(e.clientX-r.left)*bounds.w/r.width)),y:bounds.y+Math.max(0,Math.min(bounds.h-1,(e.clientY-r.top)*bounds.h/r.height))};}
    for(const type of ['pointerdown','pointermove','pointerup','pointercancel']) canvas.addEventListener(type,e=>{
        if(!bounds)return;e.preventDefault();if(type==='pointerdown'){canvas.focus();canvas.setPointerCapture(e.pointerId);}
        point=coords(e);
        const bit=[1,2,4][e.button]||0;
        if(type==='pointerdown')buttons|=bit;
        else if(type==='pointerup')buttons&=~bit;
        else if(type==='pointercancel')buttons=0;
        else if(e.buttons)buttons=(e.buttons&1)|((e.buttons&2)?4:0)|((e.buttons&4)?2:0);
        owner.pointer(point.x,point.y,buttons,id);
    });
    canvas.addEventListener('contextmenu',e=>e.preventDefault());
    canvas.addEventListener('wheel',e=>{
        if(!bounds)return;e.preventDefault();if(owner.state().viewOnly)return;
        const p=coords(e),state=owner.state();
        owner.wheel(p.x,p.y,buttons,wheel.clicks(e,macHost(state.host),state.scroll,canvas.clientHeight));
    },{passive:false});
    function draw(){
        if(stopped)return;
        if(window.opener?.closed){release();status.hidden=false;status.textContent='연결이 종료됐습니다';return;}
        let state;try{state=owner.state();}catch{status.hidden=false;status.textContent='연결을 확인할 수 없습니다';return;}
        env.S.viewOnly=state.viewOnly;
        bounds=state.screens.find(s=>s.id===id);
        status.hidden=!!state.connected&&!!bounds;
        if(!status.hidden)status.textContent=bounds?'다시 연결하는 중…':'이 모니터가 분리되었습니다. 원래 창에서 모니터를 선택하세요.';
        if(bounds&&state.canvas&&state.connected){
            const size=`${bounds.x},${bounds.y},${bounds.w},${bounds.h},${holder.clientWidth},${holder.clientHeight},${env.S.fit}`;
            if(last!==state.frame||geometry!==size){
                const scale=env.S.fit?Math.min(holder.clientWidth/bounds.w,holder.clientHeight/bounds.h):1;
                const cssW=Math.max(1,Math.floor(bounds.w*scale)),cssH=Math.max(1,Math.floor(bounds.h*scale));
                const ratio=Math.min(devicePixelRatio||1,2);
                const pixelW=Math.min(bounds.w,Math.round(cssW*ratio)),pixelH=Math.min(bounds.h,Math.round(cssH*ratio));
                if(canvas.width!==pixelW||canvas.height!==pixelH){canvas.width=pixelW;canvas.height=pixelH;}
                canvas.style.width=`${cssW}px`;canvas.style.height=`${cssH}px`;
                canvas.getContext('2d').drawImage(state.canvas,bounds.x,bounds.y,bounds.w,bounds.h,0,0,pixelW,pixelH);
                canvas.style.cursor=state.cursor;
                last=state.frame;geometry=size;
                document.title=`${state.host?.label||'Itda'} — ${bounds.manual?'영역':'모니터'} ${state.screens.indexOf(bounds)+1}`;
            }
        }
        requestAnimationFrame(draw);
    }
    canvas.focus();requestAnimationFrame(draw);
}
