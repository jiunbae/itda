// The bundled noVNC H.264 parser keeps every slice of a frame in one chunk: Intel Macs'
// hardware encoder sends a frame in several slices, and a chunk per slice failed to decode.
import fs from 'node:fs';
const src = fs.readFileSync(new URL('../web/viewer/novnc.min.js', import.meta.url), 'utf8');
const at = src.indexOf('class{constructor(i){this._data=i,this._index=0,this.profileIdc=null');
if (at < 0) throw Error('H.264 parser not found in the bundle');
let depth = 0, end = at + 'class'.length;
for (; end < src.length; end++) { if (src[end] === '{') depth++; else if (src[end] === '}' && --depth === 0) break; }
const log = src.slice(at, end).match(/default:([\w$]+)\("Unhandled unit type/)[1];
const Parser = new Function(log, `return ${src.slice(at, end + 1)}`)(() => {});

const nal = (type, ...body) => [0, 0, 0, 1, type, ...body];
// first_mb_in_slice is the first ue(v): a leading 1 bit is 0 (a frame's first slice).
const sps = nal(0x67, 100, 0, 51, 0xac), pps = nal(0x68, 0xef);
const idr = (first) => nal(0x65, first ? 0x88 : 0x40, 1, 2), p = (first) => nal(0x41, first ? 0x9a : 0x20, 3, 4);
const frames = (bytes) => { const q = new Parser(new Uint8Array(bytes)), out = []; for (let f; (f = q.parse());) out.push([f.frame.length, f.key]); return out; };
const eq = (a, b, m) => { if (JSON.stringify(a) !== JSON.stringify(b)) throw Error(`${m}: ${JSON.stringify(a)} != ${JSON.stringify(b)}`); };

const len = (...a) => a.flat().length;
eq(frames([...sps, ...pps, ...idr(1), ...idr(0), ...idr(0), ...idr(0), ...p(1), ...p(0), ...p(0)]),
  [[len(sps, pps, idr(1), idr(0), idr(0), idr(0)), true], [len(p(1), p(0), p(0)), false]], 'multi-slice frames');
eq(frames([...sps, ...pps, ...idr(1), ...p(1), ...p(1)]),
  [[len(sps, pps, idr(1)), true], [len(p(1)), false], [len(p(1)), false]], 'single-slice frames');
console.log('PASS: H.264 frames in several slices stay one chunk; single-slice frames still split');
