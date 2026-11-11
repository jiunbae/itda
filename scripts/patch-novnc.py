"""Small, checked transformations against pinned noVNC 1.7.0; fail on source drift."""
from pathlib import Path
import sys
p=Path(sys.argv[1])/'package/core/rfb.js'
s=p.read_text()
a=s.index('        for (let i = 0; i < numberOfScreens; i += 1) {',s.index('    _handleExtendedDesktopSize()'))
b=s.index('\n        /*',a)
s=s[:a]+'''        const screens = [];
        for (let i = 0; i < numberOfScreens; i += 1) {
            const screen = { id: this._sock.rQshift32(), x: this._sock.rQshift16(),
                y: this._sock.rQshift16(), w: this._sock.rQshift16(),
                h: this._sock.rQshift16(), flags: this._sock.rQshift32() };
            screens.push(screen);
            if (i === 0) { this._screenID = screen.id; this._screenFlags = screen.flags; }
        }
        if (!(this._FBU.x === 1 && this._FBU.y !== 0)) this._itdaScreens = screens;
''' +s[b:]
needle='''            this._resize(this._FBU.width, this._FBU.height);
        }

        // Normally'''
assert needle in s
s=s.replace(needle,'''            this._resize(this._FBU.width, this._FBU.height);
            this.dispatchEvent(new CustomEvent("screens", {detail: {screens: this._itdaScreens}}));
        }

        // Normally''',1)
needle='''        this._display.resize(this._fbWidth, this._fbHeight);'''
assert s.count(needle)==1
s=s.replace(needle,needle+'''\n        this.dispatchEvent(new CustomEvent("desktopresize", {detail: {width, height}}));''',1)
# Itda's "nobody is looking" hint (pseudo-encoding 'ITDB', server.rs): every view hidden, so
# the server sends a few frames a second instead of the full rate. Re-sent with SetEncodings.
needle='''        encs.push(encodings.pseudoEncodingExtendedMouseButtons);
'''
assert s.count(needle)==1
s=s.replace(needle,needle+'''        if (this._itdaBackground) {
            encs.push(0x49544442);
        }
''',1)
needle='''    get qualityLevel() {'''
assert s.count(needle)==1
s=s.replace(needle,'''    get itdaBackground() {
        return !!this._itdaBackground;
    }
    set itdaBackground(on) {
        if (!!this._itdaBackground === !!on) {
            return;
        }
        this._itdaBackground = !!on;
        if (this._rfbConnectionState === 'connected') {
            this._sendEncodings();
        }
    }

'''+needle,1)
# The same hint for servers that don't know it (Apple Screen Sharing, other VNC servers): they
# send an update for each request, so while every view is hidden the next request waits until
# a second after the last one. Itda's servers use Continuous Updates and never get here.
needle='''                if (ret && !this._enabledContinuousUpdates) {
                    RFB.messages.fbUpdateRequest(this._sock, true, 0, 0,
                                                 this._fbWidth, this._fbHeight);
                }'''
assert s.count(needle)==1
s=s.replace(needle,'''                if (ret && !this._enabledContinuousUpdates) {
                    this._itdaRequestNext();
                }''',1)
needle='''    get itdaBackground() {'''
assert s.count(needle)==1
s=s.replace(needle,'''    _itdaRequestNext() {
        if (!this._itdaBackground) {
            this._itdaSendRequest();
            return;
        }
        if (this._itdaPacer) {
            return;
        }
        const wait = Math.max(0, (this._itdaLastRequest || 0) + 1000 - Date.now());
        this._itdaPacer = setTimeout(() => {
            this._itdaPacer = null;
            this._itdaSendRequest();
        }, wait);
    }
    _itdaSendRequest() {
        if (this._rfbConnectionState !== 'connected') {
            return;
        }
        this._itdaLastRequest = Date.now();
        RFB.messages.fbUpdateRequest(this._sock, true, 0, 0, this._fbWidth, this._fbHeight);
    }

'''+needle,1)
# Seen again: a held-back request goes out now.
needle='''        this._itdaBackground = !!on;'''
assert s.count(needle)==1
s=s.replace(needle,needle+'''
        if (!on && this._itdaPacer) {
            clearTimeout(this._itdaPacer);
            this._itdaPacer = null;
            this._itdaSendRequest();
        }''',1)
needle='''        clearTimeout(this._resizeTimeout);
        clearTimeout(this._mouseMoveTimer);'''
assert s.count(needle)==1
s=s.replace(needle,needle+'''
        clearTimeout(this._itdaPacer);
        this._itdaPacer = null;''',1)
p.write_text(s)

# A frame can arrive in several slices (Intel Macs' hardware encoder: 4 per frame); noVNC cut
# the frame after the first one, and the decoder failed on the rest ("Decoder failure").
p=Path(sys.argv[1])/'package/core/decoders/h264.js'
s=p.read_text()
needle='''            if (slice) {
                break;
            }'''
assert s.count(needle)==1
s=s.replace(needle,'''            if (slice) {
                // The rest of this frame's slices (first_mb_in_slice > 0) go in the same chunk.
                while (this._index < this._data.length) {
                    const len = this._getStartSequenceLen(this._index);
                    const type = this._data[this._index + len] & 0x1f;
                    if (len == 0 || (type !== 1 && type !== 5) || (this._data[this._index + len + 1] & 0x80)) {
                        break;
                    }
                    if (type === 5) {
                        isKey = true;
                    }
                    const next = this._indexOfNextNalUnit(this._index + len);
                    this._index = next == -1 ? this._data.length : next;
                }
                break;
            }''',1)
p.write_text(s)
