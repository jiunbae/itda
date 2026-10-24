// Unstarted, loopback-only fictional gateway. Adapted from viewer-phone-e2e.mjs.
// Importing this module does not listen, launch a browser, or access any device.
// There is intentionally no runnable server CLI while pasteboard isolation is unresolved.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { randomBytes } from 'node:crypto';
import { acceptRfb, FRAMEBUFFER } from './rfb-fixture.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const HOST = { id: 'native-ios-fixture', label: 'QA 화면', name: 'QA 화면', host: 'fictional.invalid', os: 'windows', kind: 'vnc', has_pw: true, account: '',
  opts: { clipboard: false, resize: false, monitor_mode: 0, monitor_layout: 0 } };
const assets = new Map([
  ['/', 'web/viewer/index.html'], ['/index.html', 'web/viewer/index.html'],
  ...['viewer.js', 'viewer.css', 'novnc.min.js', 'monitors.js', 'interaction.js', 'manifest.webmanifest', 'sw.js'].map(n => ['/' + n, 'web/viewer/' + n]),
  ['/icon.svg', 'assets/itda.svg'], ['/icon-192.png', 'assets/itda-192.png'], ['/icon-512.png', 'assets/itda-512.png'],
  ['/hp-setup.js', 'web/hp-setup.js'], ['/hp-setup.css', 'web/hp-setup.css'],
]);
const json = (res, status, body, headers = {}) => {
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store', ...headers });
  res.end(JSON.stringify(body));
};
async function readJSON(req) {
  let size = 0; const chunks = [];
  for await (const chunk of req) { size += chunk.length; if (size > 4096) throw Error('Body too large'); chunks.push(chunk); }
  return JSON.parse(Buffer.concat(chunks).toString() || '{}');
}

export function createUnstartedFixture() {
  const controlToken = randomBytes(24).toString('hex');
  const state = { session: '', wrong: 0, lockedUntil: 0, loginStatuses: [], peers: [] };
  const sockets = new Set();
  const authed = req => state.session && (req.headers.cookie || '').split(';').some(v => v.trim() === 'qa_session=' + state.session);
  const record = status => { if (state.loginStatuses.length < 128) state.loginStatuses.push(status); return status; };
  const snapshot = () => ({ schema: 1, scope: 'Fictional gateway and RFB receiver, not Rust gateway or real VNC authentication',
    wrong: state.wrong, locked: Date.now() < state.lockedUntil, loginStatuses: [...state.loginStatuses], peers: state.peers, framebuffer: FRAMEBUFFER,
    clipboard: { emitted: false, hostOption: false, cutTextMessages: state.peers.reduce((n, p) => n + p.cutTextMessages, 0) } });
  const server = http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url, 'http://127.0.0.1');
      if (url.pathname.startsWith('/__qa/')) {
        if (req.headers['x-itda-qa'] !== controlToken) return json(res, 403, { error: 'QA control token required' });
        if (req.method === 'GET' && url.pathname === '/__qa/stats') return json(res, 200, snapshot());
        if (req.method === 'POST' && ['/__qa/reset', '/__qa/expire'].includes(url.pathname)) {
          state.session = '';
          for (const s of sockets) if (s !== req.socket) s.destroy();
          if (url.pathname === '/__qa/reset') { state.wrong = 0; state.lockedUntil = 0; state.loginStatuses = []; state.peers = []; }
          return json(res, 200, { ok: true });
        }
        return json(res, 404, { error: 'No such QA control' });
      }
      if (url.pathname === '/api/me') return json(res, authed(req) ? 200 : 401, { local: false });
      if (req.method === 'POST' && url.pathname === '/api/login') {
        if (Date.now() < state.lockedUntil) return json(res, record(429), { error: '잠시 후 다시 시도하세요 (PIN을 여러 번 틀렸습니다)' });
        if (state.lockedUntil) { state.wrong = 0; state.lockedUntil = 0; }
        const body = await readJSON(req);
        if (body.pin !== '123456') {
          state.wrong++;
          if (state.wrong >= 5) state.lockedUntil = Date.now() + 60_000;
          return json(res, record(401), { error: 'PIN이 맞지 않습니다' });
        }
        state.wrong = 0; state.session = randomBytes(24).toString('hex');
        return json(res, record(200), { ok: true }, { 'Set-Cookie': `qa_session=${state.session}; Path=/; HttpOnly; SameSite=Strict` });
      }
      if (req.method === 'POST' && url.pathname === '/api/logout') { state.session = ''; return json(res, 200, {}); }
      if (url.pathname.startsWith('/api/')) {
        if (!authed(req)) return json(res, 401, { error: '로그인이 필요합니다' });
        if (url.pathname === '/api/hosts') return json(res, 200, [HOST]);
        if (['/api/accounts', '/api/nearby'].includes(url.pathname)) return json(res, 200, []);
        return json(res, 404, { error: 'Outside the limited fixture scope' });
      }
      if (url.pathname === '/qa-context.html') {
        // Capability observation only: never invokes any Clipboard API.
        res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' });
        return res.end('<!doctype html><html lang="en"><meta name="viewport" content="width=device-width,initial-scale=1"><title>QA context</title><h1>QA context</h1><pre id="result"></pre><script>document.getElementById("result").textContent=JSON.stringify({origin:location.origin,isSecureContext:window.isSecureContext,clipboardAPIAvailable:typeof navigator.clipboard!=="undefined",note:"Capability only. Clipboard methods were not called; loopback is not representative LAN HTTP."},null,2)</script></html>');
      }
      const file = assets.get(url.pathname);
      if (!file) { res.writeHead(404); return res.end(); }
      const type = file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.svg') ? 'image/svg+xml' : file.endsWith('.png') ? 'image/png' : file.endsWith('.webmanifest') ? 'application/manifest+json' : 'text/html; charset=utf-8';
      res.writeHead(200, { 'Content-Type': type, 'Cache-Control': 'no-store' });
      res.end(fs.readFileSync(path.join(repo, file)));
    } catch { if (!res.headersSent) json(res, 400, { error: 'Invalid fixture request' }); else res.destroy(); }
  });
  server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
  server.on('upgrade', (req, socket, head) => {
    if (new URL(req.url, 'http://127.0.0.1').pathname !== '/ws/' + HOST.id || !authed(req)) return socket.destroy();
    acceptRfb(req, socket, head, state);
  });
  // Even a future caller cannot accidentally expose this fixture on the LAN.
  const listen = server.listen.bind(server);
  server.listen = (port, host, callback) => {
    if (host !== '127.0.0.1') throw Error('Fixture may only bind explicit 127.0.0.1');
    return listen(port, host, callback);
  };
  return { server, controlToken, snapshot };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  console.error('BLOCKED: no runtime launcher. Pasteboard isolation and native QA prerequisites are unresolved. Use build-only.sh for compilation only.');
  process.exitCode = 64;
}
