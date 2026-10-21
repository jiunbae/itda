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
// ENGINE=webkit runs it in WebKit, as the Mac app's WKWebView.
const engine = process.env.ENGINE || "chromium";
const exe = engine === "chromium" ? process.env.CHROMIUM : process.env.WEBKIT;
const browser = await pw[engine].launch(exe ? { executablePath: exe } : {});
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
  // (The browser's own autofocus comes first, openModal's timer after it: wait out both.)
  const settled = async () => {
    await page.waitForFunction(() => document.activeElement && document.activeElement.closest(".dialog"));
    await page.waitForTimeout(150);
  };
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
  await page.locator("#f-lan").fill(" 192.168.32.32 ");
  await page.locator("[data-x=save]").click();
  const s1 = (await saved()).at(-1).host;
  assert(s1.lan === "192.168.32.32", `LAN address not saved: ${s1.lan}`);
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
  // A late remote-guide failure remains visible after the guide panel closes.
  await page.evaluate(() => {
    ItdaHpSetup.close();
    itda.recv({ type: "hp", id: "mac", helper: true, error: "원격 Mac에 Itda가 설치되어 있지 않습니다" });
  });
  assert(await page.getByText("Mac 권한 도우미를 열지 못했습니다 — 원격 Mac에 Itda가 설치되어 있지 않습니다", { exact: true }).isVisible(), "closed-guide error was swallowed");
  // Group suggestions (drawn by the page: WKWebView shows no <datalist>): other connections'
  // groups, filtered as you type, picked with the arrows and Enter or a click.
  await page.evaluate(() => closeModal());
  const work = { ...pc, id: "work", name: "Work", host: "work.local", group: "회사, 집", groups: ["회사", "집"] };
  await page.evaluate(([a, b, c]) => { itda.recv({ type: "hosts", hosts: [a, b, c] }); openEdit(b); }, [mac, pc, work]);
  await page.waitForTimeout(150); // (openModal's own focus timer first)
  await page.locator("#f-group").focus();
  const sugs = () => page.locator("#group-suggest [data-sug]").evaluateAll((e) => e.map((x) => x.dataset.sug));
  assert(await page.locator("#group-suggest").isVisible(), "no suggestions on focus");
  assert(JSON.stringify((await sugs()).sort()) === JSON.stringify(["Home", "집", "회사"].sort()), `suggestions ${await sugs()}`);
  await page.locator("#f-group").pressSequentially("ho");
  assert(JSON.stringify(await sugs()) === JSON.stringify(["Home"]), `filtered ${await sugs()}`);
  await page.locator("#f-group").press("Enter");
  assert(JSON.stringify(await chips()) === JSON.stringify(["Home"]), `Enter did not take the suggestion: ${await chips()}`);
  assert(!(await sugs()).includes("Home"), "a group already on the connection is still suggested");
  await page.locator("#f-group").press("ArrowDown"); // with nothing typed: first suggestion
  await page.locator("#f-group").press("Enter");
  assert((await chips()).length === 2, `arrow + Enter: ${await chips()}`);
  // The active choice is exposed to assistive tech.
  await page.locator("#f-group").press("Backspace"); // removes the last chip: suggested again
  assert(JSON.stringify((await sugs()).sort()) === JSON.stringify(["집", "회사"].sort()), `Backspace did not refresh suggestions: ${await sugs()}`);
  await page.locator("#f-group").press("ArrowDown");
  const aria = () => page.evaluate(() => { const g = document.querySelector("#f-group"); const a = g.getAttribute("aria-activedescendant"); return { expanded: g.getAttribute("aria-expanded"), active: a, selected: a && document.getElementById(a)?.getAttribute("aria-selected") }; });
  let a = await aria();
  assert(a.expanded === "true" && a.active === "group-sug-0" && a.selected === "true", `ARIA ${JSON.stringify(a)}`);
  // Keys while composing (Korean IME) are the IME's: no highlight move, and Escape neither
  // closes the list nor the dialog.
  const composing = (key) => page.locator("#f-group").evaluate((g, key) => { const e = new KeyboardEvent("keydown", { key, isComposing: true, bubbles: true, cancelable: true }); g.dispatchEvent(e); return e.defaultPrevented; }, key);
  assert(!(await composing("ArrowDown")) && !(await composing("ArrowUp")), "composing arrows taken by the list");
  assert((await aria()).active === "group-sug-0", "composing arrow moved the highlight");
  await composing("Escape");
  assert(await page.locator("#group-suggest").isVisible() && await page.locator("#modals .scrim").count() === 1, "composing Escape closed something");
  // Escape closes the list (and says so); typing brings it back.
  await page.locator("#f-group").press("Escape");
  a = await aria();
  assert(await page.locator("#group-suggest").isHidden() && a.expanded === "false" && !a.active, `Escape ${JSON.stringify(a)}`);
  assert(await page.locator("#modals .scrim").count() === 1, "Escape in the list closed the dialog");
  // A real click on a choice picks it and leaves the focus in the field.
  await page.locator("#f-group").pressSequentially("집");
  await page.locator('#group-suggest [data-sug="집"]').click();
  assert(JSON.stringify(await chips()) === JSON.stringify(["Home", "집"]), `click: ${await chips()}`);
  assert(await page.evaluate(() => document.activeElement.id) === "f-group", "click took the focus");
  await page.locator('#group-suggest [data-sug="회사"]').click();
  assert(await page.locator("#group-suggest").isHidden(), "suggestions left with nothing to suggest");
  // A new group still goes in as typed, and Enter didn't save the dialog on the way.
  await page.locator("#f-group").fill("여행");
  await page.locator("#f-group").press("Enter");
  assert((await chips()).includes("여행"), "new group not added");
  assert((await saved()).filter((x) => x.host.id === "work").length === 0, "Enter in the group field saved");
  await page.evaluate(() => closeModal());
  // OS marks: a card with a preview keeps a small OS badge; without one the big mark shows.
  await page.evaluate(([a, b]) => itda.recv({ type: "hosts", hosts: [a, { ...b, thumb: 1 }] }), [mac, pc]);
  assert(await page.locator('.card[data-id="pc"] .os-badge[title="Windows"]').count() === 1, "no OS badge on a card with a preview");
  assert(await page.locator('.card[data-id="mac"] .os-badge').count() === 0, "badge on a card without a preview");
  assert(await page.locator('.card[data-id="mac"] .ph .osm-mac').count() === 1, "big Mac mark missing");
  assert(!errors.length, `page errors: ${errors}`);
  console.log("PASS: group chips, Enter/IME, HP state kept on save, HP events scoped, HP block follows kind, group suggestions, OS badges");
} finally {
  await browser.close();
  server.close();
}
