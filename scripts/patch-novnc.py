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
p.write_text(s)
