// Mac modifiers toward Itda's high-performance server: noVNC's Apple-convention keysyms for the
// physical keys go back to Super/Alt; every other key and code passes through unchanged.
import fs from 'node:fs';
import assert from 'node:assert/strict';
const source=fs.readFileSync(new URL('../web/viewer/interaction.js',import.meta.url));
const {hpKeysym}=await import('data:text/javascript;base64,'+source.toString('base64'));
// What the bundled noVNC sends on a Mac or iPad (Keyboard._handleKeyDown's Apple remap).
const novnc=fs.readFileSync(new URL('../web/viewer/novnc.min.js',import.meta.url),'utf8');
assert.match(novnc,/case f\.XK_Super_L:s=f\.XK_Alt_L;break;case f\.XK_Super_R:s=f\.XK_Super_L;break;case f\.XK_Alt_L:s=f\.XK_Mode_switch;break;case f\.XK_Alt_R:s=f\.XK_ISO_Level3_Shift/,
    'noVNC Apple modifier remap changed: review hpKeysym');
const Super_L=0xffeb,Super_R=0xffec,Alt_L=0xffe9,Alt_R=0xffea,Mode_switch=0xff7e,ISO_Level3_Shift=0xfe03,v=0x76;
assert.equal(hpKeysym(Alt_L,'MetaLeft'),Super_L,'⌘ left is Command');
assert.equal(hpKeysym(Super_L,'MetaRight'),Super_R,'⌘ right is Command');
assert.equal(hpKeysym(Mode_switch,'AltLeft'),Alt_L,'⌥ left is Option');
assert.equal(hpKeysym(ISO_Level3_Shift,'AltRight'),Alt_R,'⌥ right is Option');
// Windows/Linux browsers already send these as is; plain keys and toolbar chords (no code) too.
for(const [sym,code] of [[Super_L,'MetaLeft'],[Alt_L,'AltLeft'],[Alt_R,'AltRight'],[v,'KeyV'],[Super_L,null],[Alt_L,null],[Alt_L,undefined]])
    assert.equal(hpKeysym(sym,code),sym,`${sym.toString(16)} ${code}`);
console.log('hp mac modifier keysyms: ok');
