// The viewer's ITDA_HP_TRACE timing (web/viewer/viewer.js, viewerTrace) in a real browser: a
// real H.264 key frame through the patched VideoDecoder, a drawn frame, and the 5 s report
// with decode/draw/nextRaf, two decoders paired by their own frames, and flips without a new
// frame not counted. Off without trace=1: VideoDecoder is left alone.
// The real noVNC render path is covered by h264_e2e (ENGINE=webkit scripts/h264-e2e.mjs).
//   PLAYWRIGHT=/path/to/playwright/index.mjs ENGINE=webkit [WEBKIT=/path/to/pw_run.sh] node scripts/viewer-trace-e2e.mjs
// (ENGINE=chromium with CHROMIUM=…: Chromium's own builds have no H.264 and skip.)
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "web", "viewer");
const pw = await import(process.env.PLAYWRIGHT ? pathToFileURL(process.env.PLAYWRIGHT).href : "playwright");
const assert = (c, m) => { if (!c) throw new Error(m); };
const viewer = fs.readFileSync(path.join(root, "viewer.js"), "utf8");
const block = viewer.slice(viewer.indexOf("const viewerTrace = (() => {"), viewer.indexOf("})();", viewer.indexOf("const viewerTrace = (() => {")) + 5);
assert(block.length > 200, "viewerTrace not found in viewer.js");
// noVNC's own H.264 probe frame and config (1920x1080 key frame).
const novnc = fs.readFileSync(path.join(root, "novnc.min.js"), "utf8");
const key = novnc.match(/Ne\.decode\("([A-Za-z0-9+/=]+)"\)/)[1];
const engine = process.env.ENGINE || "chromium";
const exe = engine === "chromium" ? process.env.CHROMIUM : process.env.WEBKIT;
const browser = await pw[engine].launch(exe ? { executablePath: exe } : {});
try {
  // (WebCodecs needs a secure context: a loopback page.)
  const server = http.createServer((_, res) => { res.setHeader("Content-Type", "text/html"); res.end("<!doctype html><title>t</title>"); });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  server.close();
  const run = (trace) => page.evaluate(async ([block, key, trace]) => {
    const Original = window.VideoDecoder;
    const posted = [];
    const TRACE = trace, post = (m) => posted.push(m);
    const viewerTrace = (0, eval)(`(TRACE, post) => { ${block}; return viewerTrace; }`)(TRACE, post);
    const patched = window.VideoDecoder !== Original;
    // A display like noVNC's: frames go to a back buffer, a flip copies it to the screen canvas.
    const mk = () => { const c = document.createElement('canvas'); c.width = 64; c.height = 64; return c; };
    const backbuffer = mk(), screen = mk();
    const display = { _backbuffer: backbuffer, _drawCtx: backbuffer.getContext('2d'), _targetCtx: screen.getContext('2d') };
    viewerTrace.attach({ _display: display });
    const flip = () => display._targetCtx.drawImage(display._backbuffer, 0, 0);
    let frames = 0;
    const decoder = () => new VideoDecoder({ output: (f) => { frames++; display._drawCtx.drawImage(f, 0, 0, 64, 64); f.close(); flip(); }, error: (e) => { throw e; } });
    const data = Uint8Array.from(atob(key), (c) => c.charCodeAt(0));
    const cfg = { codec: "avc1.42401f", codedWidth: 1920, codedHeight: 1080, optimizeForLatency: true };
    // Two decoders (noVNC keeps one per rectangle): each frame pairs with its own decoder's.
    const a = decoder(), b = decoder();
    a.configure(cfg); b.configure(cfg);
    a.decode(new EncodedVideoChunk({ timestamp: 10, type: "key", data }));
    b.decode(new EncodedVideoChunk({ timestamp: 20, type: "key", data }));
    await Promise.all([a.flush(), b.flush()]);
    // A flip with nothing new decoded, and a non-video draw, count as no frame.
    flip();
    display._drawCtx.fillRect(0, 0, 8, 8); flip();
    await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 50)));
    const early = { ...viewerTrace.counts };
    if (trace) {
      // The next frame after 5 s triggers the report.
      await new Promise((r) => setTimeout(r, 5100));
      a.decode(new EncodedVideoChunk({ timestamp: 30, type: "key", data }));
      await a.flush();
    }
    window.VideoDecoder = Original;
    return { patched, frames, posted, early };
  }, [block, key, trace]);
  // Builds without H.264 (Chromium's own) can't decode it, and noVNC wouldn't use it there.
  const decodes = await page.evaluate(async (key) => {
    try {
      const d = new VideoDecoder({ output: (f) => f.close(), error: () => {} });
      d.configure({ codec: "avc1.42401f", codedWidth: 1920, codedHeight: 1080, optimizeForLatency: true });
      d.decode(new EncodedVideoChunk({ timestamp: 0, type: "key", data: Uint8Array.from(atob(key), (c) => c.charCodeAt(0)) }));
      await d.flush();
      return true;
    } catch { return false; }
  }, key);
  if (!decodes) {
    console.log(`SKIP (${engine}): this browser build doesn't decode H.264`);
    process.exit(0);
  }
  const off = await run(false);
  assert(!off.patched && off.frames === 2 && off.posted.length === 0, `trace off changed something: ${JSON.stringify(off)}`);
  const on = await run(true);
  assert(on.patched && on.frames === 3, `decoder: ${JSON.stringify(on)}`);
  // Two decoded frames, each drawn and copied once; the empty flip and the plain draw added none.
  assert(on.early.decode === 2 && on.early.draw === 2 && on.early.nextRaf === 2, `counts ${JSON.stringify(on.early)}`);
  const report = on.posted.find((m) => m.ev === "trace");
  assert(report && /decode n=\d+ p50=[\d.]+/.test(report.text) && /draw n=\d+/.test(report.text) && /nextRaf n=\d+/.test(report.text), `report: ${JSON.stringify(on.posted)}`);
  console.log(`PASS (${engine}): trace off leaves VideoDecoder alone; on: ${report.text}`);
} finally {
  await browser.close();
}
