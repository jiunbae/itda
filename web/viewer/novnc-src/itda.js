// Itda's pinned noVNC adapter. Screens and native sibling windows share one decoder.
import RFB from './package/core/rfb.js';
import Display from './package/core/display.js';
import Keyboard from './package/core/input/keyboard.js';
import { Player } from './audio.js';
window.RFB = RFB;
window.ItdaKeyboard = Keyboard;
const clip = RFB.prototype._updateClip, scale = RFB.prototype._updateScale;
const resize = RFB.prototype._requestRemoteResize;
RFB.prototype._requestRemoteResize = function () {
    if (this._itdaDisableResize || this._itdaRect || this._itdaScreens?.length > 1) return;
    return resize.call(this);
};
RFB.prototype.itdaViewport = function (rect, fit = true) {
    this._itdaRect = rect;
    this._scaleViewport = fit;
    this._updateClip(); this._updateScale();
};
RFB.prototype._updateClip = function () {
    const r = this._itdaRect;
    if (!r) return clip.call(this);
    const d = this._display;
    d.clipViewport = true;
    d.viewportChangeSize(r.w, r.h);
    d.viewportChangePos(r.x - d._viewportLoc.x, r.y - d._viewportLoc.y);
    this._setClippingViewport(false);
};
RFB.prototype._updateScale = function () {
    const r = this._itdaRect;
    if (!r) return scale.call(this);
    const size = this._screenSize();
    this._display.scale = this._scaleViewport ? Math.min(size.w / r.w, size.h / r.h) : 1;
    this._fixScrollbars();
};
RFB.prototype.itdaPointer = function (x, y, mask) {
    if (this._rfbConnectionState !== 'connected' || this._viewOnly) return;
    if (![x,y,mask].every(Number.isFinite)) return;
    x = Math.max(0, Math.min(this._fbWidth - 1, Math.round(x)));
    y = Math.max(0, Math.min(this._fbHeight - 1, Math.round(y)));
    RFB.messages.pointerEvent(this._sock, x, y, mask & 127);
};
RFB.prototype.itdaWheel = function(x,y,held,clicks,absolute=false) {
    if(this._rfbConnectionState!=='connected'||this._viewOnly||!clicks.length)return;
    if(![x,y,held].every(Number.isFinite))return;
    if(!absolute){
        this._flushMouseMoveTimer(x,y);
        x=this._display.absX(x);y=this._display.absY(y);
    }
    x=Math.max(0,Math.min(this._fbWidth-1,Math.round(x)));
    y=Math.max(0,Math.min(this._fbHeight-1,Math.round(y)));
    held&=7;
    if(!absolute)this._mouseButtonMask=held;
    for(const bit of clicks.slice(0,80)){
        if(![8,16,32,64].includes(bit))continue;
        for(const mask of [held|bit,held]){
            this._sock.sQpush8(5);this._sock.sQpush8(mask);
            this._sock.sQpush16(x);this._sock.sQpush16(y);
        }
    }
    this._sock.flush();
};

// Relative pointer (Itda's own servers only): the server announces it with QEMU's Pointer
// Motion Change (-257) carrying Itda's mark in y, and says with x whether its pointer is hidden
// (x = 0: a game wants relative movement). While the page holds a pointer lock, noVNC's own
// absolute mouse handling stands aside and the page sends movement through itdaRelative.
const POINTER_MOTION=-257, ITDA_MARK=0x4954, sendEncodings=RFB.prototype._sendEncodings;
RFB.prototype._sendEncodings=function(){
    const enc=RFB.messages.clientEncodings;
    RFB.messages.clientEncodings=(sock,encs)=>enc(sock,[...encs,POINTER_MOTION]);
    try{return sendEncodings.call(this);}finally{RFB.messages.clientEncodings=enc;}
};
const handleRect=RFB.prototype._handleRect, handleMouse=RFB.prototype._handleMouse;
RFB.prototype._handleRect=function(){
    if(this._FBU.encoding!==POINTER_MOTION)return handleRect.call(this);
    if(this._FBU.y===ITDA_MARK){
        this.itdaRelativeOk=true;
        this.dispatchEvent(new CustomEvent('pointermotion',{detail:{hidden:this._FBU.x===0}}));
    }
    return true;
};
RFB.prototype._handleMouse=function(ev){
    if(this._itdaLocked&&ev.type!=='click')return;
    return handleMouse.call(this,ev);
};
RFB.prototype.itdaRelative=function(dx,dy,mask){
    if(!this.itdaRelativeOk||this._rfbConnectionState!=='connected'||this._viewOnly)return;
    const c=v=>Math.max(-32768,Math.min(32767,Math.round(v)))&0xffff;
    this._sock.sQpush8(0xea);this._sock.sQpush8(mask&127);
    this._sock.sQpush16(c(dx));this._sock.sQpush16(c(dy));
    this._sock.flush();
};

// A relay may deliver several completed updates in a single WebSocket message.
// Yield between protocol messages so decoding that burst cannot monopolize the
// UI thread. Never discard/reorder a rectangle, CopyRect, fence or clipboard event.
const handleMessage=RFB.prototype._handleMessage, disconnect=RFB.prototype._disconnect;
RFB.prototype._handleMessage=function(){
    if(this._rfbConnectionState!=='connected')return handleMessage.call(this);
    if(this._itdaResume!=null)return;
    const start=performance.now();
    while(!this._flushing&&!this._sock.rQwait('message',1)){
        if(!this._normalMsg()||this._rfbConnectionState!=='connected')break;
        if(performance.now()-start>=8&&!this._sock.rQwait('message',1)){
            this._itdaResume=setTimeout(()=>{this._itdaResume=null;this._handleMessage();},0);
            break;
        }
    }
};
RFB.prototype._disconnect=function(){
    clearTimeout(this._itdaResume);this._itdaResume=null;
    this._itdaPlayer?.close();this._itdaPlayer=null;
    return disconnect.call(this);
};

// The remote machine's sound: QEMU Audio (pseudo-encoding -259). Listed only while the viewer
// wants sound, so other servers see the same encodings as before; the server confirms with an
// empty -259 rectangle, then gets the format (S16, stereo, 48 kHz) and enable / disable.
const ENC_AUDIO=-259;
const audioSendEncodings=RFB.prototype._sendEncodings, audioHandleRect=RFB.prototype._handleRect, normalMsg=RFB.prototype._normalMsg;
RFB.prototype._sendEncodings=function(){
    const encodings=RFB.messages.clientEncodings;
    if(this._itdaAudioOn)RFB.messages.clientEncodings=(sock,encs)=>encodings(sock,[...encs,ENC_AUDIO]);
    try{return audioSendEncodings.call(this);}finally{RFB.messages.clientEncodings=encodings;}
};
RFB.prototype._handleRect=function(){
    if(this._FBU.encoding!==ENC_AUDIO)return audioHandleRect.call(this);
    const first=!this._itdaAudioServer;
    this._itdaAudioServer=true;
    if(first&&this._itdaAudioOn)this._itdaAudioSend(true);
    this.dispatchEvent(new CustomEvent('audiosupport'));
    return true;
};
RFB.prototype._itdaAudioSend=function(on){
    const s=this._sock;
    if(on){s.sQpush8(255);s.sQpush8(1);s.sQpush16(2);s.sQpush8(3);s.sQpush8(2);s.sQpush32(48000);}
    s.sQpush8(255);s.sQpush8(1);s.sQpush16(on?0:1);
    s.flush();
};
// Server message 255/1: stream begin (1), end (0), data (2: length, samples). Taken only once
// all of it has arrived.
RFB.prototype._normalMsg=function(){
    const s=this._sock;
    if(this._FBU.rects>0||s.rQpeek8()!==255)return normalMsg.call(this);
    if(s.rQwait('QEMU message',4))return false;
    const h=s.rQpeekBytes(4,false);
    if(h[1]!==1){this._fail('Unexpected QEMU server message '+h[1]);return true;}
    const op=(h[2]<<8)|h[3];
    if(op===2){
        if(s.rQwait('QEMU audio',8))return false;
        const b=s.rQpeekBytes(8,false), len=((b[4]<<24)|(b[5]<<16)|(b[6]<<8)|b[7])>>>0;
        if(len>(4<<20)){this._fail('QEMU audio data too long');return true;}
        if(s.rQwait('QEMU audio',8+len))return false;
        s.rQskipBytes(8);
        const data=s.rQshiftBytes(len,false);
        if(this._itdaAudioOn)this._itdaPlayer?.push(data);
        return true;
    }
    s.rQskipBytes(4);
    if(op===1)this._itdaPlayer?.begin();
    return true;
};
// Turn sound on or off. The first time, call it from a click or key, or the browser keeps the
// sound waiting (itdaAudioWaiting) until the next one (itdaAudioResume).
RFB.prototype.itdaAudio=function(on){
    on=!!on;
    if(on&&!this._itdaPlayer){try{this._itdaPlayer=new Player();}catch(e){return false;}}
    if(on)this._itdaPlayer.resume();
    const was=!!this._itdaAudioOn;
    this._itdaAudioOn=on;
    if(!on){this._itdaPlayer?.close();this._itdaPlayer=null;}
    if(this._rfbConnectionState==='connected'&&on!==was){
        if(this._itdaAudioServer)this._itdaAudioSend(on);
        else if(on)this._sendEncodings(); // (answered by the confirmation in the next update)
    }
    return true;
};
RFB.prototype.itdaAudioResume=function(){this._itdaPlayer?.resume();};
RFB.prototype.itdaAudioWaiting=function(){return !!this._itdaPlayer&&!this._itdaPlayer.running;};
const flip = Display.prototype.flip;
Display.prototype.flip = function (queued) {
    const dirty = this._damageBounds.right > this._damageBounds.left && this._damageBounds.bottom > this._damageBounds.top;
    const apply = !this._renderQ.length || queued;
    const result = flip.call(this, queued);
    if (dirty && apply) this.itdaFrame = (this.itdaFrame || 0) + 1;
    return result;
};
