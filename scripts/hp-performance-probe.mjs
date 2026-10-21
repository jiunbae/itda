// Receive-only RFB performance probe. No input, clipboard writes or permission prompts.
// Run on the receiving Mac: node scripts/hp-performance-probe.mjs jiun-mbp --port 15910
// Run on the serving Mac:  node scripts/hp-performance-probe.mjs --local --port 15910
// The server must already be running. Its password stays in memory, never argv/output.
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import {spawn, spawnSync} from 'node:child_process';
import {createCipheriv} from 'node:crypto';
import {performance} from 'node:perf_hooks';

const args = process.argv.slice(2);
const options = {port: 5910, seconds: 10, mode: 'continuous', local: false, samples: false};
let target;
for (let i = 0; i < args.length; i++) {
  const arg = args[i];
  if (arg === '--local') options.local = true;
  else if (arg === '--samples') options.samples = true;
  else if (['--port', '--seconds', '--mode', '--credential-file'].includes(arg)) {
    if (!args[i + 1]) throw Error(`Missing value for ${arg}`);
    options[arg.slice(2)] = args[++i];
  } else if (!arg.startsWith('-') && !target) target = arg;
  else throw Error(`Unknown argument: ${arg}`);
}
options.port = Number(options.port);
options.seconds = Number(options.seconds);
if ((!options.local && (!target || !/^[-a-zA-Z0-9_.@]+$/.test(target) || target.startsWith('-')))
    || !Number.isInteger(options.port) || options.port < 1 || options.port > 65535
    || !Number.isFinite(options.seconds) || options.seconds < 1 || options.seconds > 120
    || (options['credential-file'] && !options.local)
    || !['continuous', 'classic'].includes(options.mode)) {
  throw Error('Usage: node scripts/hp-performance-probe.mjs [ssh-host | --local] [--port N] [--seconds 1..120] [--mode continuous|classic] [--samples] [--credential-file PATH (local only)]');
}

const sshOptions = ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=8',
  '-o', 'StrictHostKeyChecking=yes', '-o', 'ControlMaster=no',
  '-o', 'ControlPath=none', '-o', 'IPQoS=lowdelay'];
const key = Buffer.alloc(8);
let socket, tunnel, reader, pingTimer;
const started = performance.now();
let deadline = Infinity, measuredAt;
const updates = [], rtts = [], pending = new Map();
let sequence = 0, requests = 0, continuousOffered = false, continuousEnabled = false;
let width, height, handshakeMs, firstFrameMs, failure;
function interrupt() {
  failure = 'Measurement interrupted';
  socket?.destroy(); tunnel?.kill('SIGTERM');
}
process.once('SIGINT', interrupt);
process.once('SIGTERM', interrupt);

class Reader {
  constructor(stream) {
    this.buffer = Buffer.alloc(0);
    this.ended = false;
    stream.on('data', b => { this.buffer = Buffer.concat([this.buffer, b]); this.wake?.(); });
    stream.on('error', () => { this.ended = true; this.wake?.(); });
    stream.on('close', () => { this.ended = true; this.wake?.(); });
  }
  async read(n) {
    const until = Math.min(deadline, performance.now() + 12000);
    while (this.buffer.length < n) {
      if (this.ended) throw Error('Connection closed before the measurement ended');
      if (performance.now() >= until) {
        if (performance.now() >= deadline) throw Error('measurement-ended');
        throw Error('Server response timed out');
      }
      await new Promise(resolve => {
        const timer = setTimeout(done, Math.min(100, Math.max(1, until - performance.now())));
        const self = this;
        function done() { clearTimeout(timer); self.wake = undefined; resolve(); }
        this.wake = done;
      });
    }
    const b = this.buffer.subarray(0, n);
    this.buffer = this.buffer.subarray(n);
    return b;
  }
}

function request(full = false) {
  const b = Buffer.alloc(10);
  b[0] = 3; b[1] = full ? 0 : 1;
  b.writeUInt16BE(width, 6); b.writeUInt16BE(height, 8);
  socket.write(b); requests++;
}
function fence(flags, payload) {
  const b = Buffer.alloc(9 + payload.length);
  b[0] = 248; b.writeUInt32BE(flags, 4); b[8] = payload.length;
  payload.copy(b, 9); socket.write(b);
}
function ping() {
  const p = Buffer.alloc(8);
  p.writeUInt32BE(0x50524f42); p.writeUInt32BE(++sequence, 4);
  pending.set(p.toString('hex'), performance.now()); fence(0x80000000, p);
}
function hasIdr(payload) {
  for (let i = 0; i + 4 < payload.length; i++) {
    if (payload[i] === 0 && payload[i + 1] === 0 && payload[i + 2] === 1 && (payload[i + 3] & 31) === 5) return true;
    if (payload[i] === 0 && payload[i + 1] === 0 && payload[i + 2] === 0 && payload[i + 3] === 1 && (payload[i + 4] & 31) === 5) return true;
  }
  return false;
}
function percentile(a, p) {
  if (!a.length) return null;
  a = [...a].sort((x, y) => x - y);
  return a[Math.min(a.length - 1, Math.floor(a.length * p))];
}

try {
  const credential = options.local
    ? {status: 0, stdout: fs.readFileSync(options['credential-file'] || path.join(os.homedir(), 'Library/Application Support/Itda/hp-password'))}
    : spawnSync('ssh', [...sshOptions, target, 'cat "$HOME/Library/Application Support/Itda/hp-password"'], {timeout: 12000, maxBuffer: 4096});
  if (credential.status !== 0) throw Error('Cannot read the sharing credential over trusted SSH');
  credential.stdout.copy(key, 0, 0, 8); credential.stdout.fill(0);
  for (let i = 0; i < 8; i++) {
    let n = key[i], reversed = 0;
    for (let b = 0; b < 8; b++) { reversed = (reversed << 1) | (n & 1); n >>= 1; }
    key[i] = reversed;
  }
  let port = options.port;
  if (!options.local) {
    const listener = net.createServer();
    await new Promise(r => listener.listen(0, '127.0.0.1', r));
    port = listener.address().port;
    await new Promise(r => listener.close(r));
    tunnel = spawn('ssh', [...sshOptions, '-o', 'ExitOnForwardFailure=yes', '-N', '-L',
      `127.0.0.1:${port}:127.0.0.1:${options.port}`, target], {stdio: 'ignore'});
    tunnel.on('error', () => {});
  }
  const connectUntil = performance.now() + 15000;
  while (performance.now() < connectUntil) {
    if (tunnel && (tunnel.exitCode !== null || tunnel.signalCode !== null)) throw Error('SSH forward failed');
    try {
      socket = await new Promise((resolve, reject) => {
        const s = net.connect({host: '127.0.0.1', port});
        s.once('connect', () => resolve(s)); s.once('error', reject);
      });
      break;
    } catch { await new Promise(r => setTimeout(r, 50)); }
  }
  if (!socket) throw Error('Cannot connect to the sharing server');
  socket.setNoDelay(true); reader = new Reader(socket);
  if (!(await reader.read(12)).toString().startsWith('RFB ')) throw Error('Not an RFB server');
  socket.write('RFB 003.008\n');
  const count = (await reader.read(1))[0], types = await reader.read(count);
  if (!types.includes(2)) throw Error('VNC authentication not offered');
  socket.write(Buffer.from([2]));
  const challenge = await reader.read(16), triple = Buffer.concat([key, key, key]);
  try {
    const cipher = createCipheriv('des-ede3', triple, null); cipher.setAutoPadding(false);
    socket.write(Buffer.concat([cipher.update(challenge), cipher.final()]));
  } finally { key.fill(0); triple.fill(0); }
  if ((await reader.read(4)).readUInt32BE()) throw Error('Sharing authentication rejected');
  socket.write(Buffer.from([1]));
  const init = await reader.read(24);
  width = init.readUInt16BE(); height = init.readUInt16BE(2);
  const nameLength = init.readUInt32BE(20);
  if (nameLength > 65536) throw Error('Oversized server name');
  await reader.read(nameLength); // do not print desktop/user names
  handshakeMs = performance.now() - started;
  const encodings = [50, 16, -313, -312], enc = Buffer.alloc(4 + encodings.length * 4);
  enc[0] = 2; enc.writeUInt16BE(encodings.length, 2);
  encodings.forEach((e, i) => enc.writeInt32BE(e, 4 + i * 4));
  socket.write(enc); request(true);
  measuredAt = performance.now(); deadline = measuredAt + options.seconds * 1000;
  pingTimer = setInterval(ping, 200); ping();
  while (performance.now() < deadline) {
    const type = (await reader.read(1))[0];
    if (type === 150) {
      continuousOffered = true;
      if (options.mode === 'continuous' && !continuousEnabled) {
        const b = Buffer.alloc(10); b[0] = 150; b[1] = 1;
        b.writeUInt16BE(width, 6); b.writeUInt16BE(height, 8);
        socket.write(b); continuousEnabled = true;
      }
    } else if (type === 248) {
      const h = await reader.read(8), flags = h.readUInt32BE(3);
      if (h[7] > 64) throw Error('Oversized fence');
      const payload = await reader.read(h[7]);
      if (flags & 0x80000000) fence(0, payload);
      else {
        const id = payload.toString('hex'), sent = pending.get(id);
        if (sent !== undefined) { rtts.push(performance.now() - sent); pending.delete(id); }
      }
    } else if (type === 0) {
      const h = await reader.read(3), rectangles = h.readUInt16BE(1);
      const u = {ms: 0, bytes: 0, rectangles, h264: 0, zrle: 0, idr: 0};
      for (let i = 0; i < rectangles; i++) {
        const rect = await reader.read(12), encoding = rect.readInt32BE(8);
        if (encoding !== 50 && encoding !== 16) throw Error(`Unexpected encoding ${encoding}`);
        const header = await reader.read(encoding === 50 ? 8 : 4), length = header.readUInt32BE();
        if (length > 64 * 1024 * 1024) throw Error('Oversized frame');
        const payload = await reader.read(length); u.bytes += length;
        if (encoding === 50) { u.h264++; if (hasIdr(payload)) u.idr++; }
        else u.zrle++;
      }
      u.ms = performance.now() - measuredAt;
      if (firstFrameMs === undefined && rectangles) firstFrameMs = performance.now() - started;
      updates.push(u); // one update may contain many ZRLE rectangles
      if (options.mode === 'classic' || !continuousEnabled) request();
    } else if (type === 3) {
      const h = await reader.read(7), length = h.readUInt32BE(3);
      if (length > 16 * 1024 * 1024) throw Error('Oversized clipboard message');
      await reader.read(length); // consume protocol bytes, never read/write local clipboard
    } else if (type !== 2) throw Error(`Unexpected server message ${type}`);
  }
} catch (e) {
  if (e.message !== 'measurement-ended') failure = e.message;
} finally {
  clearInterval(pingTimer); key.fill(0); socket?.destroy();
  process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
  if (tunnel) {
    tunnel.kill('SIGTERM');
    await Promise.race([new Promise(r => tunnel.exitCode !== null || tunnel.signalCode !== null ? r() : tunnel.once('exit', r)), new Promise(r => setTimeout(r, 2000))]);
  }
}
if (!failure && !updates.some(u => u.rectangles)) failure = 'No framebuffer received';
if (!failure && options.mode === 'continuous' && !continuousEnabled) failure = 'Server did not enable ContinuousUpdates';
const intervals = updates.slice(1).map((u, i) => u.ms - updates[i].ms);
const summary = {status: failure ? 'failed' : 'ok', error: failure,
  transport: options.local ? 'remote-loopback' : 'ssh', mode: options.mode,
  width, height, seconds: options.seconds, handshake_ms: handshakeMs,
  first_frame_ms: firstFrameMs, first_after_init_ms: firstFrameMs === undefined ? undefined : firstFrameMs - handshakeMs,
  continuous_offered: continuousOffered, requests, updates: updates.length,
  fps: failure ? null : updates.length / options.seconds,
  h264_rectangles: updates.reduce((n, u) => n + u.h264, 0), zrle_rectangles: updates.reduce((n, u) => n + u.zrle, 0),
  idr: updates.reduce((n, u) => n + u.idr, 0), bytes: updates.reduce((n, u) => n + u.bytes, 0),
  interval_p50_ms: percentile(intervals, .5), interval_p95_ms: percentile(intervals, .95),
  rtt_p50_ms: percentile(rtts, .5), rtt_p95_ms: percentile(rtts, .95),
  ...(options.samples ? {samples: updates, rtt_samples_ms: rtts} : {})};
console.log(JSON.stringify(summary));
if (failure) process.exitCode = 1;
