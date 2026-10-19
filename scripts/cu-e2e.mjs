// Drives src/cu_e2e.rs: the bundled noVNC in headless Chromium (classic, continuous, slow link).
//   cargo test --release cu_e2e -- --ignored --nocapture &   then   node scripts/cu-e2e.mjs
// Needs Playwright (PLAYWRIGHT=/path/to/node_modules/playwright; CHROMIUM=browser binary).
const pw = await import(process.env.PLAYWRIGHT ?? 'playwright');
const { chromium } = pw.chromium ? pw : pw.default;
const browser = await chromium.launch(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {});
// Wait for the test server (it may still be compiling).
for (let i = 0; ; i++) {
  try {
    await fetch('http://127.0.0.1:18790/ping');
    break;
  } catch (e) {
    if (i > 600) throw e;
    await new Promise((r) => setTimeout(r, 500));
  }
}
try {
  for (const mode of ['classic', 'cu', 'slow']) {
    const page = await browser.newPage();
    page.on('console', (m) => m.type() === 'error' && console.log(`[${mode}] ${m.text()}`));
    await page.goto(`http://127.0.0.1:18790/?mode=${mode}`);
    await page.waitForFunction(() => document.title === 'done', null, { timeout: 15000 });
    await page.close();
  }
} finally {
  await browser.close();
}
