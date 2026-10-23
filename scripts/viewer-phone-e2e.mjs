// The phone web viewer (web/viewer) against a fixture gateway API and a fake WebSocket that
// closes with the gateway's close codes: sign-in copy, PIN lockout, an expired session,
// fixing a failed login from the error card, and the toolbar at phone width.
//   PLAYWRIGHT=/path/to/playwright/index.mjs CHROMIUM=/path/to/chrome node scripts/viewer-phone-e2e.mjs
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
const repo = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const pw = await import(process.env.PLAYWRIGHT ? pathToFileURL(process.env.PLAYWRIGHT).href : "playwright");
const shots = process.env.SCREENSHOT_DIR;
if (shots) fs.mkdirSync(shots, { recursive: true });
const assert = (c, m) => { if (!c) throw new Error(m); };

// The gateway's state: one session, wrong PINs (5 → 429 like src/gateway.rs), saved devices.
const st = { authed: false, wrong: 0, edits: [], holdMe: false, held: [], hosts: [
  { id: "mom", label: "엄마의 iMac", host: "mom-imac.local", user: "younghee", own_user: "younghee", os: "mac", kind: "vnc", has_pw: true, account: "" },
  { id: "pc", label: "거실 PC", host: "192.168.0.40", os: "windows", kind: "vnc", has_pw: false, account: "" },
  { id: "lab", label: "연구실 Mac", host: "lab.local", user: "kim", os: "mac", kind: "vnc", has_pw: true, account: "a1" },
], accounts: [{ id: "a1", name: "회사 계정", user: "kim", has_pw: true, uses: 2 }] };
const json = (res, code, body) => { res.writeHead(code, { "Content-Type": "application/json" }); res.end(JSON.stringify(body)); };
const body = (req) => new Promise((r) => { let b = ""; req.on("data", (d) => (b += d)); req.on("end", () => r(JSON.parse(b || "{}"))); });
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, "http://x").pathname;
  if (p === "/api/me" && st.holdMe) return st.held.push(res); // a network that doesn't answer
  if (p === "/api/me") return json(res, st.authed ? 200 : 401, { ok: st.authed, local: false });
  if (p === "/api/login") {
    const { pin } = await body(req);
    if (st.wrong >= 5) return json(res, 429, { error: "잠시 후 다시 시도하세요 (PIN을 여러 번 틀렸습니다)" });
    if (pin === "123456") { st.authed = true; st.wrong = 0; return json(res, 200, { ok: true }); }
    st.wrong++;
    return json(res, 401, { error: "PIN이 맞지 않습니다" });
  }
  if (p === "/api/logout") { st.authed = false; return json(res, 200, {}); }
  if (p.startsWith("/api/") && !st.authed) return json(res, 401, { error: "로그인이 필요합니다" });
  if (p === "/api/hosts") return json(res, 200, st.hosts);
  if (p === "/api/accounts") return json(res, 200, st.accounts);
  if (p === "/api/nearby") return json(res, 200, []);
  if (p === "/api/edit") {
    const b = await body(req);
    st.edits.push(b);
    return json(res, 200, { id: b.id, hosts: st.hosts, accounts: st.accounts });
  }
  const file = p === "/" ? "web/viewer/index.html" : p === "/icon.svg" ? "assets/itda.svg" : p.startsWith("/hp-setup") ? `web${p}` : `web/viewer${p}`;
  try {
    const type = file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : file.endsWith(".svg") ? "image/svg+xml" : "text/html";
    const data = fs.readFileSync(path.join(repo, file));
    res.writeHead(200, { "Content-Type": type });
    res.end(data);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/`;

const browser = await pw.chromium.launch(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {});
try {
  const ctx = await browser.newContext({
    viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true,
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
  });
  // A WebSocket that never opens and closes with window.closeCode (the gateway's codes).
  await ctx.addInitScript(() => {
    window.closeCode = 4010;
    window.wsUrls = [];
    window.WebSocket = class extends EventTarget {
      constructor(u) {
        super(); window.wsUrls.push(u); this.url = u; this.readyState = 0; this.binaryType = "arraybuffer"; this.protocol = "binary";
        this.onopen = this.onclose = this.onerror = this.onmessage = null;
        setTimeout(() => { this.readyState = 3; const e = new CloseEvent("close", { code: window.closeCode, reason: "" }); this.onclose?.(e); this.dispatchEvent(e); }, 150);
      }
      send() {} close() {}
    };
  });
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const text = (s) => page.locator(s).innerText();
  const shot = (n) => shots && page.screenshot({ path: path.join(shots, `${n}.png`) });

  await page.goto(base);
  await page.waitForSelector("#login:not([hidden])");
  const viewport = await page.evaluate(() => document.querySelector("meta[name=viewport]").content);
  assert(!/user-scalable|maximum-scale/.test(viewport), `pinch zoom stays allowed: ${viewport}`);
  const login = await text("#login");
  assert(/공유/.test(login) && /Mac 또는 Windows PC/.test(login), `login names the Share page on Mac or PC: ${login}`);
  assert(!/다른 기기에서 보기|Windows PC의 Itda 설정/.test(login), "no Windows-only settings path");
  assert(await page.locator("#loginNote").isHidden(), "no expiry note on a first visit");
  await shot("01-login");

  // Wrong PIN: says how to check it and the limit; then the lockout counts down and blocks.
  await page.fill("#pin", "111111");
  await page.locator("#loginForm button").tap();
  await page.waitForFunction(() => document.querySelector("#loginError").textContent.length > 0);
  assert(/공유 페이지/.test(await text("#loginError")) && /5번 틀리면 1분/.test(await text("#loginError")), "wrong-PIN advice");
  for (let i = 0; i < 5; i++) {
    await page.fill("#pin", "222222");
    await Promise.all([page.waitForResponse((r) => r.url().endsWith("/api/login")), page.locator("#loginForm button").tap()]);
  }
  await page.waitForFunction(() => /초 뒤/.test(document.querySelector("#loginError").textContent), null, { timeout: 5000 });
  const locked = await text("#loginError");
  assert(/(\d+)초 뒤/.test(locked) && Number(locked.match(/(\d+)초/)[1]) <= 60, `lockout gives a bounded time: ${locked}`);
  assert(await page.locator("#loginForm button").isDisabled(), "sign-in button waits out the lockout");
  await shot("02-locked");
  // Fast-forward the lockout: the button comes back with no extra request.
  const before = st.wrong;
  await page.evaluate(() => { const n = Date.now; Date.now = () => n() + 61000; });
  await page.waitForFunction(() => !document.querySelector("#loginForm button").disabled, null, { timeout: 3000 });
  assert(/다시 입력할 수 있습니다/.test(await text("#loginError")) && st.wrong === before, "lockout ends on time");
  await page.evaluate(() => location.reload());
  st.wrong = 0;
  await page.waitForSelector("#login:not([hidden])");
  await page.fill("#pin", "123456");
  await page.locator("#loginForm button").tap();
  await page.waitForSelector("#list:not([hidden])");

  // List: touch targets and the full "needs a password" line.
  const small = await page.evaluate(() => [...document.querySelectorAll("#list button")].filter((b) => b.offsetParent)
    .map((b) => { const r = b.getBoundingClientRect(); return [b.getAttribute("aria-label") || b.textContent.trim().slice(0, 12), Math.round(r.width), Math.round(r.height)]; })
    .filter(([, w, h]) => w < 44 || h < 44));
  assert(!small.length, `list touch targets under 44px: ${JSON.stringify(small)}`);
  const warn = await page.locator(".host[data-id=pc] .warn").evaluate((e) => [e.scrollWidth, e.clientWidth, e.textContent]);
  assert(warn[0] <= warn[1], `password line fits: ${JSON.stringify(warn)}`);
  await shot("03-list");

  // A Mac login failure: Mac-specific advice and a primary fix route into the editor,
  // which never shows the saved password; saving reconnects.
  await page.locator(".host[data-id=mom] .open").tap();
  await page.waitForSelector("#status [data-a=fix]");
  const card = await text("#status");
  assert(/Mac 로그인에 실패/.test(card) && /Apple ID 아님/.test(card), `Mac advice: ${card}`);
  assert(await page.locator("#status [data-a=fix]").evaluate((b) => b.classList.contains("go")), "fixing is the primary action");
  assert(/같은 정보로 다시/.test(await text("#status [data-a=retry]")), "retry says it reuses the saved login");
  await shot("04-auth-failed");
  await page.locator("#status [data-a=fix]").tap();
  await page.waitForSelector("#editor:not([hidden])");
  const pass = await page.locator("#edForm [name=password]").evaluate((i) => [i.type, i.value, i.placeholder]);
  assert(pass[0] === "password" && pass[1] === "" && /저장됨/.test(pass[2]), `saved password not shown: ${JSON.stringify(pass)}`);
  assert(/새 암호를 넣고 저장하면 바로 다시 연결/.test(await text("#edForm")), "editor says saving reconnects");
  await shot("05-fix-editor");
  const sockets = await page.evaluate(() => window.wsUrls.length);
  await page.fill("#edForm [name=password]", "new-secret");
  await page.locator("#edForm button[type=submit]").tap();
  await page.waitForFunction((n) => window.wsUrls.length > n, sockets);
  const saved = st.edits.at(-1);
  assert(saved.cmd === "save_host" && saved.id === "mom" && saved.password === "new-secret", `saved: ${JSON.stringify(saved)}`);
  await page.waitForSelector("#status [data-a=list]");
  await page.locator("#status [data-a=list]").tap();
  await page.waitForSelector("#list:not([hidden])");

  // A device on a shared account: the fix opens that account and says who else uses it.
  await page.locator(".host[data-id=lab] .open").tap();
  await page.waitForSelector("#status [data-a=fix]");
  assert(/계정 고치기/.test(await text("#status [data-a=fix]")) && /2대/.test(await text("#status")), "shared account route");
  await page.locator("#status [data-a=fix]").tap();
  await page.waitForSelector("#edTitle");
  assert((await text("#edTitle")) === "계정 편집", "account editor");
  await page.locator("[data-a=cancel]").tap();
  assert(await page.locator("#status").isVisible(), "cancel keeps the error card");
  await page.locator("#status [data-a=list]").tap();

  // Refused (not a login problem): no fix route, retry stays primary.
  await page.evaluate(() => (window.closeCode = 4012));
  await page.locator(".host[data-id=mom] .open").tap();
  await page.waitForSelector("#status [data-a=retry]");
  assert(!(await page.locator("#status [data-a=fix]").count()) && /Mac이 연결을 거부/.test(await text("#status")), "refused card");

  // A dropped connection shows its card at once, without waiting on the session check; a
  // late answer to that check touches neither the next connection nor the list.
  const until = async (f) => { for (let i = 0; i < 100 && !f(); i++) await page.waitForTimeout(50); assert(f(), "fixture state"); };
  const release = async (code) => {
    const answered = page.waitForResponse((r) => r.url().endsWith("/api/me"));
    st.holdMe = false;
    for (const res of st.held.splice(0)) json(res, code, { ok: code === 200 });
    await answered;
    await page.waitForTimeout(100); // the viewer's handling of that answer
  };
  st.holdMe = true;
  await page.evaluate(() => (window.closeCode = 1006));
  await page.locator("#status [data-a=list]").tap();
  await page.waitForSelector("#list:not([hidden])");
  await page.locator(".host[data-id=mom] .open").tap();
  await page.waitForFunction(() => /연결이 끊겼습니다/.test(document.querySelector("#status").textContent), null, { timeout: 3000 });
  await until(() => st.held.length === 1);
  await page.locator("#status [data-a=list]").tap();
  await page.evaluate(() => (window.closeCode = 4012));
  await page.locator(".host[data-id=lab] .open").tap();
  await page.waitForSelector("#status [data-a=retry]");
  await release(401);
  assert(/Mac이 연결을 거부/.test(await text("#status")) && (await page.locator("#view").isVisible()), "late answer ignored by the next connection");
  await page.locator("#status [data-a=list]").tap();
  st.holdMe = true;
  await page.evaluate(() => (window.closeCode = 1006));
  await page.waitForSelector("#list:not([hidden])");
  await page.locator(".host[data-id=mom] .open").tap();
  await page.waitForFunction(() => /연결이 끊겼습니다/.test(document.querySelector("#status").textContent), null, { timeout: 3000 });
  await until(() => st.held.length === 1);
  await page.locator("#status [data-a=list]").tap();
  await page.waitForSelector("#list:not([hidden])");
  await page.locator(".host[data-id=mom] [data-edit]").tap();
  await page.fill("#edForm [name=name]", "편집 중");
  await release(401);
  assert(await page.locator("#editor").isVisible() && (await page.inputValue("#edForm [name=name]")) === "편집 중", "late answer ignored on the list");
  await page.locator("[data-a=cancel]").tap();
  await page.evaluate(() => (window.closeCode = 4012));
  await page.locator(".host[data-id=mom] .open").tap();
  await page.waitForSelector("#status [data-a=retry]");

  // Toolbar at 390px: every control on screen, labelled, 44px or more.
  const tools = await page.evaluate(() => [...document.querySelectorAll("#tools button")].map((b) => {
    const r = b.getBoundingClientRect();
    return { t: b.dataset.t, l: Math.round(r.left), r: Math.round(r.right), w: Math.round(r.width), h: Math.round(r.height), label: b.textContent.trim() };
  }));
  for (const b of tools) {
    assert(b.l >= 0 && b.r <= 390, `toolbar button on screen: ${JSON.stringify(b)}`);
    assert(b.w >= 44 && b.h >= 44, `toolbar target: ${JSON.stringify(b)}`);
    assert(b.label, `toolbar button has a visible name: ${JSON.stringify(b)}`);
  }
  await page.locator("#tools [data-t=keys]").tap();
  const gap = await page.evaluate(() => document.querySelector("#sheet").getBoundingClientRect().top - document.querySelector("#tools").getBoundingClientRect().bottom);
  assert(gap >= 0, `sheet opens below the toolbar (${gap})`);
  await shot("06-tools");
  await page.locator("#tools [data-t=keys]").tap();

  // The session ends (12 h, new PIN, Itda restarted): back to sign-in with the reason.
  st.authed = false;
  await page.evaluate(() => (window.closeCode = 1006));
  await page.locator("#tools [data-t=back]").tap();
  await page.waitForSelector("#login:not([hidden])");
  assert(/다시 PIN을 넣어야/.test(await text("#loginNote")) && /12시간/.test(await text("#loginNote")), "expiry is explained");
  await shot("07-expired");
  // Logging out on purpose shows no expiry note.
  st.authed = true;
  await page.fill("#pin", "123456");
  await page.locator("#loginForm button").tap();
  await page.waitForSelector("#list:not([hidden])");
  await page.locator("#logout").tap();
  await page.waitForSelector("#login:not([hidden])");
  assert(await page.locator("#loginNote").isHidden(), "no expiry note after logging out");

  assert(!errors.length, `page errors: ${errors.join("; ")}`);
  console.log("PASS: phone viewer sign-in, lockout, expiry, login fix route, toolbar");
} finally {
  await browser.close();
  server.close();
}
