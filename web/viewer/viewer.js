// Itda web viewer. Signs in with the PIN, lists and edits the address book, and shows a
// Mac or Windows PC through the gateway's WebSocket with noVNC (window.RFB from
// novnc.min.js). Installable as an app (manifest + sw.js).
'use strict';
import './novnc.min.js';
import { MonitorController, startMirror } from './monitors.js';
import { WheelInput, macHost, hpKeys, macActions, macMenu, sendChord, sendMacAction } from './interaction.js';

const $ = (s) => document.querySelector(s);
// Language (web/i18n.js): the app's windows follow the app (window.ITDA_LANG); a browser follows
// its own language unless the person picks one on the list or sign-in screen (kept here).
const LANG_KEY = 'itda.lang';
if (!window.ITDA_LANG) { let p = null; try { p = localStorage.getItem(LANG_KEY); } catch (_) {} I18N.setPref(p || 'auto'); }
I18N.apply();
// Text that follows a language change: `v` is a string, or a function giving it.
const tx = (el, v) => { el.itdaText = typeof v === 'function' ? v : null; el.textContent = typeof v === 'function' ? v() : v; };
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

// Small stroke icons (no icon font on phones).
const P = {
  back: '<path d="M15 5l-7 7 7 7"/>',
  keyboard: '<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6 10h.01M9.5 10h.01M13 10h.01M16.5 10h.01M7 14h10"/>',
  command: '<path d="M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z"/>',
  fit: '<path d="M4 9V5h4M20 9V5h-4M4 15v4h4M20 15v4h-4"/>',
  actual: '<rect x="5" y="5" width="14" height="14" rx="2"/><path d="M10 9.5l2-1.5v8"/>',
  clip: '<rect x="6" y="4.5" width="12" height="16" rx="2"/><path d="M9.5 4.5h5v2.5h-5z"/>',
  full: '<path d="M8 4H4v4M16 4h4v4M8 20H4v-4M16 20h4v-4"/>',
  eye: '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/>',
  refresh: '<path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7"/>',
  mission: '<rect x="3" y="4" width="8" height="7" rx="1"/><rect x="13" y="4" width="8" height="7" rx="1"/><rect x="7" y="13" width="10" height="7" rx="1"/>',
  monitor: '<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M9 20h6M12 16.5V20"/>',
  bolt: '<path d="M13 3 5 14h6l-1 7 8-11h-6z"/>',
  sparkle: '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM18.5 15l.7 2 2 .7-2 .7-.7 2-.7-2-2-.7 2-.7z"/>',
  lock: '<rect x="5" y="10.5" width="14" height="9.5" rx="2"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/>',
  mouse: '<rect x="6.5" y="3" width="11" height="18" rx="5.5"/><path d="M12 3v6.5M6.5 9.5h11"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>',
  sound: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>',
  mute: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path d="M16 9.5l5 5M21 9.5l-5 5"/>',
  pc: '<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M8 20h8M12 16.5V20M3 13h18"/>',
  globe: '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.4 2.4 3.5 5.2 3.5 8.5s-1.1 6.1-3.5 8.5c-2.4-2.4-3.5-5.2-3.5-8.5s1.1-6.1 3.5-8.5z"/>',
};
const icon = (n) => `<svg class="ic" viewBox="0 0 24 24" aria-hidden="true">${P[n]}</svg>`;

// Operating system marks, filled (the same as the home page's, web/app.js).
const OS_SVG = {
  mac: '<path d="M16.37 12.73c-.02-2.1 1.72-3.12 1.8-3.17-.98-1.43-2.5-1.63-3.05-1.65-1.3-.13-2.53.76-3.19.76-.66 0-1.67-.74-2.75-.72-1.41.02-2.72.82-3.44 2.09-1.47 2.55-.38 6.32 1.05 8.39.7 1.01 1.53 2.15 2.62 2.11 1.05-.04 1.45-.68 2.72-.68s1.63.68 2.74.66c1.13-.02 1.85-1.03 2.54-2.05.8-1.17 1.13-2.31 1.15-2.37-.03-.01-2.2-.85-2.22-3.37zM14.29 6.55c.58-.7.97-1.68.86-2.65-.83.03-1.84.55-2.44 1.25-.53.62-1 1.61-.88 2.56.93.07 1.88-.47 2.46-1.16z"/>',
  windows: '<path d="M3.5 5.6l7-1v6.9h-7zM11.5 4.5l9-1.3v8.3h-9zM3.5 12.5h7v6.9l-7-1zM11.5 12.5h9v8.3l-9-1.3z"/>',
  linux: '<path d="M12 2.6c-2.3 0-3.6 1.9-3.6 4.4 0 1.3.3 2.2-.6 3.6-1.2 1.8-2.9 3.9-2.9 6.6 0 .6.1 1.1.3 1.6-.8.4-1.5.9-1.4 1.5.2.9 2.1.6 3.4 1.1 1.1.4 2 .6 2.6-.2.5-.1 1.4-.2 2.2-.2s1.7.1 2.2.2c.6.8 1.5.6 2.6.2 1.3-.5 3.2-.2 3.4-1.1.1-.6-.6-1.1-1.4-1.5.2-.5.3-1 .3-1.6 0-2.7-1.7-4.8-2.9-6.6-.9-1.4-.6-2.3-.6-3.6 0-2.5-1.3-4.4-3.6-4.4z"/><path class="cut" d="M12 10.2c-1.9 0-3.2 2.6-3.2 5.6 0 1.7.6 3.2 1.3 4h3.8c.7-.8 1.3-2.3 1.3-4 0-3-1.3-5.6-3.2-5.6z"/><circle class="cut" cx="10.6" cy="6.6" r=".9"/><circle class="cut" cx="13.4" cy="6.6" r=".9"/><path class="cut" d="M10.9 8.4h2.2L12 9.6z"/>',
};
const osName = (x) => (x.this || x.kind === 'rdp' ? 'windows' : String(x.os || '').toLowerCase() === 'macos' ? 'mac' : x.os);
const osMark = (x) => {
  const os = osName(x);
  return OS_SVG[os] ? `<svg class="ic osm" viewBox="0 0 24 24" aria-hidden="true">${OS_SVG[os]}</svg>` : icon('monitor');
};

const XK = { cmd: 0xffeb, opt: 0xffe9, ctrl: 0xffe3, shift: 0xffe1, esc: 0xff1b, tab: 0xff09, enter: 0xff0d, back: 0xff08, del: 0xffff, space: 0x20,
  left: 0xff51, up: 0xff52, right: 0xff53, down: 0xff54, f3: 0xffc0, f4: 0xffc1, win: 0xffeb, alt: 0xffe9 };

const S = { hosts: [], accounts: [], rfb: null, ws: null, host: null, fit: true, viewOnly: false, mods: new Set(), clip: '', ever: false, tries: 0, gen: 0 };

// App window (Itda for Mac): this page shows one connection, chosen by the app, and reports
// to the app's home instead of offering the list.
const OPEN = new URLSearchParams(location.search).get('open');
const MIRROR = new URLSearchParams(location.search).get('mirror');
// Relative pointer state (see mouseLock below).
const rel = { dx: 0, dy: 0, mask: 0, raf: 0, hidden: false, auto: false, toasted: false };
const relAvailable = () => !!S.rfb?.itdaRelativeOk && !S.viewOnly && MIRROR === null && matchMedia('(pointer: fine)').matches;
const relLocked = () => !!S.rfb?._canvas && document.pointerLockElement === S.rfb._canvas;
// ITDA_HP_TRACE (the app adds trace=1): time this viewer's part of each H.264 frame.
const TRACE = new URLSearchParams(location.search).get('trace') === '1';
const APP = !!(OPEN && window.ipc);
const post = (m) => { try { window.ipc.postMessage(JSON.stringify(m)); } catch (_) {} };

// Where an H.264 frame spends its time in this viewer (TRACE only), p50/p95 in ms every 5 s:
//   decode  — handed to its VideoDecoder → that frame out (matched by the chunk's timestamp)
//   draw    — decoded → copied to the visible canvas (noVNC draws it to its back buffer, then
//             a flip copies that to the screen canvas)
//   nextRaf — on the visible canvas → the next animation-frame callback: when the browser
//             next gets to render, a scheduling proxy, not when the glass shows it
// `counts` keeps totals since the start (for tests).
const viewerTrace = (() => {
  const stats = { decode: [], draw: [], nextRaf: [] };
  const counts = { decode: 0, draw: 0, nextRaf: 0 };
  const decodedAt = new WeakMap(); // VideoFrame → when it came out of its decoder
  let since = performance.now();
  const add = (k, ms) => {
    stats[k].push(ms);
    counts[k]++;
    if (performance.now() - since < 5000) return;
    const q = (v, p) => v[Math.min(v.length - 1, Math.round((v.length - 1) * p))];
    const text = Object.entries(stats).filter(([, v]) => v.length).map(([k, v]) => {
      v.sort((a, b) => a - b);
      return `${k} n=${v.length} p50=${q(v, 0.5).toFixed(1)} p95=${q(v, 0.95).toFixed(1)}`;
    }).join(' | ');
    for (const k in stats) stats[k] = [];
    since = performance.now();
    console.log('viewer trace:', text);
    post({ ev: 'trace', text });
  };
  if (TRACE && window.VideoDecoder) {
    const Base = window.VideoDecoder;
    window.VideoDecoder = class extends Base {
      constructor(init) {
        // This decoder's chunks in flight: timestamp → when each was handed over, oldest first
        // (noVNC gives every access unit of one rectangle the same timestamp).
        const submitted = new Map();
        super({ ...init, output: (f) => {
          const q = submitted.get(f.timestamp);
          const t = q && q.shift();
          if (q && !q.length) submitted.delete(f.timestamp);
          if (t !== undefined) {
            const now = performance.now();
            add('decode', now - t);
            decodedAt.set(f, now);
          }
          init.output(f);
        } });
        this.itdaSubmitted = submitted;
      }
      decode(chunk) {
        const s = this.itdaSubmitted;
        if (s.size > 64) s.clear(); // (a decoder error leaves its chunks behind)
        const q = s.get(chunk.timestamp) || [];
        q.push(performance.now());
        s.set(chunk.timestamp, q);
        try {
          return super.decode(chunk);
        } catch (e) {
          q.pop(); // not submitted after all
          if (!q.length) s.delete(chunk.timestamp);
          throw e;
        }
      }
      reset() { this.itdaSubmitted.clear(); return super.reset(); }
      close() { this.itdaSubmitted.clear(); return super.close(); }
    };
  }
  return {
    counts,
    attach(rfb) {
      const d = rfb._display;
      const back = d && d._drawCtx, screen = d && d._targetCtx;
      if (!back || !screen) return;
      // The newest decoded frame drawn to the back buffer and not yet on the visible canvas.
      let pending;
      const toBack = back.drawImage.bind(back);
      back.drawImage = (img, ...a) => {
        const t = decodedAt.get(img);
        if (t !== undefined) pending = t;
        return toBack(img, ...a);
      };
      const toScreen = screen.drawImage.bind(screen);
      screen.drawImage = (img, ...a) => {
        const r = toScreen(img, ...a);
        if (pending !== undefined && img === d._backbuffer) {
          const now = performance.now();
          add('draw', now - pending);
          pending = undefined;
          requestAnimationFrame(() => add('nextRaf', performance.now() - now));
        }
        return r;
      };
    },
  };
})();
const monitors = new MonitorController({S,app:APP,post,api,render:renderTools,toast,close:leave,visibility:()=>syncBackground()});
const opt = (k, dflt) => (S.host && S.host.opts && k in S.host.opts ? S.host.opts[k] : dflt);
// What to call the remote device in messages: the key for a Mac, a PC or another device.
const byDev = (mac, pc, device, vars) => T(isWin(S.host) ? pc : S.host && kindOf(S.host) === 'linux' ? device : mac, vars);

function show(id) {
  for (const s of ['login', 'list', 'view']) $('#' + s).hidden = s !== id;
}

function toast(text, ms = 2600) {
  const t = $('#toast');
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), ms);
}

async function api(path, body) {
  const r = await fetch(path, body === undefined ? { credentials: 'same-origin' } : {
    method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
  });
  const data = await r.json().catch(() => ({}));
  return { ok: r.ok, status: r.status, data };
}

// ---------------------------------------------------------------- sign in & list

// Where passwords end up, for the editor's reassurance line: this Mac's Keychain, or the
// computer running Itda (the Keychain on a Mac, Credential Manager on Windows).
const vaultText = () => T(S.local ? 'viewer.vault.local' : 'viewer.vault.remote');

async function boot() {
  if (MIRROR !== null) { document.body.classList.add('mirror', ...(APP ? ['app'] : [])); return startMirror({S,app:APP,post},Number(MIRROR)); }
  const me = await api('/api/me');
  // Local mode: this page is the Mac app's own window (no PIN, nothing to install).
  S.local = !!(me.data && me.data.local);
  if (me.data && me.data.scroll) S.macScroll = me.data.scroll.mac;
  if (S.local) {
    document.body.classList.add('local');
    $('#logout').hidden = true;
    $('#install').hidden = true;
    $('#hint').hidden = true;
  }
  syncLangPick();
  if (APP && me.ok) return openFromApp();
  if (me.ok) return loadHosts();
  signIn();
}

// The sign-in screen; `why` says why a session that was open needs the PIN again.
function signIn(why = '') {
  S.gen++; // a late answer for an earlier connection must not show over this screen
  close();
  closeEditor();
  status(null);
  tx($('#loginError'), '');
  tx($('#loginNote'), why);
  $('#loginNote').hidden = !why;
  show('login');
  setTimeout(() => $('#pin').focus(), 50);
}
// The gateway forgets a session after 12 hours, when the PIN is renewed and when its Itda
// stops or restarts (the web viewer turned off, the computer restarted).
const expired = () => signIn(() => T('viewer.login.expired'));

// Wrong PINs: 5 from one device lock it out for 1 minute (src/gateway.rs MAX_FAILS, LOCKOUT).
// The lock started at the latest with the first refusal seen here, so 60 s is an upper bound.
const LOCK_SECS = 60;
let lockTimer;
function lockedOut() {
  const btn = $('#loginForm button');
  if (lockTimer) return;
  const until = Date.now() + LOCK_SECS * 1000;
  btn.disabled = true;
  const tick = () => {
    const left = Math.ceil((until - Date.now()) / 1000);
    if (left <= 0) {
      clearInterval(lockTimer); lockTimer = null;
      btn.disabled = false;
      tx($('#loginError'), () => T('viewer.login.unlocked'));
      return;
    }
    tx($('#loginError'), () => T('viewer.login.locked', { count: left }));
  };
  tick();
  lockTimer = setInterval(tick, 1000);
}

$('#loginForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  if (lockTimer) return;
  const r = await api('/api/login', { pin: $('#pin').value });
  if (r.ok) {
    tx($('#loginError'), '');
    $('#loginNote').hidden = true;
    $('#pin').value = '';
    return loadHosts();
  }
  if (r.status === 429) { $('#pin').value = ''; return lockedOut(); }
  tx($('#loginError'), r.status === 401 ? () => T('viewer.login.wrongPin') : r.data.error || (() => T('viewer.login.failed')));
  $('#pin').select();
});

$('#logout').addEventListener('click', async () => {
  await api('/api/logout', {});
  S.signedIn = false;
  signIn();
});
$('#refresh').innerHTML = icon('refresh');
// The language picker (list bar, sign-in card): not in the app's windows, which follow the app,
// nor when the URL sets the language.
const langPicks = [...document.querySelectorAll('.lang select')];
document.querySelectorAll('.lang').forEach((l) => l.insertAdjacentHTML('afterbegin', icon('globe')));
function syncLangPick() {
  const off = !!window.ITDA_LANG || I18N.forced || APP || S.local;
  for (const s of langPicks) { s.closest('.lang').hidden = off; s.value = I18N.pref; }
}
for (const s of langPicks) s.addEventListener('change', () => {
  try { localStorage.setItem(LANG_KEY, s.value); } catch (_) {}
  I18N.setPref(s.value);
  syncLangPick();
});
syncLangPick();
$('#refresh').addEventListener('click', loadHosts);

const isWin = (h) => !!(h && (h.this || h.kind === 'rdp' || h.os === 'windows'));
// Editor kinds (same as the PC's Itda): what to ask for per device.
const KINDS = () => ({
  mac: { kind: 'vnc', os: 'mac', label: 'Mac', user: T('viewer.kind.mac.user'), userHint: T('viewer.kind.mac.userHint'), pass: T('viewer.kind.mac.pass'), port: 5900, ssh: true },
  windows: { kind: 'vnc', os: 'windows', label: 'Windows · Itda', user: null, pass: T('viewer.kind.windows.pass'), passHint: T('viewer.kind.windows.passHint'), port: 5900 },
  rdp: { kind: 'rdp', os: 'windows', label: T('viewer.kind.rdp.label'), user: T('viewer.kind.rdp.user'), userHint: T('viewer.kind.rdp.userHint'), pass: T('viewer.kind.rdp.pass'), passHint: T('viewer.kind.rdp.passHint'), port: 3389 },
  linux: { kind: 'vnc', os: 'linux', label: 'Linux · VNC', user: T('viewer.kind.linux.user'), userHint: T('viewer.kind.linux.userHint'), pass: T('viewer.kind.linux.pass'), port: 5900, ssh: true },
});
const KIND_IDS = ['mac', 'windows', 'rdp', 'linux'];
const kindOf = (h) => (h.kind === 'rdp' ? 'rdp' : KIND_IDS.includes(h.os) ? h.os : 'mac');
const KIND_TEXT = (h) => (h.this ? T('viewer.kindText.this') : kindOf(h) === 'linux' && !h.ssh ? 'Linux · VNC' : T({ mac: h.ssh ? 'viewer.kindText.macSsh' : 'viewer.kindText.mac', windows: 'viewer.kindText.windows', rdp: 'viewer.kindText.rdp', linux: 'viewer.kindText.linuxSsh' }[kindOf(h)]));

async function loadHosts() {
  const [r, a] = await Promise.all([api('/api/hosts'), api('/api/accounts')]);
  if (r.status === 401) return S.signedIn ? expired() : signIn();
  S.signedIn = true;
  S.hosts = r.data || [];
  S.accounts = a.ok ? a.data || [] : [];
  renderHosts();
  renderAccounts();
  show('list');
}

async function openFromApp() {
  document.body.classList.add('app');
  const r = await api('/api/hosts');
  const h = (r.data || []).find((x) => x.id === OPEN);
  if (!h) return post({ ev: 'error', kind: 'other', msg: T('viewer.err.4004.title') });
  document.title = h.label;
  S.fit = !(h.opts && h.opts.actual);
  S.viewOnly = !!(h.opts && h.opts.view_only);
  open(h);
}

function renderHosts() {
  const el = $('#hosts');
  const add = `<button class="host add" data-new="host">${icon('plus')}<b>${T('viewer.hosts.add')}</b><span>${T('viewer.hosts.addSub')}</span></button>`;
  // Previews already shown stay as they are (a new <img> would be decoded again and flash).
  const shown = new Map([...el.querySelectorAll('img')].map((i) => [i.getAttribute('src'), i]));
  el.innerHTML = S.hosts.map((h) => `
      <div class="host${h.has_pw ? '' : ' nopw'}" data-id="${esc(h.id)}">
        <button class="open" aria-label="${T('viewer.hosts.open', { name: esc(h.label) })}">
          <div class="th ${esc(osName(h) || '')}">${osMark(h)}${h.thumb ? `<img alt="" src="thumb/${esc(h.id)}.png?v=${esc(String(h.thumb))}">` : ''}</div>
          <div class="meta"><b>${esc(h.label)}</b>
            <span>${h.ssh ? icon('lock') : ''}${esc(h.this ? T('viewer.hosts.thisPc') : h.user ? `${h.user}@${h.host}` : h.host)}</span>
            ${h.has_pw ? `<span class="kind">${esc(KIND_TEXT(h))}</span>` : `<span class="warn">${T('viewer.hosts.needPw')}</span>`}</div>
        </button>
        ${h.this ? '' : `<button class="more" data-edit="${esc(h.id)}" title="${T('viewer.hosts.edit')}" aria-label="${T('viewer.hosts.editName', { name: esc(h.label) })}">${icon('edit')}</button>`}
      </div>`).join('') + add;
  el.querySelectorAll('img').forEach((img) => {
    const old = shown.get(img.getAttribute('src'));
    if (old) img.replaceWith(old);
    else img.addEventListener('error', () => img.remove());
  });
}

function renderAccounts() {
  const el = $('#accounts');
  el.innerHTML = `<p class="lead">${T('viewer.acct.lead')}</p>` +
    S.accounts.map((a) => `
      <button class="acct" data-acct="${esc(a.id)}">
        <span class="av">${esc((a.name || a.user || '?').slice(0, 1).toUpperCase())}</span>
        <span class="who"><b>${esc(a.name || a.user)}</b><small>${esc(a.user)} · ${T('viewer.acct.uses', { count: a.uses })}${a.has_pw ? '' : ` · ${T('viewer.acct.noPw')}`}</small></span>
        ${icon('edit')}
      </button>`).join('') +
    `<button class="acct add" data-new="account">${icon('plus')}<b>${T('viewer.acct.add')}</b></button>`;
}

document.querySelector('.tabs').addEventListener('click', (e) => {
  const t = e.target.closest('[data-tab]');
  if (!t) return;
  document.querySelectorAll('.tabs [data-tab]').forEach((b) => b.setAttribute('aria-selected', String(b === t)));
  $('#hosts').hidden = t.dataset.tab !== 'hosts';
  $('#accounts').hidden = t.dataset.tab !== 'accounts';
});

$('#hosts').addEventListener('click', (e) => {
  const ed = e.target.closest('[data-edit]');
  if (ed) return editHost(S.hosts.find((h) => h.id === ed.dataset.edit));
  if (e.target.closest('[data-new]')) return editHost(null);
  const card = e.target.closest('.host');
  if (!card) return;
  const h = S.hosts.find((x) => x.id === card.dataset.id);
  if (h && !h.has_pw) return editHost(h, T('viewer.hosts.enterPw'));
  open(h);
});

$('#accounts').addEventListener('click', (e) => {
  const a = e.target.closest('[data-acct]');
  if (a) return editAccount(S.accounts.find((x) => x.id === a.dataset.acct));
  if (e.target.closest('[data-new]')) editAccount(null);
});

// ---------------------------------------------------------------- editing (saved on the PC)

const field = (label, input, hint = '') => `<label class="f"><span>${label}</span>${input}${hint ? `<small>${hint}</small>` : ''}</label>`;

function editor(html) {
  $('#edForm').innerHTML = html;
  $('#editor').hidden = false;
  const first = $('#edForm').querySelector('input:not([type=radio]):not([type=checkbox])');
  if (first && matchMedia('(pointer: fine)').matches) first.focus();
}

function closeEditor() {
  $('#editor').hidden = true;
  $('#edForm').innerHTML = '';
  $('#edForm').onsubmit = null;
}

$('#editor').addEventListener('click', (e) => {
  if (e.target === $('#editor') || e.target.closest('[data-a=cancel]')) closeEditor();
});
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !$('#editor').hidden) closeEditor(); });

async function saveEdit(body, after) {
  const btn = $('#edForm').querySelector('button[type=submit]');
  if (btn) btn.disabled = true;
  const r = await api('/api/edit', body);
  if (btn) btn.disabled = false;
  if (r.status === 401) return expired();
  if (!r.ok) { $('#edErr').textContent = r.data.error || T('viewer.ed.saveFailed'); return false; }
  S.hosts = r.data.hosts || S.hosts;
  S.accounts = r.data.accounts || S.accounts;
  renderHosts();
  renderAccounts();
  closeEditor();
  if (after) after(r.data.id);
  return true;
}

function editHost(h, note = '', reopen = false) {
  const isNew = !h;
  h = h || { kind: 'vnc', os: 'mac', account: '', name: '', host: '', own_user: '', group: '', ssh: false };
  const acctOpts = `<option value="">${T('viewer.ed.ownAccount')}</option>` + S.accounts.map((a) => `<option value="${esc(a.id)}" ${a.id === h.account ? 'selected' : ''}>${esc(a.name || a.user)} (${esc(a.user)})</option>`).join('');
  const savedPw = !isNew && h.has_pw && !h.account;
  editor(`
    <h2 id="edTitle">${T(isNew ? 'viewer.hosts.add' : 'viewer.ed.editHost')}</h2>
    ${note ? `<p class="note">${esc(note)}</p>` : ''}
    ${isNew ? `<div id="nearby" class="nearby"><span class="muted">${T('viewer.nearby.searching')}</span></div>` : ''}
    <div class="seg four" role="radiogroup" aria-label="${T('viewer.ed.kind')}">
      ${Object.entries(KINDS()).map(([k, d]) => `<label><input type="radio" name="kind" value="${k}" ${kindOf(h) === k ? 'checked' : ''}><span>${d.label}</span></label>`).join('')}
    </div>
    ${field(T('viewer.ed.address'), `<input name="host" required value="${esc(h.host)}" placeholder="${T('viewer.ed.addressPh')}" autocapitalize="off" spellcheck="false">`, '<span id="portHint"></span>')}
    ${field(T('viewer.ed.name'), `<input name="name" value="${esc(h.name)}" placeholder="${T('viewer.ed.namePh')}">`)}
    ${field(T('viewer.ed.login'), `<select name="account">${acctOpts}</select>`)}
    <div data-own>
      <div id="userBox">${field('<span id="userLabel"></span>', `<input name="user" value="${esc(h.own_user)}" autocapitalize="off" spellcheck="false">`, '<span id="userHint"></span>')}</div>
      ${field('<span id="passLabel"></span>', `<input name="password" type="password" autocomplete="new-password" placeholder="${savedPw ? T('viewer.ed.pwSaved') : ''}">`, '<span id="passHint"></span>')}
    </div>
    <label class="check" id="sshBox"><input type="checkbox" name="ssh" ${h.ssh ? 'checked' : ''}><span>${T('viewer.ed.ssh')}<small>${T('viewer.ed.sshHint')}</small></span></label>
    ${field(T('viewer.ed.group'), `<input name="group" value="${esc(h.group)}" placeholder="${T('viewer.ed.groupPh')}">`)}
    <p id="edErr" class="error" role="alert"></p>
    <div class="ed-actions">
      ${isNew ? '' : `<button type="button" class="danger" data-a="delete">${T('viewer.ed.delete')}</button>`}
      <span class="grow"></span>
      <button type="button" data-a="cancel">${T('viewer.ed.cancel')}</button>
      <button type="submit" class="primary">${T(isNew ? 'viewer.ed.add' : 'viewer.ed.save')}</button>
    </div>`);
  const f = $('#edForm');
  const sync = () => {
    const d = KINDS()[f.kind.value];
    $('#portHint').textContent = T('viewer.ed.portHint', { port: d.port });
    $('#userBox').hidden = !d.user;
    $('#userLabel').textContent = d.user || '';
    $('#userHint').textContent = d.userHint || '';
    $('#passLabel').textContent = d.pass;
    $('#passHint').textContent = d.passHint || T(S.local ? 'viewer.ed.pwStoredLocal' : 'viewer.ed.pwStoredRemote');
    if (!savedPw) f.password.placeholder = d.pass;
    $('#sshBox').hidden = !d.ssh;
    f.querySelector('[data-own]').hidden = !!f.account.value;
  };
  f.addEventListener('change', sync);
  sync();
  if (isNew) showNearby(f, sync);
  f.querySelector('[data-a=delete]')?.addEventListener('click', async () => {
    if (!confirm(T('viewer.ed.deleteHost', { name: h.label }))) return;
    await saveEdit({ cmd: 'delete_host', id: h.id });
  });
  f.onsubmit = async (e) => {
    e.preventDefault();
    const d = KINDS()[f.kind.value];
    const body = {
      cmd: 'save_host', id: h.id || '', kind: d.kind, os: d.os, host: f.host.value, name: f.name.value, group: f.group.value,
      account: f.account.value, user: d.user ? f.user.value : '', password: f.account.value ? '' : f.password.value, ssh: !!d.ssh && f.ssh.checked,
    };
    const wantOpen = reopen || (!h.has_pw && (body.password || body.account));
    await saveEdit(body, (id) => {
      const saved = S.hosts.find((x) => x.id === id);
      if (saved && wantOpen && saved.has_pw) open(saved);
      else toast(T(isNew ? 'viewer.toast.added' : 'viewer.toast.saved'));
    });
  };
}

// "기기 추가": devices found on this network (Bonjour) and on Tailscale, one tap to fill in.
async function showNearby(f, sync) {
  const box = $('#nearby');
  const r = await api('/api/nearby');
  if (!box.isConnected) return;
  const list = r.ok ? r.data || [] : [];
  if (!list.length) { box.innerHTML = `<span class="muted">${T('viewer.nearby.none')}</span>`; return; }
  const OS = { mac: 'Mac', windows: 'Windows', linux: 'Linux' };
  box.innerHTML = `<b>${T('viewer.nearby.title')}</b><div class="chips">${list.map((d, i) =>
    `<button type="button" class="chip ${esc(osName(d) || '')}" data-i="${i}">${osMark(d)}<span>${esc(d.name)}<small>${OS[d.os] || ''}${d.kind === 'rdp' ? ` · ${T('viewer.kind.rdp.label')}` : ''}${d.via === 'tailscale' ? ' · Tailscale' : ''}${d.off ? ` · ${T('viewer.nearby.off')}` : ''}</small></span></button>`).join('')}</div>`;
  box.onclick = (e) => {
    const c = e.target.closest('[data-i]');
    if (!c) return;
    const d = list[+c.dataset.i];
    const kind = d.kind === 'rdp' ? 'rdp' : d.os;
    const radio = f.querySelector(`[name=kind][value=${kind}]`);
    if (radio) radio.checked = true;
    f.host.value = d.host;
    if (!f.name.value) f.name.value = d.name;
    box.querySelectorAll('.chip').forEach((x) => x.classList.toggle('on', x === c));
    sync();
    (f.user && !$('#userBox').hidden ? f.user : f.password).focus();
  };
}

function editAccount(a, after) {
  const isNew = !a;
  a = a || { name: '', user: '', ssh_key: '', uses: 0 };
  editor(`
    <h2 id="edTitle">${T(isNew ? 'viewer.acct.add' : 'viewer.acct.edit')}</h2>
    ${field(T('viewer.acct.user'), `<input name="user" required value="${esc(a.user)}" autocapitalize="off" spellcheck="false" placeholder="${T('viewer.acct.userPh')}">`)}
    ${field(T('viewer.acct.name'), `<input name="name" value="${esc(a.name)}" placeholder="${T('viewer.acct.namePh')}">`)}
    ${field(T('viewer.acct.pw'), `<input name="password" type="password" autocomplete="new-password" placeholder="${a.has_pw ? T('viewer.ed.pwSaved') : T('viewer.acct.pw')}">`, T('viewer.acct.pwStored', { vault: vaultText() }))}
    ${a.ssh_key ? `<p class="note">${T(S.local ? 'viewer.acct.sshKey' : 'viewer.acct.sshKeyRemote', { key: esc(a.ssh_key) })}</p>` : ''}
    ${isNew ? '' : `<p class="note">${T('viewer.acct.usedBy', { count: a.uses })}</p>`}
    <p id="edErr" class="error" role="alert"></p>
    <div class="ed-actions">
      ${isNew ? '' : `<button type="button" class="danger" data-a="delete">${T('viewer.ed.delete')}</button>`}
      <span class="grow"></span>
      <button type="button" data-a="cancel">${T('viewer.ed.cancel')}</button>
      <button type="submit" class="primary">${T(isNew ? 'viewer.ed.add' : 'viewer.ed.save')}</button>
    </div>`);
  const f = $('#edForm');
  f.querySelector('[data-a=delete]')?.addEventListener('click', async () => {
    if (!confirm(T('viewer.acct.deleteConfirm', { name: a.name || a.user }))) return;
    await saveEdit({ cmd: 'delete_account', id: a.id });
  });
  f.onsubmit = async (e) => {
    e.preventDefault();
    await saveEdit({ cmd: 'save_account', id: a.id || '', user: f.user.value, name: f.name.value, password: f.password.value }, () => (after ? after() : toast(T(isNew ? 'viewer.toast.accountAdded' : 'viewer.toast.saved'))));
  };
}

// ---------------------------------------------------------------- install as an app

let installEvt = null;
const IOS = /iP(hone|ad|od)/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
const STANDALONE = matchMedia('(display-mode: standalone)').matches || navigator.standalone === true;
const remembered = (k) => { try { return localStorage.getItem(k) === '1'; } catch (_) { return false; } };
const remember = (k) => { try { localStorage.setItem(k, '1'); } catch (_) {} };

if ('serviceWorker' in navigator && window.isSecureContext) navigator.serviceWorker.register('sw.js').catch(() => {});

window.addEventListener('beforeinstallprompt', (e) => {
  e.preventDefault();
  installEvt = e;
  $('#install').hidden = false;
});
window.addEventListener('appinstalled', () => { $('#install').hidden = true; toast(T('viewer.toast.installed')); });
$('#install').addEventListener('click', async () => {
  if (!installEvt) return;
  installEvt.prompt();
  await installEvt.userChoice.catch(() => {});
  installEvt = null;
  $('#install').hidden = true;
});

function installHint() {
  if (STANDALONE || remembered('itda.hint')) return;
  const tip = IOS
    ? T('viewer.hint.ios')
    : !window.isSecureContext
      ? T('viewer.hint.https')
      : '';
  if (!tip) return;
  $('#hint').innerHTML = `<span>${esc(tip)}</span><button class="ghost" aria-label="${T('viewer.close')}">✕</button>`;
  $('#hint').hidden = false;
  $('#hint button').onclick = () => { $('#hint').hidden = true; remember('itda.hint'); };
}
installHint();

// ---------------------------------------------------------------- viewer

// Close codes from the gateway → [title, advice]; functions see the host.
const CLOSE_TEXT = {
  4004: () => [T('viewer.err.4004.title'), T('viewer.err.4004.text')],
  4001: () => [T('viewer.err.4001.title'), T('viewer.err.4001.text')],
  4010: (h) => [byDev('viewer.err.4010.title.mac', 'viewer.err.4010.title.pc', 'viewer.err.4010.title.device'), T(h.this || (h.os === 'windows' && h.kind !== 'rdp') ? 'viewer.err.4010.itda' : h.kind === 'rdp' ? 'viewer.err.4010.rdp' : kindOf(h) === 'linux' ? 'viewer.err.4010.linux' : 'viewer.err.4010.mac')],
  4011: () => [T('viewer.err.4011.title'), T('viewer.err.4011.text')],
  4012: (h) => [byDev('viewer.err.4012.title.mac', 'viewer.err.4012.title.pc', 'viewer.err.4012.title.device'), T(h.kind === 'rdp' ? 'viewer.err.4012.rdp' : h.this ? 'viewer.err.4012.this' : { windows: 'viewer.err.4012.windows', linux: 'viewer.err.4012.linux' }[kindOf(h)] || 'viewer.err.4012.mac')],
  4013: () => [byDev('viewer.err.4013.title.mac', 'viewer.err.4013.title.pc', 'viewer.err.4013.title.device'), T('viewer.err.4013.text')],
  4014: () => [T('viewer.err.4014.title'), T('viewer.err.4014.text')],
  4020: (h) => [T('viewer.err.4020.title'), T(kindOf(h) === 'mac' ? 'viewer.err.4020.mac' : 'viewer.err.4020.other')],
  4021: () => [T('viewer.err.4021.title'), T('viewer.err.4021.text')],
  4022: () => [T('viewer.err.4022.title'), T('viewer.err.4022.text')],
};
// Failures that the same saved login can't fix: offer to correct it instead of only retrying
// (the editor never shows a saved password; it only replaces it).
const FIX_LOGIN = new Set([4001, 4010, 4011, 4020]);

// `title` and `text` may be functions (the card follows a language change).
function status(kind, title, text, code = 0) {
  const st = $('#status');
  S.status = kind ? [kind, title, text, code] : null;
  if (!kind) { st.hidden = true; return; }
  st.hidden = false;
  if (typeof title === 'function') title = title();
  if (typeof text === 'function') text = text();
  const fix = !APP && FIX_LOGIN.has(code) && S.host && !S.host.this;
  const acct = fix && S.host.account && S.accounts.find((a) => a.id === S.host.account);
  st.innerHTML = kind === 'busy'
    ? `<div class="card" role="status"><div class="spin"></div><h2>${esc(title)}</h2><p>${esc(text || '')}</p></div>`
    : `<div class="card" role="alert"><h2>${esc(title)}</h2><p>${esc(text || '')}</p>
        ${fix && acct ? `<p class="small">${T('viewer.status.sharedAccount', { name: esc(acct.name || acct.user), count: acct.uses })}</p>` : ''}
        <div class="row">${fix ? `<button class="go" data-a="fix">${T(acct ? 'viewer.status.fixAccount' : code === 4011 ? 'viewer.status.enterUser' : code === 4001 ? 'viewer.status.enterPw' : 'viewer.status.fixLogin')}</button>` : ''}
          <button data-a="list">${T(APP ? 'viewer.close' : 'viewer.status.list')}</button>
          <button ${fix ? '' : 'class="go" '}data-a="retry">${T(fix && code !== 4001 ? 'viewer.status.retrySame' : 'viewer.status.reconnect')}</button></div></div>`;
  st.querySelector('button.go')?.focus();
}

$('#status').addEventListener('click', (e) => {
  const a = e.target.closest('[data-a]');
  if (!a) return;
  const h = S.host;
  if (a.dataset.a === 'retry') open(h);
  else if (a.dataset.a === 'fix') {
    const acct = h.account && S.accounts.find((x) => x.id === h.account);
    if (acct) editAccount(acct, () => open(S.hosts.find((x) => x.id === h.id) || h));
    else editHost(h, T('viewer.status.pwHidden'), true);
  }
  else if (APP) post({ ev: 'close' });
  else leave();
});

// Close codes → the app's error kinds (the same words the Windows viewer reports).
const KIND_OF = { 4010: 'auth', 4011: 'need_user', 4012: 'refused', 4013: 'unreachable', 4014: 'dns', 4020: 'ssh_auth', 4021: 'ssh_hostkey', 4022: 'ssh_refused', 4023: 'ssh_missing' };
const RETRIES = 10;

// High-performance video this browser can't decode (some Mac's encoder output it rejects) left
// the screen black until that Mac gave up on the frames nobody acknowledged (30 s). Instead,
// reconnect this window at once through Screen Sharing; the saved choice stays on, and the
// mode button tries high-performance again.
if (window.VideoDecoder) {
  const Base = window.VideoDecoder;
  window.VideoDecoder = class extends Base {
    constructor(init) {
      // noVNC makes a connection's decoder while that connection is the current one, and may
      // leave it alive past it: a late error from an old one is not this connection's.
      const owner = S.rfb;
      super({ ...init, error: (e) => { try { hpUndecodable(owner, e); } finally { init.error(e); } } });
    }
  };
}
function hpUndecodable(owner, e) {
  const h = S.host;
  if (!S.rfb || owner !== S.rfb || !h?.hp?.on || S.hpMode === 'fallback' || S.hpNoDecode === h.id) return;
  console.warn('high-performance video could not be decoded:', e?.message || e);
  S.hpNoDecode = h.id;
  toast(T('viewer.toast.hpUndecodable'));
  open(h);
}

function open(h) {
  if (!h) return;
  close();
  const gen = ++S.gen;
  S.host = h;
  S.audio = !!opt('audio', false);
  show('view');
  renderTools();
  const tries = S.tries;
  status('busy', () => (tries ? T('viewer.status.reconnecting', { name: h.label, n: tries, max: RETRIES }) : T('viewer.status.connecting', { name: h.label })), () => (h.ssh ? T('viewer.status.openingSsh') : h.kind === 'rdp' ? T('viewer.status.rdpLogin') : ''));
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  // Remote Desktop follows this window's size (not on phones: a desktop that small is no use).
  const follow = h.kind === 'rdp' && opt('resize', true) && !opt('monitor_layout', 0) && opt('monitor_mode',0)!==2 && (APP || matchMedia('(pointer: fine)').matches);
  const scr = $('#screen');
  const size = follow && scr.clientWidth ? `w=${Math.round(scr.clientWidth)}&h=${Math.round(scr.clientHeight)}&` : '';
  // This attempt's own token: its high-performance outcome, not another viewer's.
  const attempt = Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('');
  S.hpFallback = null; S.hpFallbackSetup = false; S.hpActual = false; S.hpActions = false; S.hpQuality = null; S.hpStatusReq = null; S.hpMode = null; S.hpModeContext = null; S.hpCurtain = undefined; S.hpCurtainAsked = false; S.hpQualityFailed = false; S.hpQualityPending = false; renderTools();
  const nohp = S.hpNoDecode === h.id ? 'nohp=1&' : '';
  const ws = new WebSocket(`${proto}//${location.host}/ws/${encodeURIComponent(h.id)}?${size}${nohp}attempt=${attempt}`, ['binary']);
  let closeInfo = null;
  ws.addEventListener('close', (e) => { closeInfo = e; });
  S.ws = ws;
  const rfb = new window.RFB($('#screen'), ws, { shared: true });
  S.rfb = rfb;
  if (TRACE) viewerTrace.attach(rfb);
  monitors.attach(rfb);
  rfb.scaleViewport = S.fit;
  rfb.clipViewport = !S.fit;
  rfb.resizeSession = follow;
  rfb.dragViewport = false;
  rfb.viewOnly = S.viewOnly;
  rfb.focusOnClick = true;
  rfb.showDotCursor = true;
  rfb.qualityLevel = 7;
  // Sound remembered on: asked for from the start (the browser may hold it until a click).
  if (S.audio) rfb.itdaAudio?.(true);
  // Every key (this window's, a monitor window's) leaves through sendKey: the Mac modifiers go
  // as Itda's server reads them, unless this connection is known to have fallen back to Screen
  // Sharing (before the mode is known too: the first ⌘C mustn't go as ⌥C).
  const sendKey = rfb.sendKey?.bind(rfb);
  if (sendKey) rfb.sendKey = hpKeys(sendKey, () => S.rfb === rfb && macHost(S.host) && (S.hpActual || (!!S.host.hp?.on && S.hpMode !== 'fallback')));
  let name = '';
  rfb.addEventListener('desktopname', (e) => { name = (e.detail && e.detail.name) || ''; });
  rfb.addEventListener('connect', () => {
    status(null);
    rfb.focus();
    flushPaste();
    syncBackground();
    dimToolsSoon();
    // High-performance mode is the saved choice; say so when this connection didn't get it.
    if (APP && S.host?.hp?.on) refreshHpMode(h, rfb, attempt);
    // A browser has no mode button, but its keys still depend on what this connection got.
    // Screen Sharing standing in for a locked Mac switches to high-performance once it's
    // unlocked, as in the app.
    else if (S.host?.hp?.on && macHost(S.host)) fetch(`/api/mode/${encodeURIComponent(h.id)}?attempt=${attempt}`).then((r) => r.json()).then((m) => {
      if (S.rfb !== rfb || typeof m?.hp !== 'boolean') return;
      S.hpMode = m.hp ? 'active' : 'fallback';
      if (!m.hp && m.locked) {
        const context = { h, rfb, attempt, otherUser: !!m.other_user };
        S.hpModeContext = context;
        toast(unavailableText(context));
        watchUnlock(context);
      }
    }).catch(() => {});
    if (!APP) return toast(T('viewer.toast.connected', { name: h.label }));
    post(S.tries ? { ev: 'reconnected' } : { ev: 'connected', name, w: rfb._fbWidth || 0, h: rfb._fbHeight || 0 });
    S.ever = true;
    S.tries = 0;
    thumbSoon(2500);
  });
  rfb.addEventListener('pointermotion', (e) => {
    rel.hidden = e.detail.hidden;
    if (rel.hidden && !rel.toasted && !relLocked() && !S.viewOnly && relAvailable()) { rel.toasted = true; toast(T('viewer.toast.pointerHidden')); }
    if (!rel.hidden && rel.auto && relLocked()) mouseLock(false);
    renderTools();
  });
  rfb.addEventListener('disconnect', () => {
    rel.hidden = rel.toasted = false;
    if (S.rfb !== rfb) return;
    S.rfb = null;
    S.hpActual = false; S.hpActions = false; S.hpQuality = null; S.hpStatusReq = null; S.hpModeContext = null; S.hpMode = null; S.hpCurtain = undefined; S.hpCurtainAsked = false; renderTools();
    // Only for the attempt still current: a replacement opened meanwhile (the HP toggle, a
    // quality change) owns the window now.
    if (APP) return setTimeout(() => { if (S.gen === gen && !lockedDrop(h, closeInfo)) appDropped(h, closeInfo, gen); }, 50);
    setTimeout(async () => {
      if (S.gen !== gen || lockedDrop(h, closeInfo)) return;
      const code = closeInfo ? closeInfo.code : 0;
      const known = CLOSE_TEXT[code];
      const reason = closeInfo && closeInfo.reason;
      const say = () => (known ? known(h) : [T('viewer.err.lost.title'), reason || T('viewer.err.lost.text')]);
      status('error', () => say()[0], () => say()[1], code);
      if (known) return;
      // The gateway refuses a WebSocket without a live session before it opens (no close code):
      // ask, but don't wait long on a network that is gone.
      const me = await Promise.race([api('/api/me').catch(() => ({})), new Promise((r) => setTimeout(() => r({}), 5000))]);
      if (S.gen === gen && me.status === 401) expired();
    }, 50);
  });
  rfb.addEventListener('clipboard', (e) => {
    S.clip = e.detail.text || '';
    if (APP) { if (opt('clipboard', true)) post({ ev: 'clip', text: S.clip }); return; }
    toast(byDev('viewer.toast.clipReceived.mac', 'viewer.toast.clipReceived.pc', 'viewer.toast.clipReceived.device'));
  });
}

// That Mac's screen locked during high-performance sharing, which then shows only the lock
// screen's picture: the gateway says so (LOCKED_CLOSE). Reconnect at once; the Mac refuses
// high-performance while locked, so this lands on Screen Sharing, where it can be unlocked
// (and refreshHpMode/watchUnlock switch back once it is). Once per 15 s, so a Mac that keeps
// ending sessions this way gets the usual drop handling instead of a loop.
const LOCKED_CLOSE = 4030;
function lockedDrop(h, info) {
  if (info?.code !== LOCKED_CLOSE || !h?.hp?.on || (S.lockDropAt != null && Date.now() - S.lockDropAt < 15000)) return false;
  S.lockDropAt = Date.now();
  toast(info.reason || T('viewer.toast.lockedDrop'));
  open(h);
  return true;
}

// App window: a first attempt that fails goes back to the home (which explains it); a drop
// after that reconnects on its own, like the Windows viewer.
function appDropped(h, info, gen) {
  const code = info ? info.code : 0;
  const msg = (info && info.reason) || T('viewer.app.lost');
  if (!S.ever) return post({ ev: 'error', kind: KIND_OF[code] || 'other', msg });
  if (opt('reconnect', true) && S.tries < RETRIES && !KIND_OF[code]?.includes('auth')) {
    S.tries += 1;
    post({ ev: 'reconnecting', n: S.tries });
    const tries = S.tries;
    status('busy', () => T('viewer.status.dropped', { name: h.label }), () => T('viewer.status.retryIn', { count: Math.min(tries * 2, 10), n: tries, max: RETRIES }));
    return setTimeout(() => { if (S.gen === gen && !S.rfb) open(h); }, Math.min(S.tries * 2000, 10000));
  }
  post({ ev: 'error', kind: 'lost', msg });
}

// The card picture in the app's address book: a small copy of the screen now and then.
let thumbTimer;
let lastThumb = '';
function thumbSoon(ms) {
  clearTimeout(thumbTimer);
  thumbTimer = setTimeout(() => {
    const src = $('#screen canvas');
    if (S.rfb && src && src.width) {
      const k = Math.min(1, 480 / src.width);
      const c = document.createElement('canvas');
      c.width = Math.round(src.width * k);
      c.height = Math.round(src.height * k);
      c.getContext('2d').drawImage(src, 0, 0, c.width, c.height);
      try {
        // An unchanged screen isn't sent again (the app would rewrite the file and redraw).
        const png = c.toDataURL('image/png');
        if (png !== lastThumb) post({ ev: 'thumb', png });
        lastThumb = png;
      } catch (_) {}
    }
    if (S.rfb) thumbSoon(60000);
  }, ms);
}

// Text copied on the Mac, handed over by the app when this window comes forward.
// Text copied on this device (the app offers it when the window comes forward). Kept until the
// connection is up, and held a moment after for the server's Extended Clipboard offer: without
// it noVNC sends Latin-1 only, and Korean would arrive as '?'.
window.itdaPaste = (t) => { if (t) { S.paste = t; flushPaste(); } };
function flushPaste(waited = 0) {
  const r = S.rfb;
  if (!S.paste || !r || r._rfbConnectionState !== 'connected') return;
  if (S.viewOnly || !opt('clipboard', true)) { S.paste = null; return; }
  if (!r._clipboardServerCapabilitiesFormats?.[1] && waited < 1500) { clearTimeout(S.pasteTimer); S.pasteTimer = setTimeout(() => S.rfb === r && flushPaste(waited + 100), 100); return; }
  r.clipboardPasteFrom(S.paste); S.paste = null;
}

// One WebSocket packet per wheel event; fractional motion is shared with mirrors.
const wheel = new WheelInput();
const BTN = (b) => (b & 1) | ((b & 4) >> 1) | ((b & 2) << 1); // DOM buttons → RFB mask
function onWheel(e) {
  const rfb = S.rfb;
  if (!rfb || !rfb._canvas || !rfb._canvas.contains(e.target) || relLocked()) return;
  e.preventDefault();
  e.stopPropagation(); // noVNC's own handler sits on the canvas
  if (S.viewOnly || e.ctrlKey) return; // Ctrl + wheel is a pinch: kept here
  const r = rfb._canvas.getBoundingClientRect();
  const x = Math.max(0, Math.min(r.width - 1, e.clientX - r.left));
  const y = Math.max(0, Math.min(r.height - 1, e.clientY - r.top));
  rfb.itdaWheel(x,y,BTN(e.buttons),wheel.clicks(e,macHost(S.host),S.macScroll,rfb._canvas.clientHeight));
}
$('#screen').addEventListener('wheel', onWheel, { capture: true, passive: false });

// noVNC turns a pinch into Ctrl + mouse wheel on the remote device, which zooms every app
// there (and can leave Ctrl held). Keep pinches here: spread = 1:1, pinch in = fit.
let pinchFrom = 0;
const pinchSize = (d) => Math.hypot(d.magnitudeX || 0, d.magnitudeY || 0);
for (const t of ['gesturestart', 'gesturemove', 'gestureend']) {
  $('#screen').addEventListener(t, (e) => {
    if (!e.detail || e.detail.type !== 'pinch') return;
    e.stopPropagation();
    if (t === 'gesturestart') pinchFrom = pinchSize(e.detail);
    if (t !== 'gestureend' || !S.rfb) return;
    const grow = pinchSize(e.detail) / Math.max(1, pinchFrom);
    if ((grow > 1.25 && S.fit) || (grow < 0.8 && !S.fit)) $('#tools [data-t=scale]')?.click();
  }, true);
}

function close(notifyNative=true) {
  window.ItdaHpSetup?.close(); S.hpBusy=false;
  wheel.reset();
  monitors.disconnect(notifyNative);
  if (S.rfb) { const r = S.rfb; S.rfb = null; try { r.disconnect(); } catch (_) {} }
  S.mods.clear();
  $('#sheet').hidden = true;
}

function leave() {
  S.gen++;
  close();
  status(null);
  loadHosts();
}

// ---------------------------------------------------------------- high-performance mode

// A Mac connected the usual way (Apple's Screen Sharing: ZRLE, ~8 fps at 1080p) can move to
// Itda's own sharing on that Mac (hardware H.264): the app sets it up over SSH, the person
// answers macOS's permission prompts on that Mac through this very connection, and then this
// window reconnects to it. If it can't be reached, the app falls back to Screen Sharing.
const hpCapable = () => APP && macHost(S.host) && S.host?.kind !== 'rdp';
// A saved preference is not evidence of this connection's transport. Retrying this
// diagnostic must keep the framebuffer, monitor windows and input session intact.
function refreshHpMode(h, rfb, attempt) {
  if (S.rfb !== rfb || S.host?.id !== h.id) return;
  const context = { h, rfb, attempt };
  S.hpModeContext = context;
  S.hpMode = 'checking';
  renderTools();
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 8000);
  fetch(`/api/mode/${encodeURIComponent(h.id)}?attempt=${attempt}`, { signal: controller.signal }).then(r => {
    if (!r.ok) throw Error('mode unavailable');
    return r.json();
  }).then(m => {
    if (S.hpModeContext !== context || S.rfb !== rfb || S.host?.id !== h.id) return;
    if (typeof m?.hp !== 'boolean') throw Error('mode unknown');
    S.hpMode = m.hp ? 'active' : 'fallback';
    S.hpActual = m.hp;
    S.hpFallback = m.hp ? null : m.reason || T('viewer.hp.connectFailed');
    S.hpFallbackSetup = !m.hp && !!m.setup;
    S.hpQuality = null;
    renderTools();
    if (m.hp) {
      requestHpQuality();
    } else if (m.locked) {
      context.otherUser = !!m.other_user;
      toast(unavailableText(context));
      watchUnlock(context);
    } else {
      toast(T('viewer.hp.fellBack', { reason: S.hpFallback }));
    }
  }).catch(() => {
    if (S.hpModeContext !== context || S.rfb !== rfb || S.host?.id !== h.id) return;
    S.hpMode = 'unknown';
    S.hpActual = false;
    renderTools();
    toast(T('viewer.hp.modeUnknown'));
  }).finally(() => clearTimeout(timeout));
}
// Screen Sharing stood in because that Mac was locked: once it's unlocked there (often through
// this very window), switch to high-performance. The app keeps this Screen Sharing session a
// few seconds past the switch, so the Mac doesn't lock again before the new connection is in.
// Asks every 4 s for the first 15 minutes, then every 30 s for as long as this window stays on it
// (each ask is an SSH login there).
function watchUnlock(context, tries = 0) {
  const live = () => S.hpModeContext === context && S.rfb === context.rfb && S.host?.id === context.h.id && S.host.hp?.on && S.hpMode === 'fallback';
  if (!live()) return;
  setTimeout(() => {
    if (!live()) return;
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 12000);
    fetch(`/api/hp-ready/${encodeURIComponent(context.h.id)}`, { signal: controller.signal }).then((r) => r.json()).then((m) => {
      if (!live()) return;
      if (!m?.ready) return watchUnlock(context, tries + 1);
      S.hpFallback = null; S.hpFallbackSetup = false;
      toast(T(context.otherUser ? 'viewer.hp.screenBack' : 'viewer.hp.unlocked'));
      S.unlockSwitchAt = Date.now();
      open(S.host);
    }).catch(() => watchUnlock(context, tries + 1)).finally(() => clearTimeout(timeout));
  }, tries < 225 ? 4000 : 30000);
}
function unavailableText(context) {
  return T(context.otherUser ? 'viewer.hp.otherUser' : 'viewer.hp.locked');
}
function requestHpQuality() {
  if (!S.rfb || !S.hpActual || !S.hpModeContext || (S.hpQualityPending && !S.hpQualityFailed)) return;
  S.hpQualityPending = true;
  const nonce = Array.from(crypto.getRandomValues(new Uint8Array(16)), b => b.toString(16).padStart(2, '0')).join('');
  S.hpStatusReq = `${S.hpModeContext.attempt}:${nonce}`;
  const req = S.hpStatusReq;
  S.hpQualityFailed = false;
  S.hpStatusTries = 0;
  setTimeout(() => {
    if (S.hpStatusReq === req && S.rfb && !S.hpQuality) { S.hpQualityFailed = true; S.hpQualityPending = false; renderTools(); }
  }, 13000);
  post({ ev: 'hp-status', req: S.hpStatusReq });
  renderTools();
}
function hpModeLabel() {
  if (S.host.hp?.on && !S.hpModeContext) return T('viewer.hp.label.enabled');
  if (S.host.hp?.on && S.hpMode === 'checking') return T('viewer.hp.label.checking');
  if (S.host.hp?.on && (!S.hpMode || S.hpMode === 'unknown')) return T('viewer.hp.label.unknown');
  if (S.host.hp?.on && S.hpFallback) return T(S.hpFallbackSetup ? 'viewer.hp.label.fallbackSetup' : 'viewer.hp.label.fallbackRetry', { reason: S.hpFallback });
  if (S.host.hp?.on && S.hpActual) return T('viewer.hp.label.on');
  return T(S.hpBusy ? 'viewer.hp.label.preparing' : 'viewer.hp.label.off');
}
function hpDisable() {
  if (!hpCapable() || S.hpBusy || !S.host.hp?.on) return;
  S.host.hp = { ...S.host.hp, on: false };
  post({ ev: 'hp-off' });
  toast(T('viewer.hp.turnedOff'));
  return open(S.host);
}
function hpToggle() {
  if (!hpCapable() || S.hpBusy) return;
  if (S.host.hp?.on && S.hpModeContext?.rfb === S.rfb && S.hpModeContext && (S.hpMode === 'unknown' || S.hpMode === 'checking')) {
    const c = S.hpModeContext;
    if (c && S.hpMode !== 'checking') refreshHpMode(c.h, c.rfb, c.attempt);
    return;
  }
  // Saved on but this connection fell back: try high-performance again (e.g. after unlocking),
  // or, when it isn't on there at all (e.g. Itda reinstalled), set it up again below.
  if (S.host.hp?.on && S.hpFallback && !S.hpFallbackSetup) {
    S.hpFallback = null; S.hpFallbackSetup = false; S.hpNoDecode = null;
    toast(T('viewer.hp.retrying'));
    return open(S.host);
  }
  // The switch after unlocking can land just as the person presses this button to try again:
  // that press must not turn high-performance off.
  if (S.host.hp?.on && !S.hpFallback && Date.now() - (S.unlockSwitchAt || 0) < 3000) {
    toast(T('viewer.hp.switchingUnlocked'));
    return;
  }
  if (S.host.hp?.on && !S.hpFallback) {
    return hpDisable();
  }
  S.hpBusy = true; renderTools();
  const id=S.host.id;
  window.ItdaHpSetup.begin({id,name:S.host.label||S.host.name||S.host.remote_name||S.host.host,send:(op,extra={})=>post({ev:`hp-${op}`,...extra}),
    done:()=>{if(S.host?.id!==id)return;S.hpBusy=false;S.hpNoDecode=null;S.host.hp={...(S.host.hp||{}),on:true};toast(T('viewer.hp.switching'));open(S.host);},
    cancel:()=>{S.hpBusy=false;renderTools();}
  });
  post({ ev: 'hp-upgrade' });
}
window.itdaHp = (m) => {
  if (S.host?.id !== m.id) return;
  if (m.status && m.req && m.req === S.hpStatusReq && m.error) {
    S.hpQualityFailed = true; S.hpQualityPending = false; renderTools(); return;
  }
  // The Mac's report of what it serves: the toggle shows that, not just the saved choice.
  // Only the answer to this connection's own request (an older one may arrive late).
  if (m.status && typeof m.quality === 'string' && m.req && m.req === S.hpStatusReq) {
    // While connected the capture is active with a real size; a report saying otherwise is
    // stale or settling: unknown.
    // A Mac whose Itda takes screen actions itself (else the default shortcuts go as keys).
    S.hpActions = !!m.actions;
    const real = m.quality === '' || (m.quality_active && m.capture_width > 0 && m.capture_height > 0);
    S.hpQuality = real ? { quality: m.quality, active: !!m.quality_active, w: m.capture_width, h: m.capture_height } : null;
    S.hpQualityFailed = !real && (S.hpStatusTries || 0) >= 4;
    if (real || S.hpQualityFailed) S.hpQualityPending = false;
    // The curtain there: true/false, or null from an Itda without it.
    if (m.curtain !== undefined) { S.hpCurtain = m.curtain; S.hpCurtainError = m.curtain_error || ''; }
    // This device covers that Mac on every connection once chosen (asked again even when it
    // says covered: that may be the previous connection's curtain on its way down).
    if (typeof m.curtain === 'boolean' && S.host.hp?.curtain && !S.hpCurtainAsked && !S.hpCurtainBusy) {
      S.hpCurtainAsked = true; S.hpCurtainBusy = true;
      post({ ev: 'hp-curtain', on: true });
    }
    renderTools();
    // Settling (the status file is rewritten every 2 s): ask again a few times, same request.
    if (!real && (S.hpStatusTries = (S.hpStatusTries || 0) + 1) < 5) {
      const req = m.req;
      setTimeout(() => { if (S.hpStatusReq === req && S.rfb && !S.hpQualityFailed) post({ ev: 'hp-status', req }); }, 2500);
    }
  }
  if (!m.status && typeof m.curtain === 'boolean') {
    S.hpCurtainBusy = false;
    if (!m.ok) { renderTools(); return toast(T('viewer.hp.curtainFailed', { reason: m.error || T('viewer.hp.noRemote') })); }
    S.host.hp = { ...(S.host.hp || {}), curtain: m.curtain };
    S.hpCurtainError = '';
    toast(T(m.curtain ? 'viewer.hp.curtainOn' : 'viewer.hp.curtainOff'));
    // What that Mac actually did (the request only reached it): checked until it settles.
    const req = S.hpStatusReq;
    S.hpCurtainSettling = m.curtain ? Date.now() + 12000 : 0;
    if (req) for (const ms of [2500, 6000, 12500]) setTimeout(() => { if (S.hpStatusReq === req && S.rfb) post({ ev: 'hp-status', req }); }, ms);
    return renderTools();
  }
  if (typeof m.quality === 'boolean') {
    S.hpQualityBusy = false;
    if (!m.ok) { renderTools(); return toast(T('viewer.hp.qualityFailed', { reason: m.error || T('viewer.hp.noRemote') })); }
    S.host.hp = { ...(S.host.hp || {}), native: !!m.quality };
    toast(T(m.quality ? 'viewer.hp.toSharp' : 'viewer.hp.toFast'));
    return open(S.host);
  }
  window.ItdaHpSetup?.message(m);
};
// The sharpness toggle: the saved choice for this Mac against what it actually serves now.
function hpQualityShown() { return S.hpQuality?.quality === 'native'; }
function hpQualityLabel() {
  if (S.hpQualityBusy) return T('viewer.hp.quality.busy');
  const want = !!S.host.hp?.native, q = S.hpQuality?.quality;
  if (q === undefined) return T(S.hpQualityFailed ? 'viewer.hp.quality.unknown' : 'viewer.hp.quality.checking');
  if (q === '') return T('viewer.hp.quality.update');
  if (want && q === 'native') return T('viewer.hp.quality.on', { w: S.hpQuality.w, h: S.hpQuality.h });
  if (want) return T('viewer.hp.quality.pending');
  if (q === 'native') return T('viewer.hp.quality.other');
  return T('viewer.hp.quality.off');
}

// Covering that Mac's own screens: the choice saved for this device.
function hpCurtainLabel() {
  if (S.hpCurtainBusy) return T('viewer.hp.curtain.busy');
  if (S.hpCurtain === null) return T('viewer.hp.curtain.update');
  if (S.hpCurtain === true) return T('viewer.hp.curtain.on');
  if (S.host.hp?.curtain && S.hpCurtainError === 'local') return T('viewer.hp.curtain.lifted');
  if (S.host.hp?.curtain && S.hpCurtainError) return T('viewer.hp.curtain.failed', { why: T(S.hpCurtainError === 'input' ? 'viewer.hp.curtain.needsAccess' : 'viewer.hp.curtain.noExclude') });
  if (S.host.hp?.curtain && Date.now() < (S.hpCurtainSettling || 0)) return T('viewer.hp.curtain.starting');
  return T('viewer.hp.curtain.off');
}
// Pressed: what that Mac reports (the saved choice is asked again on each connection).
const hpCurtainShown = () => S.hpCurtain === true;

// ---------------------------------------------------------------- toolbar

// Visible names under the toolbar icons (touch screens show no tooltips); the full
// description stays the button's accessible name.
const TOOL_NAME = { back: 'viewer.tool.back', keyboard: 'viewer.tool.keyboard', keys: 'viewer.tool.keys', 'mac-actions': 'viewer.tool.macActions', 'apple-screen-sharing': 'viewer.tool.appleScreenSharing',
  'hp-quality': 'viewer.tool.hpQuality', 'hp-curtain': 'viewer.tool.hpCurtain', hp: 'viewer.tool.hp', 'hp-disable': 'viewer.tool.hpDisable', clip: 'viewer.tool.clip', 'mouse-lock': 'viewer.tool.mouseLock', audio: 'viewer.tool.audio', monitors: 'viewer.tool.monitors', scale: () => (S.fit ? 'viewer.tool.actual' : 'viewer.tool.fit'), viewonly: 'viewer.tool.viewonly', full: 'viewer.tool.full' };
const toolName = (t) => (typeof TOOL_NAME[t] === 'function' ? T(TOOL_NAME[t]()) : TOOL_NAME[t] ? T(TOOL_NAME[t]) : '');

function renderTools() {
  const t = $('#tools');
  const focused = t.contains(document.activeElement) ? document.activeElement.dataset.t : null;
  t.innerHTML = [
    ...(APP ? [] : [['back', 'back', T('viewer.status.list')], 'sep', ['keyboard', 'keyboard', T('viewer.tool.keyboard')]]),
    ['keys', 'command', T(isWin(S.host) ? 'viewer.tip.keys.win' : 'viewer.tip.keys.mac')],
    ...(macHost(S.host) ? [['mac-actions', 'mission', T('viewer.tip.macActions')]] : []),
    ...(APP && /Mac/.test(navigator.platform) && !IOS && ['mac', 'macos'].includes(S.host?.os) && S.host?.kind !== 'rdp' ? [['apple-screen-sharing', 'monitor', T('viewer.tip.appleScreenSharing')]] : []),
    ...(hpCapable() && S.hpActual ? [['hp-quality', 'sparkle', hpQualityLabel(), hpQualityShown()]] : []),
    ...(hpCapable() && S.hpActual ? [['hp-curtain', 'lock', hpCurtainLabel(), hpCurtainShown()]] : []),
    ...(hpCapable() ? [['hp', 'bolt', hpModeLabel(), !!S.hpActual]] : []),
    ...(hpCapable() && S.host.hp?.on && ['unknown', 'checking'].includes(S.hpMode) ? [['hp-disable', 'bolt', T('viewer.tip.hpDisable')]] : []),
    ...(APP ? [] : [['clip', 'clip', T('viewer.tool.clip')]]),
    ['audio', S.audio ? 'sound' : 'mute', T(S.audio ? 'viewer.tip.soundOff' : 'viewer.tip.soundOn'), S.audio],
    'sep',
    ...(relAvailable() ? [['mouse-lock', 'mouse', T(relLocked() ? 'viewer.tip.mouseUnlock' : 'viewer.tip.mouseLock'), relLocked()]] : []),
    ['monitors', 'monitor', T('viewer.tip.monitors')],
    ['scale', S.fit ? 'fit' : 'actual', T(S.fit ? 'viewer.tip.actual' : 'viewer.tip.fit')],
    ['viewonly', 'eye', T('viewer.tool.viewonly'), S.viewOnly],
    ['full', 'full', T('viewer.tool.full')],
  ].map((b) => (b === 'sep' ? '<span class="sep"></span>' : `<button data-t="${b[0]}" title="${esc(b[2])}" aria-label="${esc(b[2])}"${['viewonly', 'audio'].includes(b[0]) ? ` aria-pressed="${!!b[3]}"` : ''} class="${b[3] ? 'on' : ''}">${icon(b[1])}<span class="tl" aria-hidden="true">${toolName(b[0])}</span></button>`)).join('');
  if (focused) t.querySelector(`[data-t="${focused}"]`)?.focus();
}

// Sheets open just below the toolbar, which wraps to two rows on a phone.
new ResizeObserver(() => $('#view').style.setProperty('--tools-h', `${$('#tools').offsetHeight}px`)).observe($('#tools'));

let dimTimer;
function dimToolsSoon() {
  $('#tools').classList.remove('dim');
  clearTimeout(dimTimer);
  dimTimer = setTimeout(() => { if ($('#sheet').hidden) $('#tools').classList.add('dim'); }, 3500);
}
$('#tools').addEventListener('pointerdown', dimToolsSoon);

$('#tools').addEventListener('click', (e) => {
  const b = e.target.closest('[data-t]');
  if (!b) return;
  const r = S.rfb;
  switch (b.dataset.t) {
    case 'back': return leave();
    case 'keyboard': return toggleKeyboard();
    case 'keys': return toggleSheet('keys');
    case 'mac-actions': return toggleSheet('mac-actions');
    case 'apple-screen-sharing':
      if (APP && /Mac/.test(navigator.platform) && !IOS && ['mac', 'macos'].includes(S.host?.os) && S.host?.kind !== 'rdp') {
        post({ev:'apple-screen-sharing'});
        toast(T('viewer.toast.appleScreenSharing'));
      }
      return;
    case 'hp': return hpToggle();
    case 'hp-disable': return hpDisable();
    case 'hp-quality':
      if (S.hpQualityBusy) return;
      if (!S.hpQuality) return requestHpQuality();
      S.hpQualityBusy = true; renderTools();
      // The opposite of what is on screen now (or of the saved choice while that is unknown).
      return post({ ev: 'hp-quality', native: !(S.hpQuality ? hpQualityShown() : !!S.host.hp?.native) });
    case 'hp-curtain':
      if (S.hpCurtainBusy) return;
      if (S.hpCurtain === null) return toast(T('viewer.hp.curtain.updateToast'));
      S.hpCurtainBusy = true; renderTools();
      return post({ ev: 'hp-curtain', on: !hpCurtainShown() });
    case 'audio': return audioToggle();
    case 'monitors': return toggleSheet('monitors');
    case 'clip': return toggleSheet('clip');
    case 'scale':
      S.fit = !S.fit;
      if (r) { r.scaleViewport = S.fit; r.clipViewport = !S.fit; r.dragViewport = !S.fit && !monitors.selected; monitors.apply(); }
      toast(T(S.fit ? 'viewer.tip.fit' : 'viewer.toast.actual'));
      return renderTools();
    case 'mouse-lock':
      rel.auto = false;
      mouseLock(!relLocked());
      return;
    case 'viewonly':
      monitors.option('view_only',!S.viewOnly);
      toast(T(S.viewOnly ? 'viewer.toast.viewOnlyOn' : 'viewer.toast.viewOnlyOff'));
      return renderTools();
    case 'full': {
      if (APP) return post({ ev: 'fullscreen' });
      const d = document.documentElement;
      if (document.fullscreenElement) document.exitFullscreen();
      else if (d.requestFullscreen) d.requestFullscreen().catch(() => toast(T('viewer.toast.noFullscreen')));
      else toast(T('viewer.toast.fullscreenHome'));
    }
  }
});

// The remote machine's sound (Itda's own sharing sends it), remembered per connection. Off by
// default; browsers play sound only after a click or key, so a remembered choice starts with
// the first one in this page.
function audioToggle() {
  const r = S.rfb;
  S.audio = !S.audio;
  if (r && r.itdaAudio && !r.itdaAudio(S.audio)) {
    S.audio = false;
    renderTools();
    return toast(T('viewer.toast.noSound'));
  }
  monitors.persist({ audio: S.audio });
  toast(T(S.audio ? 'viewer.toast.soundOn' : 'viewer.toast.soundOff'));
  clearTimeout(S.audioCheck);
  // No confirmation from the server: it isn't Itda's sharing (Screen Sharing, Remote Desktop).
  if (S.audio && r) S.audioCheck = setTimeout(() => { if (S.rfb === r && S.audio && !r._itdaAudioServer) toast(byDev('viewer.toast.noRemoteSound.mac', 'viewer.toast.noRemoteSound.pc', 'viewer.toast.noRemoteSound.device')); }, 4000);
  renderTools();
}
for (const ev of ['pointerdown', 'keydown']) document.addEventListener(ev, () => { if (S.audio) S.rfb?.itdaAudioResume?.(); }, true);

// [keys shown, the description's message ID, keysyms]
const COMBOS = [
  ['⌘ Space', 'viewer.keys.spotlight', [XK.cmd, XK.space]],
  ['⌘ Tab', 'viewer.keys.switchApps', [XK.cmd, XK.tab]],
  ['⌘ C', 'viewer.keys.copy', [XK.cmd, 0x63]],
  ['⌘ V', 'viewer.keys.paste', [XK.cmd, 0x76]],
  ['⌘ Q', 'viewer.keys.quitApp', [XK.cmd, 0x71]],
  ['⌘ W', 'viewer.keys.closeWindow', [XK.cmd, 0x77]],
  ['⌥⌘ Esc', 'viewer.keys.forceQuit', [XK.opt, XK.cmd, XK.esc]],
  ['⌃⌘ Q', 'viewer.keys.lockScreen', [XK.ctrl, XK.cmd, 0x71]],
  ...macActions.slice(0,5).map(([,label,key,keys])=>[key,label,keys]),
  ['Esc', '', [XK.esc]],
  ['Tab', '', [XK.tab]],
  ['⌫', 'viewer.keys.backspace', [XK.back]],
];

const WIN_COMBOS = [
  ['Ctrl Alt Del', 'viewer.keys.securityScreen', [XK.ctrl, XK.alt, XK.del]],
  ['Alt Tab', 'viewer.keys.switchApps', [XK.alt, XK.tab]],
  ['Ctrl C', 'viewer.keys.copy', [XK.ctrl, 0x63]],
  ['Ctrl V', 'viewer.keys.paste', [XK.ctrl, 0x76]],
  ['Ctrl Z', 'viewer.keys.undo', [XK.ctrl, 0x7a]],
  ['Alt F4', 'viewer.keys.closeWindow', [XK.alt, XK.f4]],
  ['⊞ Win', 'viewer.keys.start', [XK.win]],
  ['⊞ D', 'viewer.keys.desktop', [XK.win, 0x64]],
  ['Ctrl Shift Esc', 'viewer.keys.taskManager', [XK.ctrl, XK.shift, XK.esc]],
  ['Esc', '', [XK.esc]],
  ['Tab', '', [XK.tab]],
  ['⌫', 'viewer.keys.backspace', [XK.back]],
];
const combos = () => (isWin(S.host) ? WIN_COMBOS : COMBOS);

function sendCombo(keys) {
  const r = S.rfb;
  if (!r || S.viewOnly) return;
  sendChord(r,keys);
}

function toggleSheet(kind) {
  const sh = $('#sheet');
  if (!sh.hidden && sh.dataset.kind === kind) { sh.hidden = true; return; }
  sh.dataset.kind = kind;
  if (kind === 'mac-actions') { sh.innerHTML = macMenu(); } else if (kind === 'monitors') { sh.innerHTML = monitors.menu(); monitors.bindMenu(sh); } else if (kind === 'keys') {
    const mod = (k, label) => `<button data-mod="${k}" class="${S.mods.has(k) ? 'on' : ''}">${label}</button>`;
    const mods = isWin(S.host)
      ? mod('ctrl', 'Ctrl') + mod('alt', 'Alt') + mod('shift', '⇧') + mod('win', '⊞')
      : mod('cmd', '⌘') + mod('opt', '⌥') + mod('ctrl', '⌃') + mod('shift', '⇧');
    sh.innerHTML = `<h3>${T('viewer.keys.hold')}</h3><div class="mods">${mods}</div>
      <h3>${T('viewer.keys.send')}</h3><div class="keys">${combos().map((c, i) => `<button data-combo="${i}">${esc(c[0])}${c[1] ? `<small>${esc(T(c[1]))}</small>` : ''}</button>`).join('')}</div>`;
  } else {
    const sendTo = byDev('viewer.clip.sendTo.mac', 'viewer.clip.sendTo.pc', 'viewer.clip.sendTo.device');
    sh.innerHTML = `<h3>${byDev('viewer.clip.from.mac', 'viewer.clip.from.pc', 'viewer.clip.from.device')}</h3><textarea id="clipIn" readonly placeholder="${T('viewer.clip.empty')}">${esc(S.clip)}</textarea>
      <h3 style="margin-top:12px">${sendTo}</h3><textarea id="clipOut" placeholder="${T('viewer.clip.outPh')}"></textarea>
      <div class="row"><button data-a="copy">${T('viewer.clip.copy')}</button><button class="go" data-a="send">${sendTo}</button></div>`;
  }
  sh.hidden = false;
  dimToolsSoon();
}

$('#sheet').addEventListener('click', async (e) => {
  if (MIRROR !== null) return; // The mirror routes its menu through the owner's bridge.
  const action = e.target.closest('[data-mac-action]');
  if(action) { const a=macActions.find(a=>a[0]===action.dataset.macAction); if(a&&macHost(S.host)&&S.rfb&&!S.viewOnly){const r=S.rfb;sendMacAction(a,!!(S.hpActual&&S.hpActions),(k,down)=>r.sendKey(k,null,down));} $('#sheet').hidden=true; S.rfb?.focus(); return; }
  const m = e.target.closest('[data-mod]');
  if (m) {
    const k = m.dataset.mod;
    S.mods.has(k) ? S.mods.delete(k) : S.mods.add(k);
    m.classList.toggle('on', S.mods.has(k));
    return;
  }
  const c = e.target.closest('[data-combo]');
  if (c) {
    sendCombo(combos()[+c.dataset.combo][2]);
    return;
  }
  const a = e.target.closest('[data-a]');
  if (!a) return;
  if (a.dataset.a === 'send') {
    const t = $('#clipOut').value;
    if (S.rfb && t) { S.rfb.clipboardPasteFrom(t); toast(T(isWin(S.host) ? 'viewer.toast.clipSent.pc' : 'viewer.toast.clipSent.mac')); }
  } else if (a.dataset.a === 'copy') {
    const box = $('#clipIn');
    try { await navigator.clipboard.writeText(box.value); toast(T('viewer.toast.copied')); }
    catch (_) { box.select(); toast(T('viewer.toast.copyByHand')); }
  }
});

// ---------------------------------------------------------------- phone keyboard

const kbd = $('#kbd');
let composing = false;

function toggleKeyboard() {
  if (document.activeElement === kbd) { kbd.blur(); return; }
  kbd.value = ' '; // a placeholder so Backspace has something to delete (and fires)
  kbd.focus();
}

function keysymOf(ch) {
  const cp = ch.codePointAt(0);
  return cp < 0x100 ? cp : 0x01000000 | cp;
}

function typeText(text) {
  const r = S.rfb;
  if (!r || S.viewOnly) return;
  const mods = [...S.mods].map((m) => XK[m]);
  for (const ch of text) {
    if (ch === '\n') { sendCombo([...mods, XK.enter]); continue; }
    sendCombo([...mods, keysymOf(ch)]);
  }
  if (S.mods.size) {
    S.mods.clear();
    document.querySelectorAll('[data-mod]').forEach((b) => b.classList.remove('on'));
  }
}

kbd.addEventListener('compositionstart', () => { composing = true; });
kbd.addEventListener('compositionend', (e) => { composing = false; typeText(e.data || ''); kbd.value = ' '; });
kbd.addEventListener('input', (e) => {
  if (composing || e.isComposing) return;
  if (e.inputType === 'deleteContentBackward') sendCombo([XK.back]);
  else if (e.data) typeText(e.data);
  kbd.value = ' ';
});
kbd.addEventListener('keydown', (e) => {
  const special = { Backspace: XK.back, Enter: XK.enter, Tab: XK.tab, Escape: XK.esc, ArrowLeft: XK.left, ArrowRight: XK.right, ArrowUp: XK.up, ArrowDown: XK.down, Delete: XK.del }[e.key];
  if (special && !composing) {
    e.preventDefault();
    sendCombo([...[...S.mods].map((m) => XK[m]), special]);
  }
});

// Full screen in a browser that has the Keyboard Lock API (Chrome, Edge): ⌘/Ctrl+W, Alt+Tab,
// Esc and the like go to the remote device instead of the browser (hold Esc to leave).
document.addEventListener('fullscreenchange', () => {
  const k = navigator.keyboard;
  if (!k || !k.lock) return;
  if (document.fullscreenElement) k.lock().catch(() => {});
  else k.unlock();
});
// Nobody looking (this window and every monitor window hidden: another tab, minimized, a
// locked phone): the server drops to a few frames a second until someone looks again.
function syncBackground() {
  if (MIRROR !== null || !S.rfb || !('itdaBackground' in S.rfb)) return;
  S.rfb.itdaBackground = document.hidden && !monitors.anyVisible();
}
document.addEventListener('visibilitychange', syncBackground);

// Relative pointer (Itda's own servers, which announce it): a game on the other side hides the
// pointer and reads only how far the mouse moves, pulling it back to the middle, so absolute
// positions fight it. With the pointer locked here, movement goes as deltas (novnc-src/itda.js).
// Locks by itself on a click while the remote pointer is hidden; the toolbar or Ctrl+Alt+Shift+M
// lock it by hand; Esc releases it.
function mouseLock(on) {
  const c = S.rfb?._canvas;
  if (!on) { if (document.pointerLockElement) document.exitPointerLock(); return; }
  if (!c || !relAvailable() || !c.requestPointerLock) return;
  // Raw movement where the browser offers it (no pointer acceleration applied twice).
  try { const p = c.requestPointerLock({ unadjustedMovement: true }); p?.catch?.(() => c.requestPointerLock()); } catch { c.requestPointerLock(); }
}
function relSend() {
  rel.raf = 0;
  if (!relLocked()) return;
  S.rfb.itdaRelative(rel.dx, rel.dy, rel.mask);
  rel.dx = rel.dy = 0;
}
document.addEventListener('pointerlockchange', () => {
  const r = S.rfb;
  if (!r) return;
  r._itdaLocked = relLocked();
  // The click that locked it pressed a button through noVNC, whose release now comes here; noVNC
  // sends its stored mask with every absolute move, so it must forget the press.
  if (r._itdaLocked) { r._mouseButtonMask = 0; clearTimeout(r._mouseMoveTimer); r._mouseMoveTimer = null; }
  if (!r._itdaLocked) {
    if (rel.mask) r.itdaRelative(0, 0, 0); // no button stays down on the other side
    rel.mask = rel.dx = rel.dy = 0;
  } else toast(T('viewer.toast.mouseLocked'));
  renderTools();
});
document.addEventListener('mousemove', (e) => {
  if (!relLocked()) return;
  rel.dx += e.movementX; rel.dy += e.movementY;
  rel.raf ||= requestAnimationFrame(relSend); // one message a frame
});
for (const type of ['mousedown', 'mouseup']) document.addEventListener(type, (e) => {
  if (!relLocked()) {
    // A click while the remote pointer is hidden locks it (a user gesture, as browsers require).
    if (type === 'mousedown' && rel.hidden && e.target === S.rfb?._canvas && relAvailable()) { rel.auto = true; mouseLock(true); }
    return;
  }
  e.preventDefault();
  const bit = [1, 2, 4][e.button] || 0;
  rel.mask = type === 'mousedown' ? rel.mask | bit : rel.mask & ~bit;
  relSend();
}, true);
document.addEventListener('wheel', (e) => {
  if (!relLocked()) return;
  e.preventDefault();
  e.stopPropagation(); // noVNC's own wheel handler would send it again, as an absolute event
  const bit = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? (e.deltaX > 0 ? 64 : 32) : (e.deltaY > 0 ? 16 : 8);
  relSend();
  S.rfb.itdaRelative(0, 0, rel.mask | bit);
  S.rfb.itdaRelative(0, 0, rel.mask);
}, { passive: false, capture: true });
window.addEventListener('keydown', (e) => {
  if (!(e.ctrlKey && e.altKey && e.shiftKey && e.code === 'KeyM') || !relAvailable()) return;
  e.preventDefault(); e.stopImmediatePropagation();
  rel.auto = false;
  mouseLock(!relLocked());
}, true);

// Another language (the picker, or the app): what is on screen again in it. The editor isn't
// redrawn (it would lose what was typed; the picker is behind it).
I18N.onChange(() => {
  syncLangPick();
  if (MIRROR !== null) return;
  document.querySelectorAll('#loginError, #loginNote').forEach((el) => { if (el.itdaText) el.textContent = el.itdaText(); });
  if (S.signedIn) { renderHosts(); renderAccounts(); }
  if (!$('#hint').hidden) installHint();
  if (S.status) status(...S.status);
  if (!$('#view').hidden) renderTools();
  const sh = $('#sheet');
  if (!sh.hidden) {
    const out = $('#clipOut')?.value;
    sh.hidden = true;
    toggleSheet(sh.dataset.kind);
    if (out && $('#clipOut')) $('#clipOut').value = out;
  }
});

window.addEventListener('beforeunload', () => { if (MIRROR === null) close(false); });
boot();
