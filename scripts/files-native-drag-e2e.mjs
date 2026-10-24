// Actual file-browser frontend, DOM pointer/keyboard events, and in-memory IPC.
// Proves renderer handoff/pending/cancel/terminal routing only: no native OLE,
// Explorer/Finder, system clipboard, SSH, user files, or physical input is used.
// PLAYWRIGHT=/path/to/playwright/index.mjs node scripts/files-native-drag-e2e.mjs
// BROWSER=webkit WEBKIT=/path/to/pw_run.sh runs the same fixture in WebKit.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const pw = await import(process.env.PLAYWRIGHT ? pathToFileURL(process.env.PLAYWRIGHT).href : 'playwright');
const assert = (condition, message) => { if (!condition) throw Error(message); };
const equal = (actual, expected, message) => assert(JSON.stringify(actual) === JSON.stringify(expected), `${message}: ${JSON.stringify(actual)}`);
const sockets = new Set();
const server = http.createServer((req, res) => {
  const name = new URL(req.url, 'http://localhost').pathname;
  const file = path.resolve(root, '.' + (name === '/web/' ? '/web/index.html' : name));
  if (!file.startsWith(root + path.sep) || !fs.existsSync(file) || !fs.statSync(file).isFile()) { res.writeHead(404); return res.end(); }
  res.setHeader('Content-Type', name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : name.endsWith('.svg') ? 'image/svg+xml' : name.endsWith('.png') ? 'image/png' : 'text/html');
  res.end(fs.readFileSync(file));
});
server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const base = `http://127.0.0.1:${server.address().port}`;
const engine = process.env.BROWSER || 'chromium';
const executablePath = engine === 'webkit' ? process.env.WEBKIT : process.env.CHROMIUM;
let browser;

try {
  browser = await pw[engine].launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
  for (const platform of ['windows', 'mac']) {
    const context = await browser.newContext({ viewport: { width: 1200, height: 800 }, reducedMotion: 'reduce',
      userAgent: platform === 'windows' ? 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' : 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)' });
    const page = await context.newPage(), pageErrors = [];
    page.on('pageerror', error => pageErrors.push(error.message));
    await context.addInitScript(platform => {
      // A real file-window identity, distinct from the device and page token.
      window.ITDA_FILE_HOST = 'drag-device';
      window.ITDA_FILE_WINDOW = 'qa-file-window';
      window.itdaNativeDrag = platform === 'windows';
      window.itdaNativeFiles = platform === 'mac';
      window.testPackets = [];
      window.ipc = { postMessage(raw) {
        const m = JSON.parse(raw); testPackets.push(m);
        if (m.cmd !== 'files') return; // Native requests are captured, never executed.
        const entries = [{ name: 'Folder', dir: true, size: 0, modified: 1780000000 }, { name: 'Notes.txt', dir: false, size: 100, modified: 1780000000 }];
        const reply = { ...m, type: 'files_result', ok: true };
        if (m.op === 'connect') reply.data = { path: '/fixture', entries, local_path: '/local', local_entries: [], rsync: false };
        else if (m.op === 'list') reply.data = { path: m.path, entries };
        else return;
        queueMicrotask(() => itda.recv(reply));
      } };
    }, platform);
    await page.goto(base + '/web/');
    await page.evaluate(() => {
      S.hosts = [{ id: 'drag-device', name: 'QA Device', host: 'fictional.invalid', kind: 'vnc', os: 'linux', login_user: 'qa', has_pw: true, files: { method: 'sftp', user: 'qa' } }];
      S.settings = { ...S.settings, onboarded: true, theme: 'light' };
      render(); ItdaFiles.launch(S.hosts[0]);
    });
    const layer = page.locator('.files-layer');
    const row = name => layer.locator('[data-pane="remote"] [data-index]').filter({ hasText: name }).first();
    await row('Notes.txt').waitFor();
    const packets = cmd => page.evaluate(cmd => testPackets.filter(m => m.cmd === cmd), cmd);
    const pageToken = await page.evaluate(() => ItdaFiles.windowToken);
    const bindings = await packets('files_window_context');
    assert(bindings.some(m => m.window === 'qa-file-window' && m.page === pageToken && m.id === 'drag-device'), `${platform}: source window/page was not bound`);

    // A normal interior DOM drag creates the ghost and folder highlight. Crossing
    // the edge must clear both before the host has sent any terminal response.
    async function pointerGesture() {
      const source = await row('Notes.txt').boundingBox(), folder = await row('Folder').boundingBox();
      assert(source && folder, 'Fixture rows must be visible');
      await page.mouse.move(source.x + 100, source.y + source.height / 2);
      await page.mouse.down();
      await page.mouse.move(source.x + 114, source.y + source.height / 2, { steps: 3 });
      const ghostBefore = await page.locator('.files-drag').count();
      await page.mouse.move(folder.x + 100, folder.y + folder.height / 2, { steps: 3 });
      const targetsBefore = await page.locator('.files-drop-target').count();
      await page.mouse.move(0, source.y + source.height / 2, { steps: 3 });
      // Leave the button down so a quick Escape can precede any simulated host end.
      return { ghostBefore, targetsBefore };
    }
    async function handoff(previous = null) {
      const before = (await packets('files_native_drag')).length;
      const dom = await pointerGesture();
      assert(dom.ghostBefore === 1 && dom.targetsBefore > 0, `${platform}: no interior DOM ghost/target before edge handoff`);
      const all = await packets('files_native_drag');
      assert(all.length === before + 1, `${platform}: edge must emit exactly one native handoff`);
      assert(await page.locator('.files-drag,.files-drop-target').count() === 0, `${platform}: DOM drag survived native handoff`);
      const m = all.at(-1);
      equal({ cmd: m.cmd, id: m.id, window: m.window, side: m.side, path: m.path, items: m.items },
        { cmd: 'files_native_drag', id: 'drag-device', window: pageToken, side: 'remote', path: '/fixture', items: [{ name: 'Notes.txt', dir: false }] }, `${platform}: export source payload changed`);
      if (platform === 'windows') {
        assert(m.source_window === 'qa-file-window', 'Windows handoff lost native source-window identity');
        assert(typeof m.gesture === 'string' && m.gesture.length > 0 && Number.isSafeInteger(m.sequence) && m.sequence > 0, 'Windows handoff needs an opaque nonce and positive sequence');
        if (previous) assert(m.gesture !== previous.gesture && m.sequence > previous.sequence, 'New native handoff reused an old nonce/sequence');
      } else {
        for (const key of ['source_window', 'gesture', 'sequence']) assert(!(key in m), `Mac handoff gained Windows-only ${key}`);
      }
      return m;
    }
    const terminal = (m, extra = {}) => page.evaluate(m => itda.recv(m), {
      type: 'files_native_drag_end', id: m.id, window: m.window, gesture: m.gesture, dropped: false, ...extra,
    });
    async function stillPending(message) {
      const count = (await packets('files_native_drag')).length;
      const dom = await pointerGesture();
      await page.mouse.up();
      assert(dom.ghostBefore === 0 && dom.targetsBefore === 0, `${message}: a new DOM drag started while native handoff remained pending`);
      assert((await packets('files_native_drag')).length === count, `${message}: a second native handoff was admitted while pending`);
    }

    const first = await handoff();
    if (platform === 'mac') {
      await page.mouse.up();
      await page.keyboard.press('Escape');
      assert((await packets('files_native_drag_cancel')).length === 0, 'Mac Escape gained a Windows cancellation IPC');
      await page.evaluate(m => itda.recv(m), { type: 'files_native_drag_end', id: first.id, window: first.window, dropped: false });
    } else {
      // No host terminal has run. An immediate Escape survives the DOM clear and
      // carries exactly the begin identity, even if Escape is pressed again.
      await page.keyboard.press('Escape');
      await page.keyboard.press('Escape');
      equal(await packets('files_native_drag_cancel'), [{ cmd: 'files_native_drag_cancel', source_window: first.source_window, window: first.window, gesture: first.gesture, sequence: first.sequence }], 'Quick Escape cancellation identity/count');
      await page.mouse.up();
      await stillPending('Escape before terminal');
      const ignored = '다른 드래그의 오류는 표시하지 마세요';
      await terminal(first, { gesture: first.gesture + '-wrong', error: ignored });
      await stillPending('Wrong nonce terminal');
      await terminal(first, { window: first.window + '-other-page', error: ignored });
      await stillPending('Wrong page terminal');
      assert(await layer.locator('.files-error').isHidden(), 'Mismatched terminal exposed an error');
      await terminal(first, { trace: { phase: 'early_cancel', cancelled: true } });
      assert(await layer.locator('.files-error').isHidden(), 'Normal cancelled terminal became an inline failure');
      assert(await page.locator('#toasts .toast.err').count() === 0, 'Normal cancelled terminal became a failure toast');

      const second = await handoff(first); await page.mouse.up();
      await terminal(first, { error: ignored });
      await stillPending('Stale prior gesture terminal');
      assert(await layer.locator('.files-error').isHidden(), 'Stale prior gesture exposed an error');
      const rejected = '파일 창을 다시 확인하고 다시 끌어 놓으세요';
      await terminal(second, { error: rejected, trace: { phase: 'rejected', cancelled: true } });
      assert(await layer.locator('.files-error').isVisible() && await layer.locator('.files-error').textContent() === rejected, 'Rejected terminal lost actionable inline feedback');

      const third = await handoff(second); await page.mouse.up();
      const failure = '끌어 놓기를 준비하지 못했습니다 — 복사·붙여넣기로 다시 시도하세요';
      await terminal(third, { error: failure, trace: { phase: 'failed', cancelled: false } });
      assert(await layer.locator('.files-error').isVisible() && await layer.locator('.files-error').textContent() === failure, 'Matching error lost actionable inline feedback');
      const fourth = await handoff(third); await page.mouse.up();
      await terminal(fourth, { trace: { phase: 'ole_returned', cancelled: true } });
      assert((await packets('files_native_drag')).length === 4, 'A terminal recovery duplicated or lost a native handoff');
      assert((await packets('files_native_drag_cancel')).length === 1, 'Cancel was resent after the original Escape');
    }
    assert(pageErrors.length === 0, `${platform}: ${pageErrors.join('\n')}`);
    await context.close();
  }
  console.log(`PASS (${engine} frontend fixture): Windows edge identity/ghost cleanup, quick Escape once, pending exclusion, stale/wrong terminal isolation and rejected/cancelled/error recovery; Mac native payload unchanged. No native OLE acceptance.`);
} finally {
  if (browser) await browser.close();
  for (const socket of sockets) socket.destroy();
  await new Promise(resolve => server.close(resolve));
}
