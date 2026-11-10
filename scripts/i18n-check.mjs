// Translation guard, run in CI:
//  1. every message ID is in both web catalogs (web/locales/ko.js, en.js), defined once;
//  2. a message has the same {placeholders} (and plural shape) in both languages;
//  3. every T('id') / data-i18n="id" in the UI names a message that exists;
//  4. no Hangul is left in UI code outside the catalogs: string literals in web/*.js,
//     web/viewer/*.js, the HTML pages and src/**/*.rs (comments and #[cfg(test)] modules
//     don't count), except ALLOW below.
//   node scripts/i18n-check.mjs
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const rel = (p) => path.relative(root, p).split(path.sep).join("/");
const errors = [];
const fail = (m) => errors.push(m);

// Hangul that is data, not interface text. Keep this short: { file, has, why }.
const ALLOW = [
  { file: "web/app.js", has: "title = title.replace(/Mac이/g", why: "Korean particles follow the device noun (Korean only)" },
  { file: "web/app.js", has: "lead = lead.replace(/Mac이/g", why: "Korean particles follow the device noun (Korean only)" },
];

// Files not converted yet (a later change moves their text into the catalogs).
const PENDING = new Set([
  // Phase 2: the rest of the web UI.
  "web/files.js",
  "web/help.js",
  "web/permission-guide.html",
  "web/permission-guide.js",
  "web/permissions.html",
  "web/permissions.js",
  "web/remote-setup.js",
  // Phase 3: text the app renders itself (menus, dialogs, the Windows viewer, errors).
  "src/autostart.rs",
  "src/capture_support.rs",
  "src/curtain_mac.rs",
  "src/files/drag.rs",
  "src/files/export.rs",
  "src/files/locations.rs",
  "src/files/mod.rs",
  "src/files/preview.rs",
  "src/files/preview_image.rs",
  "src/files/sftp.rs",
  "src/files/transfer.rs",
  "src/files/vdrag.rs",
  "src/files_mac.rs",
  "src/files_win.rs",
  "src/gateway.rs",
  "src/home.rs",
  "src/home_mac.rs",
  "src/home_win.rs",
  "src/host_mac.rs",
  "src/hp_upgrade.rs",
  "src/main.rs",
  "src/net.rs",
  "src/permissions_mac.rs",
  "src/rdp.rs",
  "src/remote_setup.rs",
  "src/rfb.rs",
  "src/server.rs",
  "src/session.rs",
  "src/share.rs",
  "src/store.rs",
  "src/tailscale.rs",
  "src/tunnel.rs",
  "src/ui.rs",
  "src/update.rs",
]);

// ---- 1-2: catalogs --------------------------------------------------------------------
const catalogs = { ko: {}, en: {} };
const where = {};
const locales = path.join(root, "web/locales");
for (const f of fs.readdirSync(locales).sort()) {
  const m = /^(?:.+\.)?(ko|en)\.js$/.exec(f);
  if (!m) { fail(`web/locales/${f}: not a catalog (name it ko.js or en.js)`); continue; }
  const lang = m[1];
  const seen = {};
  vm.runInNewContext(fs.readFileSync(path.join(locales, f), "utf8"), {
    I18N: { add(l, dict) { if (l !== lang) fail(`web/locales/${f}: adds '${l}' messages`); Object.assign(seen, dict); } },
  }, { filename: f });
  for (const [k, v] of Object.entries(seen)) {
    if (k in catalogs[lang]) fail(`${k}: defined in both ${where[lang + k]} and ${f}`);
    catalogs[lang][k] = v;
    where[lang + k] = f;
  }
}
const holes = (v) => {
  const forms = typeof v === "string" ? [v] : v && typeof v === "object" ? Object.values(v) : [];
  return [...new Set(forms.flatMap((s) => [...String(s).matchAll(/\{(\w+)\}/g)].map((x) => x[1])))].sort().join(",");
};
const shape = (k, v, lang) => {
  if (typeof v === "string") return "s";
  if (v && typeof v === "object" && typeof v.other === "string" && Object.keys(v).every((f) => ["zero", "one", "two", "few", "many", "other"].includes(f) && typeof v[f] === "string")) {
    if (!/\{count\}/.test(v.other) && !Object.values(v).some((s) => s.includes("{count}"))) fail(`${k} (${lang}): plural message without {count}`);
    return "p";
  }
  fail(`${k} (${lang}): a message is a string or { one, other } plural forms`);
  return "?";
};
for (const k of Object.keys(catalogs.ko)) {
  if (!(k in catalogs.en)) { fail(`${k}: missing in English`); continue; }
  const a = catalogs.ko[k], b = catalogs.en[k];
  if (shape(k, a, "ko") !== shape(k, b, "en")) fail(`${k}: plural in one language only`);
  if (holes(a) !== holes(b)) fail(`${k}: placeholders differ — ko {${holes(a)}} en {${holes(b)}}`);
  if (typeof b === "string" ? /[가-힣]/.test(b) && !/^lang\./.test(k) : Object.values(b).some((s) => /[가-힣]/.test(s))) fail(`${k}: English text has Hangul`);
}
for (const k of Object.keys(catalogs.en)) if (!(k in catalogs.ko)) fail(`${k}: missing in Korean`);

// ---- 3-4: sources ---------------------------------------------------------------------
const walk = (dir, test) => fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
  const p = path.join(dir, e.name);
  return e.isDirectory() ? walk(p, test) : test(p) ? [p] : [];
});
const web = [
  ...fs.readdirSync(path.join(root, "web")).filter((f) => /\.(js|html)$/.test(f) && f !== "i18n.js").map((f) => path.join(root, "web", f)),
  ...fs.readdirSync(path.join(root, "web/viewer")).filter((f) => /\.(js|html)$/.test(f) && !/^novnc/.test(f)).map((f) => path.join(root, "web/viewer", f)),
];
const rust = walk(path.join(root, "src"), (p) => p.endsWith(".rs") && !/_e2e\.rs$/.test(p));

// Code with comments blanked out (strings kept), line numbers preserved.
function stripJs(src) {
  let out = "", i = 0, q = null;
  while (i < src.length) {
    const c = src[i], n = src[i + 1];
    if (q) {
      out += c;
      if (c === "\\") { out += n ?? ""; i += 2; continue; }
      if (c === q) q = null;
      i++;
    } else if (c === "/" && n === "/") {
      while (i < src.length && src[i] !== "\n") i++;
    } else if (c === "/" && n === "*") {
      const end = src.indexOf("*/", i + 2), stop = end < 0 ? src.length : end + 2;
      out += src.slice(i, stop).replace(/[^\n]/g, " ");
      i = stop;
    } else if (c === "/" && /[(,=:[!&|?{};+\-*%<>~^]$|^$|\breturn$/.test(out.replace(/\s+$/, "").slice(-6))) {
      // A regular expression literal (its quotes are not strings).
      let j = i + 1, cls = false;
      for (; j < src.length && src[j] !== "\n"; j++) {
        if (src[j] === "\\") { j++; continue; }
        if (src[j] === "[") cls = true; else if (src[j] === "]") cls = false;
        else if (src[j] === "/" && !cls) break;
      }
      out += src.slice(i, j + 1);
      i = j + 1;
    } else {
      if (c === "'" || c === '"' || c === "`") q = c;
      out += c;
      i++;
    }
  }
  return out;
}
function stripHtml(src) {
  return src.replace(/<!--[\s\S]*?-->/g, (m) => m.replace(/[^\n]/g, " "));
}
function stripRust(src) {
  // Line comments (not inside a string), then #[cfg(test)] modules.
  const lines = src.split("\n").map((line) => {
    let q = false;
    for (let i = 0; i < line.length; i++) {
      const c = line[i];
      if (q) { if (c === "\\") i++; else if (c === '"') q = false; }
      else if (c === '"') q = true;
      else if (c === "/" && line[i + 1] === "/") return line.slice(0, i);
    }
    return line;
  });
  for (let i = 0; i < lines.length; i++) {
    if (!/^\s*#\[cfg\(test\)\]\s*$/.test(lines[i]) || !/^\s*(pub(\(crate\))?\s+)?mod\s+\w+\s*\{/.test(lines[i + 1] || "")) continue;
    let depth = 0, j = i + 1;
    for (; j < lines.length; j++) {
      for (const c of lines[j]) { if (c === "{") depth++; else if (c === "}") depth--; }
      lines[j] = "";
      if (depth <= 0) break;
    }
    lines[i] = "";
    i = j;
  }
  return lines.join("\n");
}

const used = new Set();
const hangul = /[가-힣]/;
for (const file of [...web, ...rust]) {
  const r = rel(file), src = fs.readFileSync(file, "utf8");
  const code = file.endsWith(".rs") ? stripRust(src) : file.endsWith(".html") ? stripHtml(src) : stripJs(src);
  if (!file.endsWith(".rs")) {
    for (const m of code.matchAll(/\bT\(\s*(['"])([\w.-]+)\1/g)) used.add(m[2]);
    for (const m of code.matchAll(/data-i18n(?:-html)?=["']([\w.-]+)["']/g)) used.add(m[1]);
    for (const m of code.matchAll(/data-i18n-attr=["']([^"']+)["']/g)) for (const pair of m[1].split(";")) used.add(pair.split(":")[1]?.trim());
  }
  if (PENDING.has(r)) continue;
  code.split("\n").forEach((line, i) => {
    if (!hangul.test(line)) return;
    const orig = src.split("\n")[i];
    if (ALLOW.some((a) => a.file === r && orig.includes(a.has))) return;
    fail(`${r}:${i + 1}: Hangul outside the catalogs — move it to web/locales (or src/i18n.rs): ${orig.trim().slice(0, 120)}`);
  });
}
for (const a of ALLOW) {
  const src = fs.readFileSync(path.join(root, a.file), "utf8");
  if (!src.includes(a.has)) fail(`ALLOW entry no longer matches ${a.file}: ${a.has}`);
}
for (const p of PENDING) if (!fs.existsSync(path.join(root, p))) fail(`PENDING file is gone: ${p}`);
used.delete(undefined);
for (const k of used) if (!(k in catalogs.ko)) fail(`${k}: used in the UI but not in the catalogs`);

if (errors.length) {
  console.error(errors.join("\n"));
  console.error(`FAIL: ${errors.length} translation problem(s)`);
  process.exit(1);
}
console.log(`PASS: ${Object.keys(catalogs.ko).length} messages in Korean and English, ${used.size} referenced, no stray Hangul (${PENDING.size} files pending)`);
