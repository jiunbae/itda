// Drives src/h264_e2e.rs: the bundled noVNC in headless Chromium (zrle, then h264).
//   cargo test --release h264_e2e -- --ignored --nocapture &   then   node scripts/cu-e2e.mjs
// Needs Playwright (PLAYWRIGHT=/path/to/node_modules/playwright; CHROMIUM=browser binary).
const pw = await import(process.env.PLAYWRIGHT ?? 'playwright');
const engines = pw.chromium ? pw : pw.default;
// ENGINE=webkit runs it in WebKit (as the Mac app's WKWebView; headless Chromium may not
// offer H.264 to noVNC). WEBKIT/CHROMIUM: browser binary.
const engine = process.env.ENGINE || 'chromium';
const exe = engine === 'chromium' ? process.env.CHROMIUM : process.env.WEBKIT;
const browser = await engines[engine].launch(exe ? { executablePath: exe } : {});
// Wait for the test server (it may still be compiling).
for (let i = 0; ; i++) {
  try {
    await fetch('http://127.0.0.1:18791/ping');
    break;
  } catch (e) {
    if (i > 600) throw e;
    await new Promise((r) => setTimeout(r, 500));
  }
}
try {
  for (const mode of ["zrle", "h264"]) {
    const page = await browser.newPage();
    page.on('console', (m) => m.type() === 'error' && console.log(`[${mode}] ${m.text()}`));
    await page.goto(`http://127.0.0.1:18791/?mode=${mode}`);
    await page.waitForFunction(() => document.title === 'done', null, { timeout: 20000 });
    await page.close();
  }
} finally {
  await browser.close();
}
