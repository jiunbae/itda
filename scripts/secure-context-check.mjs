// The web viewer is served over plain HTTP on a LAN or tailnet, which browsers don't treat as
// a secure context: APIs limited to secure contexts must not be needed there. Fails on use of
// any of them in the app's own scripts (bundled noVNC excluded).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "web");
const banned = [/crypto\.randomUUID\s*\(/, /navigator\.clipboard\.(read|write)\b/, /crypto\.subtle\b/];
const hits = [];
const walk = (d) => { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); if (e.isDirectory()) walk(p); else if (e.name.endsWith(".js") && !e.name.includes("novnc")) fs.readFileSync(p, "utf8").split("\n").forEach((l, i) => { for (const b of banned) if (b.test(l) && !/secure context/.test(l)) hits.push(`${path.relative(root, p)}:${i + 1}: ${l.trim().slice(0, 120)}`); }); } };
walk(root);
if (hits.length) { console.error("secure-context-only APIs used:\n" + hits.join("\n")); process.exit(1); }
console.log("PASS: no secure-context-only APIs in web/ scripts");
