// RFB has discrete wheel clicks, not macOS pixel/gesture scrolling. Keep fractional
// deltas, but send every event as one packet without timers or a delayed input queue.
export class WheelInput {
    constructor() { this.reset(); }
    reset() { this.x=0; this.y=0; this.at=0; }
    clicks(e, mac, speed='fast', height=800) {
        const now=performance.now();
        if(now-this.at>180)this.x=this.y=0;
        this.at=now;
        if(e.ctrlKey){this.x=this.y=0;return [];}
        const step=mac ? 20/({normal:1,fast:2,faster:3}[speed]||2) : 50;
        const unit=e.deltaMode===1?16:e.deltaMode===2?height:1;
        const clicks=[];
        for(const [axis,d,back,forward]of [['y',e.deltaY,8,16],['x',e.deltaX,32,64]]) {
            if(!Number.isFinite(d)||!d)continue;
            if(Math.sign(d)!==Math.sign(this[axis]))this[axis]=0;
            this[axis]+=d*unit;
            const n=Math.min(40,Math.trunc(Math.abs(this[axis])/step));
            // Discard an oversized event's excess whole clicks: no seconds-long tail.
            this[axis]=Math.sign(this[axis])*(Math.abs(this[axis])%step);
            for(let i=0;i<n;i++)clicks.push(d<0?back:forward);
        }
        return clicks;
    }
}

export const macHost = h => !!h && !h.this && h.kind!=='rdp' && ['mac','macos'].includes(h.os||'mac');
// On a Mac or iPad noVNC sends ⌘ as Alt and ⌥ as Mode_switch, the way Apple's Screen Sharing
// reads them. Itda's high-performance server takes keysyms as they are (⌘ Super, ⌥ Alt), so
// ⌘V arrived as ⌥V ('√'). Matched on the physical key, so a release undoes its own press.
const HP_MODIFIERS = {MetaLeft:[0xffe9,0xffeb], MetaRight:[0xffeb,0xffec], AltLeft:[0xff7e,0xffe9], AltRight:[0xfe03,0xffea]};
export const hpKeysym = (sym, code) => HP_MODIFIERS[code]?.[0] === sym ? HP_MODIFIERS[code][1] : sym;
// [id, label, default shortcut, its keys, macOS symbolic hotkey number]
export const macActions = [
    ['mission','Mission Control','⌃ ↑',[0xffe3,0xff52],32],
    ['expose','앱 Exposé','⌃ ↓',[0xffe3,0xff54],33],
    ['desktop','데스크탑 보기','F11',[0xffc8],36],
    ['space-left','이전 데스크탑','⌃ ←',[0xffe3,0xff51],79],
    ['space-right','다음 데스크탑','⌃ →',[0xffe3,0xff53],81],
    ['spotlight','Spotlight','⌘ Space',[0xffeb,0x20],64],
];
// Itda's high-performance server takes an action as one of its own keysyms (a range no
// keyboard sends) and uses that Mac's shortcut for it, which may not be the default one.
export const ACTION_KEYSYM = 0x11000000;
export function sendChord(rfb, keys, send=(k,down)=>rfb.sendKey(k,null,down)) {
    keys.forEach(k=>send(k,true));
    [...keys].reverse().forEach(k=>send(k,false));
}
export function sendMacAction(action, hp, send) {
    sendChord(null, hp ? [ACTION_KEYSYM + action[4]] : action[3], send);
}
export function macMenu() {
    return `<h3>Mac 화면 동작</h3><div class="keys">${macActions.map(([id,label,key])=>`<button data-mac-action="${id}">${label}<small>${key}</small></button>`).join('')}</div><p class="note">원격 Mac의 기본 키보드 단축키를 보냅니다. 원격 Mac에서 단축키를 변경하거나 끈 경우 해당 설정을 확인하세요.</p>`;
}
