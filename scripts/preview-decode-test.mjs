// Exercise the shipped executable's early child mode, including GUI-subsystem Windows pipes.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {deflateSync} from 'node:zlib';

const exe = process.argv[2];
assert.ok(exe, 'usage: node scripts/preview-decode-test.mjs <release executable>');
function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const body = Buffer.concat([Buffer.from(type), data]);
  const out = Buffer.alloc(body.length + 8);
  out.writeUInt32BE(data.length); body.copy(out, 4); out.writeUInt32BE(crc32(body), out.length - 4);
  return out;
}
function png(w, h, alpha = false) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = alpha ? 6 : 2;
  const raw = Buffer.alloc((w * (alpha ? 4 : 3) + 1) * h);
  return Buffer.concat([Buffer.from('\x89PNG\r\n\x1a\n', 'latin1'), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}
function run(code, bytes, ok = true) {
  const p = spawnSync(exe, ['--preview-decode'], {
    input: Buffer.concat([Buffer.from([code]), bytes]), timeout: 7000, maxBuffer: 17 * 1024 * 1024,
    // Neither user configuration nor launch-time sharing flags should be touched in this mode.
    windowsHide: true,
  });
  assert.ifError(p.error);
  if (!ok) { assert.notEqual(p.status, 0); assert.equal(p.stdout.length, 0); return; }
  assert.equal(p.status, 0, p.stderr.toString());
  assert.equal(p.stdout.subarray(0, 5).toString(), 'ITDP1');
  assert.equal(p.stderr.length, 0);
  assert.ok(p.stdout.length > 6 && p.stdout.length <= 16 * 1024 * 1024 + 6);
  return {code: p.stdout[5], bytes: p.stdout.subarray(6)};
}
function jpegDimensions(b) {
  assert.equal(b.readUInt16BE(0), 0xffd8);
  let at = 2;
  while (at < b.length) {
    assert.equal(b[at++], 0xff);
    while (b[at] === 0xff) at++;
    const marker = b[at++], size = b.readUInt16BE(at);
    if (marker >= 0xc0 && marker <= 0xcf && ![0xc4, 0xc8, 0xcc].includes(marker)) return [b.readUInt16BE(at + 5), b.readUInt16BE(at + 3)];
    assert.ok(size >= 2); at += size;
  }
  assert.fail('missing JPEG dimensions');
}

const small = png(8, 4);
assert.deepEqual(run(1, small), {code: 1, bytes: small});
const opaque = run(1, png(2048, 1024));
assert.equal(opaque.code, 2); assert.deepEqual(jpegDimensions(opaque.bytes), [1600, 800]);
const transparent = run(1, png(2048, 1024, true));
assert.equal(transparent.code, 1);
assert.deepEqual([transparent.bytes.readUInt32BE(16), transparent.bytes.readUInt32BE(20)], [1600, 800]);
assert.deepEqual(run(2, opaque.bytes), opaque);
run(1, Buffer.from('malformed image'), false);
run(99, small, false);
run(1, Buffer.alloc(16 * 1024 * 1024 + 1), false);
assert.deepEqual(run(1, small), {code: 1, bytes: small});
console.log('PASS: release preview child protocol, PNG/JPEG, downscale/transparency, malformed/oversize input and recovery');
