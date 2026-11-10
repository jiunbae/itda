// The interface in English: the home window (home, Nearby, Settings → Language, help), the
// macOS permission pages, the phone web viewer (sign-in, device list, toolbar), and a file
// window (Explorer and Finder layouts: list, menus, dialogs, transfers, Home/Network), against
// the real web/ frontend. No
// Korean may show except the language's own name and fixture data, no message ID may leak,
// and a language change redraws. Also: ?lang=en wins over a Korean browser.
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/i18n-en-e2e.mjs
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
const repo = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const pw = await import(process.env.PLAYWRIGHT ? pathToFileURL(process.env.PLAYWRIGHT).href : "playwright");
const shots = process.env.SCREENSHOT_DIR;
if (shots) fs.mkdirSync(shots, { recursive: true });
const assert = (c, m) => { if (!c) throw new Error(m); };

const hosts = [
  { id: "mac", name: "Studio Mac", host: "studio.local", user: "alex", os: "mac", group: "Home", groups: ["Home"], opts: {}, hp: { on: false, port: 0 }, ssh: { on: false, user: "", port: 0 }, favorite: true, last_used: Math.floor(Date.now() / 1000) - 7200 },
  { id: "pc", name: "Office PC", host: "office.local", user: "", os: "windows", group: "", groups: [], opts: {}, hp: { on: false, port: 0 }, ssh: { on: false, user: "", port: 0 } },
];
const viewerHosts = [
  { id: "mac", label: "Studio Mac", host: "studio.local", user: "alex", own_user: "alex", os: "mac", kind: "vnc", has_pw: true, account: "" },
  { id: "pc", label: "Office PC", host: "192.0.2.40", os: "windows", kind: "vnc", has_pw: false, account: "" },
];
let authed = false;
const json = (res, code, body) => { res.writeHead(code, { "Content-Type": "application/json" }); res.end(JSON.stringify(body)); };
const server = http.createServer((req, res) => {
  const p = new URL(req.url, "http://x").pathname;
  if (p === "/api/me") return json(res, authed ? 200 : 401, { ok: authed, local: false });
  if (p === "/api/login") { authed = true; return json(res, 200, { ok: true }); }
  if (p === "/api/hosts") return json(res, 200, viewerHosts);
  if (p === "/api/accounts") return json(res, 200, []);
  if (p === "/api/nearby") return json(res, 200, []);
  // The home window is web/ at /, the web viewer web/viewer at /viewer/ (sharing i18n.js and
  // locales/, which the gateway serves at its root).
  let file = p === "/" ? "web/index.html" : p === "/viewer/" ? "web/viewer/index.html" : `web${p}`;
  if (/^\/viewer\/(i18n\.js|locales\/|hp-setup)/.test(p)) file = `web/${p.slice(8)}`;
  if (p === "/viewer/icon.svg") file = "assets/itda.svg";
  try {
    const data = fs.readFileSync(path.join(repo, file));
    res.writeHead(200, { "Content-Type": file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : file.endsWith(".svg") ? "image/svg+xml" : "text/html" });
    res.end(data);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}`;

// Visible Korean (but 한국어 and the fixture's own names) and leaked message IDs.
const stray = (page, allow = []) => page.evaluate((allow) => {
  const out = [];
  const keyLike = /\b(viewer|hpSetup|lang|nav|home|settings|editor|toast|share|nearby|common|card|title|top|files|remoteSetup|transfers|help|perm|permGuide|onboard)\.[a-zA-Z]+[\w.]*\b/;
  const look = (s, where) => {
    if (!s) return;
    let rest = s.replace(/한국어/g, "");
    for (const a of allow) rest = rest.split(a).join("");
    if (/[가-힣]/.test(rest)) out.push(`Korean in ${where}: ${s.trim().slice(0, 100)}`);
    const k = s.match(keyLike);
    if (k && !/\.(local|com|net)\b/.test(k[0])) out.push(`message ID in ${where}: ${k[0]}`);
  };
  document.querySelectorAll("details").forEach((d) => (d.open = true));
  look(document.body.innerText, "text");
  for (const e of document.querySelectorAll("[title],[aria-label],[placeholder]")) {
    if (!(e.offsetParent || e.getClientRects().length)) continue;
    for (const a of ["title", "aria-label", "placeholder"]) look(e.getAttribute(a), `${e.tagName.toLowerCase()}[${a}]`);
  }
  return out;
}, allow);

const browser = await pw.chromium.launch(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {});
const problems = [];
const watch = (page) => {
  page.on("pageerror", (e) => problems.push(`page error: ${e.message}`));
  page.on("console", (m) => { if (/i18n: missing/.test(m.text())) problems.push(m.text()); });
};
try {
  // ---- Home window, English system --------------------------------------------------------
  const page = await browser.newPage({ locale: "en-US", viewport: { width: 1280, height: 800 }, userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36" });
  watch(page);
  await page.addInitScript(() => (window.ipc = { postMessage: (s) => (window.sent ??= []).push(JSON.parse(s)) }));
  await page.goto(base + "/");
  await page.evaluate((hosts) => itda.recv({ type: "state", hosts, accounts: [], settings: { onboarded: true, theme: "light", lang: "auto", defaults: {}, web: { on: false, port: 8780 }, share: { on: false } } }), hosts);
  assert(await page.evaluate(() => document.documentElement.lang) === "en", "home: <html lang> follows the language");
  assert(/All connections/i.test(await page.locator("#title").innerText()), `home title in English: ${await page.locator("#title").innerText()}`);
  assert(await page.getByRole("button", { name: "Settings", exact: true }).count() === 1, "settings nav is named in English");
  problems.push(...(await stray(page)).map((s) => `home: ${s}`));
  if (shots) await page.screenshot({ path: path.join(shots, "en-home.png") });

  await page.evaluate(() => {
    S.found = [{ name: "Design Mac", host: "design.local", hostname: "design.local", ip: "192.0.2.21", os: "mac", itda: true, share: true }];
    S.page = "nearby"; render();
  });
  problems.push(...(await stray(page)).map((s) => `nearby: ${s}`));
  if (shots) await page.screenshot({ path: path.join(shots, "en-nearby.png") });

  await page.evaluate(() => { S.page = "settings"; render(); });
  problems.push(...(await stray(page)).map((s) => `settings: ${s}`));
  assert(await page.getByText("Language", { exact: true }).count() >= 1, "settings has a Language row");
  // Pick 한국어: saved with the settings, and the window redraws in Korean at once.
  await page.getByRole("button", { name: "한국어", exact: true }).first().click();
  await page.waitForFunction(() => I18N.lang === "ko");
  const saved = await page.evaluate(() => (window.sent || []).filter((m) => m.cmd === "settings").pop());
  assert(saved && saved.settings.lang === "ko", `language choice saved: ${JSON.stringify(saved && saved.settings.lang)}`);
  assert(await page.getByText("언어", { exact: true }).count() >= 1, "settings redrew in Korean");
  await page.evaluate(() => { S.page = "all"; render(); });
  assert(/모든 연결/.test(await page.locator("#title").innerText()), "home title in Korean after the switch");
  await page.close();

  // ---- ?lang=en wins over a Korean browser ------------------------------------------------
  const forced = await browser.newPage({ locale: "ko-KR", viewport: { width: 1280, height: 800 } });
  watch(forced);
  await forced.addInitScript(() => (window.ipc = { postMessage: () => {} }));
  await forced.goto(base + "/?lang=en");
  await forced.evaluate((hosts) => itda.recv({ type: "state", hosts, accounts: [], settings: { onboarded: true, theme: "light", lang: "ko", defaults: {} } }), hosts);
  assert(await forced.evaluate(() => I18N.lang) === "en", "?lang=en overrides the browser and the saved choice");
  problems.push(...(await stray(forced)).map((s) => `forced: ${s}`));
  await forced.close();

  // ---- Phone web viewer, English browser --------------------------------------------------
  const ctx = await browser.newContext({
    locale: "en-US", viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true,
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
  });
  await ctx.addInitScript(() => {
    window.WebSocket = class extends EventTarget {
      constructor(u) {
        super(); this.url = u; this.readyState = 0; this.binaryType = "arraybuffer"; this.protocol = "binary";
        this.onopen = this.onclose = this.onerror = this.onmessage = null;
        setTimeout(() => { this.readyState = 3; const e = new CloseEvent("close", { code: 4012, reason: "" }); this.onclose?.(e); this.dispatchEvent(e); }, 150);
      }
      send() {} close() {}
    };
  });
  const phone = await ctx.newPage();
  watch(phone);
  await phone.goto(base + "/viewer/");
  await phone.waitForSelector("#login:not([hidden])");
  assert(/Share/.test(await phone.locator("#login").innerText()), "sign-in explains the Share page in English");
  problems.push(...(await stray(phone)).map((s) => `viewer sign-in: ${s}`));
  await phone.fill("#pin", "123456");
  await phone.locator("#loginForm button").tap();
  await phone.waitForSelector("#list:not([hidden])");
  problems.push(...(await stray(phone)).map((s) => `viewer list: ${s}`));
  if (shots) await phone.screenshot({ path: path.join(shots, "en-phone-list.png") });
  await phone.locator(".host[data-id=mac] .open").tap();
  await phone.waitForSelector("#status [data-a=retry]");
  assert(/refused/i.test(await phone.locator("#status").innerText()), `refusal explained in English: ${await phone.locator("#status").innerText()}`);
  const tools = await phone.evaluate(() => [...document.querySelectorAll("#tools button")].map((b) => [b.getAttribute("aria-label") || b.title, b.textContent.trim()].join(" | ")));
  assert(tools.length >= 5 && tools.some((t) => /Keyboard/.test(t)) && tools.some((t) => /Full screen/i.test(t)), `toolbar in English: ${tools.join(", ")}`);
  problems.push(...(await stray(phone)).map((s) => `viewer toolbar: ${s}`));
  if (shots) await phone.screenshot({ path: path.join(shots, "en-phone-toolbar.png") });
  await ctx.close();

  // ---- File window, Windows (Explorer) and Mac (Finder) -------------------------------------
  for (const platform of ["windows", "mac"]) {
    const fc = await browser.newContext({ locale: "en-US", viewport: { width: 1200, height: 800 }, reducedMotion: "reduce",
      userAgent: platform === "mac" ? "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" : "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" });
    await fc.addInitScript(() => {
      const entry = (name, dir = false, size = 100) => ({ name, dir, size, modified: 1780000000 });
      const home = "/Users/alex", tree = { [home]: [entry("Documents", true), entry("Notes.txt"), entry("Zebra.pdf", false, 2000)], [home + "/Documents"]: [entry("Alpha.txt")] };
      window.ipc = { postMessage: (raw) => { const m = JSON.parse(raw); if (m.cmd !== "files") return;
        const result = { ...m, type: "files_result", ok: true };
        if (m.op === "connect") result.data = { path: home, entries: tree[home], local_path: home, local_entries: tree[home], rsync: true };
        else if (m.op === "list") result.data = { path: m.path, entries: tree[m.path] || [] };
        else if (m.op === "preview") result.data = { mime: "text/plain", text: "Preview fixture" };
        else return;
        queueMicrotask(() => itda.recv(result));
      } };
    });
    const fp = await fc.newPage();
    watch(fp);
    if (platform === "windows") await fp.route("**/icons.js", (route) => route.fulfill({ contentType: "text/javascript", body: fs.readFileSync(path.join(repo, "web/icons.js"), "utf8").replace("if (/Windows/.test(navigator.userAgent)) return;", "") }));
    await fp.goto(base + "/");
    await fp.evaluate(() => { S.hosts = [{ id: "test", name: "Studio Mac", host: "studio.local", os: "mac", kind: "vnc", login_user: "alex", has_pw: true, files: { method: "sftp", user: "alex" } }]; S.settings = { ...S.settings, onboarded: true, theme: "light" }; S.page = "files"; render(); });
    problems.push(...(await stray(fp)).map((s) => `${platform} file browser: ${s}`));
    await fp.evaluate(() => ItdaFiles.launch(S.hosts[0]));
    const layer = fp.locator(".files-layer");
    await layer.locator("[data-index]").first().waitFor();
    const heads = (await layer.locator("th").allTextContents()).join("|");
    assert(heads.includes(platform === "mac" ? "Name ▴|Date modified|Size|Kind" : "Name ▴|Date modified|Type|Size"), `${platform} column headers in English: ${heads}`);
    assert(await fp.evaluate(() => document.title) === "Studio Mac — Files", `${platform} window title in English`);
    await layer.locator("[data-index]").filter({ hasText: "Notes.txt" }).first().click();
    assert(await layer.locator("[data-status]").textContent() === "3 items · 1 selected", `${platform} status in English: ${await layer.locator("[data-status]").textContent()}`);
    problems.push(...(await stray(fp)).map((s) => `${platform} file list: ${s}`));
    const click = (a) => layer.locator(`[data-action="${a}"]`).first().evaluate((b) => b.click());
    for (const menu of ["more", "sort"]) {
      await click(menu);
      problems.push(...(await stray(fp)).map((s) => `${platform} ${menu} menu: ${s}`));
      await fp.keyboard.press("Escape");
      await fp.evaluate(() => document.querySelectorAll(".files-menu").forEach((e) => e.remove()));
    }
    await click("copy"); await click("paste");
    const dialog = fp.getByRole("dialog", { name: "Copy files", exact: true });
    await dialog.waitFor();
    await dialog.locator("summary").click();
    problems.push(...(await stray(fp)).map((s) => `${platform} copy dialog: ${s}`));
    if (shots) await fp.screenshot({ path: path.join(shots, `en-files-${platform}-copy.png`) });
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await fp.evaluate(() => ItdaFiles.message({ type: "files_job", window: "app", id: "test", job: "j1", seq: 1, state: "failed", error: "boom", request: { names: ["a", "b"], from: "local", source: "/x", to: "remote", destination: "/y" }, completed_names: ["a"] }));
    problems.push(...(await stray(fp)).map((s) => `${platform} transfers: ${s}`));
    if (shots) await fp.screenshot({ path: path.join(shots, `en-files-${platform}.png`) });
    // A language change redraws the open window (lists, sidebar, tool labels), both ways.
    await fp.evaluate(() => I18N.setPref("ko"));
    assert((await layer.locator("th").first().textContent()).startsWith("이름"), `${platform} file window redrew in Korean`);
    assert(await layer.locator('[data-action="copy"]').getAttribute("title") === "복사", `${platform} tool labels follow the language`);
    await fp.evaluate(() => I18N.setPref("en"));
    assert((await layer.locator("th").first().textContent()).startsWith("Name"), `${platform} file window redrew in English`);
    problems.push(...(await stray(fp)).map((s) => `${platform} after switching back: ${s}`));
    await layer.locator('[data-action="network"]').first().click();
    await layer.locator(".files-network h2").waitFor();
    problems.push(...(await stray(fp)).map((s) => `${platform} network: ${s}`));
    await fc.close();
  }

  // ---- Help center (every topic, Mac and Windows) and the macOS permission pages -----------
  for (const [os, ua] of [["mac", "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36"], ["windows", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"]]) {
    const hp = await browser.newPage({ locale: "en-US", userAgent: ua, viewport: { width: 1100, height: 800 } });
    watch(hp);
    await hp.addInitScript(() => (window.ipc = { postMessage: () => {} }));
    await hp.goto(base + "/");
    await hp.evaluate(() => itda.recv({ type: "state", hosts: [], accounts: [], settings: { onboarded: true, theme: "light", lang: "auto", defaults: {}, web: { on: false, port: 8780 }, share: { on: false } } }));
    for (const topic of ["start", "mac", "windows", "linux", "share", "phone", "macpc", "assist", "outside", "accounts", "settings", "words", "trouble", "keys"]) {
      await hp.evaluate((t) => { S.helpTopic = t; S.page = "help"; render(); }, topic);
      problems.push(...(await stray(hp)).map((s) => `help ${os}/${topic}: ${s}`));
    }
    await hp.close();
  }
  const perm = await browser.newPage({ locale: "en-US", viewport: { width: 420, height: 720 } });
  watch(perm);
  await perm.addInitScript(() => (window.ipc = { postMessage: () => {} }));
  await perm.goto(base + "/permissions.html");
  const b = { screen: false, access: false, app_path: "/Applications/Itda.app", requested: "screen", requested_seq: 1, version: "1.0.22" };
  for (const m of [b, { ...b, configured_screen: true, active_transfers: 2 }, { ...b, screen: true, requested: "access" }, { ...b, screen: true, access: true, enabled: true, hp_serving: true }]) {
    await perm.evaluate((m) => itda.recv({ type: "permissions", ...m }), m);
    problems.push(...(await stray(perm)).map((s) => `permissions: ${s}`));
  }
  const title = await perm.evaluate(() => document.title);
  assert(title && title === (await perm.evaluate(() => T("perm.title"))) && !/[가-힣]/.test(title), `permission helper title in English: ${title}`);
  await perm.goto(base + "/permission-guide.html");
  for (const m of [{ permission: "screen", step: "Step 2 of 2", side: "left", app_path: "/Applications/Itda.app" }, { stale: true }]) {
    await perm.evaluate((m) => itda.recv({ type: "guide", ...m }), m);
    problems.push(...(await stray(perm)).map((s) => `permission guide: ${s}`));
  }
  await perm.close();

  assert(!problems.length, problems.join("\n"));
  console.log("PASS: English home, Nearby and Settings → Language (saved, redraws), ?lang=en, phone sign-in, list, refusal and toolbar; file windows (Explorer and Finder) in English and after a language switch; every help topic; permission helper and guide");
} finally {
  await browser.close();
  server.close();
}
