// The Windows exe must start on a fresh Windows: it may import only DLLs that Windows itself
// ships, never the Visual C++ runtime (VCRUNTIME140.dll and friends come with the
// Redistributable, which a new PC or a test VM doesn't have).
//   node scripts/windows-deps-check.mjs target/release/itda.exe
import { readFileSync } from 'node:fs';

const exe = process.argv[2] || 'target/release/itda.exe';
const d = readFileSync(exe);
const u16 = (o) => d.readUInt16LE(o), u32 = (o) => d.readUInt32LE(o);
const pe = u32(0x3c);
if (d.toString('latin1', pe, pe + 4) !== 'PE\0\0') throw new Error(`${exe}: not a PE file`);
const nsec = u16(pe + 6), optSize = u16(pe + 20), opt = pe + 24;
const dirs = opt + (u16(opt) === 0x20b ? 112 : 96);
const secs = [...Array(nsec)].map((_, i) => {
  const o = opt + optSize + i * 40;
  return { vsize: u32(o + 8), va: u32(o + 12), rsize: u32(o + 16), raw: u32(o + 20) };
});
const off = (rva) => {
  const s = secs.find((s) => rva >= s.va && rva < s.va + Math.max(s.vsize, s.rsize));
  if (!s) throw new Error(`RVA ${rva.toString(16)} outside every section`);
  return rva - s.va + s.raw;
};
const cstr = (o) => d.toString('latin1', o, d.indexOf(0, o));
// Directory 1: imports (20-byte descriptors, name at +12); 13: delay imports (32 bytes, name at +4).
const names = (dir, size, at) => {
  const rva = u32(dirs + dir * 8);
  const out = [];
  if (!rva) return out;
  for (let o = off(rva); ; o += size) {
    const name = u32(o + at);
    if (!name) break;
    out.push(cstr(off(name)));
  }
  return out;
};
const all = [...names(1, 20, 12), ...names(13, 32, 4)];
const bad = all.filter((n) => /^(vcruntime|msvcp|ucrtbase|concrt|vccorlib|api-ms-win-crt-)/i.test(n));
console.log(`${exe}: ${all.length} DLLs imported`);
if (bad.length) {
  console.error(`needs the Visual C++ runtime, which a fresh Windows lacks: ${bad.join(', ')}`);
  process.exit(1);
}
console.log('no Visual C++ runtime DLLs: starts on a fresh Windows');
