// The connection editor against the real web/ frontend (no app): group chips, the
// high-performance block and its events, and Enter while composing Korean.
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/editor-e2e.mjs
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
  const page = await browser.newPage({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36" });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript(() => (window.ipc = { postMessage: (s) => (window.sent ??= []).push(JSON.parse(s)) }));
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  const mac = { id: "mac", name: "Mac", host: "mac.local", user: "a", os: "mac", group: "Home", groups: ["Home"], opts: {}, hp: { on: false, port: 0 }, ssh: { on: false, user: "", port: 0 } };
  const pc = { id: "pc", name: "PC", host: "pc.local", user: "a", os: "windows", group: "", groups: [], opts: {}, hp: { on: false, port: 0 }, ssh: { on: false, user: "", port: 0 } };
  const saved = () => page.evaluate(() => (window.sent || []).filter((x) => x.cmd === "save_host"));
  await page.evaluate(([a, b]) => { itda.recv({ type: "state", hosts: [a, b], settings: { onboarded: true, theme: "light", defaults: {} } }); openEdit(a); }, [mac, pc]);
  // Chips: Enter and comma add, existing spelling wins, × removes.
  // openModal focuses the first field 30 ms later; let that happen before typing elsewhere.
  const settled = () => page.waitForFunction(() => document.activeElement && document.activeElement.closest(".dialog"));
  await settled();
  const chips = () => page.locator("#group-chips [data-chip]").evaluateAll((e) => e.map((x) => x.dataset.chip));
  await page.locator("#f-group").fill("회사");
  await page.locator("#f-group").press("Enter");
  await page.locator("#f-group").pressSequentially("home,여행,");
  assert(JSON.stringify(await chips()) === JSON.stringify(["Home", "회사", "여행"]), `chips ${await chips()}`);
  await page.locator('[data-chip-x="회사"]').click();
  assert((await saved()).length === 0, "Enter in the group field saved the dialog");
  // Enter finishing Korean composition: neither a chip nor a save.
  await page.locator("#f-group").fill("집");
  await page.locator("#f-group").evaluate((e) => e.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true })));
  assert((await saved()).length === 0, "composing Enter saved the dialog");
  // HP turned on from the editor, then saved: the setup stays on.
  await page.locator("[data-tab=conn]").click();
  await page.locator("#hp-enable").click();
  assert((await page.evaluate(() => sent.at(-1))).cmd === "hp_upgrade", "켜기 did not ask for the upgrade");
  await page.evaluate((h) => { itda.recv({ type: "hosts", hosts: [{ ...h, hp: { on: true, port: 5910 } }] }); itda.recv({ type: "hp", id: h.id, ready: true, screen: true, access: true }); }, mac);
  await page.locator("[data-x=save]").click();
  const s1 = (await saved()).at(-1).host;
  assert(s1.hp.on === true && s1.hp.port === 5910, `save reverted hp: ${JSON.stringify(s1.hp)}`);
  assert(JSON.stringify(s1.groups) === JSON.stringify(["Home", "여행", "집"]) && s1.group === "Home, 여행, 집", `groups ${JSON.stringify(s1.groups)} ${s1.group}`);
  // Another connection's HP result leaves this editor alone.
  await page.evaluate((h) => openEdit(h), pc);
  await settled();
  await page.locator("[data-tab=conn]").click();
  assert(await page.locator(".hp-box").isHidden(), "HP block shown for a Windows connection");
  await page.evaluate(() => itda.recv({ type: "hp", id: "mac", ready: true, screen: true, access: true }));
  assert(await page.locator(".hp-box").isHidden(), "another connection's HP event changed this editor");
  // Switching the kind to Mac shows it.
  await page.locator("[data-tab=general]").click();
  await page.locator('.kind-seg [data-kind="mac"]').click();
  await page.locator("[data-tab=conn]").click();
  assert(await page.locator(".hp-box").isVisible(), "HP block missing after switching to Mac");
  assert(!errors.length, `page errors: ${errors}`);
  console.log("PASS: group chips, Enter/IME, HP state kept on save, HP events scoped, HP block follows kind");
} finally {
  await browser.close();
  server.close();
}
