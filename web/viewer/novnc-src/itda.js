// Itda's pinned noVNC adapter. Screens and native sibling windows share one decoder.
import RFB from './package/core/rfb.js';
import Display from './package/core/display.js';
import Keyboard from './package/core/input/keyboard.js';
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
    return disconnect.call(this);
};
const flip = Display.prototype.flip;
Display.prototype.flip = function (queued) {
    const dirty = this._damageBounds.right > this._damageBounds.left && this._damageBounds.bottom > this._damageBounds.top;
    const apply = !this._renderQ.length || queued;
    const result = flip.call(this, queued);
    if (dirty && apply) this.itdaFrame = (this.itdaFrame || 0) + 1;
    return result;
};
