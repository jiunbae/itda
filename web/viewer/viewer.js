// Itda web viewer. Signs in with the PIN, lists and edits the address book, and shows a
// Mac or Windows PC through the gateway's WebSocket with noVNC (window.RFB from
// novnc.min.js). Installable as an app (manifest + sw.js).
'use strict';
import './novnc.min.js';
import { MonitorController, startMirror } from './monitors.js';
import { WheelInput, macHost, macActions, macMenu, sendChord } from './interaction.js';

const $ = (s) => document.querySelector(s);
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
  lock: '<rect x="5" y="10.5" width="14" height="9.5" rx="2"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>',
  pc: '<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M8 20h8M12 16.5V20M3 13h18"/>',
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

const S = { hosts: [], accounts: [], rfb: null, ws: null, host: null, fit: true, viewOnly: false, mods: new Set(), clip: '', ever: false, tries: 0 };

// App window (Itda for Mac): this page shows one connection, chosen by the app, and reports
// to the app's home instead of offering the list.
const OPEN = new URLSearchParams(location.search).get('open');
const MIRROR = new URLSearchParams(location.search).get('mirror');
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
        // This decoder's frames in flight: chunk timestamp → when it was handed over.
        const submitted = new Map();
        super({ ...init, output: (f) => {
          const t = submitted.get(f.timestamp);
          submitted.delete(f.timestamp);
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
        if (this.itdaSubmitted.size > 64) this.itdaSubmitted.clear(); // (errors leave entries)
        this.itdaSubmitted.set(chunk.timestamp, performance.now());
        return super.decode(chunk);
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
const monitors = new MonitorController({S,app:APP,post,api,render:renderTools,toast,close:leave});
const opt = (k, dflt) => (S.host && S.host.opts && k in S.host.opts ? S.host.opts[k] : dflt);
// What to call the remote device in messages.
const dev = () => (isWin(S.host) ? 'PC' : S.host && kindOf(S.host) === 'linux' ? '기기' : 'Mac');

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

// Where passwords end up, for the editor's reassurance line.
const vault = () => (S.local ? '이 Mac의 키체인' : 'PC의 Windows 자격 증명 관리자');

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
  if (APP && me.ok) return openFromApp();
  if (me.ok) return loadHosts();
  show('login');
  setTimeout(() => $('#pin').focus(), 50);
}

$('#loginForm').addEventListener('submit', async (e) => {
  e.preventDefault();
  const r = await api('/api/login', { pin: $('#pin').value });
  if (r.ok) {
    $('#loginError').textContent = '';
    $('#pin').value = '';
    return loadHosts();
  }
  $('#loginError').textContent = r.data.error || '들어갈 수 없습니다';
  $('#pin').select();
});

$('#logout').addEventListener('click', async () => {
  await api('/api/logout', {});
  show('login');
});
$('#refresh').innerHTML = icon('refresh');
$('#refresh').addEventListener('click', loadHosts);

const isWin = (h) => !!(h && (h.this || h.kind === 'rdp' || h.os === 'windows'));
// Editor kinds (same as the PC's Itda): what to ask for per device.
const KINDS = {
  mac: { kind: 'vnc', os: 'mac', label: 'Mac', user: 'Mac 계정 이름', userHint: 'Mac 로그인에 쓰는 짧은 이름 (Apple ID 아님)', pass: 'Mac 로그인 암호', port: 5900, ssh: true },
  windows: { kind: 'vnc', os: 'windows', label: 'Windows · Itda', user: null, pass: '공유 암호', passHint: '그 PC의 Itda → 공유 페이지에 표시된 8글자 암호 (Windows 로그인 암호나 이메일 아님)', port: 5900 },
  rdp: { kind: 'rdp', os: 'windows', label: '원격 데스크톱', user: 'Windows 계정 (Microsoft 계정은 이메일)', userHint: 'Microsoft 계정은 이메일 주소(예: alex@outlook.com), 로컬 계정은 이름, 회사 계정은 회사\\이름', pass: 'Windows 암호 (PIN 아님)', passHint: 'Microsoft 계정이면 그 계정의 암호입니다. 로그인 화면의 숫자 PIN은 쓸 수 없습니다', port: 3389 },
  linux: { kind: 'vnc', os: 'linux', label: 'Linux · VNC', user: '사용자 이름 (선택)', userHint: 'VNC 암호만 쓰는 서버는 비워 두세요', pass: 'VNC 암호', port: 5900, ssh: true },
};
const kindOf = (h) => (h.kind === 'rdp' ? 'rdp' : KINDS[h.os] ? h.os : 'mac');
const KIND_TEXT = (h) => (h.this ? 'Windows · 이 PC 공유' : { mac: h.ssh ? 'Mac · SSH 암호화' : 'Mac · 화면 공유', windows: 'Windows · Itda 공유', rdp: 'Windows 원격 데스크톱', linux: h.ssh ? 'Linux · SSH 암호화' : 'Linux · VNC' }[kindOf(h)]);

async function loadHosts() {
  const [r, a] = await Promise.all([api('/api/hosts'), api('/api/accounts')]);
  if (r.status === 401) return show('login');
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
  if (!h) return post({ ev: 'error', kind: 'other', msg: '연결을 찾을 수 없습니다' });
  document.title = h.label;
  S.fit = !(h.opts && h.opts.actual);
  S.viewOnly = !!(h.opts && h.opts.view_only);
  open(h);
}

function renderHosts() {
  const el = $('#hosts');
  const add = `<button class="host add" data-new="host">${icon('plus')}<b>기기 추가</b><span>Mac 또는 Windows PC</span></button>`;
  // Previews already shown stay as they are (a new <img> would be decoded again and flash).
  const shown = new Map([...el.querySelectorAll('img')].map((i) => [i.getAttribute('src'), i]));
  el.innerHTML = S.hosts.map((h) => `
      <div class="host${h.has_pw ? '' : ' nopw'}" data-id="${esc(h.id)}">
        <button class="open" aria-label="${esc(h.label)} 열기">
          <div class="th ${esc(osName(h) || '')}">${osMark(h)}${h.thumb ? `<img alt="" src="thumb/${esc(h.id)}.png?v=${esc(String(h.thumb))}">` : ''}</div>
          <div class="meta"><b>${esc(h.label)}</b>
            <span>${h.ssh ? icon('lock') : ''}${esc(h.this ? 'Itda가 실행 중인 PC' : h.user ? `${h.user}@${h.host}` : h.host)}</span>
            ${h.has_pw ? `<span class="kind">${esc(KIND_TEXT(h))}</span>` : '<span class="warn">암호를 저장해야 열 수 있습니다 — 눌러서 입력</span>'}</div>
        </button>
        ${h.this ? '' : `<button class="more" data-edit="${esc(h.id)}" title="편집" aria-label="${esc(h.label)} 편집">${icon('edit')}</button>`}
      </div>`).join('') + add;
  el.querySelectorAll('img').forEach((img) => {
    const old = shown.get(img.getAttribute('src'));
    if (old) img.replaceWith(old);
    else img.addEventListener('error', () => img.remove());
  });
}

function renderAccounts() {
  const el = $('#accounts');
  el.innerHTML = `<p class="lead">여러 기기에 같은 계정으로 로그인한다면 계정을 한 번만 저장하고 기기에서 골라 쓰세요.</p>` +
    S.accounts.map((a) => `
      <button class="acct" data-acct="${esc(a.id)}">
        <span class="av">${esc((a.name || a.user || '?').slice(0, 1).toUpperCase())}</span>
        <span class="who"><b>${esc(a.name || a.user)}</b><small>${esc(a.user)} · 기기 ${a.uses}대${a.has_pw ? '' : ' · 암호 없음'}</small></span>
        ${icon('edit')}
      </button>`).join('') +
    `<button class="acct add" data-new="account">${icon('plus')}<b>계정 추가</b></button>`;
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
  if (h && !h.has_pw) return editHost(h, '이 기기의 암호를 입력하면 바로 열 수 있습니다.');
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
  if (r.status === 401) { closeEditor(); return show('login'); }
  if (!r.ok) { $('#edErr').textContent = r.data.error || '저장하지 못했습니다'; return false; }
  S.hosts = r.data.hosts || S.hosts;
  S.accounts = r.data.accounts || S.accounts;
  renderHosts();
  renderAccounts();
  closeEditor();
  if (after) after(r.data.id);
  return true;
}

function editHost(h, note = '') {
  const isNew = !h;
  h = h || { kind: 'vnc', os: 'mac', account: '', name: '', host: '', own_user: '', group: '', ssh: false };
  const acctOpts = `<option value="">이 기기만의 계정</option>` + S.accounts.map((a) => `<option value="${esc(a.id)}" ${a.id === h.account ? 'selected' : ''}>${esc(a.name || a.user)} (${esc(a.user)})</option>`).join('');
  editor(`
    <h2 id="edTitle">${isNew ? '기기 추가' : '기기 편집'}</h2>
    ${note ? `<p class="note">${esc(note)}</p>` : ''}
    ${isNew ? '<div id="nearby" class="nearby"><span class="muted">주변 기기를 찾는 중…</span></div>' : ''}
    <div class="seg four" role="radiogroup" aria-label="종류">
      ${Object.entries(KINDS).map(([k, d]) => `<label><input type="radio" name="kind" value="${k}" ${kindOf(h) === k ? 'checked' : ''}><span>${d.label}</span></label>`).join('')}
    </div>
    ${field('주소', `<input name="host" required value="${esc(h.host)}" placeholder="예: my-mac.local, 192.168.0.10" autocapitalize="off" spellcheck="false">`, '<span id="portHint"></span>')}
    ${field('이름 <i>(선택)</i>', `<input name="name" value="${esc(h.name)}" placeholder="예: 거실 iMac">`)}
    ${field('로그인', `<select name="account">${acctOpts}</select>`)}
    <div data-own>
      <div id="userBox">${field('<span id="userLabel"></span>', `<input name="user" value="${esc(h.own_user)}" autocapitalize="off" spellcheck="false">`, '<span id="userHint"></span>')}</div>
      ${field('<span id="passLabel"></span>', `<input name="password" type="password" autocomplete="new-password" placeholder="${!isNew && h.has_pw && !h.account ? '저장됨 — 바꿀 때만 입력' : ''}">`, '<span id="passHint"></span>')}
    </div>
    <label class="check" id="sshBox"><input type="checkbox" name="ssh" ${h.ssh ? 'checked' : ''}><span>암호화 연결 (SSH 터널)<small>상대 기기에 SSH(Mac은 ‘원격 로그인’)가 켜져 있어야 합니다</small></span></label>
    ${field('그룹 <i>(선택)</i>', `<input name="group" value="${esc(h.group)}" placeholder="예: 집, 회사">`)}
    <p id="edErr" class="error" role="alert"></p>
    <div class="ed-actions">
      ${isNew ? '' : '<button type="button" class="danger" data-a="delete">삭제</button>'}
      <span class="grow"></span>
      <button type="button" data-a="cancel">취소</button>
      <button type="submit" class="primary">${isNew ? '추가' : '저장'}</button>
    </div>`);
  const f = $('#edForm');
  const sync = () => {
    const d = KINDS[f.kind.value];
    $('#portHint').textContent = `포트가 ${d.port}이 아니면 주소:포트`;
    $('#userBox').hidden = !d.user;
    $('#userLabel').textContent = d.user || '';
    $('#userHint').textContent = d.userHint || '';
    $('#passLabel').textContent = d.pass;
    $('#passHint').textContent = d.passHint || (S.local ? '암호는 이 Mac의 키체인에만 저장됩니다' : `암호는 ${vault()}에만 저장되고 이 브라우저에는 남지 않습니다`);
    if (!f.password.placeholder.startsWith('저장됨')) f.password.placeholder = d.pass;
    $('#sshBox').hidden = !d.ssh;
    f.querySelector('[data-own]').hidden = !!f.account.value;
  };
  f.addEventListener('change', sync);
  sync();
  if (isNew) showNearby(f, sync);
  f.querySelector('[data-a=delete]')?.addEventListener('click', async () => {
    if (!confirm(`‘${h.label}’을(를) 지울까요? 저장된 암호도 함께 지워집니다.`)) return;
    await saveEdit({ cmd: 'delete_host', id: h.id });
  });
  f.onsubmit = async (e) => {
    e.preventDefault();
    const d = KINDS[f.kind.value];
    const body = {
      cmd: 'save_host', id: h.id || '', kind: d.kind, os: d.os, host: f.host.value, name: f.name.value, group: f.group.value,
      account: f.account.value, user: d.user ? f.user.value : '', password: f.account.value ? '' : f.password.value, ssh: !!d.ssh && f.ssh.checked,
    };
    const wantOpen = !h.has_pw && (body.password || body.account);
    await saveEdit(body, (id) => {
      const saved = S.hosts.find((x) => x.id === id);
      if (saved && wantOpen && saved.has_pw) open(saved);
      else toast(isNew ? '추가했습니다' : '저장했습니다');
    });
  };
}

// "기기 추가": devices found on this network (Bonjour) and on Tailscale, one tap to fill in.
async function showNearby(f, sync) {
  const box = $('#nearby');
  const r = await api('/api/nearby');
  if (!box.isConnected) return;
  const list = r.ok ? r.data || [] : [];
  if (!list.length) { box.innerHTML = '<span class="muted">주변에서 찾은 기기가 없습니다 — 아래에 주소를 직접 넣으세요.</span>'; return; }
  const OS = { mac: 'Mac', windows: 'Windows', linux: 'Linux' };
  box.innerHTML = `<b>주변에서 찾은 기기</b><div class="chips">${list.map((d, i) =>
    `<button type="button" class="chip ${esc(osName(d) || '')}" data-i="${i}">${osMark(d)}<span>${esc(d.name)}<small>${OS[d.os] || ''}${d.kind === 'rdp' ? ' · 원격 데스크톱' : ''}${d.via === 'tailscale' ? ' · Tailscale' : ''}${d.off ? ' · 공유 꺼짐' : ''}</small></span></button>`).join('')}</div>`;
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

function editAccount(a) {
  const isNew = !a;
  a = a || { name: '', user: '', ssh_key: '', uses: 0 };
  editor(`
    <h2 id="edTitle">${isNew ? '계정 추가' : '계정 편집'}</h2>
    ${field('사용자 이름', `<input name="user" required value="${esc(a.user)}" autocapitalize="off" spellcheck="false" placeholder="로그인 이름">`)}
    ${field('표시 이름 <i>(선택)</i>', `<input name="name" value="${esc(a.name)}" placeholder="예: 내 Mac 계정">`)}
    ${field('암호', `<input name="password" type="password" autocomplete="new-password" placeholder="${a.has_pw ? '저장됨 — 바꿀 때만 입력' : '암호'}">`, `암호는 ${vault()}에만 저장됩니다`)}
    ${a.ssh_key ? `<p class="note">SSH 키: ${esc(a.ssh_key)}${S.local ? '' : ' (PC의 Itda에서 바꿀 수 있습니다)'}</p>` : ''}
    ${isNew ? '' : `<p class="note">이 계정을 쓰는 기기: ${a.uses}대</p>`}
    <p id="edErr" class="error" role="alert"></p>
    <div class="ed-actions">
      ${isNew ? '' : '<button type="button" class="danger" data-a="delete">삭제</button>'}
      <span class="grow"></span>
      <button type="button" data-a="cancel">취소</button>
      <button type="submit" class="primary">${isNew ? '추가' : '저장'}</button>
    </div>`);
  const f = $('#edForm');
  f.querySelector('[data-a=delete]')?.addEventListener('click', async () => {
    if (!confirm(`‘${a.name || a.user}’ 계정을 지울까요? 이 계정을 쓰던 기기는 다음에 암호를 다시 물어봅니다.`)) return;
    await saveEdit({ cmd: 'delete_account', id: a.id });
  });
  f.onsubmit = async (e) => {
    e.preventDefault();
    await saveEdit({ cmd: 'save_account', id: a.id || '', user: f.user.value, name: f.name.value, password: f.password.value }, () => toast(isNew ? '계정을 추가했습니다' : '저장했습니다'));
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
window.addEventListener('appinstalled', () => { $('#install').hidden = true; toast('앱으로 설치했습니다'); });
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
    ? '홈 화면에 추가하면 앱처럼 전체 화면으로 열립니다 — Safari의 공유 버튼 → ‘홈 화면에 추가’'
    : !window.isSecureContext
      ? '앱으로 설치하려면 PC의 Itda 설정에서 Tailscale HTTPS 주소(https://…ts.net)를 켜고 그 주소로 여세요'
      : '';
  if (!tip) return;
  $('#hint').innerHTML = `<span>${esc(tip)}</span><button class="ghost" aria-label="닫기">✕</button>`;
  $('#hint').hidden = false;
  $('#hint button').onclick = () => { $('#hint').hidden = true; remember('itda.hint'); };
}
installHint();

// ---------------------------------------------------------------- viewer

// Close codes from the gateway → [title, advice]; functions see the host.
const CLOSE_TEXT = {
  4004: () => ['연결을 찾을 수 없습니다', '목록에서 지워졌을 수 있습니다.'],
  4001: () => ['저장된 암호가 없습니다', '목록에서 이 기기를 편집해 암호를 저장하세요.'],
  4010: (h) => [`${dev()} 로그인에 실패했습니다`, h.this || (h.os === 'windows' && h.kind !== 'rdp') ? '그 PC의 Itda → 공유 페이지에 표시된 8글자 공유 암호를 넣으세요. Windows 로그인 암호나 Microsoft 계정 암호가 아닙니다.' : h.kind === 'rdp' ? 'Microsoft 계정이면 사용자 이름에 이메일 주소, 암호에 Microsoft 계정 암호를 넣으세요. 숫자 PIN은 쓸 수 없습니다.' : '이 기기(또는 계정)에 저장된 사용자 이름과 암호를 확인하세요.'],
  4011: () => ['사용자 이름이 필요합니다', '기기를 편집해 사용자 이름을 넣으세요.'],
  4012: (h) => [`${dev()}이(가) 연결을 거부했습니다`, h.kind === 'rdp' ? 'Windows 설정 → 시스템 → 원격 데스크톱이 켜져 있는지 확인하세요. (Home 에디션은 원격 데스크톱 서버가 없습니다 — 대신 그 PC에서 Itda의 ‘이 PC 공유’를 켜세요.)' : h.this ? 'PC의 Itda에서 ‘이 PC 공유’가 켜져 있는지 확인하세요.' : { windows: '그 PC의 Itda에서 ‘공유 → 이 PC 공유’가 켜져 있는지 확인하세요.', linux: 'VNC 서버가 실행 중인지, 포트(보통 5900·5901)가 맞는지 확인하세요.' }[kindOf(h)] || 'Mac에서 화면 공유가 켜져 있는지 확인하세요.'],
  4013: () => [`${dev()}이(가) 응답하지 않습니다`, '기기가 켜져 있고 Itda가 있는 PC와 같은 네트워크(또는 Tailscale)에 있는지 확인하세요.'],
  4014: () => ['주소를 찾을 수 없습니다', '기기를 편집해 주소를 확인하세요.'],
  4020: () => ['SSH 로그인에 실패했습니다', 'Mac 계정 암호를 확인하세요.'],
  4021: () => ['Mac의 보안 키가 바뀌었습니다', 'PC의 Itda에서 한 번 연결해 새 키를 확인하세요.'],
  4022: () => ['원격 로그인(SSH)이 꺼져 있습니다', 'Mac의 공유 설정에서 원격 로그인을 켜세요.'],
};

function status(kind, title, text) {
  const st = $('#status');
  if (!kind) { st.hidden = true; return; }
  st.hidden = false;
  st.innerHTML = kind === 'busy'
    ? `<div class="card"><div class="spin"></div><h2>${esc(title)}</h2><p>${esc(text || '')}</p></div>`
    : `<div class="card"><h2>${esc(title)}</h2><p>${esc(text || '')}</p>
        <div class="row"><button data-a="list">${APP ? '닫기' : '목록으로'}</button><button class="go" data-a="retry">다시 연결</button></div></div>`;
}

$('#status').addEventListener('click', (e) => {
  const a = e.target.closest('[data-a]');
  if (!a) return;
  if (a.dataset.a === 'retry') open(S.host);
  else if (APP) post({ ev: 'close' });
  else leave();
});

// Close codes → the app's error kinds (the same words the Windows viewer reports).
const KIND_OF = { 4010: 'auth', 4011: 'need_user', 4012: 'refused', 4013: 'unreachable', 4014: 'dns', 4020: 'ssh_auth', 4021: 'ssh_hostkey', 4022: 'ssh_refused', 4023: 'ssh_missing' };
const RETRIES = 10;

function open(h) {
  if (!h) return;
  close();
  S.host = h;
  show('view');
  renderTools();
  status('busy', S.tries ? `${h.label}에 다시 연결하는 중 (${S.tries}/${RETRIES})` : `${h.label}에 연결하는 중`, h.ssh ? '암호화 연결(SSH)을 여는 중…' : h.kind === 'rdp' ? '원격 데스크톱에 로그인하는 중…' : '');
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  // Remote Desktop follows this window's size (not on phones: a desktop that small is no use).
  const follow = h.kind === 'rdp' && opt('resize', true) && !opt('monitor_layout', 0) && opt('monitor_mode',0)!==2 && (APP || matchMedia('(pointer: fine)').matches);
  const scr = $('#screen');
  const size = follow && scr.clientWidth ? `?w=${Math.round(scr.clientWidth)}&h=${Math.round(scr.clientHeight)}` : '';
  const ws = new WebSocket(`${proto}//${location.host}/ws/${encodeURIComponent(h.id)}${size}`, ['binary']);
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
  let name = '';
  rfb.addEventListener('desktopname', (e) => { name = (e.detail && e.detail.name) || ''; });
  rfb.addEventListener('connect', () => {
    status(null);
    rfb.focus();
    dimToolsSoon();
    if (!APP) return toast(`${h.label}에 연결했습니다`);
    post(S.tries ? { ev: 'reconnected' } : { ev: 'connected', name, w: rfb._fbWidth || 0, h: rfb._fbHeight || 0 });
    S.ever = true;
    S.tries = 0;
    thumbSoon(2500);
  });
  rfb.addEventListener('disconnect', () => {
    if (S.rfb !== rfb) return;
    S.rfb = null;
    if (APP) return setTimeout(() => appDropped(h, closeInfo), 50);
    setTimeout(() => {
      const known = closeInfo && CLOSE_TEXT[closeInfo.code];
      const [t, d] = known ? known(h) : ['연결이 끊겼습니다', closeInfo && closeInfo.reason ? closeInfo.reason : '네트워크 상태를 확인하고 다시 연결하세요.'];
      status('error', t, d);
    }, 50);
  });
  rfb.addEventListener('clipboard', (e) => {
    S.clip = e.detail.text || '';
    if (APP) { if (opt('clipboard', true)) post({ ev: 'clip', text: S.clip }); return; }
    toast(`${dev()} 클립보드를 받았습니다 — 📋에서 볼 수 있습니다`);
  });
}

// App window: a first attempt that fails goes back to the home (which explains it); a drop
// after that reconnects on its own, like the Windows viewer.
function appDropped(h, info) {
  const code = info ? info.code : 0;
  const msg = (info && info.reason) || '원격 기기와의 연결이 끊겼습니다';
  if (!S.ever) return post({ ev: 'error', kind: KIND_OF[code] || 'other', msg });
  if (opt('reconnect', true) && S.tries < RETRIES && !KIND_OF[code]?.includes('auth')) {
    S.tries += 1;
    post({ ev: 'reconnecting', n: S.tries });
    status('busy', `${h.label} 연결이 끊겼습니다`, `${Math.min(S.tries * 2, 10)}초 뒤 다시 연결합니다… (${S.tries}/${RETRIES})`);
    return setTimeout(() => { if (!S.rfb) open(h); }, Math.min(S.tries * 2000, 10000));
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
window.itdaPaste = (t) => { if (S.rfb && !S.viewOnly && opt('clipboard', true) && t) S.rfb.clipboardPasteFrom(t); };

// One WebSocket packet per wheel event; fractional motion is shared with mirrors.
const wheel = new WheelInput();
const BTN = (b) => (b & 1) | ((b & 4) >> 1) | ((b & 2) << 1); // DOM buttons → RFB mask
function onWheel(e) {
  const rfb = S.rfb;
  if (!rfb || !rfb._canvas || !rfb._canvas.contains(e.target)) return;
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
function hpToggle() {
  if (!hpCapable() || S.hpBusy) return;
  if (S.host.hp?.on) {
    S.host.hp = { ...S.host.hp, on: false };
    post({ ev: 'hp-off' });
    toast('고성능 모드를 껐습니다 — 기본 화면 공유로 다시 연결합니다');
    return open(S.host);
  }
  S.hpBusy = true; renderTools();
  const id=S.host.id;
  window.ItdaHpSetup.begin({id,name:S.host.label||S.host.name||S.host.remote_name||S.host.host,send:(op,extra={})=>post({ev:`hp-${op}`,...extra}),
    done:()=>{if(S.host?.id!==id)return;S.hpBusy=false;S.host.hp={...(S.host.hp||{}),on:true};toast('고성능 모드로 전환합니다');open(S.host);},
    cancel:()=>{S.hpBusy=false;renderTools();}
  });
  post({ ev: 'hp-upgrade' });
}
window.itdaHp = (m) => {if(S.host?.id===m.id)window.ItdaHpSetup?.message(m);};

// ---------------------------------------------------------------- toolbar

function renderTools() {
  const t = $('#tools');
  t.innerHTML = [
    ...(APP ? [] : [['back', 'back', '목록으로'], 'sep', ['keyboard', 'keyboard', '키보드']]),
    ['keys', 'command', `${isWin(S.host) ? 'Windows' : 'Mac'} 특수 키`],
    ...(macHost(S.host) ? [['mac-actions', 'mission', 'Mac 화면 동작 — Mission Control · Exposé']] : []),
    ...(APP && /Mac/.test(navigator.platform) && !IOS && ['mac', 'macos'].includes(S.host?.os) && S.host?.kind !== 'rdp' ? [['apple-screen-sharing', 'monitor', 'Apple 화면 공유 앱에서 열기 · 별도 앱으로 직접 연결']] : []),
    ...(hpCapable() ? [['hp', 'bolt', S.host.hp?.on ? '고성능 모드 켜짐 — 누르면 끕니다' : S.hpBusy ? '고성능 모드 준비 중…' : '고성능 모드 — 이 Mac에 Itda 고성능 공유를 켜고 전환합니다 (SSH)', !!S.host.hp?.on]] : []),
    ...(APP ? [] : [['clip', 'clip', '클립보드']]),
    'sep',
    ['monitors', 'monitor', '모니터 선택 및 창 배치'],
    ['scale', S.fit ? 'fit' : 'actual', S.fit ? '실제 크기로' : '화면에 맞춤'],
    ['viewonly', 'eye', '보기 전용', S.viewOnly],
    ['full', 'full', '전체 화면'],
  ].map((b) => (b === 'sep' ? '<span class="sep"></span>' : `<button data-t="${b[0]}" title="${b[2]}" aria-label="${b[2]}" class="${b[3] ? 'on' : ''}">${icon(b[1])}</button>`)).join('');
}

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
        toast('Apple 화면 공유 앱에 연결을 요청했습니다. 이 세션은 유지됩니다.');
      }
      return;
    case 'hp': return hpToggle();
    case 'monitors': return toggleSheet('monitors');
    case 'clip': return toggleSheet('clip');
    case 'scale':
      S.fit = !S.fit;
      if (r) { r.scaleViewport = S.fit; r.clipViewport = !S.fit; r.dragViewport = !S.fit && !monitors.selected; monitors.apply(); }
      toast(S.fit ? '화면에 맞춤' : '실제 크기 — 끌어서 이동');
      return renderTools();
    case 'viewonly':
      monitors.option('view_only',!S.viewOnly);
      toast(S.viewOnly ? '보기 전용 켜짐' : '보기 전용 꺼짐');
      return renderTools();
    case 'full': {
      if (APP) return post({ ev: 'fullscreen' });
      const d = document.documentElement;
      if (document.fullscreenElement) document.exitFullscreen();
      else if (d.requestFullscreen) d.requestFullscreen().catch(() => toast('이 브라우저는 전체 화면을 지원하지 않습니다. 홈 화면에 추가하면 전체 화면으로 열립니다.'));
      else toast('홈 화면에 추가하면 전체 화면으로 열립니다');
    }
  }
});

const COMBOS = [
  ['⌘ Space', 'Spotlight', [XK.cmd, XK.space]],
  ['⌘ Tab', '앱 전환', [XK.cmd, XK.tab]],
  ['⌘ C', '복사', [XK.cmd, 0x63]],
  ['⌘ V', '붙여넣기', [XK.cmd, 0x76]],
  ['⌘ Q', '앱 종료', [XK.cmd, 0x71]],
  ['⌘ W', '창 닫기', [XK.cmd, 0x77]],
  ['⌥⌘ Esc', '강제 종료', [XK.opt, XK.cmd, XK.esc]],
  ['⌃⌘ Q', '화면 잠금', [XK.ctrl, XK.cmd, 0x71]],
  ...macActions.slice(0,5).map(([,label,key,keys])=>[key,label,keys]),
  ['Esc', '', [XK.esc]],
  ['Tab', '', [XK.tab]],
  ['⌫', '지우기', [XK.back]],
];

const WIN_COMBOS = [
  ['Ctrl Alt Del', '보안 화면', [XK.ctrl, XK.alt, XK.del]],
  ['Alt Tab', '앱 전환', [XK.alt, XK.tab]],
  ['Ctrl C', '복사', [XK.ctrl, 0x63]],
  ['Ctrl V', '붙여넣기', [XK.ctrl, 0x76]],
  ['Ctrl Z', '실행 취소', [XK.ctrl, 0x7a]],
  ['Alt F4', '창 닫기', [XK.alt, XK.f4]],
  ['⊞ Win', '시작 메뉴', [XK.win]],
  ['⊞ D', '바탕 화면', [XK.win, 0x64]],
  ['Ctrl Shift Esc', '작업 관리자', [XK.ctrl, XK.shift, XK.esc]],
  ['Esc', '', [XK.esc]],
  ['Tab', '', [XK.tab]],
  ['⌫', '지우기', [XK.back]],
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
    sh.innerHTML = `<h3>누른 채로 (다음 키와 함께)</h3><div class="mods">${mods}</div>
      <h3>보내기</h3><div class="keys">${combos().map((c, i) => `<button data-combo="${i}">${esc(c[0])}${c[1] ? `<small>${esc(c[1])}</small>` : ''}</button>`).join('')}</div>`;
  } else {
    const d = dev();
    sh.innerHTML = `<h3>${d}에서 받은 클립보드</h3><textarea id="clipIn" readonly placeholder="아직 받은 내용이 없습니다">${esc(S.clip)}</textarea>
      <h3 style="margin-top:12px">${d}(으)로 보내기</h3><textarea id="clipOut" placeholder="여기에 붙여 넣고 보내기를 누르세요"></textarea>
      <div class="row"><button data-a="copy">받은 내용 복사</button><button class="go" data-a="send">${d}(으)로 보내기</button></div>`;
  }
  sh.hidden = false;
  dimToolsSoon();
}

$('#sheet').addEventListener('click', async (e) => {
  if (MIRROR !== null) return; // The mirror routes its menu through the owner's bridge.
  const action = e.target.closest('[data-mac-action]');
  if(action) { const a=macActions.find(a=>a[0]===action.dataset.macAction); if(a&&macHost(S.host))sendCombo(a[3]); $('#sheet').hidden=true; S.rfb?.focus(); return; }
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
    if (S.rfb && t) { S.rfb.clipboardPasteFrom(t); toast(isWin(S.host) ? 'PC 클립보드로 보냈습니다 — PC에서 Ctrl+V' : 'Mac 클립보드로 보냈습니다 — Mac에서 ⌘V'); }
  } else if (a.dataset.a === 'copy') {
    const box = $('#clipIn');
    try { await navigator.clipboard.writeText(box.value); toast('복사했습니다'); }
    catch (_) { box.select(); toast('선택된 내용을 길게 눌러 복사하세요'); }
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

window.addEventListener('beforeunload', () => { if (MIRROR === null) close(false); });
boot();
