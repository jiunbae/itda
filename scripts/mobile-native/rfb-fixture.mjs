// Functional RFB/WebSocket receiver adapted from scripts/monitors-e2e.mjs.
// No remote connections, clipboard frames, animation, load, or benchmark loop.
import { createHash } from 'node:crypto';

const WIDTH = 640, HEIGHT = 480, MAX_WIRE = 2 * 1024 * 1024;
const u16 = n => { const b = Buffer.alloc(2); b.writeUInt16BE(n); return b; };
const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32BE(n >>> 0); return b; };
function frame(data, opcode = 2) {
  let header;
  if (data.length < 126) header = Buffer.from([0x80 | opcode, data.length]);
  else if (data.length < 65536) { header = Buffer.alloc(4); header[0] = 0x80 | opcode; header[1] = 126; header.writeUInt16BE(data.length, 2); }
  else { header = Buffer.alloc(10); header[0] = 0x80 | opcode; header[1] = 127; header.writeBigUInt64BE(BigInt(data.length), 2); }
  return Buffer.concat([header, data]);
}
function framebuffer() {
  // B,G,R,unused for the declared 32-bit little-endian RGB pixel format.
  // A unique magenta field lets native screenshot analysis identify the real canvas.
  const pixels = Buffer.alloc(WIDTH * HEIGHT * 4);
  for (let at = 0; at < pixels.length; at += 4) { pixels[at] = 181; pixels[at + 1] = 37; pixels[at + 2] = 201; }
  return Buffer.concat([Buffer.from([0, 0]), u16(1), u16(0), u16(0), u16(WIDTH), u16(HEIGHT), u32(0), pixels]);
}

export function acceptRfb(req, socket, head, state) {
  const key = req.headers['sec-websocket-key'];
  if (typeof key !== 'string' || req.headers.upgrade?.toLowerCase() !== 'websocket') return socket.destroy();
  const accept = createHash('sha1').update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
  socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\nSec-WebSocket-Protocol: binary\r\n\r\n');
  const peer = { pointer: [], keys: [], resizes: 0, cutTextMessages: 0, errors: [], closed: false };
  state.peers.push(peer);
  const send = bytes => socket.write(frame(bytes));
  send(Buffer.from('RFB 003.008\n'));
  let wire = Buffer.alloc(0), input = Buffer.alloc(0), phase = 0, painted = false;
  const fail = reason => { peer.errors.push(reason); socket.destroy(); };
  function consume() {
    while (!socket.destroyed) {
      if (phase === 0) {
        if (input.length < 12) return;
        if (input.subarray(0, 12).toString() !== 'RFB 003.008\n') return fail('Unsupported RFB version');
        input = input.subarray(12); phase = 1; send(Buffer.from([1, 1]));
      } else if (phase === 1) {
        if (!input.length) return;
        if (input[0] !== 1) return fail('Only fictional no-auth RFB is supported');
        input = input.subarray(1); phase = 2; send(u32(0));
      } else if (phase === 2) {
        if (!input.length) return;
        input = input.subarray(1); phase = 3;
        const name = Buffer.from('Itda native iOS QA receiver');
        send(Buffer.concat([u16(WIDTH), u16(HEIGHT), Buffer.from([32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]), u32(name.length), name]));
      } else {
        if (!input.length) return;
        const type = input[0]; let n = ({ 0: 20, 3: 10, 4: 8, 5: 6, 150: 10, 248: 9 })[type];
        if (type === 2) { if (input.length < 4) return; n = 4 + input.readUInt16BE(2) * 4; }
        if (type === 251) { if (input.length < 8) return; n = 8 + input[6] * 16; }
        if (type === 6) {
          // Do not inspect, retain, or acknowledge clipboard content.
          peer.cutTextMessages++; return fail('Clipboard traffic is outside the permitted fixture scope');
        }
        if (!n || n > MAX_WIRE) return fail('Unsupported or oversized RFB message');
        if (input.length < n) return;
        const m = input.subarray(0, n); input = input.subarray(n);
        if (type === 3 && !painted) { painted = true; send(framebuffer()); }
        if (type === 5 && peer.pointer.length < 4096) peer.pointer.push({ mask: m[1], x: m.readUInt16BE(2), y: m.readUInt16BE(4) });
        if (type === 4 && peer.keys.length < 4096) peer.keys.push({ down: !!m[1], key: m.readUInt32BE(4) });
        if (type === 251) peer.resizes++;
      }
    }
  }
  function receive(chunk) {
    if (wire.length + chunk.length > MAX_WIRE) return fail('Oversized WebSocket input');
    wire = Buffer.concat([wire, chunk]);
    while (wire.length >= 2 && !socket.destroyed) {
      const op = wire[0] & 15, final = !!(wire[0] & 128);
      if (!final || (wire[0] & 0x70) || !(wire[1] & 128)) return fail('Unsupported WebSocket frame');
      let n = wire[1] & 127, at = 2;
      if (n === 126) { if (wire.length < 4) return; n = wire.readUInt16BE(2); at = 4; }
      else if (n === 127) {
        if (wire.length < 10) return;
        const big = wire.readBigUInt64BE(2); if (big > BigInt(MAX_WIRE)) return fail('Oversized WebSocket frame');
        n = Number(big); at = 10;
      }
      if (n > MAX_WIRE || (![2, 8, 9, 10].includes(op))) return fail('Unsupported WebSocket payload');
      const start = at + 4; if (wire.length < start + n) return;
      const data = Buffer.from(wire.subarray(start, start + n));
      for (let i = 0; i < n; i++) data[i] ^= wire[at + i % 4];
      wire = wire.subarray(start + n);
      if (op === 8) return socket.end(frame(data, 8));
      if (op === 9) { socket.write(frame(data, 10)); continue; }
      if (op === 10) continue;
      if (input.length + data.length > MAX_WIRE) return fail('Oversized RFB input');
      input = Buffer.concat([input, data]); consume();
    }
  }
  socket.on('data', receive);
  socket.on('error', () => {});
  socket.on('close', () => { peer.closed = true; });
  if (head.length) receive(head);
}
