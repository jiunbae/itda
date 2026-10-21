// Synthetic RFB integration checks for measurement accuracy; no real credentials/devices.
import assert from 'node:assert/strict';
import net from 'node:net';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {createCipheriv, timingSafeEqual} from 'node:crypto';

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'itda-hp-probe-test-'));
const credential = path.join(temporary, 'credential');
const password = 'probe123';
fs.writeFileSync(credential, password, {mode: 0o600});
const probe = fileURLToPath(new URL('./hp-performance-probe.mjs', import.meta.url));
const totals = [];

async function check(mode) {
  const sockets = new Set(), errors = [];
  let interval, requests = 0, continuous = false, sent = 0;
  const server = net.createServer(s => {
    sockets.add(s); s.on('close', () => sockets.delete(s)); s.on('error', () => {});
    let buffer = Buffer.alloc(0), stage = 'version';
    const challenge = Buffer.alloc(16, 0x42);
    function take(n) { const b = buffer.subarray(0, n); buffer = buffer.subarray(n); return b; }
    function frame() {
      if (s.destroyed || mode === 'closed') return;
      const chunks = [Buffer.from([0, 0, 0, 3])];
      for (let r = 0; r < 3; r++) {
        const video = r === 0, payload = video
          ? Buffer.from([0, 0, 0, 1, sent === 0 ? 0x65 : 0x41, 0, 0])
          : Buffer.from([1, 2, 3, 4]);
        const h = Buffer.alloc(video ? 20 : 16);
        h.writeUInt16BE(32, 4); h.writeUInt16BE(16, 6);
        h.writeInt32BE(video ? 50 : 16, 8); h.writeUInt32BE(payload.length, 12);
        chunks.push(h, payload);
      }
      sent++;
      // Fragment headers deliberately; the probe must count whole updates, not rectangles.
      const b = Buffer.concat(chunks);
      s.write(b.subarray(0, 9)); s.write(b.subarray(9));
    }
    function fail(e) { errors.push(e); s.destroy(); }
    s.on('data', bytes => {
      buffer = Buffer.concat([buffer, bytes]);
      try {
        while (true) {
          if (stage === 'version') {
            if (buffer.length < 12) return;
            assert.equal(take(12).toString(), 'RFB 003.008\n');
            s.write(Buffer.from([1, 2])); stage = 'type';
          } else if (stage === 'type') {
            if (!buffer.length) return;
            assert.equal(take(1)[0], 2); s.write(challenge); stage = 'auth';
          } else if (stage === 'auth') {
            if (buffer.length < 16) return;
            const key = Buffer.from(password);
            for (let i = 0; i < 8; i++) {
              let n = key[i], v = 0;
              for (let j = 0; j < 8; j++) { v = (v << 1) | (n & 1); n >>= 1; }
              key[i] = v;
            }
            const cipher = createCipheriv('des-ede3', Buffer.concat([key, key, key]), null);
            cipher.setAutoPadding(false);
            assert(timingSafeEqual(take(16), Buffer.concat([cipher.update(challenge), cipher.final()])));
            key.fill(0); s.write(Buffer.alloc(4)); stage = 'shared';
          } else if (stage === 'shared') {
            if (!buffer.length) return;
            assert.equal(take(1)[0], 1);
            const init = Buffer.alloc(24); init.writeUInt16BE(32); init.writeUInt16BE(16, 2);
            s.write(init); stage = 'messages';
          } else {
            if (!buffer.length) return;
            const t = buffer[0];
            if (t === 2) {
              if (buffer.length < 4) return;
              const n = 4 + buffer.readUInt16BE(2) * 4;
              if (buffer.length < n) return;
              take(n); if (mode !== 'no-continuous') s.write(Buffer.from([150]));
            } else if (t === 3) {
              if (buffer.length < 10) return;
              take(10); requests++;
              if (mode === 'closed') { s.end(); return; }
              if (!continuous) setTimeout(frame, 80);
            } else if (t === 150) {
              if (buffer.length < 10) return;
              take(10); continuous = true; interval = setInterval(frame, 80);
            } else if (t === 248) {
              if (buffer.length < 9) return;
              const n = 9 + buffer[8]; if (buffer.length < n) return;
              const b = Buffer.from(take(n)); b.writeUInt32BE(0, 4); s.write(b);
            } else throw Error(`Probe sent an unexpected message/input: ${t}`);
          }
        }
      } catch (e) { fail(e); }
    });
    s.write('RFB 003.008\n');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const child = spawn(process.execPath, [probe, '--local', '--port', String(server.address().port),
      '--credential-file', credential, '--seconds', '1', '--samples',
      '--mode', mode === 'classic' ? 'classic' : 'continuous']);
    let stdout = '', stderr = '';
    child.stdout.on('data', b => stdout += b); child.stderr.on('data', b => stderr += b);
    const code = await new Promise(resolve => child.on('exit', resolve));
    assert.equal(stderr, ''); assert(!stdout.includes(password));
    const result = JSON.parse(stdout);
    if (mode === 'continuous' || mode === 'classic') {
      assert.equal(code, 0); assert.equal(result.status, 'ok'); assert(result.updates > 5);
      if (mode === 'continuous') {
        assert.equal(result.requests, 1); assert.equal(requests, 1); assert(continuous);
      } else {
        assert(requests > 5); assert.equal(continuous, false);
      }
      assert.equal(result.h264_rectangles, result.updates);
      assert.equal(result.zrle_rectangles, result.updates * 2);
      assert.equal(result.idr, 1); assert.equal(result.fps, result.updates);
      assert(result.rtt_samples_ms.length > 0);
      assert(result.samples.every(u => u.rectangles === 3 && u.bytes === 15));
    } else {
      assert.equal(code, 1); assert.equal(result.status, 'failed'); assert.equal(result.fps, null);
      assert(mode === 'closed' ? /closed/i.test(result.error) : /ContinuousUpdates/.test(result.error));
    }
    assert.equal(errors.length, 0, errors.map(String).join('\n'));
    totals.push({mode, status: 'passed', updates: result.updates});
  } finally {
    clearInterval(interval); for (const s of sockets) s.destroy();
    await new Promise(resolve => server.close(resolve));
  }
}
try {
  await check('continuous'); await check('classic'); await check('closed'); await check('no-continuous');
  console.log(JSON.stringify(totals));
} finally { fs.rmSync(temporary, {recursive: true, force: true}); }
