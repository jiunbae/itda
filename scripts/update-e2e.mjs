// Self-update UI against the real web/ frontend (no app, no network): a found release shows in
// the sidebar and Settings, installing asks first and reports progress, and the automatic
// check can be turned off.
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/update-e2e.mjs
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "web");
const pw = await import(process.env.PLAYWRIGHT ? pathToFileURL(process.env.PLAYWRIGHT).href : "playwright");
const assert = (c, m) => { if (!c) throw new Error(m); };
const server = http.createServer((req, res) => {
  const p = new URL(req.url, "http://localhost").pathname;
  try {
    res.setHeader("Content-Type", p.endsWith(".js") ? "text/javascript" : p.endsWith(".css") ? "text/css" : "text/html");
    res.end(fs.readFileSync(path.join(root, p === "/" ? "index.html" : p)));
  } catch { res.writeHead(404); res.end(); }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const browser = await pw.chromium.launch(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {});
try {
  for (const ua of ["Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"]) {
    const page = await browser.newPage({ userAgent: ua });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(() => (window.ipc = { postMessage: (s) => (window.sent ??= []).push(JSON.parse(s)) }));
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    const sent = (cmd) => page.evaluate((c) => (window.sent || []).filter((x) => x.cmd === c), cmd);
    const settings = { onboarded: true, theme: "light", defaults: {}, auto_update: true };
    await page.evaluate((st) => itda.recv({ type: "state", version: "0.9.11", hosts: [], settings: st, sessions: {} }), settings);
    assert(await page.locator("#nav-update").isHidden(), "Update entry shown without a release");
    // Manual check with nothing newer.
    await page.locator('.nav-item[data-page="settings"]:not(#nav-update)').click();
    await page.locator('[data-act="update-check"]').click();
    assert((await sent("update_check")).length === 1, "Check was not requested");
    await page.evaluate(() => itda.recv({ type: "update", checked: true, available: null }));
    await page.locator(".toast").filter({ hasText: "최신 버전" }).waitFor();
    // The background check finds a release.
    await page.evaluate(() => itda.recv({ type: "update", found: true, available: { version: "0.9.12", page: "https://github.com/jiunbae/itda/releases/tag/v0.9.12" } }));
    assert(await page.locator("#nav-update").isVisible(), "Sidebar does not offer the release");
    await page.locator('[data-act="update-install"]').click();
    // Installing asks first, naming the open connections that will close.
    await page.locator('#modals [data-key="confirm"]').waitFor();
    assert((await sent("update_install")).length === 0, "Installed without asking");
    await page.locator('#modals [data-key="confirm"] [data-x="cancel"]').click();
    assert((await sent("update_install")).length === 0, "Cancel still installed");
    await page.evaluate(() => { S.sessions = { a: {}, b: {} }; });
    await page.locator('[data-act="update-install"]').click();
    assert((await page.locator('#modals [data-key="confirm"]').textContent()).includes("2개"), "Open connections not mentioned");
    await page.locator('#modals [data-key="confirm"] [data-x="ok"]').click();
    assert((await sent("update_install")).length === 1, "Install not requested");
    assert(await page.locator('[data-act="update-install"]').isDisabled(), "Install button stays active while installing");
    await page.evaluate(() => itda.recv({ type: "update", stage: "내려받는 중…" }));
    assert((await page.locator("#content").textContent()).includes("내려받는 중"), "Progress not shown");
    // A failure re-enables the button.
    await page.evaluate(() => itda.recv({ type: "update", error: "내려받은 파일이 릴리스와 다릅니다" }));
    await page.locator(".toast").filter({ hasText: "업데이트하지 못했습니다" }).waitFor();
    assert(!(await page.locator('[data-act="update-install"]').isDisabled()), "Install button stuck after an error");
    // The automatic check can be turned off.
    await page.locator('input[data-set="auto_update"]').evaluate((e) => e.click());
    const saved = (await sent("settings")).at(-1);
    assert(saved && saved.settings.auto_update === false, "Turning off the automatic check was not saved");
    // Starting at login is off until turned on.
    const login = page.locator('input[data-set="login_start"]');
    assert(!(await login.isChecked()), "Login start on by default");
    await login.evaluate((e) => e.click());
    assert((await sent("settings")).at(-1).settings.login_start === true, "Turning on login start was not saved");
    // The system's answer wins: waiting for approval offers System Settings, and an item
    // removed there shows as off.
    assert((await sent("login_item")).length >= 1, "Opening settings did not ask for the login item's state");
    await page.evaluate(() => itda.recv({ type: "login_item", state: "approval" }));
    await page.locator('[data-act="login-item-settings"]').click();
    assert((await sent("login_item_settings")).length === 1, "Login Items not opened");
    await page.evaluate(() => itda.recv({ type: "login_item", state: "off" }));
    assert(!(await page.locator('input[data-set="login_start"]').isChecked()), "Removed login item still shown on");
    assert(await page.locator('[data-act="login-item-settings"]').isHidden(), "Approval prompt left after removal");
    assert(!errors.length, errors.join("\n"));
    await page.close();
  }
  console.log("PASS: update offered in sidebar/settings, manual check, confirm before install with open connections, progress/error, auto-check and login-start toggles");
} finally { await browser.close(); server.close(); }
