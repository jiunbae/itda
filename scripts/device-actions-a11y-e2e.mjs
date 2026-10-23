// Device cards and list rows on the real home frontend (no app): every action button says
// which device it acts on, the card or row is a named group, and the keyboard reaches the
// right action and device. Names come from the accessibility tree (Playwright's roles).
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/device-actions-a11y-e2e.mjs
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
    const data = fs.readFileSync(path.join(root, p === "/" ? "index.html" : p));
    res.setHeader("Content-Type", p.endsWith(".js") ? "text/javascript" : p.endsWith(".css") ? "text/css" : "text/html");
    res.end(data);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const engine = process.env.ENGINE || "chromium";
const exe = engine === "chromium" ? process.env.CHROMIUM : process.env.WEBKIT;
const browser = await pw[engine].launch(exe ? { executablePath: exe } : {});
try {
  const page = await browser.newPage({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36", viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript(() => (window.ipc = { postMessage: (s) => (window.sent ??= []).push(JSON.parse(s)) }));
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  const base = { os: "mac", kind: "vnc", user: "a", has_pw: true, opts: {}, hp: { on: false, port: 0 }, ssh: { on: false, user: "", port: 0 }, groups: [] };
  // Three devices: idle, connecting, connected; one name with characters that must be escaped.
  const hosts = [
    { ...base, id: "mom", name: "엄마의 iMac", host: "mom.local", favorite: true },
    { ...base, id: "lab", name: 'Lab <"A"> & Co', host: "lab.local" },
    { ...base, id: "pc", name: "거실 PC", host: "pc.local", os: "windows" },
  ];
  const sessions = { lab: { state: "connecting", stage: "" }, pc: { state: "connected" } };
  await page.evaluate(([hosts, sessions]) => itda.recv({ type: "state", hosts, sessions, settings: { onboarded: true, theme: "light", defaults: {}, view: "grid" } }), [hosts, sessions]);
  const sent = () => page.evaluate(() => window.sent || []);
  const lastCmd = async (cmd) => (await sent()).filter((x) => x.cmd === cmd).at(-1);
  const btn = (name) => page.getByRole("button", { name, exact: true });
  for (const view of ["grid", "list"]) {
    await page.evaluate((view) => { S.settings.view = view; render(); }, view);
    // The card/row is a group named for its device (not a button: it holds buttons).
    for (const name of ["엄마의 iMac", 'Lab <"A"> & Co', "거실 PC"]) {
      assert(await page.getByRole("group", { name, exact: true }).count() === 1, `${view}: no group named ${name}`);
    }
    // Each action names its device; the visible words stay part of the name.
    for (const [name, label] of [["엄마의 iMac 화면 연결", "화면 연결"], ['Lab <"A"> & Co 연결 취소', "취소"], ["거실 PC 창으로 이동", "창으로 이동"], ["엄마의 iMac 더보기", null], ['Lab <"A"> & Co 더보기', null], ["거실 PC 더보기", null]]) {
      assert(await btn(name).count() === 1, `${view}: no single button named "${name}" (have: ${JSON.stringify(await page.locator("#content button").evaluateAll((b) => b.map((x) => x.getAttribute("aria-label") || x.textContent.trim())))})`);
      if (label) assert((await btn(name).textContent()).includes(label), `${view}: visible label changed for ${name}`);
    }
    assert(await page.getByRole("button", { name: "화면 연결", exact: true }).count() === 0 && await page.getByRole("button", { name: "더보기", exact: true }).count() === 0, `${view}: an action without its device is left`);
  }
  // Favorites (cards): a toggle with a stable name and its state.
  await page.evaluate(() => { S.settings.view = "grid"; render(); });
  const fav = btn("엄마의 iMac 즐겨찾기");
  assert(await fav.getAttribute("aria-pressed") === "true", "favorite state not exposed");
  assert(await btn("거실 PC 즐겨찾기").getAttribute("aria-pressed") === "false", "non-favorite state not exposed");
  await fav.focus(); await page.keyboard.press("Enter");
  assert((await lastCmd("favorite"))?.id === "mom" && (await lastCmd("favorite")).on === false, "Enter on the favorite toggle did not toggle that device: " + JSON.stringify(await lastCmd("favorite")));
  assert(!(await lastCmd("connect")), "Enter on the favorite toggle connected instead");
  assert(await btn("엄마의 iMac 즐겨찾기").getAttribute("aria-pressed") === "false", "toggle state did not follow");
  // Keyboard on actions: each sends its own device.
  for (const view of ["grid", "list"]) {
    await page.evaluate((view) => { S.settings.view = view; render(); }, view);
    await btn("엄마의 iMac 화면 연결").focus(); await page.keyboard.press("Enter");
    assert((await lastCmd("connect"))?.id === "mom", `${view}: connect went elsewhere`);
    await page.evaluate(() => closeModal()); // the "connecting" progress dialog
    await btn('Lab <"A"> & Co 연결 취소').focus(); await page.keyboard.press("Space");
    assert((await lastCmd("cancel"))?.id === "lab", `${view}: cancel went elsewhere`);
    await btn("거실 PC 창으로 이동").focus(); await page.keyboard.press("Enter");
    assert((await lastCmd("focus"))?.id === "pc", `${view}: focus went elsewhere`);
    const before = (await sent()).filter((x) => x.cmd === "connect").length;
    await btn("거실 PC 더보기").focus(); await page.keyboard.press("Enter");
    await page.locator("#ctx:not([hidden])").waitFor();
    assert((await sent()).filter((x) => x.cmd === "connect").length === before, `${view}: Enter on More connected the device`);
    await page.keyboard.press("Escape");
    assert(await btn("거실 PC 더보기").evaluate(b => b === document.activeElement), `${view}: Escape did not return focus to More`);
    await page.keyboard.press("Enter");
    await page.locator("#ctx:not([hidden])").waitFor();
    await page.locator("#search").click();
    assert(await page.locator("#search").evaluate(b => b === document.activeElement), `${view}: closing More by an outside click stole focus`);
    // Inner buttons retain device editing/navigation shortcuts; only their activation
    // keys belong to the button. The editor must be for this device, without connecting.
    await btn("거실 PC 더보기").focus(); await page.keyboard.press("F2");
    await page.locator("#f-host").waitFor({ timeout: 6000 });
    assert(await page.locator("#f-host").inputValue() === "pc.local", `${view}: F2 edited another device`);
    assert((await sent()).filter((x) => x.cmd === "connect").length === before, `${view}: F2 connected instead of editing`);
    await page.keyboard.press("Escape");
    const groups = page.locator(view === "grid" ? "#content .card" : "#content .list-row:not(.list-head)");
    const nextId = await groups.nth(1).getAttribute("data-id");
    await groups.first().locator("button[data-act=more]").focus();
    await page.keyboard.press("ArrowRight");
    assert(await page.evaluate(id => document.activeElement?.getAttribute("role") === "group" && document.activeElement.dataset.id === id, nextId), `${view}: ArrowRight on a device button lost device navigation`);
  }
  // The group itself keeps its shortcut: Enter connects its device.
  const connects = (await sent()).filter((x) => x.cmd === "connect").length;
  await page.getByRole("group", { name: 'Lab <"A"> & Co', exact: true }).focus(); await page.keyboard.press("Enter");
  // (Lab is connecting: Enter shows its progress; an idle one connects.)
  await page.evaluate(() => closeModal());
  await page.getByRole("group", { name: "엄마의 iMac", exact: true }).focus(); await page.keyboard.press("Enter");
  assert((await sent()).filter((x) => x.cmd === "connect").length === connects + 1 && (await lastCmd("connect"))?.id === "mom", "Enter on the device group did not connect it");
  await page.evaluate(() => closeModal());
  // Names are text, not markup.
  assert(await page.locator("#content [aria-label*='<\"A\">']").count() > 0 && !(await page.locator("#content a, #content script").count()), "special characters not kept as text");
  // Non-element targets can occur for programmatically dispatched shortcut events.
  await page.evaluate(() => document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })));
  assert(!errors.length, errors.join("\n"));
  console.log(`PASS (${engine}): device groups and per-device action names in cards and list, favorite toggle state, keyboard actions reach their own device`);
} finally {
  await browser.close();
  server.close();
}
