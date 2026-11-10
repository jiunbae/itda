// Itda home page. Talks to the Rust host through window.ipc (out) and itda.recv (in).
'use strict';

const S = {
  hosts: [],
  accounts: [],
  settings: { theme: 'system', view: 'grid', toolbar: 'auto', hotkey: 'ctrl_alt', mac_scroll: 'fast', scroll_accel: true, tray: true, notify: true, thumbnails: true, defaults: {} },
  sessions: {},
  fileWindows: [],
  online: {},
  found: [],
  version: '',
  sysDark: false,
  page: 'all',
  query: '',
  scanning: false,
  pendingPw: {},
};

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const I = (code) => `<span class="i" aria-hidden="true">&#x${code};</span>`;

// Operating system marks (Segoe Fluent has no logos, so these are SVG everywhere).
const OS_SVG = {
  mac: '<path d="M16.37 12.73c-.02-2.1 1.72-3.12 1.8-3.17-.98-1.43-2.5-1.63-3.05-1.65-1.3-.13-2.53.76-3.19.76-.66 0-1.67-.74-2.75-.72-1.41.02-2.72.82-3.44 2.09-1.47 2.55-.38 6.32 1.05 8.39.7 1.01 1.53 2.15 2.62 2.11 1.05-.04 1.45-.68 2.72-.68s1.63.68 2.74.66c1.13-.02 1.85-1.03 2.54-2.05.8-1.17 1.13-2.31 1.15-2.37-.03-.01-2.2-.85-2.22-3.37zM14.29 6.55c.58-.7.97-1.68.86-2.65-.83.03-1.84.55-2.44 1.25-.53.62-1 1.61-.88 2.56.93.07 1.88-.47 2.46-1.16z"/>',
  windows: '<path d="M3.5 5.6l7-1v6.9h-7zM11.5 4.5l9-1.3v8.3h-9zM3.5 12.5h7v6.9l-7-1zM11.5 12.5h9v8.3l-9-1.3z"/>',
  linux: '<path d="M12 2.6c-2.3 0-3.6 1.9-3.6 4.4 0 1.3.3 2.2-.6 3.6-1.2 1.8-2.9 3.9-2.9 6.6 0 .6.1 1.1.3 1.6-.8.4-1.5.9-1.4 1.5.2.9 2.1.6 3.4 1.1 1.1.4 2 .6 2.6-.2.5-.1 1.4-.2 2.2-.2s1.7.1 2.2.2c.6.8 1.5.6 2.6.2 1.3-.5 3.2-.2 3.4-1.1.1-.6-.6-1.1-1.4-1.5.2-.5.3-1 .3-1.6 0-2.7-1.7-4.8-2.9-6.6-.9-1.4-.6-2.3-.6-3.6 0-2.5-1.3-4.4-3.6-4.4z"/><path class="cut" d="M12 10.2c-1.9 0-3.2 2.6-3.2 5.6 0 1.7.6 3.2 1.3 4h3.8c.7-.8 1.3-2.3 1.3-4 0-3-1.3-5.6-3.2-5.6z"/><circle class="cut" cx="10.6" cy="6.6" r=".9"/><circle class="cut" cx="13.4" cy="6.6" r=".9"/><path class="cut" d="M10.9 8.4h2.2L12 9.6z"/>',
};

const OS_NAME = { mac: 'Mac', windows: 'Windows', linux: 'Linux' };

// 'mac' | 'windows' | 'linux' | '' for a host in the book, a Bonjour find or a tailnet peer.
function osOf(x) {
  const o = String(x.os || '').toLowerCase();
  if (o === 'mac' || o === 'macos') return 'mac';
  if (o === 'windows') return 'windows';
  if (o === 'linux') return 'linux';
  return x.kind === 'rdp' || x.this ? 'windows' : '';
}

// The device's OS mark, or the generic screen when unknown.
function osIcon(x) {
  const os = osOf(x);
  return OS_SVG[os] ? `<span class="osm osm-${os}"><svg viewBox="0 0 24 24" aria-hidden="true">${OS_SVG[os]}</svg></span>` : I('E7F4');
}

// Itda for Mac shows this same home; the words and a few controls follow the platform.
const MAC = /Macintosh|Mac OS X/.test(navigator.userAgent) && !/Windows/.test(navigator.userAgent);
const MOD = MAC ? '⌘' : 'Ctrl';
// 'this Mac' / 'this PC' mid-sentence, in the current language (files.js and help.js read it too).
Object.defineProperty(window, 'HERE', { get: () => (MAC ? T('common.here.mac') : T('common.here.pc')) });
I18N.apply();
if (MAC) {
  document.documentElement.classList.add('mac');
  document.querySelector('.search kbd').textContent = '⌘F';
}

function send(cmd, data = {}) {
  try {
    window.ipc.postMessage(JSON.stringify({ cmd, ...data }));
  } catch (e) {
    console.log('ipc (no host)', cmd);
  }
}

window.itda = { recv: (m) => onMessage(m) };

// ---------------------------------------------------------------- helpers

const label = (h) => h.name || h.host;
const hostById = (id) => S.hosts.find((h) => h.id === id);
const hue = (id) => [...String(id)].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 7);
const hk = () => (S.settings.hotkey === 'ctrl_shift' ? 'Ctrl+Shift' : 'Ctrl+Alt');

function ago(sec) {
  if (!sec) return T('time.never');
  const d = Date.now() / 1000 - sec;
  if (d < 60) return T('time.justNow');
  if (d < 3600) return T('time.minutesAgo', { count: Math.floor(d / 60) });
  if (d < 86400) return T('time.hoursAgo', { count: Math.floor(d / 3600) });
  if (d < 172800) return T('time.yesterday');
  if (d < 86400 * 30) return T('time.daysAgo', { count: Math.floor(d / 86400) });
  const t = new Date(sec * 1000);
  return T('time.date', { y: t.getFullYear(), m: t.getMonth() + 1, d: t.getDate() });
}

function matches(h, q) {
  if (!q) return true;
  q = q.toLowerCase();
  return [h.name, h.host, h.user, hostGroups(h).join(' '), h.remote_name].some((v) => (v || '').toLowerCase().includes(q));
}

function sorted(list) {
  return [...list].sort((a, b) => (b.favorite - a.favorite) || (b.last_used - a.last_used) || label(a).localeCompare(label(b), 'ko'));
}

// A connection's groups (any number); older data kept them as one comma-separated "group".
function hostGroups(h) {
  if (h.groups && h.groups.length) return h.groups;
  return (h.group || '').split(',').map((g) => g.trim()).filter(Boolean);
}
function groups() {
  return [...new Set(S.hosts.flatMap(hostGroups))].sort((a, b) => a.localeCompare(b, 'ko'));
}

function inBook(f) {
  return S.hosts.find((h) => [f.host, f.ip, f.hostname].filter(Boolean).some((a) => h.host.toLowerCase() === a.toLowerCase()));
}

function looksLikeAddress(q) {
  return /^[\w.\-\[\]:]+$/.test(q) && (/[.:]/.test(q) || /^[a-z0-9-]+$/i.test(q));
}

function applyTheme() {
  const t = S.settings.theme;
  const dark = t === 'dark' || (t !== 'light' && S.sysDark);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
}

// Home zoom like a desktop app: Ctrl +/-/0, Ctrl+wheel, or Settings → 화면 크기.
const ZOOMS = [80, 90, 100, 110, 125, 150, 175];
const zoomStep = (dir) => {
  const z = S.settings.zoom || 100;
  return dir > 0 ? ZOOMS.find((x) => x > z) || ZOOMS[ZOOMS.length - 1] : [...ZOOMS].reverse().find((x) => x < z) || ZOOMS[0];
};
let zoomToast;
function setZoom(z) {
  if (z === (S.settings.zoom || 100)) return;
  S.settings.zoom = z;
  saveSettings();
  clearTimeout(zoomToast);
  zoomToast = setTimeout(() => toast(T('toast.zoom', { zoom: z, mod: MOD }), 'E71E'), 120);
}
document.addEventListener('wheel', (e) => {
  if (!e.ctrlKey) return;
  e.preventDefault();
  setZoom(zoomStep(e.deltaY < 0 ? 1 : -1));
}, { passive: false });

// ---------------------------------------------------------------- messages from Rust

function onMessage(m) {
  if (['state','hosts','session','online','found','tailnet'].includes(m.type)) queueMicrotask(() => window.ItdaFiles?.refreshNetwork());
  if (window.ItdaFiles && window.ItdaFiles.message(m)) return;
  switch (m.type) {
    case 'state':
      Object.assign(S, { accounts: m.accounts || [], hosts: m.hosts, settings: m.settings, sessions: m.sessions || {}, fileWindows: m.file_windows || [], online: m.online || {}, found: m.found || [], version: m.version, sysDark: !!m.dark, sshOk: m.ssh_ok, tailnet: m.tailnet, update: m.update || null, loginItem: m.login_item || null });
      I18N.setPref(S.settings.lang || 'auto');
      applyTheme();
      render();
      window.ItdaFiles?.boot();
      if (window.ITDA_FILE_HOST) break;
      if (m.update_failed) toast(T('toast.updateFailedLast', { error: m.update_failed }), 'E7BA', true);
      if (m.quick) quickConnect(m.quick);
      else if (!S.settings.onboarded && !S.hosts.length && !topModal()) showOnboarding();
      break;
    case 'login_item':
      // macOS names the new background item after the app's signer, not the app.
      if (S.loginAsked && m.state !== 'off') toast(T('toast.loginItemSigner'));
      S.loginAsked = false;
      S.loginItem = m.state;
      if (S.settings) S.settings.login_start = m.state !== 'off';
      if (S.page === 'settings') render();
      break;
    case 'apple_screen_sharing_result':
      toast(m.ok ? T('toast.appleScreenSharing.ok') : m.error || T('toast.appleScreenSharing.fail'), 'E7F4', !m.ok);
      break;
    case 'hosts':
      S.hosts = m.hosts;
      if (m.file_windows) S.fileWindows = m.file_windows;
      if (m.accounts) S.accounts = m.accounts;
      render();
      break;
    case 'session': {
      const prev = S.sessions[m.id];
      if (m.state === 'idle') delete S.sessions[m.id];
      else S.sessions[m.id] = { state: m.state, stage: m.stage };
      updateConnecting(m.id);
      if (m.state === 'connected' && prev && prev.state === 'connecting') {
        const h = hostById(m.id);
        toast(T('toast.connectedTo', { name: h ? label(h) : 'Mac' }), 'E73E');
      }
      render();
      break;
    }
    case 'online':
      if (JSON.stringify(m.online) === JSON.stringify(S.online)) break;
      S.online = m.online;
      render();
      break;
    case 'account_saved': {
      if (m.grouped) toast(T('toast.accountsGrouped', { count: m.grouped }), 'E73E');
      const em = S.pickAccountFor;
      S.pickAccountFor = null;
      if (em && em.isConnected) {
        const sel = em.querySelector('#f-account');
        const acc = acctById(m.id);
        const text = acc ? T('editor.accountOption', { name: acc.name || acc.user, user: acc.user }) : T('editor.accountFallback');
        const opt = [...sel.options].find((o) => o.value === m.id);
        if (opt) opt.text = text;
        else sel.add(new Option(text, m.id), sel.options[sel.options.length - 1]);
        sel.value = m.id;
        syncAccount(em);
      }
      if (S.page === 'accounts') render();
      break;
    }
    case 'picked': {
      const f = document.getElementById(m.field);
      if (f) { f.value = m.path; f.dispatchEvent(new Event('input')); }
      break;
    }
    case 'toast':
      toast(m.text, m.err ? 'E783' : 'E73E', !!m.err);
      break;
    case 'tailnet':
      // Asked every 30 s: draw again only when something changed.
      if (JSON.stringify(m.tailnet) === JSON.stringify(S.tailnet)) break;
      S.tailnet = m.tailnet;
      render();
      break;
    case 'found': {
      const same = JSON.stringify(m.found) === JSON.stringify(S.found);
      const was = S.scanning;
      S.found = m.found;
      S.scanning = false;
      if (!same || was) render();
      break;
    }
    case 'error':
      closeModal('connecting');
      showError(m.id, m.kind, m.msg);
      break;
    case 'need_password':
      closeModal('connecting');
      showPassword(m.id);
      break;
    case 'thumb': {
      // Load and decode the new preview first, then swap it in: the old one stays until then.
      const h = hostById(m.id);
      if (!h) break;
      if ('custom' in m) h.custom_thumb = m.custom;
      if (!m.v) { h.thumb = 0; render(); break; }
      const src = `thumb/${h.id}.png?v=${m.v}`;
      const img = new Image();
      img.alt = '';
      img.dataset.thumb = '';
      img.setAttribute('onerror', 'this.remove()');
      img.src = src;
      img.decode().catch(() => {}).then(() => {
        if (hostById(m.id) !== h) return;
        READY.set(src, img);
        h.thumb = m.v;
        render();
      });
      break;
    }
    case 'settings':
      S.settings = m.settings;
      I18N.setPref(S.settings.lang || 'auto');
      applyTheme();
      render();
      break;
    case 'saved':
      if (m.then === 'connect') connect(m.id);
      break;
    case 'quick':
      closeModal();
      quickConnect(m.address);
      break;
    case 'go':
      closeModal();
      go(m.page);
      break;
    case 'new':
      if (!topModal()) openEdit();
      break;
    case 'remote_setup': return window.ItdaRemoteSetup?.message(m);
    case 'quit_guard':
      if (!window.ITDA_FILE_HOST) quitGuard(m);
      break;
    case 'quit_pending':
      if (!window.ITDA_FILE_HOST) quitPending(m);
      break;
    case 'update':
      // A newer release: offered in the sidebar and Settings; installing always asks first.
      if (m.stage) { S.updateStage = m.stage; toast(T('toast.update.stage', { stage: m.stage }), 'E896'); }
      else if (m.error) { S.updateStage = ''; toast(T('toast.update.failed', { error: m.error }), 'E7BA', true); }
      else {
        S.update = m.available || null;
        if (m.found && S.update) toast(T('toast.update.found', { version: S.update.version }), 'E896');
        if (m.checked) toast(S.update ? T('toast.update.available', { version: S.update.version }) : T('toast.update.latest'), S.update ? 'E896' : 'E73E');
      }
      render();
      break;
    case 'hp': {
      // ⚡ high-performance mode being set up from a connection's editor (or window).
      const h = S.hosts.find((x) => x.id === m.id);
      const name = h ? label(h) : '';
      const guided=window.ItdaHpSetup?.message(m);
      if(m.helper){
        if(!guided && m.error) toast(T('toast.hp.helperFailed', { name, error: m.error }), 'E7BA');
        return;
      }
      // Only the open editor of this very connection reflects it.
      const ed = document.querySelector(`#modals [data-key="edit"][data-host="${CSS.escape(m.id || '')}"]`);
      const btn = ed && ed.querySelector('#hp-enable');
      if (m.stage && !guided) toast(T('toast.hp.stage', { name, stage: m.stage }), 'E72C');
      else if (m.error && !m.status) {
        if(!guided) toast(T('toast.hp.failed', { name, error: m.error }), 'E7BA');
        if (btn) { btn.disabled = false; btn.textContent = T('editor.hp.enable'); }
      } else if (m.ready) {
        if(!guided) toast(m.screen && m.access ? T('toast.hp.ready', { name }) : T('toast.hp.readyNeedsPermissions', { name }), 'E73E');
        const st = ed && ed.querySelector('#hp-state');
        if (st) st.textContent = T('editor.hp.stateReady');
        if (btn) btn.remove();
      }
      return;
    }
    case 'share':
      S.share = m;
      if (S.page === 'share') render();
      break;
    case 'web':
      S.web = m;
      if (S.page === 'share') render();
      break;
    case 'system_theme':
      S.sysDark = !!m.dark;
      applyTheme();
      break;
  }
}

// ---------------------------------------------------------------- rendering

function navItem(page, icon, text, count) {
  const on = S.page === page ? ' active' : '';
  const c = count != null ? `<span class="count">${count}</span>` : '';
  return `<button class="nav-item${on}" data-page="${esc(page)}" aria-label="${esc(text)}" title="${esc(text)}">${I(icon)}<span class="label">${esc(text)}</span>${c}</button>`;
}

function renderNav() {
  const fav = S.hosts.filter((h) => h.favorite).length;
  const live = Object.keys(S.sessions).length;
  let html = navItem('all', 'E80F', T('nav.all'), S.hosts.length);
  html += navItem('favorites', 'E734', T('nav.favorites'), fav || null);
  html += navItem('recent', 'E81C', T('nav.recent'), null);
  html += navItem('files', 'E8B7', T('nav.files'), null);
  html += transfersNav();
  html += navItem('accounts', 'E77B', T('nav.accounts'), S.accounts.length || null);
  if (live) html += navItem('live', 'E703', T('nav.live'), live);
  // Only devices that could still be added: the rest are in the book already.
  const addable = S.found.filter((f) => !inBook(f)).length + tailPeers().filter((p) => p.online && !peerInBook(p)).length;
  html += navItem('nearby', 'E701', T('nav.nearby'), addable || null);
  const sharing = (S.settings.web || {}).on || (S.settings.share || {}).on;
  html += navItem('share', 'E72D', T('nav.share'), null).replace('</button>', sharing ? `<span class="dot-on" title="${esc(T('nav.sharingOn'))}"></span></button>` : '</button>');
  const gs = groups();
  if (gs.length) {
    html += `<div class="nav-section">${T('nav.groups')}</div>`;
    for (const g of gs) html += navItem(`group:${g}`, 'E8B7', g, S.hosts.filter((h) => hostGroups(h).includes(g)).length);
  }
  morph($('#nav'), html);
  $$('.nav-bottom .nav-item').forEach((b) => b.classList.toggle('active', b.dataset.page === S.page));
}

const TITLES = () => ({ all: T('nav.all'), favorites: T('nav.favorites'), recent: T('nav.recent'), files: T('nav.files'), live: T('nav.live'), nearby: T('nav.nearby'), accounts: T('nav.accounts'), settings: T('nav.settings'), help: T('nav.help'), share: T('title.share') });

function renderTop() {
  const page = S.page;
  $('#title').textContent = page.startsWith('group:') ? page.slice(6) : TITLES()[page] || '';
  const listy = !['settings', 'help', 'nearby', 'accounts', 'share'].includes(page);
  $('.search').style.visibility = listy ? '' : 'hidden';
  $('#viewToggle').style.visibility = listy && page !== 'files' ? '' : 'hidden';
  $('#search').placeholder = page === 'files' ? T('top.search.files') : T('top.search');
  $('#newBtn .label-text').textContent = page === 'files' ? T('top.newDevice') : T('common.newConnection');
  $('#newBtn').title = page === 'files' ? T('top.newDevice') : T('top.newConnectionTitle', { mod: MOD });
  $$('#viewToggle button').forEach((b) => b.classList.toggle('on', b.dataset.view === (S.settings.view || 'grid')));
}

function pageHosts() {
  let list = S.hosts;
  const p = S.page;
  if (p === 'favorites') list = list.filter((h) => h.favorite);
  else if (p === 'recent') list = [...list].filter((h) => h.last_used).sort((a, b) => b.last_used - a.last_used);
  else if (p === 'live') list = list.filter((h) => S.sessions[h.id]);
  else if (p.startsWith('group:')) list = list.filter((h) => hostGroups(h).includes(p.slice(6)));
  if (p !== 'recent') list = sorted(list);
  return list.filter((h) => matches(h, S.query));
}

function stageText(st) {
  return { start: T('stage.start'), ssh: T('stage.ssh'), resolve: T('stage.resolve'), tcp: T('stage.tcp'), auth: T('stage.auth'), init: T('stage.init') }[st] || T('stage.tcp');
}

function badge(h) {
  const s = S.sessions[h.id];
  if (!s) return '';
  if (s.state === 'connected') return `<div class="badge ok">${I('E73E')}${T('state.connected')}</div>`;
  if (s.state === 'reconnecting') return `<div class="badge warn"><span class="spin"></span>${T('state.reconnecting')}</div>`;
  return `<div class="badge"><span class="spin"></span>${esc(stageText(s.stage))}</div>`;
}

function dot(h) {
  if (S.sessions[h.id] && S.sessions[h.id].state === 'connected') return `<span class="dot on" title="${T('state.connected')}"></span>`;
  const o = S.online[h.id];
  if (o === true) return `<span class="dot on" title="${T('dot.online')}"></span>`;
  if (o === false) return `<span class="dot off" title="${T('dot.offline')}"></span>`;
  return `<span class="dot unknown" title="${T('dot.checking')}"></span>`;
}

function thumb(h) {
  const img = h.thumb ? `<img data-thumb src="thumb/${esc(h.id)}.png?v=${h.thumb}" alt="" loading="lazy" onerror="this.remove()">` : '';
  const initial = esc((label(h)[0] || '?').toUpperCase());
  // A preview hides the big OS mark: keep a small one in the corner.
  const os = osOf(h);
  const mark = img && OS_SVG[os] ? `<span class="os-badge" title="${OS_NAME[os]}">${osIcon(h)}</span>` : '';
  return `<div class="thumb" style="--h:${hue(h.id)}">
    <span class="ph">${osIcon(h)}</span><span class="ph-initial">${initial}</span>${img}${mark}`;
}

// Every card and row has the same buttons: their names say which device they act on (the
// visible words remain part of the name, so voice control still finds them).
const named = (text) => `aria-label="${esc(text)}"`;
function primaryAction(h) {
  const s = S.sessions[h.id];
  if (!s) return `<button class="btn primary" data-act="connect" data-id="${esc(h.id)}" ${named(T('card.aria.connect', { name: label(h) }))}>${I('E768')}${T('card.connect')}</button>`;
  if (s.state === 'connecting') return `<button class="btn" data-act="cancel" data-id="${esc(h.id)}" ${named(T('card.aria.cancel', { name: label(h) }))}>${I('E711')}${T('common.cancel')}</button>`;
  return `<button class="btn primary" data-act="focus" data-id="${esc(h.id)}" ${named(T('card.aria.focus', { name: label(h) }))}>${I('E8A7')}${T('card.focus')}</button>`;
}

function card(h) {
  const live = S.sessions[h.id] ? ' live' : '';
  const lu = h.login_user || h.user;
  const who = (h.ssh && h.ssh.on ? `<span class="i lock" title="${T('card.sshLock')}">&#xE72E;</span>` : '') + (lu ? `${esc(lu)}@${esc(h.host)}` : esc(h.host));
  const extra = [h.kind === 'rdp' ? T('card.rdp') : '', ago(h.last_used), h.remote_size, hostGroups(h).join(', ')].filter(Boolean).map(esc).join(' · ');
  return `<div class="card${live}" tabindex="0" data-id="${esc(h.id)}" role="group" aria-label="${esc(label(h))}">
    ${thumb(h)}
      ${badge(h)}
      <button class="star i${h.favorite ? ' on' : ''}" data-act="fav" data-id="${esc(h.id)}" title="${h.favorite ? T('card.unfavorite') : T('card.favorite')}" ${named(T('card.aria.favorite', { name: label(h) }))} aria-pressed="${!!h.favorite}">${h.favorite ? '&#xE735;' : '&#xE734;'}</button>
      <div class="connect-overlay">${primaryAction(h)}</div>
    </div>
    <div class="meta">
      <div class="row1">${dot(h)}<span class="name" title="${esc(label(h))}">${esc(label(h))}</span>
        <button class="more i" data-act="more" data-id="${esc(h.id)}" title="${T('card.more')}" ${named(T('card.aria.more', { name: label(h) }))}>&#xE712;</button></div>
      <div class="sub">${who}</div>
      <div class="sub2">${extra}</div>
    </div>
  </div>`;
}

function row(h) {
  const s = S.sessions[h.id];
  const state = s
    ? s.state === 'connected'
      ? `<span class="pill ok">${I('E73E')}${T('state.connected')}</span>`
      : s.state === 'reconnecting'
        ? `<span class="pill warn">${T('state.reconnecting')}</span>`
        : `<span class="pill mute"><span class="spin" style="width:10px;height:10px;border-width:2px"></span>${esc(stageText(s.stage))}</span>`
    : S.online[h.id] === true
      ? `<span class="pill ok">${T('row.online')}</span>`
      : S.online[h.id] === false
        ? `<span class="pill mute">${T('row.offline')}</span>`
        : '';
  return `<div class="list-row" tabindex="0" data-id="${esc(h.id)}" role="group" aria-label="${esc(label(h))}">
    ${thumb(h)}</div>
    <div class="nm"><b>${dot(h)}${esc(label(h))}${h.favorite ? ` <span class="i" style="color:#ffc83d;font-size:12px">&#xE735;</span>` : ''}</b><small>${h.ssh && h.ssh.on ? `<span class="i lock" title="${T('card.sshLock')}">&#xE72E;</span>` : ''}${esc(h.host)}</small></div>
    <span class="col c-user">${h.user ? esc(h.user) : `<span style="color:var(--text-3)">${T('row.vncPassword')}</span>`}</span>
    <span class="col c-last">${esc(ago(h.last_used))}</span>
    <span class="col">${state}</span>
    <div class="actions">${primaryAction(h).replace('class="btn', 'class="btn sm')}
      <button class="btn subtle icon" data-act="more" data-id="${esc(h.id)}" title="${T('card.more')}" ${named(T('card.aria.more', { name: label(h) }))}>${I('E712')}</button></div>
  </div>`;
}

function tailPeers() {
  const t = S.tailnet;
  if (!t || !t.running) return [];
  return (t.peers || []).filter((p) => ['macOS', 'windows', 'linux'].includes(p.os));
}

function peerInBook(p) {
  const keys = [p.dns, p.ip, p.name, `${p.name}.local`].filter(Boolean).map((v) => v.toLowerCase());
  return S.hosts.find((h) => keys.includes(h.host.toLowerCase().replace(/:\d+$/, '')));
}

function foundBanner() {
  if (S.page !== 'all' || S.query) return '';
  const fresh = S.found.filter((f) => !inBook(f)).map((f) => f.name);
  const macs = tailPeers().filter((p) => p.os === 'macOS' && !peerInBook(p)).map((p) => p.name);
  const names = [...new Set([...fresh, ...macs])];
  if (!names.length) return '';
  const list = names.slice(0, 3).map(esc).join(', ') + (names.length > 3 ? ` ${T('banner.found.more', { count: names.length - 3 })}` : '');
  return `<div class="banner">${I('E701')}<div class="txt"><b>${T('banner.found.title')}</b><span>${list}${macs.length ? ` — ${T('banner.found.tailscale')}` : ''}</span></div>
    <button class="btn" data-act="page" data-page="nearby">${T('banner.found.view')}</button></div>`;
}
function renderHosts() {
  const list = pageHosts();
  const view = S.settings.view || 'grid';
  if (!S.hosts.length) return emptyBook();
  let html = foundBanner();
  if (!list.length) {
    if (S.query) {
      const addr = looksLikeAddress(S.query.trim());
      html += `<div class="empty"><div class="art">${I('E721')}</div><h2>${T('home.empty.search.title', { query: esc(S.query) })}</h2>
        ${addr ? `<p>${T('home.empty.search.enter', { address: esc(S.query.trim()) })}</p><div class="actions"><button class="btn primary" data-act="quick">${I('E768')}${T('common.connectTo', { name: esc(S.query.trim()) })}</button></div>` : `<p>${T('home.empty.search.hint')}</p>`}</div>`;
    } else {
      const msg = { favorites: [T('home.empty.favorites.title'), T('home.empty.favorites.text')], recent: [T('home.empty.recent.title'), T('home.empty.recent.text')], live: [T('home.empty.live.title'), ''] }[S.page] || [T('home.empty.none.title'), ''];
      html += `<div class="empty"><div class="art">${I('E7F4')}</div><h2>${msg[0]}</h2><p>${msg[1]}</p></div>`;
    }
    return html;
  }
  if (view === 'list') {
    html += `<div class="list"><div class="list-row list-head"><span></span><span>${T('list.name')}</span><span class="c-user">${T('list.user')}</span><span class="c-last">${T('list.lastConnected')}</span><span>${T('list.status')}</span><span></span></div>${list.map(row).join('')}</div>`;
  } else {
    html += `<div class="grid">${list.map(card).join('')}${S.page === 'all' && !S.query ? `<button class="add-card" data-act="new">${I('E710')}<span>${T('common.newConnection')}</span></button>` : ''}</div>`;
  }
  return html;
}

// ---------------------------------------------------------------- first-run guide

function finishOnboarding() {
  closeModal('onboard');
  if (!S.settings.onboarded) {
    S.settings.onboarded = true;
    saveSettings();
  }
}

function showAccountNameHelp() {
  const m = openModal('account-name-help', `<div class="dialog small"><div class="body"><h2>${T('accountHelp.title')}</h2>
    <p class="lead">${T('accountHelp.lead')}</p>
    <ol><li>${T('accountHelp.step1')}</li><li>${T('accountHelp.step2')}</li><li>${T('accountHelp.step3')}</li></ol>
    <p><a href="https://support.apple.com/guide/mac-help/mh35548/mac" data-ext>${T('accountHelp.appleLink')}</a></p>
    </div><div class="foot"><button class="btn primary" data-x="close">${T('common.ok')}</button></div></div>`);
  m.querySelector('[data-x="close"]').onclick = () => closeModal('account-name-help');
}

function showOnboarding(step = 'welcome') {
  const choice = (key, icon, title, sub) => `<button class="ob-choice" data-ob="${key}">${I(icon)}<b>${title}</b><span>${sub}</span></button>`;
  const check = (n, html) => `<li><span class="ob-n">${n}</span><span>${html}</span></li>`;
  const back = `<button class="btn subtle left" data-ob="welcome">${I('E72B')}${T('onboard.back')}</button>`;
  const steps = {
    welcome: () => `<div class="ob-hero"><div class="logo ob-logo"></div><h2>${T('onboard.welcome.title')}</h2>
        <p>${MAC ? T('onboard.welcome.text.mac') : T('onboard.welcome.text.pc')}<br>${T('onboard.welcome.ask')}</p></div>
      <div class="ob-choices">
        ${choice('mac', 'E7F4', T('onboard.choice.mac'), T('onboard.choice.mac.sub'))}
        ${choice('win', 'E7F8', T('onboard.choice.win'), T('onboard.choice.win.sub'))}
        ${choice('phone', 'E8EA', MAC ? T('onboard.choice.phone.mac') : T('onboard.choice.phone.pc'), T('onboard.choice.phone.sub'))}
        ${choice('linux', 'E756', T('onboard.choice.linux'), T('onboard.choice.linux.sub'))}
      </div>
      <div class="foot"><span class="ob-dots"><i class="on"></i><i></i><i></i></span><button class="btn subtle" data-ob="skip">${T('onboard.skip')}</button></div>`,
    mac: () => `<div class="ob-body"><h2>${I('E7F4')} ${T('onboard.choice.mac')}</h2>
        <div class="ob-split"><ol class="ob-list">
          ${check(1, T('onboard.mac.step1'))}
          ${check(2, T('onboard.mac.step2'))}
          ${check(3, `${T('onboard.mac.step3')} <button class="btn sm" data-act="account-name-help">${T('common.findAccountName')}</button>`)}
          ${check(4, T('onboard.mac.step4'))}
        </ol><div class="ob-art">${typeof macWindow === 'function' ? macWindow('general', T('onboard.mac.artSharing'), macSharing('screen')) : ''}</div></div></div>
      <div class="foot">${back}<span class="ob-dots"><i></i><i class="on"></i><i></i></span>
        <button class="btn" data-ob="mac-manual">${T('onboard.mac.manual')}</button><button class="btn primary" data-ob="mac-nearby">${I('E701')}${T('onboard.mac.nearby')}</button></div>`,
    win: () => `<div class="ob-body"><h2>${I('E7F8')} ${T('onboard.choice.win')}</h2>
        <div class="ob-two">
          <div class="ob-card"><b>${T('onboard.win.a.title')} <span class="pill ok">${T('onboard.win.a.pill')}</span></b>
            <ol class="ob-list">${check(1, T('onboard.win.a.step1'))}${check(2, T('onboard.win.a.step2'))}${check(3, T('onboard.win.a.step3'))}</ol>
            <button class="btn primary" data-ob="win-itda">${I('E710')}${T('onboard.win.a.add')}</button></div>
          <div class="ob-card"><b>${T('onboard.win.b.title')} <span class="pill mute">Pro·Enterprise</span></b>
            <ol class="ob-list">${check(1, T('onboard.win.b.step1'))}${check(2, T('onboard.win.b.step2'))}</ol>
            <button class="btn" data-ob="win-rdp">${I('E710')}${T('onboard.win.b.add')}</button></div>
        </div></div>
      <div class="foot">${back}<span class="ob-dots"><i></i><i class="on"></i><i></i></span></div>`,
    phone: () => MAC ? `<div class="ob-body"><h2>${I('E8EA')} ${T('onboard.phone.title.mac')}</h2>
        <ol class="ob-list">
          ${check(1, T('onboard.phone.mac.step1'))}
          ${check(2, T('onboard.phone.mac.step2'))}
          ${check(3, T('onboard.phone.mac.step3'))}
          ${check(4, T('onboard.phone.mac.step4'))}
        </ol>${note(T('onboard.phone.mac.note'))}</div>
      <div class="foot">${back}<span class="ob-dots"><i></i><i class="on"></i><i></i></span><button class="btn primary" data-ob="phone-on">${I('E768')}${T('onboard.phone.on')}</button></div>`
    : `<div class="ob-body"><h2>${I('E8EA')} ${T('onboard.phone.title.pc')}</h2>
        <ol class="ob-list">
          ${check(1, T('onboard.phone.pc.step1'))}
          ${check(2, T('onboard.phone.pc.step2'))}
          ${check(3, T('onboard.phone.pc.step3'))}
          ${check(4, T('onboard.phone.pc.step4'))}
        </ol>${note(T('onboard.phone.pc.note'))}</div>
      <div class="foot">${back}<span class="ob-dots"><i></i><i class="on"></i><i></i></span><button class="btn primary" data-ob="phone-on">${I('E768')}${T('onboard.phone.on')}</button></div>`,
  };
  const m = openModal('onboard', `<div class="dialog onboard">${steps[step]()}</div>`, { sticky: true });
  const accountHelp = m.querySelector('[data-act=account-name-help]');
  if (accountHelp) accountHelp.onclick = showAccountNameHelp;
  m.querySelectorAll('[data-ob]').forEach((b) => (b.onclick = () => {
    const a = b.dataset.ob;
    if (steps[a]) return showOnboarding(a);
    finishOnboarding();
    if (a === 'mac-nearby') { go('nearby'); scan(); }
    else if (a === 'mac-manual') openEdit({ os: 'mac' });
    else if (a === 'win-itda') openEdit({ os: 'windows' });
    else if (a === 'win-rdp') openEdit({ kind: 'rdp', os: 'windows' });
    else if (a === 'linux') openEdit({ os: 'linux' });
    else if (a === 'phone-on') { go('share'); act('share-all-on'); }
  }));
}

function emptyBook() {
  const found = S.found.filter((f) => !inBook(f));
  let html = `<div class="empty">
    <div class="art">${I('E7F4')}</div>
    <h2>${T('home.emptyBook.title')}</h2>
    <p>${T('home.emptyBook.text')}</p>
    <div class="actions"><button class="btn primary" data-act="new">${I('E710')}${T('common.newConnection')}</button><button class="btn" data-act="onboard">${I('E768')}${T('common.startGuide')}</button><button class="btn" data-help="start">${I('E897')}${T('nav.help')}</button></div>
  </div>`;
  if (found.length) {
    html += `<div class="page" style="margin:36px auto 0"><div class="section-head">${I('E701')}${T('home.emptyBook.found')}<span class="line"></span></div><div class="found-list">${found.map(foundRow).join('')}</div></div>`;
  }
  return html;
}

// A device found on this Wi-Fi: what it is, how it would open, and what is missing.
// Windows PCs announce themselves while Itda runs, even with sharing off.
function foundRow(f) {
  const win = f.os === 'windows';
  const rdp = win && !f.share && f.rdp;
  const addr = [f.hostname, f.ip].filter(Boolean).filter((v, i, a) => a.indexOf(v) === i).join(' · ');
  const act = `<button class="btn sm primary" data-act="add-found" data-host="${esc(f.host)}" data-name="${esc(f.name)}" data-os="${esc(f.os || '')}" data-itda="${win && !rdp ? '1' : ''}">${I('E710')}${T('nearby.add')}</button>`;
  const pill = win
    ? ` <span class="pill ${f.share || rdp ? 'ok' : 'mute'}">${f.share ? T('nearby.pill.sharing') : rdp ? T('nearby.pill.rdp') : 'Windows · Itda'}</span>`
    : f.itda ? ` <span class="pill ${f.share ? 'ok' : 'mute'}">Mac · Itda</span>` : '';
  const off = !f.itda || f.share || rdp ? ''
    : win ? `<span class="off">${T('nearby.off.pc')}</span>`
      : `<span class="off">${T('nearby.off.mac')}</span>`;
  return `<div class="found"><div class="ico">${osIcon({ os: win ? 'windows' : 'mac' })}</div><div class="txt"><b>${esc(f.name)}${pill}</b><span>${esc(addr)}</span>${off}</div>${act}</div>`;
}

function peerRow(p) {
  const os = { macOS: 'Mac', windows: 'Windows', linux: 'Linux' }[p.os] || p.os;
  const act = `<button class="btn sm primary" data-act="add-found" data-host="${esc(p.dns || p.ip)}" data-name="${esc(p.name)}" data-os="${esc(p.os)}">${I('E710')}${T('nearby.add')}</button>`;
  return `<div class="found"><div class="ico">${osIcon(p)}</div><div class="txt"><b>${p.online ? '<span class="dot on"></span> ' : '<span class="dot off"></span> '}${esc(p.name)} <span class="pill mute">${esc(os)}</span></b><span>${esc(p.dns)} · ${esc(p.ip)}</span></div>${act}</div>`;
}

function renderNearby() {
  const head = `<div class="banner">${I('E946')}<div class="txt"><b>${T('nearby.head.title')}</b><span>${T('nearby.head.text')}</span></div>
    <button class="btn" data-act="scan" ${S.scanning ? 'disabled' : ''}>${S.scanning ? `<span class="spin"></span>${T('nearby.scanning')}` : `${I('E72C')}${T('nearby.rescan')}`}</button></div>`;
  // Devices already in the address book are left out: they're on the home page.
  const fresh = S.found.filter((f) => !inBook(f));
  const lan = fresh.length
    ? `<div class="found-list">${fresh.map(foundRow).join('')}</div>`
    : S.found.length
      ? `<p class="note-line">${T('nearby.lan.allInBook')}</p>`
      : `<p class="note-line">${S.scanning ? T('nearby.lan.searching') : T('nearby.lan.none')}</p>`;
  const networkHelp = MAC && !S.found.length && !S.scanning ? `<p class="note-line">${T('nearby.lan.localNetwork')}</p>` : '';
  const all = tailPeers();
  const peers = all.filter((p) => !peerInBook(p));
  const t = S.tailnet;
  const ts = !t
    ? `<p class="note-line">${T('nearby.ts.install')} <a href="https://tailscale.com/download" data-ext>${T('nearby.ts.get')}</a></p>`
    : !t.running
      ? `<p class="note-line">${T('nearby.ts.off')}</p>`
      : peers.length
        ? `<div class="found-list">${peers.map(peerRow).join('')}</div>`
        : all.length
          ? `<p class="note-line">${T('nearby.ts.allInBook')}</p>`
          : `<p class="note-line">${T('nearby.ts.none')}</p>`;
  return `<div class="page">${head}
    <div class="section-head">${I('E701')}${T('common.sameWifi')}<span class="line"></span></div>${lan}${networkHelp}
    <div class="section-head">${I('E774')}${T('nearby.ts.head')}${t && t.running ? ` · ${esc(t.me.name)}` : ''}<span class="line"></span></div>${ts}
    <p class="note-line" style="margin-top:18px">${T('nearby.addManually')}</p></div>`;
}
function toggle(key, on, path = 'settings') {
  return `<label class="switch"><input type="checkbox" data-set="${esc(key)}" data-path="${path}" ${on ? 'checked' : ''}><span></span></label>`;
}

function seg(key, value, options, path = 'settings') {
  return `<div class="seg">${options.map(([v, t]) => `<button data-seg="${esc(key)}" data-path="${path}" data-v="${esc(v)}" class="${String(value) === String(v) ? 'on' : ''}">${esc(t)}</button>`).join('')}</div>`;
}

let accessibleId = 0;
function setting(icon, title, desc, control, last = false) {
  const id = `setting-label-${++accessibleId}`;
  control = control.replace(/<(input|select)(?=\s)/g, `<$1 aria-labelledby="${id}"`);
  control = control.replace('<div class="seg">', `<div class="seg" role="group" aria-labelledby="${id}">`);
  return `<div class="setting${last ? ' last' : ''}">${I(icon)}<div class="txt"><b id="${id}">${title}</b>${desc ? `<span>${desc}</span>` : ''}</div>${control}</div>`;
}

function sharePanel(sh) {
  const info = S.share || {};
  if (info.error) {
    return `<div class="setting last">${I('E7BA')}<div class="txt"><b>${T('share.vnc.error.title')}</b><span>${T('share.vnc.error.text', { error: esc(info.error) })}</span></div>
      <input class="input port" id="share-port" value="${esc(sh.port)}" inputmode="numeric" aria-label="${T('share.port')}"></div>`;
  }
  if (!info.running) return `<div class="setting last"><span class="spin"></span><div class="txt"><b>${T('stage.start')}</b></div></div>`;
  const addrs = (info.addrs || []).filter((a) => !(info.tailscale && a.kind === 'tailscale'))
    .map((a) => `<div class="url-row"><code class="url">${esc(a.addr)}</code><span class="pill mute">${T('common.sameWifi')}</span></div>`).join('');
  const ts = info.tailscale ? `<div class="url-row"><code class="url">${esc(info.tailscale)}</code><span class="pill mute">${T('share.pill.tsAnywhere')}</span></div>` : '';
  const viewers = (info.viewers || []).length
    ? `<div class="viewers">${I('E7B3')}<b>${T('share.vnc.viewers', { count: info.viewers.length })}</b><span>${info.viewers.map((v) => esc(v.peer)).join(', ')}</span><button class="btn sm danger" data-act="share-kick">${T('share.vnc.kick')}</button></div>`
    : '';
  return `<div class="setting">${I('E8EA')}<div class="txt">
        <b>${T('share.vnc.connectHow')}</b>
        <span>${T('share.vnc.notBrowser')}</span>
        <div class="urls">${addrs}${ts}</div>
        <div class="pin-row"><span>${T('share.vnc.password')}</span><code class="pin" id="share-pw" data-pin="${esc(info.password)}">••••••••</code>
          <button class="btn sm" data-act="share-show">${I('E7B3')}${T('common.show')}</button>
          <button class="btn sm subtle" data-act="share-reset">${I('E72C')}${T('share.vnc.newPassword')}</button></div>
        <span>${T('share.vnc.nearbyName', { name: esc(info.name) })}</span>
        ${viewers}
      </div></div>
    ${setting('E8D7', T('share.vnc.ask'), T('share.vnc.ask.desc'), toggle('ask', sh.ask, 'share'))}
    ${sh.ask ? setting('E7F8', T('share.vnc.trustNear'), T('share.vnc.trustNear.desc'), toggle('trust_near', sh.trust_near === true, 'share')) : ''}
    ${setting('E765', T('share.vnc.cmdAsCtrl'), T('share.vnc.cmdAsCtrl.desc'), toggle('cmd_as_ctrl', sh.cmd_as_ctrl, 'share'))}
    ${setting('E890', T('share.vnc.viewOnly'), T('share.vnc.viewOnly.desc'), toggle('view_only', sh.view_only, 'share'))}
    <div class="setting last">${I('E968')}<div class="txt"><b>${T('share.port')}</b><span>${T('share.vnc.port.desc')}</span></div>
      <input class="input port" id="share-port" value="${esc(sh.port)}" inputmode="numeric" aria-label="${T('share.port')}"></div>`;
}

function webPanel(w) {
  const info = S.web || {};
  if (info.error) {
    return `<div class="setting last">${I('E7BA')}<div class="txt"><b>${T('share.web.error.title')}</b><span>${T('share.web.error.text', { error: esc(info.error) })}</span></div>
      <input class="input port" id="web-port" value="${esc(w.port)}" inputmode="numeric" aria-label="${T('share.port')}"></div>`;
  }
  if (!info.running) return `<div class="setting last"><span class="spin"></span><div class="txt"><b>${T('stage.start')}</b></div></div>`;
  const urls = (info.urls || []).filter((u) => !(info.tailscale && u.kind === 'tailscale')).map((u) => `<div class="url-row"><code class="url">${esc(u.url)}</code>${u.kind === 'tailscale' ? `<span class="pill mute">${T('share.pill.tsOutside')}</span>` : `<span class="pill mute">${T('common.sameWifi')}</span>`}</div>`).join('') || `<span>${MAC ? T('share.web.noAddress.mac') : T('share.web.noAddress.pc')}</span>`;
  const ts = info.tailscale;
  const useTs = ts && S.qrMode === 'ts';
  const qrSwitch = ts ? `<div class="seg qr-seg"><button data-act="qr-lan" class="${useTs ? '' : 'on'}">${T('common.sameWifi')}</button><button data-act="qr-ts" class="${useTs ? 'on' : ''}">Tailscale</button></div>` : '';
  const tsBlock = !ts ? '' : `<div class="ts-box">
      <div class="url-row"><code class="url">${esc(ts.url)}</code><span class="pill mute">${T('share.pill.tsAnywhere')}</span></div>
      <span>${T('share.web.ts.text')}</span>
      <div class="pin-row">${ts.serving
        ? `${I('E72E')}<span>${T('share.web.ts.serving')}</span><button class="btn sm subtle" data-act="ts-serve-off">${T('share.web.ts.off')}</button>`
        : ts.https
          ? `<button class="btn sm" data-act="ts-serve-on">${I('E72E')}${T('share.web.ts.makeHttps')}</button><span>${T('share.web.ts.makeHttps.desc')}</span>`
          : `<span>${T('share.web.ts.enableHttps')}</span>`}</div>
      ${ts.error ? `<span class="err">${esc(ts.error)}</span>` : ''}
    </div>`;
  return `<div class="setting web-panel">
      <div class="qr-col"><div class="qr" aria-label="${T('share.web.qr')}">${(useTs ? ts.qr : info.qr) || ''}</div>${qrSwitch}</div>
      <div class="txt">
        <b>${T('share.web.scan')}</b>
        <div class="urls">${urls}</div>${tsBlock}
        <div class="pin-row"><span>${T('share.web.pin')}</span><code class="pin" id="web-pin" data-pin="${esc(info.pin)}">••••••</code>
          <button class="btn sm" data-act="pin-show">${I('E7B3')}${T('common.show')}</button>
          <button class="btn sm subtle" data-act="pin-reset">${I('E72C')}${T('share.web.newPin')}</button></div>
        <span>${MAC ? T('share.web.safety.mac') : T('share.web.safety.pc')}</span>
      </div></div>
    <div class="setting last">${I('E968')}<div class="txt"><b>${T('share.port')}</b><span>${T('share.web.port.desc')}</span></div>
      <input class="input port" id="web-port" value="${esc(w.port)}" inputmode="numeric" aria-label="${T('share.port')}"></div>`;
}

// One place for everything other devices use: the web viewer (browsers) and this PC's
// VNC server (other Itdas, VNC apps). Turning on "this PC from a browser" needs both.
function renderShare() {
  if (MAC) return renderShareMac();
  const st = S.settings;
  const web = st.web || {}, sh = st.share || {};
  const both = !web.on && !sh.on
    ? `<div class="share-hero">${I('E8EA')}<div class="txt"><b>${T('share.hero.pc.title')}</b>
        <span>${T('share.hero.pc.text')}</span></div>
        <button class="btn primary" data-act="share-all-on">${I('E768')}${T('share.hero.pc.allOn')}</button></div>`
    : '';
  const nudge = sh.on && !web.on
    ? `<div class="banner warn">${I('E7BA')}<div class="txt"><b>${T('share.nudge.title')}</b><span>${T('share.nudge.text', { port: esc(sh.port) })}</span></div>
        <button class="btn primary" data-act="web-on">${T('share.web.turnOn')}</button></div>`
    : '';
  return `<div class="page share-page">
    ${both}${nudge}
    <div class="group-title" id="web-viewer">${I('E774')} ${T('share.group.web')} <span class="pill ok">${T('share.recommended')}</span><a href="#" class="help-link" data-help="phone">${T('common.learnMore')}</a></div>
    ${setting('E8EA', T('share.web.title'), T('share.web.desc.pc'), toggle('on', web.on, 'web'), !web.on)}
    ${web.on ? webPanel(web) : ''}
    <div class="group-title" id="share-pc">${I('E7F8')} ${T('share.group.pc')}<a href="#" class="help-link" data-help="settings#share">${T('common.explain')}</a></div>
    ${setting('E7F8', T('share.pc.title'), T('share.pc.desc'), toggle('on', sh.on, 'share'), !sh.on)}
    ${sh.on ? sharePanel(sh) : ''}
  </div>`;
}

// macOS: the web viewer as on Windows; this Mac itself is shared with the system's own
// Screen Sharing, so the page shows how to turn it on and what others should type.
function renderShareMac() {
  const web = S.settings.web || {};
  const info = S.share || {};
  const hero = !web.on
    ? `<div class="share-hero">${I('E8EA')}<div class="txt"><b>${T('share.hero.mac.title')}</b>
        <span>${T('share.hero.mac.text')}</span></div>
        <button class="btn primary" data-act="web-on">${I('E768')}${T('share.web.turnOn')}</button></div>`
    : '';
  const addr = info.local_name || 'mac.local';
  const ts = info.tailscale ? `<div class="url-row"><code class="url">${esc(info.tailscale)}</code><span class="pill mute">${T('share.pill.tsAnywhere')}</span></div>` : '';
  return `<div class="page share-page">
    ${hero}
    <div class="group-title" id="web-viewer">${I('E774')} ${T('share.group.web')} <span class="pill ok">${T('share.recommended')}</span><a href="#" class="help-link" data-help="phone">${T('common.learnMore')}</a></div>
    ${setting('E8EA', T('share.web.title'), T('share.web.desc.mac'), toggle('on', web.on, 'web'), !web.on)}
    ${web.on ? webPanel(web) : ''}
    <div class="group-title">${I('E945')} ${T('share.mac.hpGroup')}</div>
    <div class="setting last">${I('E73E')}<div class="txt"><b>${T('share.mac.hp.title')}</b><span>${T('share.mac.hp.desc')}</span></div><button class="btn" data-act="permission-helper">${T('share.mac.hp.open')}</button></div>
    <div class="group-title" id="share-pc">${I('E7F4')} ${T('share.mac.group')}<a href="#" class="help-link" data-help="share">${T('common.learnMore')}</a></div>
    <div class="setting">${I('E7F4')}<div class="txt"><b>${T('share.mac.ss.title')}</b>
        <span>${T('share.mac.ss.desc')}</span></div>
      <button class="btn" data-act="sys-sharing">${I('E8A7')}${T('share.mac.ss.open')}</button></div>
    <div class="setting last">${I('E946')}<div class="txt"><b>${T('share.mac.howto.title')}</b>
        <span>${info.user ? T('share.mac.howto.user', { user: esc(info.user) }) : T('share.mac.howto.noUser')}</span>
        <div class="urls"><div class="url-row"><code class="url">${esc(addr)}</code><span class="pill mute">${T('common.sameWifi')}</span></div>${ts}</div>
        <span>${T('share.mac.howto.ssh')}</span></div></div>
  </div>`;
}

function renderSettings() {
  const st = S.settings;
  const d = st.defaults || {};
  const p = hk();
  const keys = [
    [T('settings.keys.fullscreen'), 'Enter'], [T('settings.keys.fit'), '0 / 1'], [T('settings.keys.stats'), 'S'], [T('settings.keys.viewOnly'), 'V'], [T('settings.keys.screenshot'), 'P'],
    [T('settings.keys.info'), 'I'], [T('settings.keys.pinToolbar'), 'T'], [T('settings.keys.reconnect'), 'R'], [T('settings.keys.menu'), 'M'],
  ];
  const more = (k) => `<a href="#" class="help-link" data-help="settings#${k}">${T('common.explain')}</a>`;
  const scrollSpeeds = [['normal', T('settings.scroll.normal')], ['fast', T('settings.scroll.fast')], ['faster', T('settings.scroll.faster')]];
  return `<div class="page">
    <div class="banner">${I('E897')}<div class="txt"><b>${T('settings.banner.title')}</b><span>${T('settings.banner.text')}</span></div>
      <button class="btn" data-help="settings">${I('E713')}${T('settings.banner.more')}</button></div>
    <div class="group-title">${T('settings.group.look')}${more('look')}</div>
    ${setting('E790', T('settings.theme'), MAC ? T('settings.theme.desc.mac') : T('settings.theme.desc.pc'), seg('theme', st.theme, [['system', T('settings.theme.system')], ['light', T('settings.theme.light')], ['dark', T('settings.theme.dark')]]))}
    ${setting('E774', T('settings.lang'), T('settings.lang.desc'), seg('lang', st.lang || 'auto', [['auto', T('lang.auto')], ['ko', T('lang.ko')], ['en', T('lang.en')]]))}
    ${setting('F0E2', T('settings.view'), '', seg('view', st.view, [['grid', T('settings.view.grid')], ['list', T('settings.view.list')]]))}
    ${setting('E71E', T('settings.zoom'), MAC ? T('settings.zoom.desc.mac') : T('settings.zoom.desc.pc'), seg('zoom', st.zoom || 100, [[90, '90%'], [100, '100%'], [110, '110%'], [125, '125%'], [150, '150%']]), true)}

    ${MAC ? `<div class="group-title">${T('settings.group.session')}</div>
    ${setting('E962', T('settings.macScroll.mac'), T('settings.macScroll.mac.desc'), seg('mac_scroll', st.mac_scroll || 'fast', scrollSpeeds), true)}` : ''}
    ${MAC ? '' : `<div class="group-title">${T('settings.group.session')}${more('session')}<a href="#" class="help-link" data-help="keys">${T('settings.allShortcuts')}</a></div>
    ${setting('E7C4', T('settings.toolbar'), T('settings.toolbar.desc'), `<select class="input" data-select="toolbar"><option value="auto" ${st.toolbar === 'auto' ? 'selected' : ''}>${T('settings.toolbar.auto')}</option><option value="pinned" ${st.toolbar === 'pinned' ? 'selected' : ''}>${T('settings.toolbar.pinned')}</option><option value="hidden" ${st.toolbar === 'hidden' ? 'selected' : ''}>${T('settings.toolbar.hidden')}</option></select>`)}
    ${setting('E765', T('settings.hotkey'), T('settings.hotkey.desc'), seg('hotkey', st.hotkey, [['ctrl_alt', 'Ctrl+Alt'], ['ctrl_shift', 'Ctrl+Shift']]))}
    ${setting('E962', T('settings.macScroll.pc'), T('settings.macScroll.pc.desc'), seg('mac_scroll', st.mac_scroll || 'fast', scrollSpeeds))}
    ${setting('E945', T('settings.scrollAccel'), T('settings.scrollAccel.desc'), toggle('scroll_accel', st.scroll_accel !== false))}
    <div class="setting last" style="align-items:flex-start">${I('E8AB')}<div class="txt"><b>${T('settings.shortcuts')}</b><table class="shortcuts">${keys.map(([a, k]) => `<tr><td>${a}</td><td><kbd>${p}+${k}</kbd></td></tr>`).join('')}</table></div></div>`}

    <div class="group-title">${T('settings.group.defaults')}${more('defaults')}</div>
    ${setting('E9A6', T('settings.scale'), '', seg('actual', d.actual, [[false, T('settings.scale.fit')], [true, T('settings.scale.actual')]], 'defaults'))}
    ${setting('E77F', T('settings.clipboard'), MAC ? T('settings.clipboard.desc.mac') : T('settings.clipboard.desc.pc'), toggle('clipboard', d.clipboard, 'defaults'))}
    ${setting('E72C', T('settings.reconnect'), T('settings.reconnect.desc'), toggle('reconnect', d.reconnect, 'defaults'), MAC)}
    ${MAC ? '' : `${setting('E765', T('settings.swap'), T('settings.swap.desc'), toggle('swap', d.swap, 'defaults'))}
    ${setting('E8C1', T('settings.hangulKey'), T('settings.hangulKey.desc'), seg('hangul_caps', d.hangul_caps, [[false, 'Ctrl+Space'], [true, 'Caps Lock']], 'defaults'), true)}`}

    <div class="group-title">${T('settings.group.share')}</div>
    ${setting('E701', MAC ? T('settings.announce.mac') : T('settings.announce.pc'), MAC ? T('settings.announce.desc.mac') : T('settings.announce.desc.pc'), toggle('announce', st.announce !== false))}
    ${setting('E72D', MAC ? T('settings.sharePage.mac') : T('settings.sharePage.pc'), MAC ? T('settings.sharePage.desc.mac') : T('settings.sharePage.desc.pc'), `<button class="btn" data-act="page" data-page="share">${I('E72D')}${T('settings.sharePage.open')}</button>`, true)}

    <div class="group-title">${T('settings.group.app')}${more('app')}</div>
    ${MAC ? '' : setting('E8A5', T('settings.tray'), T('settings.tray.desc'), toggle('tray', st.tray))}
    ${S.loginItem === 'approval' || S.loginItem === 'disabled'
      ? setting('E7E8', S.loginItem === 'approval' ? T('settings.login.approval') : T('settings.login.disabled'), S.loginItem === 'approval' ? T('settings.login.approval.desc') : T('settings.login.disabled.desc'), `<button class="btn" data-act="login-item-settings">${S.loginItem === 'approval' ? T('settings.login.approval.open') : T('settings.login.disabled.open')}</button>${toggle('login_start', true)}`)
      : setting('E7E8', T('settings.login'), MAC ? T('settings.login.desc.mac') : T('settings.login.desc.pc'), toggle('login_start', !!st.login_start))}
    ${setting('EA8F', T('settings.notify'), T('settings.notify.desc'), toggle('notify', st.notify))}
    ${setting('E91B', T('settings.thumbnails'), MAC ? T('settings.thumbnails.desc.mac') : T('settings.thumbnails.desc.pc'), toggle('thumbnails', st.thumbnails), true)}

    <div class="group-title">${T('settings.group.update')}</div>
    ${setting('E896', T('settings.remoteSetup'), T('settings.remoteSetup.desc'), `<button class="btn" data-act="remote-setup">${T('settings.remoteSetup.open')}</button>`)}
    ${setting('E896', S.update ? T('settings.update.new', { version: esc(S.update.version) }) : `Itda ${esc(S.version)}`, S.updateStage ? esc(S.updateStage) : S.update ? `${T('settings.update.current', { version: esc(S.version) })}${S.update.page ? ' <a href="' + esc(S.update.page) + '" data-ext>' + T('settings.update.changes') + '</a>' : ''}` : T('settings.update.check.desc'), S.update ? `<button class="btn primary" data-act="update-install" ${S.updateStage ? 'disabled' : ''}>${I('E896')}${T('settings.update.install')}</button>` : `<button class="btn" data-act="update-check">${I('E72C')}${T('settings.update.check')}</button>`)}
    ${setting('E823', T('settings.autoUpdate'), T('settings.autoUpdate.desc'), toggle('auto_update', st.auto_update !== false), true)}

    <div class="group-title">${T('settings.group.about')}</div>
    <div class="setting last"><div class="logo" style="width:36px;height:36px"></div><div class="txt"><b>Itda ${esc(S.version)}</b><span>${T('settings.about.tagline')}</span></div>
      <button class="btn" data-act="onboard">${I('E768')}${T('common.startGuide')}</button>
      <button class="btn" data-act="open" data-what="log">${I('E9F9')}${T('settings.about.log')}</button>
      <button class="btn" data-act="open" data-what="folder">${I('E8B7')}${T('settings.about.folder')}</button>
      ${MAC ? '' : `<button class="btn" data-act="open" data-what="screenshots">${I('E722')}${T('settings.about.screenshots')}</button>`}</div>
  </div>`;
}

// New previews, loaded and decoded before render() swaps them in (see morph).
const READY = new Map();

// Bring el's children in line with `html`, changing only what differs. Rebuilding with
// innerHTML made every redraw (sessions, online checks, previews) flash: previews decoded
// again behind their placeholder, a hovered card lost its connect button until the mouse
// moved, spinners restarted, and a field being typed in lost focus. Nodes keep their
// identity here; keyed ones (id, data-id, data-page) are matched by key.
function morph(el, html) {
  const t = document.createElement('template');
  t.innerHTML = html;
  morphChildren(el, t.content);
}

function morphKey(n) {
  if (n.nodeType !== 1) return null;
  const k = n.id || n.dataset.id || n.dataset.page || n.dataset.help;
  return k ? `${n.nodeName}|${k}|${n.dataset.act || ''}` : null;
}

function morphChildren(parent, next) {
  let a = parent.firstChild;
  for (let b = next.firstChild; b; ) {
    const nb = b.nextSibling;
    const key = morphKey(b);
    let m = null;
    if (key) {
      for (let s = a; s; s = s.nextSibling) {
        if (morphKey(s) === key) { m = s; break; }
      }
    } else if (a && !morphKey(a) && a.nodeType === b.nodeType && a.nodeName === b.nodeName) {
      m = a;
    }
    if (m) {
      if (m !== a) parent.insertBefore(m, a);
      a = m.nextSibling;
      morphNode(m, b);
    } else {
      parent.insertBefore(b, a);
    }
    b = nb;
  }
  while (a) {
    const n = a.nextSibling;
    parent.removeChild(a);
    a = n;
  }
}

function morphNode(old, b) {
  if (old.nodeType !== 1) {
    if (old.nodeValue !== b.nodeValue) old.nodeValue = b.nodeValue;
    return;
  }
  if (old.nodeName === 'IMG' && old.getAttribute('src') !== b.getAttribute('src')) {
    // A new preview: the one decoded ahead of time if there is one (no blank frame).
    old.replaceWith(READY.get(b.getAttribute('src')) || b);
    return;
  }
  for (const { name } of [...old.attributes]) {
    if (!b.hasAttribute(name)) old.removeAttribute(name);
  }
  for (const { name, value } of [...b.attributes]) {
    if (old.getAttribute(name) !== value) old.setAttribute(name, value);
  }
  if (old.classList.contains('i')) {
    // Icon glyphs become SVG outside Windows (icons.js): compare the glyph, not the SVG.
    const g = b.textContent;
    if ((old._glyph ?? old.textContent) !== g) {
      old.textContent = g;
    }
    old._glyph = g;
    return;
  }
  morphChildren(old, b);
  if (old !== document.activeElement) {
    if ('checked' in old && old.type !== undefined && old.checked !== b.checked) old.checked = b.checked;
    if (('value' in old) && (old.nodeName === 'INPUT' || old.nodeName === 'TEXTAREA' || old.nodeName === 'SELECT') && old.value !== b.value) {
      old.value = old.nodeName === 'SELECT' ? (b.querySelector('option[selected]')?.value ?? old.value) : b.value;
    }
  }
}

function render() {
  renderNav();
  const nu = $('#nav-update');
  if (nu) { nu.hidden = !S.update; nu.title = S.update ? T('nav.update.title', { version: S.update.version }) : ''; }
  renderTop();
  const c = $('#content');
  const focusId = document.activeElement && document.activeElement.closest && document.activeElement.closest('[data-id]') && c.contains(document.activeElement) ? document.activeElement.closest('[data-id]').dataset.id : null;
  const scroll = c.scrollTop;
  const p = S.page;
  morph(c, p === 'files' ? (window.ItdaFiles?.renderBrowser() || '') : p === 'accounts' ? renderAccounts() : p === 'settings' ? renderSettings() : p === 'share' ? renderShare() : p === 'help' ? renderHelp() : p === 'nearby' ? renderNearby() : renderHosts());
  READY.clear();
  c.scrollTop = scroll;
  if (focusId) {
    const el = c.querySelector(`.card[data-id="${CSS.escape(focusId)}"], .list-row[data-id="${CSS.escape(focusId)}"]`);
    if (el) el.focus({ preventScroll: true });
  }
}

function go(page) {
  if (page === 'files' && !window.ITDA_FILE_HOST) return send('files_window', { id: '__network' });
  S.page = page;
  S.query = '';
  $('#search').value = '';
  $('#content').scrollTop = 0;
  if (page === 'nearby' && !S.found.length) scan();
  if (page === 'share') { send('web_info'); send('share_info'); }
  if (page === 'settings') send('login_item');
  render();
}

// ---------------------------------------------------------------- background transfers

// Copies and moves that keep running after their file window closed (files.js keeps them):
// a sidebar entry while there are any, and a list to follow, stop or dismiss them.
const transfers = () => (window.ITDA_FILE_HOST ? [] : window.ItdaFiles?.transfers() || []);
const TRANSFER_STATE = () => ({ queued: T('transfers.state.queued'), running: T('transfers.state.running'), completed: T('transfers.state.completed'), failed: T('transfers.state.failed'), cancelled: T('transfers.state.cancelled') });
function transfersNav() {
  const all = transfers();
  if (!all.length) return '';
  const live = all.filter(j => ['queued', 'running'].includes(j.state)).length;
  const failed = all.some(j => ['failed', 'cancelled'].includes(j.state));
  const text = live ? T('transfers.nav.live', { count: live }) : failed ? T('transfers.nav.failed') : T('transfers.title');
  return `<button class="nav-item nav-transfers" data-id="nav-transfers" data-transfers aria-haspopup="dialog" aria-label="${esc(text)}" title="${esc(text)}">${I('E896')}<span class="label">${T('transfers.title')}</span><span class="count">${live || (failed ? '!' : all.length)}</span></button>`;
}
function transferRows() {
  const all = transfers().slice().reverse();
  if (!all.length) return `<p class="hint">${T('transfers.empty')}</p>`;
  return `<ul class="transfer-list">${all.map(j => {
    const live = ['queued', 'running'].includes(j.state);
    const progress = live ? `<progress aria-label="${esc(T('transfers.progress', { name: j.name }))}" ${j.total ? `max="${j.total}" value="${j.bytes || 0}"` : ''}></progress>` : '';
    const done = !live && j.completed_names?.length && j.count ? ` · ${T('transfers.doneOf', { count: j.count, done: j.completed_names.length })}` : '';
    return `<li class="transfer-row ${esc(j.state || '')}" data-id="transfer-${esc(j.job)}"><div class="transfer-what"><b>${esc(j.name)}</b>
      <span>${esc(j.device)} · ${j.moving || j.request?.move ? T('transfers.move') : T('transfers.copy')} · ${esc(live && j.detail ? j.detail : TRANSFER_STATE()[j.state] || '')}${done}${j.error && j.state === 'failed' ? ` — ${esc(j.error)}` : ''}</span>${progress}</div>
      <div class="transfer-actions">${live ? `<button class="btn sm" data-id="transfer-cancel-${esc(j.job)}" data-transfer-cancel="${esc(j.job)}">${T('common.cancel')}</button>` : ''}
      <button class="btn sm" data-id="transfer-open-${esc(j.job)}" data-transfer-open="${esc(j.id)}">${['failed', 'cancelled'].includes(j.state) && j.request ? T('transfers.retry') : T('transfers.open')}</button>
      ${live ? '' : `<button class="btn sm" data-id="transfer-dismiss-${esc(j.job)}" data-transfer-dismiss="${esc(j.job)}" aria-label="${esc(T('transfers.dismiss.aria', { name: j.name }))}">${T('transfers.dismiss')}</button>`}</div></li>`;
  }).join('')}</ul>`;
}
// What the dialog's live line says: changes only when a transfer starts or ends.
function transferStatus() {
  const all = transfers(), live = all.filter(j => ['queued', 'running'].includes(j.state)).length;
  const failed = all.filter(j => j.state === 'failed').length;
  return [live ? T('transfers.status.live', { count: live }) : T('transfers.status.none'), failed ? T('transfers.status.failed', { count: failed }) : ''].filter(Boolean).join(' · ');
}
function showTransfers() {
  const m = openModal('transfers', `<div class="dialog"><div class="body"><h2>${T('transfers.title')}</h2>
    <p class="lead">${T('transfers.lead')}</p>
    <p class="transfer-status" role="status">${transferStatus()}</p>
    <div class="transfer-box" role="region" aria-label="${T('transfers.list')}">${transferRows()}</div></div>
    <div class="foot"><button class="btn primary" data-x="close">${T('common.close')}</button></div></div>`);
  m.addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    if (b.dataset.x === 'close') return closeModal('transfers');
    if (b.dataset.transferCancel) return window.ItdaFiles?.cancelTransfer(b.dataset.transferCancel);
    if (b.dataset.transferDismiss) return window.ItdaFiles?.dismissTransfer(b.dataset.transferDismiss);
    if (b.dataset.transferOpen) { closeModal('transfers'); return send('files_window', { id: b.dataset.transferOpen }); }
  });
}
window.addEventListener('itda-transfers', () => {
  if (window.ITDA_FILE_HOST || !S.settings) return;
  renderNav();
  const box = $('#modals .scrim[data-key="transfers"] .transfer-box');
  if (!box) return;
  const focused = box.contains(document.activeElement) ? document.activeElement.dataset.id : null;
  morph(box, transferRows());
  const line = $('#modals .scrim[data-key="transfers"] .transfer-status');
  if (line && line.textContent !== transferStatus()) line.textContent = transferStatus();
  if (focused) (box.querySelector(`[data-id="${CSS.escape(focused)}"]`) || box.querySelector('button') || $('#modals .scrim[data-key="transfers"] [data-x="close"]'))?.focus();
});

// ---------------------------------------------------------------- quitting with transfers running

// Quitting (or an update's restart) while files are being copied: wait for them, stop them
// (they clean up and keep their sources), or stay. The answer goes to Rust (home.rs).
function quitGuard(m) {
  const update = m.reason === 'update';
  // A stop that didn't finish: Itda stayed open. Waiting again is the safe choice; only a
  // plain quit may be forced, saying what that leaves behind.
  const body = m.stuck
    ? `<h2>${T('quit.stuck.title', { count: m.count })}</h2><p class="lead">${(update ? T('quit.stuck.lead.update') : T('quit.stuck.lead.quit'))}</p>
      <p class="hint">${(update ? T('quit.stuck.hint.update') : T('quit.stuck.hint.quit'))}</p>`
    : `<h2>${T('quit.title', { count: m.count })}</h2>
      <p class="lead">${(update ? T('quit.lead.update') : T('quit.lead.quit'))}</p>
      ${m.waiting ? `<p class="hint">${(update ? T('quit.waiting.update') : T('quit.waiting.quit'))}</p>` : ''}`;
  const buttons = m.stuck
    ? `<button class="btn" data-x="cancel">${T('common.cancel')}</button>${m.force ? `<button class="btn danger" data-x="force">${T('quit.force')}</button>` : ''}<button class="btn primary" data-x="stop" autofocus>${T('quit.waitAgain')}</button>`
    : `<button class="btn" data-x="cancel">${T('common.cancel')}</button><button class="btn danger" data-x="stop">${(update ? T('quit.stop.update') : T('quit.stop.quit'))}</button><button class="btn primary" data-x="wait" autofocus>${(update ? T('quit.wait.update') : T('quit.wait.quit'))}</button>`;
  const dlg = openModal('quit-guard', `<div class="dialog small"><div class="body">${body}</div><div class="foot">${buttons}</div></div>`, { sticky: true });
  dlg.addEventListener('click', (e) => {
    const b = e.target.closest('[data-x]');
    if (!b) return;
    closeModal('quit-guard');
    send('quit_choice', { choice: b.dataset.x, reason: update ? 'update' : 'quit', ...(m.force ? { ticket: m.force } : {}) });
  });
}
function quitPending(m) {
  let bar = $('#quit-pending');
  if (m.cancelled || !m.count) { bar?.remove(); if (m.cancelled) toast(m.stopped ? T('quit.cancelled.stopped') : T('quit.cancelled'), 'E8B7'); return; }
  if (!bar) {
    bar = document.createElement('div');
    bar.id = 'quit-pending';
    bar.className = 'quit-pending';
    bar.innerHTML = `<span role="status"></span><button class="btn sm" data-x="cancel">${T('quit.pending.cancel')}</button>`;
    bar.querySelector('button').onclick = () => send('quit_choice', { choice: 'cancel', reason: bar.dataset.reason });
    document.body.appendChild(bar);
  }
  bar.dataset.reason = m.reason;
  const update = m.reason === 'update';
  const text = m.stopping
    ? (update ? T('quit.pending.stopping.update', { count: m.count }) : T('quit.pending.stopping.quit', { count: m.count }))
    : (update ? T('quit.pending.waiting.update', { count: m.count }) : T('quit.pending.waiting.quit', { count: m.count }));
  const line = bar.querySelector('[role=status]');
  if (line.textContent !== text) line.textContent = text;
}

// ---------------------------------------------------------------- modals

const modalFocusable = (m) => [...m.querySelectorAll('button, input, select, textarea, a[href], [tabindex]')]
  .filter(e => !e.disabled && e.tabIndex >= 0 && !e.closest('[hidden], [inert]') && e.getClientRects().length);
function syncModalState() {
  const top = topModal();
  $('.app').inert = !!top;
  $$('#modals .scrim').forEach(m => { m.inert = m !== top; });
}
function openModal(key, html, opts = {}) {
  closeModal(key);
  const previous = document.activeElement;
  const scrim = document.createElement('div');
  scrim.className = 'scrim';
  scrim.dataset.key = key;
  scrim.innerHTML = html;
  scrim.returnFocus = previous;
  const dialog = scrim.querySelector('.dialog') || scrim.firstElementChild;
  dialog.setAttribute('role', 'dialog');
  dialog.setAttribute('aria-modal', 'true');
  dialog.tabIndex = -1;
  const title = dialog.querySelector('h2, h3');
  if (title) { title.id ||= `dialog-title-${++accessibleId}`; dialog.setAttribute('aria-labelledby', title.id); }
  // Associate the visible field/option label with its actual control, including nested input wraps.
  scrim.querySelectorAll('.field').forEach(field => {
    const label = field.querySelector('label:not([for])');
    const input = field.querySelector('input, select, textarea');
    if (label && input && !label.contains(input)) { input.id ||= `dialog-field-${++accessibleId}`; label.htmlFor = input.id; }
  });
  scrim.querySelectorAll('.opt').forEach(opt => {
    const label = opt.querySelector('.txt b');
    const input = opt.querySelector('input[type=checkbox]');
    if (label && input) { label.id ||= `option-label-${++accessibleId}`; input.setAttribute('aria-labelledby', label.id); }
  });
  scrim.querySelectorAll('.reveal').forEach(button => { button.removeAttribute('tabindex'); });
  scrim.addEventListener('keydown', e => {
    if (e.key !== 'Tab' || topModal() !== scrim) return;
    const items = modalFocusable(scrim), first = items[0];
    // WebKit may Tab only between text fields unless Full Keyboard Access is enabled.
    // Traverse our controls explicitly so buttons remain reachable in either OS policy.
    e.preventDefault();
    if (!first) dialog.focus();
    else {
      const i = items.indexOf(document.activeElement);
      items[i < 0 ? (e.shiftKey ? items.length - 1 : 0) : (i + (e.shiftKey ? -1 : 1) + items.length) % items.length].focus();
    }
  });
  scrim.addEventListener('mousedown', e => {
    if (e.target === scrim && !opts.sticky) closeModal(key);
  });
  $('#modals').appendChild(scrim);
  syncModalState();
  setTimeout(() => {
    if (topModal() !== scrim) return;
    const first = scrim.querySelector('[autofocus]');
    (first && modalFocusable(scrim).includes(first) ? first : modalFocusable(scrim)[0] || dialog).focus();
  }, 30);
  return scrim;
}

function closeModal(key) {
  const top = topModal();
  const removed = $$('#modals .scrim').filter(m => !key || m.dataset.key === key);
  removed.forEach(m => { m.dispatchEvent(new Event('itda-modal-close')); m.remove(); });
  syncModalState();
  if (removed.includes(top)) {
    const target = top.returnFocus;
    if (target?.isConnected && !target.closest('[inert]')) target.focus();
    else if (topModal()) (modalFocusable(topModal())[0] || topModal().querySelector('[role=dialog]')).focus();
    else $('#newBtn').focus();
  }
}

function topModal() {
  const all = $$('#modals .scrim');
  return all[all.length - 1];
}

function confirmDialog(title, text, okText, danger, onOk) {
  const m = openModal('confirm', `<div class="dialog small"><div class="body"><h2>${title}</h2><p class="lead">${text}</p></div>
    <div class="foot"><button class="btn" data-x="cancel">${T('common.cancel')}</button><button class="btn ${danger ? 'danger' : 'primary'}" data-x="ok" autofocus>${okText}</button></div></div>`);
  m.querySelector('[data-x="cancel"]').onclick = () => closeModal('confirm');
  m.querySelector('[data-x="ok"]').onclick = () => { closeModal('confirm'); onOk(); };
}

// ---------------------------------------------------------------- edit dialog

function optRow(key, title, desc, on) {
  return `<div class="opt"><div class="txt"><b>${title}</b>${desc ? `<span>${desc}</span>` : ''}</div><label class="switch"><input type="checkbox" data-opt="${key}" ${on ? 'checked' : ''}><span></span></label></div>`;
}

// ---------------------------------------------------------------- accounts

const acctById = (id) => S.accounts.find((a) => a.id === id);

function acctHint(id) {
  const a = acctById(id);
  if (!a) return '';
  return [a.user, a.has_pw ? T('accounts.pwSaved') : T('accounts.hint.noPw'), a.ssh_key ? T('accounts.sshKey') : ''].filter(Boolean).join(' · ');
}

/// The connection editor shows its own name/password only without an account.
function syncAccount(m) {
  const id = m.querySelector('#f-account').value;
  m.querySelector('#own-creds').hidden = !!id;
  m.querySelector('#acct-hint').textContent = id ? acctHint(id) : T('editor.accountHint');
}

/// Same sign-in name on several connections without an account: offer to group them.
function accountSuggestions() {
  const by = {};
  for (const h of S.hosts) if (!h.account && h.user) (by[h.user] = by[h.user] || []).push(h);
  return Object.entries(by).filter(([, hs]) => hs.length >= 2);
}

function renderAccounts() {
  const sugg = accountSuggestions().map(([user, hs]) => `<div class="banner">${I('E77B')}<div class="txt"><b>${T('accounts.suggest.title', { user: esc(user), count: hs.length })}</b>
      <span>${T('accounts.suggest.text', { names: hs.slice(0, 3).map((h) => esc(label(h))).join(', ') + (hs.length > 3 ? ` ${T('accounts.suggest.more', { count: hs.length - 3 })}` : '') })}</span></div>
      <button class="btn primary" data-act="group" data-user="${esc(user)}">${T('accounts.suggest.group')}</button></div>`).join('');
  const list = S.accounts.length
    ? `<div class="found-list">${S.accounts.map((a) => `<div class="found acct">
        <div class="ico">${I('E77B')}</div>
        <div class="txt"><b>${esc(a.name || a.user)}</b><span>${esc(a.user)} · ${T('accounts.uses', { count: a.uses })}${a.has_pw ? ` · ${T('accounts.pwSaved')}` : ` · <em class="warn-text">${T('accounts.noPw')}</em>`}${a.ssh_key ? ` · ${T('accounts.sshKeyFile', { file: esc(a.ssh_key.split(/[\\/]/).pop()) })}` : ''}</span></div>
        <button class="btn sm" data-act="acct-edit" data-id="${esc(a.id)}">${I('E70F')}${T('common.edit')}</button>
        <button class="btn sm subtle icon" data-act="acct-del" data-id="${esc(a.id)}" title="${T('common.delete')}">${I('E74D')}</button></div>`).join('')}</div>`
    : `<div class="empty" style="margin-top:3vh"><div class="art">${I('E77B')}</div><h2>${T('accounts.empty.title')}</h2>
        <p>${T('accounts.empty.text')}</p>
        <div class="actions"><button class="btn primary" data-act="acct-new">${I('E710')}${T('accounts.new')}</button></div></div>`;
  return `<div class="page">${sugg}
    ${S.accounts.length ? `<div class="section-head">${I('E77B')}${T('accounts.count', { count: S.accounts.length })}<span class="line"></span><button class="btn sm" data-act="acct-new">${I('E710')}${T('accounts.new')}</button></div>` : ''}
    ${list}
    <p class="note-line" style="margin-top:16px">${MAC ? T('accounts.vault.mac') : T('accounts.vault.pc')}</p></div>`;
}

function openAccount(init = {}) {
  const ex = init.id ? acctById(init.id) : null;
  const a = ex ? { ...ex } : { id: '', name: '', user: init.user || '', ssh_key: '' };
  const m = openModal('account', `<div class="dialog"><div class="body">
      <h2>${ex ? T('account.edit') : T('accounts.new')}</h2>
      <p class="lead">${ex && ex.uses ? T('account.lead.uses', { count: ex.uses }) : T('account.lead.new')}</p>
      <div class="row">
        <div class="field"><label>${T('account.name')}</label><input class="input" id="a-name" value="${esc(a.name)}" placeholder="${T('account.name.ph')}"></div>
        <div class="field"><label>${T('account.user')}</label><input class="input" id="a-user" value="${esc(a.user)}" placeholder="${T('editor.user.ph.alex')}" autofocus><span class="err" id="e-user"></span></div>
      </div>
      <div class="field"><label>${T('account.password')}</label><div class="input-wrap"><input class="input" id="a-pass" type="password" placeholder="${ex && ex.has_pw ? T('common.pwSavedPh') : T('kind.mac.pass')}"><button class="reveal i" tabindex="-1" title="${T('common.showPassword')}">&#xE7B3;</button></div>
        <span class="hint">${T('account.password.hint')}</span></div>
      <div class="field"><label>${T('account.sshKey')}</label>
        <div class="row" style="gap:8px"><input class="input" id="a-key" value="${esc(a.ssh_key)}" placeholder="${T('account.sshKey.ph')}"><button class="btn" data-x="browse" style="flex:none">${I('E8B7')}${T('account.browse')}</button></div></div>
      <div class="field" id="keypass-field" ${a.ssh_key ? '' : 'hidden'}><label>${T('account.keyPass')}</label><input class="input" id="a-keypass" type="password" placeholder="${ex && ex.has_keypass ? T('account.keyPass.saved') : T('account.keyPass.ph')}"></div>
    </div><div class="foot">${ex ? `<button class="btn subtle left" data-x="delete" style="color:var(--danger)">${I('E74D')}${T('common.delete')}</button>` : ''}
      <button class="btn" data-x="cancel">${T('common.cancel')}</button><button class="btn primary" data-x="save">${T('common.save')}</button></div></div>`);
  const pass = m.querySelector('#a-pass');
  if (init.password) pass.value = init.password;
  m.querySelector('.reveal').onclick = () => { pass.type = pass.type === 'password' ? 'text' : 'password'; pass.focus(); };
  m.querySelector('#a-key').oninput = (e) => (m.querySelector('#keypass-field').hidden = !e.target.value.trim());
  m.querySelector('[data-x="browse"]').onclick = () => send('pick_file', { field: 'a-key' });
  m.querySelector('[data-x="cancel"]').onclick = () => { closeModal('account'); S.pickAccountFor = null; };
  const del = m.querySelector('[data-x="delete"]');
  if (del) del.onclick = () => { closeModal('account'); askDeleteAccount(a.id); };
  const save = () => {
    const user = m.querySelector('#a-user').value.trim();
    if (!user) { m.querySelector('#e-user').textContent = T('account.user.required'); m.querySelector('#a-user').focus(); return; }
    const acc = { ...a, name: m.querySelector('#a-name').value.trim(), user, ssh_key: m.querySelector('#a-key').value.trim() };
    delete acc.has_pw; delete acc.has_keypass; delete acc.uses;
    send('save_account', { account: acc, password: pass.value, key_pass: m.querySelector('#a-keypass').value, clear_key_pass: !acc.ssh_key });
    closeModal('account');
    toast(ex ? T('toast.account.saved') : T('toast.account.created', { name: acc.name || user }), 'E73E');
  };
  m.querySelector('[data-x="save"]').onclick = save;
  m.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target.tagName === 'INPUT') { e.preventDefault(); save(); } });
}

function askDeleteAccount(id) {
  const a = acctById(id);
  if (!a) return;
  confirmDialog(T('account.delete.title', { name: esc(a.name || a.user) }), a.uses ? T('account.delete.uses', { count: a.uses }) : T('account.delete.text'), T('common.delete'), true, () => {
    send('delete_account', { id });
    toast(T('toast.account.deleted'), 'E74D');
  });
}

// What the connection editor asks for, per kind of device.
// (The words are getters: they follow the current language.)
const KINDS = {
  mac: { kind: '', os: 'mac', label: 'Mac', get hint() { return T('kind.mac.hint'); }, hostPh: '192.168.0.10, mac-mini.local',
    get user() { return T('kind.mac.user'); }, get userPh() { return T('editor.user.ph.alex'); }, get userHint() { return T('kind.mac.userHint'); }, get pass() { return T('kind.mac.pass'); }, ssh: true },
  windows: { kind: '', os: 'windows', label: 'Windows · Itda', get hint() { return T('kind.windows.hint'); }, hostPh: 'STUDY-PC.local, 192.168.0.40',
    user: null, get pass() { return T('kind.windows.pass'); }, get passHint() { return T('kind.windows.passHint'); }, ssh: false },
  rdp: { kind: 'rdp', os: 'windows', get label() { return T('kind.rdp.label'); }, get hint() { return MAC ? T('kind.rdp.hint.mac') : T('kind.rdp.hint.pc'); }, hostPh: 'OFFICE-PC, 192.168.0.40',
    get user() { return T('kind.rdp.user'); }, get userPh() { return T('kind.rdp.userPh'); }, get userHint() { return T('kind.rdp.userHint'); },
    get pass() { return T('kind.rdp.pass'); }, get passHint() { return T('kind.rdp.passHint'); }, ssh: false },
  linux: { kind: '', os: 'linux', get label() { return T('onboard.choice.linux'); }, get hint() { return T('kind.linux.hint'); }, hostPh: 'raspberrypi.local, 192.168.0.30:5901',
    get user() { return T('kind.linux.user'); }, get userPh() { return T('kind.linux.userPh'); }, get userHint() { return T('kind.linux.userHint'); }, get pass() { return T('row.vncPassword'); }, ssh: true },
};
// Which editor kind a saved host is; older hosts without `os` were Macs (or Itda PCs with no user).
function kindOf(h) {
  if (h.kind === 'rdp') return 'rdp';
  if (KINDS[h.os]) return h.os;
  return 'mac';
}
// Words for a host's device in prompts and errors.
const devWord = (h) => (!h ? 'Mac' : { mac: 'Mac', windows: 'PC', rdp: 'PC', linux: T('error.device') }[kindOf(h)]);

function openEdit(init = {}, mode = {}) {
  const existing = init.id ? hostById(init.id) : null;
  const h = existing ? JSON.parse(JSON.stringify(existing)) : { id: '', name: init.name || '', host: init.host || '', user: init.user || '', group: '', favorite: false, kind: init.kind || '', os: init.os || '', opts: { ...(S.settings.defaults || {}) } };
  const K0 = kindOf(h);
  const KD = KINDS[K0];
  const quick = !!mode.quick;
  const hasPw = existing && existing.has_pw;
  const ssh = h.ssh || { on: false, user: '', port: 0 };
  const title = quick ? T('common.connectTo', { name: esc(h.host) }) : existing ? T('editor.title.edit') : T('common.newConnection');
  const tabs = quick ? '' : `<div class="tabs" role="tablist" aria-label="${T('editor.tabs')}">
      <button role="tab" aria-selected="true" class="on" data-tab="general">${T('editor.tab.general')}</button><button role="tab" aria-selected="false" tabindex="-1" data-tab="display">${T('editor.tab.display')}</button>${MAC ? '' : `<button role="tab" aria-selected="false" tabindex="-1" data-tab="input">${T('editor.tab.input')}</button>`}<button role="tab" aria-selected="false" tabindex="-1" data-tab="conn">${T('editor.tab.conn')}</button></div>`;
  const chipList = (gs) => gs.map((g) => `<span class="chip" data-chip="${esc(g)}">${esc(g)}<button type="button" class="chip-x" data-chip-x="${esc(g)}" aria-label="${esc(T('editor.group.remove', { group: g }))}">×</button></span>`).join('');
  const groupField = quick ? '' : `<div class="field"><label for="f-group">${T('editor.group')}</label>
      <div class="group-wrap"><div class="chips input" id="group-chips">${chipList(hostGroups(h))}<input id="f-group" autocomplete="off" role="combobox" aria-autocomplete="list" aria-controls="group-suggest" aria-expanded="false" placeholder="${hostGroups(h).length ? '' : T('editor.group.ph')}" aria-label="${T('editor.group.add')}"></div>
      <div class="group-suggest" id="group-suggest" role="listbox" hidden></div></div>
      <span class="hint">${T('editor.group.hint')}</span></div>`;
  const sshBox = `<div class="ssh-box" ${KD.ssh ? '' : 'hidden'}>
        <div class="opt"><span class="i ssh-ico">&#xE72E;</span><div class="txt"><b>${T('editor.ssh')} <a href="#" class="help-link" data-help="accounts">${T('common.learnMore')}</a></b><span>${T('editor.ssh.desc')}</span></div>
          <label class="switch"><input type="checkbox" id="f-ssh" ${ssh.on ? 'checked' : ''}><span></span></label></div>
        <div class="row" id="ssh-fields" ${ssh.on ? '' : 'hidden'}>
          <div class="field"><label>${T('editor.ssh.user')}</label><input class="input" id="f-ssh-user" value="${esc(ssh.user)}" placeholder="${T('editor.ssh.user.ph')}"></div>
          <div class="field"><label>${T('editor.ssh.port')}</label><input class="input" id="f-ssh-port" value="${ssh.port || ''}" placeholder="22" inputmode="numeric"><span class="err" id="e-ssh"></span></div>
        </div>
        ${S.sshOk === false ? `<p class="warn-note" id="ssh-missing" ${ssh.on ? '' : 'hidden'}>${I('E7BA')} ${T('editor.ssh.missing')}</p>` : ''}
      </div>`;
  const hp = h.hp || {};
  const hpBox = MAC ? `<div class="field hp-box" ${h.kind !== 'rdp' && (h.os || 'mac') === 'mac' ? '' : 'hidden'}><label>${T('editor.hp.group')}</label>
      <div class="opt"><span class="hp-ico" aria-hidden="true">⚡</span><div class="txt"><b>${T('editor.hp.title')}</b><span id="hp-state">${hp.on ? T('editor.hp.stateOn') : T('editor.hp.stateOff')}</span></div>
        ${hp.on ? '<label class="switch"><input type="checkbox" id="f-hp" checked><span></span></label>' : (existing ? `<button type="button" class="btn sm" id="hp-enable">${T('editor.hp.enable')}</button>` : '')}</div>
      <span class="hint">${T('editor.hp.hint')}</span></div>` : '';
  const general = `<div data-pane="general">
      ${quick ? '' : `<div class="field"><label>${T('editor.kind')}</label><div class="seg kind-seg">
          ${Object.entries(KINDS).map(([k, d]) => `<button type="button" data-kind="${k}" class="${k === K0 ? 'on' : ''}">${d.label}</button>`).join('')}</div>
        <span class="hint" id="kind-hint">${KD.hint}</span></div>`}
      ${quick ? '' : `<div class="field"><label>${T('editor.name')}</label><input class="input" id="f-name" value="${esc(h.name)}" placeholder="${T('editor.name.ph')}"></div>`}
      ${quick ? '' : `<div class="field"><label>${T('editor.host')}</label><input class="input" id="f-host" value="${esc(h.host)}" placeholder="${KD.hostPh}" ${existing ? '' : 'autofocus'}><span class="err" id="e-host"></span></div>`}
      ${groupField}
      <div class="field"><label>${T('editor.login')}</label>
        <select class="input" id="f-account">
          <option value="">${T('editor.login.own')}</option>
          ${S.accounts.map((a) => `<option value="${esc(a.id)}" ${h.account === a.id ? 'selected' : ''}>${T('editor.accountOption', { name: esc(a.name || a.user), user: esc(a.user) })}</option>`).join('')}
          <option value="__new">${T('editor.login.newAccount')}</option>
        </select>
        <span class="hint" id="acct-hint">${h.account ? acctHint(h.account) : T('editor.accountHint')}</span></div>
      <div id="own-creds" ${h.account ? 'hidden' : ''}>
        <div class="row">
          <div class="field" id="user-field" ${KD.user ? '' : 'hidden'}><label id="user-label">${KD.user || ''}</label><input class="input" id="f-user" value="${esc(h.user)}" placeholder="${KD.userPh || ''}" ${quick || existing ? 'autofocus' : ''}><span class="hint" id="user-hint">${KD.userHint || ''}</span><button type="button" class="btn sm subtle" id="account-name-help" data-act="account-name-help" ${K0 === 'mac' ? '' : 'hidden'}>${T('common.findAccountName')}</button></div>
          <div class="field"><label id="pass-label">${KD.pass}</label><div class="input-wrap"><input class="input" id="f-pass" type="password" placeholder="${hasPw && !h.account ? T('common.pwSavedPh') : KD.pass}"><button class="reveal i" tabindex="-1" title="${T('common.showPassword')}">&#xE7B3;</button></div></div>
        </div>
        <label class="check"><input type="checkbox" id="f-save" ${hasPw || !existing ? 'checked' : ''}>${T('common.savePassword')} <span style="color:var(--text-3);font-size:12px">${MAC ? T('editor.savePw.where.mac') : T('editor.savePw.where.pc')}</span></label>
      </div>
      <span class="hint" id="pass-hint" ${KD.passHint ? '' : 'hidden'}>${KD.passHint || ''}</span>
${quick ? sshBox : ''}
    </div>`;
  const o = h.opts || {};
  const display = `<div data-pane="display" hidden>
      <div class="field"><label>${T('editor.monitorMode')}</label><select class="input" data-opt-number="monitor_mode">${[[0,T('editor.monitorMode.pick')],[1,T('editor.monitorMode.all')],[2,T('editor.monitorMode.windows')]].map(([v,t])=>`<option value="${v}" ${v===(o.monitor_mode||0)?'selected':''}>${t}</option>`).join('')}</select></div>
      <div class="field"><label>${T('editor.monitorLayout')}</label><select class="input" data-opt-number="monitor_layout">${[[0,T('editor.monitorLayout.server')],[1,T('editor.monitorLayout.h2')],[2,T('editor.monitorLayout.v2')],[3,T('editor.monitorLayout.h3')],[4,T('editor.monitorLayout.grid')]].map(([v,t])=>`<option value="${v}" ${v===(o.monitor_layout||0)?'selected':''}>${t}</option>`).join('')}</select><span class="hint">${T('editor.monitorLayout.hint')}</span></div>
      ${optRow('monitor_fullscreen', T('editor.opt.monitorFullscreen'), T('editor.opt.monitorFullscreen.desc'), o.monitor_fullscreen)}
      ${optRow('actual', T('editor.opt.actual'), T('editor.opt.actual.desc'), o.actual)}
      ${optRow('fullscreen', T('editor.opt.fullscreen'), '', o.fullscreen)}
      ${optRow('resize', T('editor.opt.resize'), MAC ? T('editor.opt.resize.desc.mac') : T('editor.opt.resize.desc.pc'), o.resize !== false)}
      ${optRow('view_only', T('settings.keys.viewOnly'), T('editor.opt.viewOnly.desc'), o.view_only)}
      ${MAC ? '' : optRow('stats', T('settings.keys.stats'), T('editor.opt.stats.desc'), o.stats)}
    </div>`;
  const input = `<div data-pane="input" hidden>
      ${optRow('swap', T('settings.swap'), T('editor.opt.swap.desc'), o.swap)}
      ${optRow('hangul_caps', T('editor.opt.hangulCaps'), T('editor.opt.hangulCaps.desc'), o.hangul_caps)}
    </div>`;
  const conn = `<div data-pane="conn" hidden>
      <div class="field"><label>${T('editor.connMethod')}</label></div>
      ${sshBox}
      ${hpBox}
      <div class="field"><label>${T('editor.lan')}</label><input class="input" id="f-lan" value="${esc(h.lan || '')}" placeholder="${T('editor.lan.ph')}">
        <span class="hint">${T('editor.lan.hint')}</span></div>
      ${optRow('clipboard', T('settings.clipboard'), T('editor.opt.clipboard.desc'), o.clipboard)}
      ${optRow('reconnect', T('settings.reconnect'), T('editor.opt.reconnect.desc'), o.reconnect)}
    </div>`;
  const foot = quick
    ? `<button class="btn" data-x="cancel">${T('common.cancel')}</button><button class="btn primary" data-x="connect">${I('E768')}${T('common.connect')}</button>`
    : `${existing ? `<button class="btn subtle left" data-x="delete" style="color:var(--danger)">${I('E74D')}${T('common.delete')}</button>` : ''}
       <button class="btn" data-x="cancel">${T('common.cancel')}</button><button class="btn" data-x="save">${T('common.save')}</button><button class="btn primary" data-x="connect">${I('E768')}${T('editor.saveConnect')}</button>`;
  const m = openModal('edit', `<div class="dialog wide"><div class="body"><h2>${title}</h2>
      <p class="lead">${quick ? T('editor.lead.quick') : T('editor.lead')}</p>
      ${tabs}${general}${quick ? '' : display + input + conn}</div><div class="foot">${foot}</div></div>`);

  m.dataset.host = h.id || '';
  m.querySelector('#account-name-help').onclick = showAccountNameHelp;
  m.querySelectorAll('[data-tab]').forEach((b) => (b.onclick = () => {
    m.querySelectorAll('[data-tab]').forEach(x => { x.classList.toggle('on', x === b); x.setAttribute('aria-selected', x === b); x.tabIndex = x === b ? 0 : -1; });
    m.querySelectorAll('[data-pane]').forEach((p) => (p.hidden = p.dataset.pane !== b.dataset.tab));
  }));
  m.querySelector('.tabs')?.addEventListener('keydown', e => {
    const tabs = [...m.querySelectorAll('[role=tab]')], i = tabs.indexOf(document.activeElement);
    if (i < 0 || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
    e.preventDefault();
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? tabs.length - 1 : (i + (e.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
    tabs[next].click(); tabs[next].focus();
  });
  const chips = m.querySelector('#group-chips');
  if (chips) {
    const gin = chips.querySelector('#f-group');
    const current = () => [...chips.querySelectorAll('[data-chip]')].map((c) => c.dataset.chip);
    const add = (raw) => {
      const known = groups();
      for (let g of raw.split(',').map((x) => x.trim().slice(0, 40)).filter(Boolean)) {
        // A group other connections already have keeps their spelling ("home" joins "Home").
        g = known.find((k) => k.toLowerCase() === g.toLowerCase()) || g;
        if (current().some((x) => x.toLowerCase() === g.toLowerCase())) continue;
        gin.insertAdjacentHTML('beforebegin', chipList([g]));
      }
      gin.value = '';
      gin.placeholder = current().length ? '' : T('editor.group.ph');
    };
    // Suggestions: the other connections' groups not on this one yet, matching what's typed
    // (anywhere in the name). The webviews don't all show <datalist>, so they are drawn here.
    const box = m.querySelector('#group-suggest');
    let pick = -1;
    const shown = () => [...box.querySelectorAll('[data-sug]')];
    const suggest = () => {
      const q = gin.value.trim().toLowerCase();
      const have = current().map((x) => x.toLowerCase());
      const list = groups().filter((g) => !have.includes(g.toLowerCase()) && (!q || g.toLowerCase().includes(q)))
        .sort((a, b) => (b.toLowerCase().startsWith(q) - a.toLowerCase().startsWith(q))).slice(0, 8);
      pick = q && list.length ? 0 : -1;
      box.innerHTML = list.map((g, i) => `<button type="button" class="sug" role="option" id="group-sug-${i}" tabindex="-1" data-sug="${esc(g)}">${esc(g)}</button>`).join('');
      show(!!list.length && document.activeElement === gin);
      mark();
    };
    const show = (on) => { box.hidden = !on; gin.setAttribute('aria-expanded', String(on)); };
    // The highlighted choice, for the eye (.on) and for screen readers (selected, active).
    const mark = () => {
      shown().forEach((b, i) => { b.classList.toggle('on', i === pick); b.setAttribute('aria-selected', String(i === pick)); });
      if (pick >= 0 && !box.hidden) gin.setAttribute('aria-activedescendant', `group-sug-${pick}`);
      else gin.removeAttribute('aria-activedescendant');
    };
    // Pressing a choice must not take the focus from the field; the click picks it.
    box.addEventListener('pointerdown', (e) => e.preventDefault());
    box.addEventListener('mousedown', (e) => e.preventDefault());
    box.addEventListener('click', (e) => {
      const b = e.target.closest('[data-sug]');
      if (!b) return;
      add(b.dataset.sug);
      gin.focus();
      suggest();
    });
    gin.addEventListener('focus', suggest);
    gin.addEventListener('blur', () => { show(false); mark(); });
    gin.addEventListener('keydown', (e) => {
      // While composing (Korean IME), keys belong to the IME: arrows move its candidates and
      // Enter only commits the syllable. Nothing here, and nothing reaching the dialog.
      if (e.isComposing || e.keyCode === 229) { if (e.key === 'Enter' || e.key === 'Escape') e.stopPropagation(); return; }
      // Enter adds a group here; it must not also save the dialog. (Empty Enter still saves.)
      if (!box.hidden && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
        e.preventDefault();
        const n = shown().length;
        pick = e.key === 'ArrowDown' ? (pick + 1) % n : (pick <= 0 ? n - 1 : pick - 1);
        mark();
        return;
      }
      if (!box.hidden && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); show(false); mark(); return; }
      // A highlighted suggestion wins over the typed text (Enter or Tab).
      if (!box.hidden && pick >= 0 && (e.key === 'Enter' || e.key === 'Tab' && gin.value.trim()) && !e.isComposing) {
        e.preventDefault(); e.stopPropagation(); add(shown()[pick].dataset.sug); suggest(); return;
      }
      if (e.key === 'Enter' && gin.value.trim() || e.key === ',') { e.preventDefault(); e.stopPropagation(); add(gin.value); suggest(); }
      else if (e.key === 'Backspace' && !gin.value) { const last = [...chips.querySelectorAll('[data-chip]')].pop(); if (last) { last.remove(); gin.placeholder = current().length ? '' : T('editor.group.ph'); suggest(); } }
    });
    gin.addEventListener('input', () => { if (gin.value.includes(',')) add(gin.value); suggest(); });
    chips.addEventListener('click', (e) => {
      const x = e.target.closest('[data-chip-x]');
      if (x) { x.closest('[data-chip]').remove(); gin.focus(); suggest(); } else gin.focus();
    });
    m._groups = () => { if (gin.value.trim()) add(gin.value); return current(); };
  }
  const hpEnable = m.querySelector('#hp-enable');
  if (hpEnable) hpEnable.onclick = () => {
    hpEnable.disabled = true;
    hpEnable.textContent = T('common.preparing');
    window.ItdaHpSetup.begin({container:m.querySelector('.dialog'),id:h.id,name:label(h),send:(op,extra={})=>send(`hp_${op}`,{id:h.id,...extra}),done:()=>toast(T('toast.hp.prepared', { name: label(h) }), 'E73E'),cancel:()=>{if(hpEnable.isConnected){hpEnable.disabled=false;hpEnable.textContent=T('editor.hp.recheck');}}});
    send('hp_upgrade', { id: h.id });
  };
  const pass = m.querySelector('#f-pass');
  m.querySelector('.reveal').onclick = () => { pass.type = pass.type === 'password' ? 'text' : 'password'; pass.focus(); };
  m.querySelectorAll('.kind-seg button').forEach((b) => (b.onclick = () => {
    m.querySelectorAll('.kind-seg button').forEach((x) => x.classList.toggle('on', x === b));
    const d = KINDS[b.dataset.kind];
    m.querySelector('.ssh-box').hidden = !d.ssh;
    m.querySelector('#user-field').hidden = !d.user;
    m.querySelector('#user-label').textContent = d.user || '';
    m.querySelector('#f-user').placeholder = d.userPh || '';
    m.querySelector('#account-name-help').hidden = b.dataset.kind !== 'mac';
    m.querySelector('#user-hint').textContent = d.userHint || '';
    m.querySelector('#pass-label').textContent = d.pass;
    if (!(hasPw && !h.account)) pass.placeholder = d.pass;
    m.querySelector('#pass-hint').hidden = !d.passHint;
    m.querySelector('#pass-hint').textContent = d.passHint || '';
    m.querySelector('#kind-hint').textContent = d.hint;
    m.querySelector('#f-host').placeholder = d.hostPh;
    const hpb = m.querySelector('.hp-box');
    if (hpb) hpb.hidden = d.kind === 'rdp' || d.os !== 'mac';
  }));
  m.querySelector('#f-account').onchange = (e) => {
    if (e.target.value === '__new') {
      e.target.value = '';
      S.pickAccountFor = m;
      return openAccount({ user: m.querySelector('#f-user').value.trim(), password: pass.value });
    }
    syncAccount(m);
  };
  m.querySelector('#f-ssh').onchange = (e) => {
    m.querySelector('#ssh-fields').hidden = !e.target.checked;
    const w = m.querySelector('#ssh-missing');
    if (w) w.hidden = !e.target.checked;
  };

  const collect = () => {
    const v = (id) => (m.querySelector(id) ? m.querySelector(id).value.trim() : null);
    const host = { ...h, opts: { ...h.opts } };
    if (!quick) {
      host.name = v('#f-name');
      host.host = v('#f-host');
      host.groups = m._groups ? m._groups() : hostGroups(h);
      host.group = host.groups.join(', ');
    }
    host.user = v('#f-user');
    if (m.querySelector('#f-lan')) host.lan = v('#f-lan');
    const acct = m.querySelector('#f-account').value;
    host.account = acct === '__new' ? '' : acct;
    const k = KINDS[(m.querySelector('.kind-seg button.on') || { dataset: { kind: K0 } }).dataset.kind];
    host.kind = k.kind;
    host.os = k.os;
    if (!k.user) host.user = '';
    // High-performance mode is set up outside this draft (hp_upgrade): keep the host's latest
    // state, unless it was switched off here.
    const live = S.hosts.find((x) => x.id === h.id);
    if (live && live.hp) host.hp = { ...live.hp };
    const hpOff = m.querySelector('#f-hp');
    if (hpOff && !hpOff.checked) host.hp = { ...(host.hp || {}), on: false };
    host.ssh = { on: k.ssh && m.querySelector('#f-ssh').checked, user: v('#f-ssh-user') || '', port: parseInt(v('#f-ssh-port'), 10) || 0 };
    m.querySelectorAll('[data-opt]').forEach((c) => (host.opts[c.dataset.opt] = c.checked));
    m.querySelectorAll('[data-opt-number]').forEach(c => host.opts[c.dataset.optNumber] = Number(c.value));
    return host;
  };
  const validate = (host) => {
    const e = m.querySelector('#e-host');
    let msg = '';
    if (!host.host) msg = T('editor.err.hostRequired');
    else if (/\s/.test(host.host)) msg = T('editor.err.hostSpace');
    else {
      const port = host.host.match(/^[^\[\]]+:(\d*)$/) || host.host.match(/^\[.*\]:(\d*)$/);
      if (port && !(+port[1] > 0 && +port[1] < 65536)) msg = T('editor.err.port');
    }
    if (e) { e.textContent = msg; m.querySelector('#f-host').classList.toggle('bad', !!msg); }
    const raw = m.querySelector('#f-ssh-port').value.trim();
    const sshBad = host.ssh.on && raw !== '' && !(+raw > 0 && +raw < 65536 && /^\d+$/.test(raw));
    m.querySelector('#e-ssh').textContent = sshBad ? T('editor.err.sshPort') : '';
    return !msg && !sshBad;
  };
  const submit = (then) => {
    const host = collect();
    if (!validate(host)) { (m.querySelector('#f-host.bad') || m.querySelector('#f-ssh-port') || pass).focus(); return; }
    const pw = pass.value;
    const save = host.account ? null : m.querySelector('#f-save').checked;
    if (!save && pw) S.pendingPw[host.id || '__new'] = pw;
    send('save_host', { host, password: pw || null, save_password: save, then });
    closeModal('edit');
    if (!then) toast(existing ? T('toast.host.saved') : T('toast.host.added', { name: label(host) }), 'E73E');
  };
  m.querySelector('[data-x="cancel"]').onclick = () => closeModal('edit');
  const saveBtn = m.querySelector('[data-x="save"]');
  if (saveBtn) saveBtn.onclick = () => submit(null);
  m.querySelector('[data-x="connect"]').onclick = () => submit('connect');
  const del = m.querySelector('[data-x="delete"]');
  if (del) del.onclick = () => { closeModal('edit'); askDelete(h.id); };
  m.addEventListener('keydown', (e) => {
    // (Enter that finishes Korean/Japanese composition only commits the text.)
    if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229 && e.target.tagName === 'INPUT' && e.target.type !== 'checkbox') { e.preventDefault(); submit('connect'); }
  });
}

function quickConnect(addr) {
  addr = addr.trim();
  const have = S.hosts.find((h) => h.host.toLowerCase() === addr.toLowerCase() || label(h).toLowerCase() === addr.toLowerCase());
  if (have) return connect(have.id);
  openEdit({ host: addr }, { quick: true });
}

function askDelete(id) {
  const h = hostById(id);
  if (!h) return;
  confirmDialog(T('host.delete.title', { name: esc(label(h)) }), T('host.delete.text'), T('common.delete'), true, () => {
    send('delete_host', { id });
    toast(T('toast.host.deleted', { name: label(h) }), 'E74D');
  });
}

// ---------------------------------------------------------------- connect flow

function connect(id) {
  const s = S.sessions[id];
  if (s && s.state !== 'connecting') return send('focus', { id });
  showConnecting(id);
  if (s) return;
  const pw = S.pendingPw[id] || S.pendingPw.__new;
  delete S.pendingPw[id];
  delete S.pendingPw.__new;
  send('connect', pw ? { id, password: pw, save: false } : { id });
}

const STAGES = () => [['resolve', T('steps.resolve')], ['tcp', T('steps.tcp')], ['auth', T('steps.auth')], ['init', T('steps.init')]];
const SSH_STAGES = () => [['ssh', T('steps.ssh')], ['tcp', T('steps.sshTcp')], ['auth', T('steps.auth')], ['init', T('steps.init')]];

function stepsHtml(stage, ssh) {
  const list = ssh ? SSH_STAGES() : STAGES();
  // Over SSH the local resolve step is part of reaching Screen Sharing.
  if (ssh && stage === 'resolve') stage = 'tcp';
  const idx = Math.max(0, list.findIndex(([k]) => k === stage));
  return list.map(([k, t], i) => {
    const cls = i < idx ? 'done' : i === idx ? 'now' : '';
    const mark = i < idx ? I('E73E') : i === idx ? '<span class="spin"></span>' : '<span class="ring"></span>';
    return `<div class="st ${cls}" data-stage="${k}"><span class="mark">${mark}</span>${t}</div>`;
  }).join('');
}

function showConnecting(id) {
  const h = hostById(id);
  if (!h) return;
  const s = S.sessions[id];
  const img = h.thumb ? `<img class="big-thumb" src="thumb/${esc(h.id)}.png?v=${h.thumb}" alt="" onerror="this.remove()">` : `<div class="logo" style="width:56px;height:56px;border-radius:16px;margin-bottom:18px"></div>`;
  const m = openModal('connecting', `<div class="dialog small" data-for="${esc(id)}"><div class="connecting">${img}
      <h2>${T('connecting.title', { name: esc(label(h)) })}</h2><div class="addr">${esc(h.user ? `${h.user}@${h.host}` : h.host)}</div>
      <div class="progress-steps">${stepsHtml(s ? s.stage : (h.ssh && h.ssh.on ? 'ssh' : 'resolve'), h.ssh && h.ssh.on)}</div></div>
      <div class="foot"><button class="btn subtle left" data-x="hide">${T('connecting.hide')}</button><button class="btn" data-x="cancel">${T('common.cancel')}</button></div></div>`, { sticky: true });
  m.querySelector('[data-x="cancel"]').onclick = () => { send('cancel', { id }); closeModal('connecting'); };
  m.querySelector('[data-x="hide"]').onclick = () => closeModal('connecting');
}

function updateConnecting(id) {
  const m = $(`#modals .scrim[data-key="connecting"] .dialog[data-for="${CSS.escape(id)}"]`);
  if (!m) return;
  const s = S.sessions[id];
  if (!s || s.state !== 'connecting') { closeModal('connecting'); return; }
  const h = hostById(id);
  m.querySelector('.progress-steps').innerHTML = stepsHtml(s.stage, h && h.ssh && h.ssh.on);
}

function showPassword(id, retry = false) {
  const h = hostById(id);
  if (!h) return;
  const m = openModal('password', `<div class="dialog small"><div class="body">
      <div class="state-icon info">${I('E72E')}</div>
      <h2>${T('password.title', { name: esc(label(h)) })}</h2>
      <p class="lead">${retry ? T('password.retry') : { mac: T('password.lead.mac'), windows: T('password.lead.windows'), rdp: T('password.lead.rdp'), linux: T('password.lead.linux') }[kindOf(h)]}</p>
      <div class="field" ${KINDS[kindOf(h)].user ? '' : 'hidden'}><label>${KINDS[kindOf(h)].user || ''}</label><input class="input" id="p-user" value="${esc(h.user)}" placeholder="${KINDS[kindOf(h)].userPh || ''}"></div>
      <div class="field"><label>${KINDS[kindOf(h)].pass}</label><div class="input-wrap"><input class="input" id="p-pass" type="password" autofocus><button class="reveal i" tabindex="-1" title="${T('common.showPassword')}">&#xE7B3;</button></div></div>
      <label class="check"><input type="checkbox" id="p-save" checked>${T('common.savePassword')}</label>
    </div><div class="foot"><button class="btn" data-x="cancel">${T('common.cancel')}</button><button class="btn primary" data-x="ok">${I('E768')}${T('common.connect')}</button></div></div>`);
  const pass = m.querySelector('#p-pass');
  m.querySelector('.reveal').onclick = () => { pass.type = pass.type === 'password' ? 'text' : 'password'; pass.focus(); };
  const go = () => {
    const user = m.querySelector('#p-user').value.trim();
    const save = m.querySelector('#p-save').checked;
    if (user !== h.user) send('save_host', { host: { ...h, user }, password: null, save_password: null });
    closeModal('password');
    showConnecting(id);
    send('connect', { id, password: pass.value, save });
  };
  m.querySelector('[data-x="cancel"]').onclick = () => closeModal('password');
  m.querySelector('[data-x="ok"]').onclick = go;
  m.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target.tagName === 'INPUT') { e.preventDefault(); go(); } });
}

// What went wrong, per error kind, in the current language (the words speak about Macs).
const ERRORS = () => ({
  auth: [T('error.auth.title'), T('error.auth.lead'), [T('error.auth.tip1'), T('error.auth.tip2'), T('error.auth.tip3')]],
  ssh_auth: [T('error.sshAuth.title'), T('error.sshAuth.lead'), [T('error.sshAuth.tip1'), T('error.sshAuth.tip2')]],
  ssh_hostkey: [T('error.sshHostkey.title'), T('error.sshHostkey.lead'), [T('error.sshHostkey.tip1'), T('error.sshHostkey.tip2')]],
  ssh_refused: [T('error.sshRefused.title'), T('error.sshRefused.lead'), [T('error.sshRefused.tip1'), T('error.sshRefused.tip2')]],
  ssh_missing: [T('error.sshMissing.title'), T('error.sshMissing.lead'), [T('error.sshMissing.tip1')]],
  declined: [T('error.declined.title'), T('error.declined.lead'), [T('error.declined.tip1')]],
  need_user: [T('error.needUser.title'), T('error.needUser.lead'), [T('error.needUser.tip1')]],
  refused: [T('error.refused.title'), T('error.refused.lead'), [T('error.refused.tip1'), T('error.refused.tip2')]],
  timeout: [T('error.timeout.title'), T('error.timeout.lead'), [T('error.timeout.tip1'), T('error.timeout.tip2'), T('error.timeout.tip3')]],
  unreachable: [T('error.unreachable.title'), MAC ? T('error.unreachable.lead.mac') : T('error.unreachable.lead.pc'), [T('error.unreachable.tip1'), T('error.unreachable.tip2')]],
  dns: [T('error.dns.title'), T('error.dns.lead'), [T('error.dns.tip1'), T('error.dns.tip2')]],
  reset: [T('error.reset.title'), T('error.reset.lead'), [T('error.reset.tip1'), T('error.reset.tip2')]],
  not_vnc: [T('error.notVnc.title'), T('error.notVnc.lead'), [T('error.notVnc.tip1')]],
  lost: [T('error.lost.title'), T('error.lost.lead'), [T('error.lost.tip1')]],
  other: [T('error.other.title'), T('error.other.lead'), [T('error.other.tip1')]],
});

function showError(id, kind, msg) {
  const h = hostById(id);
  const table = ERRORS();
  let [title, lead, tips] = table[kind] || table.other;
  const k = h ? kindOf(h) : 'mac';
  if (k !== 'mac') {
    // The tables speak about Macs; reword for PCs and other VNC servers.
    const w = devWord(h);
    if (I18N.lang === 'ko') {
      // Korean particles follow the noun: 'Mac이' becomes 'PC이(가)'.
      title = title.replace(/Mac이/g, `${w}이(가)`).replace(/Mac에/g, `${w}에`).replace(/Mac /g, `${w} `);
      lead = lead.replace(/Mac이/g, `${w}이(가)`).replace(/Mac /g, `${w} `);
    } else {
      title = title.replace(/\bMac\b/g, w);
      lead = lead.replace(/\bMac\b/g, w);
    }
    const OTHER = {
      auth: { windows: [T('error.other.auth.windows.tip1'), T('error.other.auth.windows.tip2')],
        rdp: [T('error.other.auth.rdp.tip1'), T('error.other.auth.rdp.tip2'), T('error.other.auth.rdp.tip3'), T('error.other.auth.rdp.tip4')],
        linux: [T('error.other.auth.linux.tip1'), T('error.other.auth.linux.tip2')] },
      refused: { windows: [T('error.other.refused.windows.tip1'), T('error.other.refused.windows.tip2')],
        rdp: [T('error.other.refused.rdp.tip1')],
        linux: [T('error.other.refused.linux.tip1'), T('error.other.refused.linux.tip2')] },
    };
    tips = (OTHER[kind] && OTHER[kind][k]) || tips.map((t) => t.replace(I18N.lang === 'ko' ? /Mac/g : /\bMac\b/g, w));
  }
  if (MAC && ['timeout', 'unreachable', 'dns'].includes(kind)) tips = [...tips, T('error.localNetworkTip')];
  const name = h ? label(h) : 'Mac';
  const primary = kind === 'auth' || kind === 'ssh_auth'
    ? `<button class="btn primary" data-x="password" autofocus>${I('E72E')}${T('error.btn.password')}</button>`
    : kind === 'ssh_hostkey'
      ? `<button class="btn danger" data-x="trust">${I('E8D7')}${T('error.btn.trust')}</button>`
      : kind === 'ssh_missing'
        ? ''
        : kind === 'need_user'
      ? `<button class="btn primary" data-x="edit" autofocus>${I('E70F')}${T('editor.title.edit')}</button>`
      : `<button class="btn primary" data-x="retry" autofocus>${I('E72C')}${T('error.btn.retry')}</button>`;
  const secondary = kind !== 'need_user' ? `<button class="btn" data-x="edit">${I('E70F')}${T('editor.title.edit')}</button>` : '';
  const topic = { refused: 'mac', auth: 'mac', need_user: 'mac', ssh_auth: 'accounts', ssh_refused: 'mac', ssh_hostkey: 'accounts', timeout: 'outside', unreachable: 'outside', dns: 'trouble', declined: 'windows' }[kind] || 'trouble';
  const help = `<button class="btn subtle left" data-x="help">${I('E897')}${T('error.btn.help')}</button>`;
  const m = openModal('error', `<div class="dialog"><div class="body">
      <div class="state-icon err">${I(kind === 'auth' || kind.startsWith('ssh') ? 'E72E' : 'E783')}</div>
      <h2>${esc(title)}</h2><p class="lead"><b>${esc(name)}</b> — ${esc(lead)}</p>
      <ul class="tips">${tips.map((t) => `<li>${I('E946')}<span>${t}</span></li>`).join('')}</ul>
      ${msg ? `<details class="raw"><summary>${T('error.details')}</summary><code>${esc(msg)}</code></details>` : ''}
    </div><div class="foot">${help}<button class="btn" data-x="close">${T('common.close')}</button>${secondary}${primary}</div></div>`);
  const on = (x, f) => { const b = m.querySelector(`[data-x="${x}"]`); if (b) b.onclick = () => { closeModal('error'); f(); }; };
  on('close', () => {});
  on('retry', () => connect(id));
  on('password', () => showPassword(id, true));
  on('trust', () => { showConnecting(id); send('trust_new_key', { id }); });
  on('edit', () => openEdit({ id }));
  on('help', () => helpGo(topic));
}

// ---------------------------------------------------------------- context menu

let ctxReturn = null;
function showCtx(id, x, y, trigger = null) {
  const h = hostById(id);
  if (!h) return;
  const s = S.sessions[id];
  const items = [];
  if (!s) items.push(['connect', 'E768', T('common.connect')]);
  else if (s.state === 'connecting') items.push(['cancel', 'E711', T('ctx.cancel')]);
  else items.push(['focus', 'E8A7', T('card.focus')], ['disconnect', 'E711', T('ctx.disconnect')]);
  if (MAC && ['mac', 'macos'].includes(h.os) && h.kind !== 'rdp') items.push(['apple-screen-sharing', 'E7F4', T('ctx.appleScreenSharing')]);
  items.push(['files', 'E8B7', T('ctx.files')]);
  if(h.os!=='linux')items.push(['remote-setup','E896',T('ctx.remoteSetup')]);
  items.push('-', ['edit', 'E70F', T('common.edit')], ['dup', 'E8C8', T('ctx.duplicate')], ['fav', h.favorite ? 'E8D9' : 'E734', h.favorite ? T('card.unfavorite') : T('ctx.favorite')]);
  items.push(['thumb-pick', 'E91B', T('ctx.thumbPick')]);
  if (h.custom_thumb) items.push(['thumb-reset', 'E72C', T('ctx.thumbReset')]);
  if (h.has_pw) items.push(['forget', 'E8D7', T('ctx.forget')]);
  if (h.rdp_key) items.push(['forget-rdp-key', 'E72C', T('ctx.forgetRdpKey')]);
  items.push('-', ['del', 'E74D', T('common.delete'), 'danger']);
  const el = $('#ctx');
  ctxReturn = trigger || document.activeElement;
  el.innerHTML = items.map((it) => (it === '-' ? '<hr>' : `<button data-act="${it[0]}" data-id="${esc(id)}" class="${it[3] || ''}">${I(it[1])}${it[2]}</button>`)).join('');
  el.hidden = false;
  const r = el.getBoundingClientRect();
  el.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  el.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
  const first = el.querySelector('button');
  if (first) first.focus();
}

function hideCtx(restoreFocus = false) {
  $('#ctx').hidden = true;
  const trigger = ctxReturn;
  ctxReturn = null;
  if (restoreFocus && trigger?.isConnected) trigger.focus();
}

// ---------------------------------------------------------------- toasts

function toast(text, icon = 'E946', err = false) {
  const t = document.createElement('div');
  t.className = `toast${err ? ' err' : ''}`;
  t.innerHTML = `${I(icon)}<span>${esc(text)}</span>`;
  $('#toasts').appendChild(t);
  setTimeout(() => { t.classList.add('out'); setTimeout(() => t.remove(), 220); }, 2800);
}

// ---------------------------------------------------------------- actions

function scan() {
  S.scanning = true;
  send('scan');
  render();
  setTimeout(() => { if (S.scanning) { S.scanning = false; render(); } }, 6000);
}

function act(a, el) {
  const id = el.dataset.id;
  switch (a) {
    case 'apple-screen-sharing':
      if (MAC && ['mac', 'macos'].includes(hostById(id)?.os) && hostById(id)?.kind !== 'rdp') send('apple_screen_sharing', { id });
      return;
    case 'files': return window.ItdaFiles.launch(hostById(id));
    case 'forget-rdp-key': return send('forget_rdp_key', { id });
    case 'remote-setup': return window.ItdaRemoteSetup.open({id,hosts:S.hosts,accounts:S.accounts,send,modal:openModal,close:closeModal,toast,esc});
    case 'connect': return connect(id);
    case 'focus': return send('focus', { id });
    case 'cancel': return send('cancel', { id });
    case 'disconnect': return send('disconnect', { id });
    case 'edit': return openEdit({ id });
    case 'dup': send('duplicate', { id }); return toast(T('toast.duplicated'), 'E8C8');
    case 'fav': {
      const h = hostById(id);
      if (h) { h.favorite = !h.favorite; render(); send('favorite', { id, on: h.favorite }); }
      return;
    }
    case 'forget': send('forget_password', { id }); return toast(T('toast.passwordForgotten'), 'E8D7');
    case 'thumb-pick': return pickThumb(id);
    case 'thumb-reset': send('set_thumb', { id, png: null }); return toast(T('toast.thumbReset'), 'E72C');
    case 'del': return askDelete(id);
    case 'more': {
      const r = el.getBoundingClientRect();
      return showCtx(id, r.left, r.bottom + 4, el);
    }
    case 'new': return openEdit();
    case 'quick': return quickConnect(S.query);
    case 'page': return go(el.dataset.page);
    case 'login-item-settings': return send('login_item_settings');
    case 'scan': return scan();
    case 'add-found': {
      const os = el.dataset.os;
      const rdp = os === 'windows' && !el.dataset.itda;
      return openEdit({ host: el.dataset.host, name: el.dataset.name, kind: rdp ? 'rdp' : '', os: os === 'windows' ? 'windows' : os === 'linux' ? 'linux' : 'mac' });
    }
    case 'open': return send('open', { what: el.dataset.what });
    case 'pin-show': {
      const p = document.getElementById('web-pin');
      const shown = p.textContent !== '••••••';
      p.textContent = shown ? '••••••' : p.dataset.pin;
      return;
    }
    case 'acct-new': return openAccount();
    case 'acct-edit': return openAccount({ id: el.dataset.id });
    case 'acct-del': return askDeleteAccount(el.dataset.id);
    case 'group': return send('group_accounts', { user: el.dataset.user });
    case 'qr-lan': S.qrMode = 'lan'; return render();
    case 'qr-ts': S.qrMode = 'ts'; return render();
    case 'ts-serve-on': toast(T('toast.tsHttps'), 'E72E'); return send('web_tailscale', { on: true });
    case 'ts-serve-off': return send('web_tailscale', { on: false });
    case 'share-show': {
      const p = document.getElementById('share-pw');
      p.textContent = p.textContent === '••••••••' ? p.dataset.pin : '••••••••';
      return;
    }
    case 'share-reset':
      return confirmDialog(T('share.resetPw.title'), T('share.resetPw.text'), T('share.resetPw.ok'), false, () => { S.share = {}; send('share_reset_pw'); toast(T('share.resetPw.done'), 'E72C'); });
    case 'share-kick': return send('share_kick');
    case 'onboard': return showOnboarding();
    case 'update-check': toast(T('toast.update.checking'), 'E72C'); return send('update_check');
    case 'update-install': {
      if (!S.update) return send('update_check');
      const open = Object.keys(S.sessions || {}).length;
      return confirmDialog(T('update.confirm.title', { version: esc(S.update.version) }), open ? T('update.confirm.textOpen', { count: open }) : T('update.confirm.text'), T('settings.update.install'), false, () => { S.updateStage = T('common.preparing'); render(); send('update_install'); });
    }
    case 'permission-helper': return send('permission_helper');
    case 'sys-sharing': return send('url', { url: 'x-apple.systempreferences:com.apple.Sharing-Settings.extension' });
    case 'share-all-on':
      if (MAC) return act('web-on', el);
      S.settings.share = { port: 5900, ask: true, trust_near: false, cmd_as_ctrl: true, ...S.settings.share, on: true };
      S.share = {};
    // falls through
    case 'web-on':
      S.settings.web = { port: 8780, ...S.settings.web, on: true };
      S.web = {};
      saveSettings();
      return S.page === 'share' ? undefined : go('share');
    case 'new-os': {
      const os = el.dataset.os;
      return openEdit(os === 'rdp' ? { kind: 'rdp', os: 'windows' } : { os });
    }
    case 'pin-reset':
      return confirmDialog(T('share.resetPin.title'), T('share.resetPin.text'), T('share.resetPin.ok'), false, () => { S.web = {}; send('web_reset_pin'); toast(T('share.resetPin.done'), 'E72C'); });
  }
}

// A picture of the user's choosing for the card, so look-alike screens are told apart.
// Shrunk here to card size; the home keeps it until it is reset.
function pickThumb(id) {
  const f = document.createElement('input');
  f.type = 'file';
  f.accept = 'image/*';
  f.hidden = true;
  document.body.appendChild(f);
  f.onchange = () => {
    const file = f.files[0];
    f.remove();
    if (!file) return;
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      const k = Math.min(1, 800 / img.naturalWidth, 500 / img.naturalHeight);
      const c = document.createElement('canvas');
      c.width = Math.max(1, Math.round(img.naturalWidth * k));
      c.height = Math.max(1, Math.round(img.naturalHeight * k));
      c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
      send('set_thumb', { id, png: c.toDataURL('image/png') });
      toast(T('toast.thumbChanged'), 'E91B');
    };
    img.onerror = () => { URL.revokeObjectURL(url); toast(T('toast.thumbBad'), 'E783', true); };
    img.src = url;
  };
  f.click();
}

function saveSettings() {
  send('settings', { settings: S.settings });
  applyTheme();
  I18N.setPref(S.settings.lang || 'auto');
  render();
}

document.addEventListener('click', (e) => {
  const ext = e.target.closest('a[data-ext]');
  if (ext) { e.preventDefault(); return send('url', { url: ext.href }); }
  const inCtx = e.target.closest('#ctx');
  if (!inCtx) hideCtx();
  if (e.target.closest('[data-transfers]')) return showTransfers();
  const nav = e.target.closest('.nav-item');
  if (nav) return go(nav.dataset.page);
  const b = e.target.closest('[data-act]');
  if (b && (b.closest('#content') || inCtx)) {
    e.stopPropagation();
    hideCtx();
    return act(b.dataset.act, b);
  }
  const sg = e.target.closest('[data-seg]');
  if (sg) {
    const raw = sg.dataset.v;
    const v = raw === 'true' ? true : raw === 'false' ? false : /^\d+$/.test(raw) ? Number(raw) : raw;
    if (sg.dataset.path === 'defaults') S.settings.defaults = { ...S.settings.defaults, [sg.dataset.seg]: v };
    else S.settings[sg.dataset.seg] = v;
    return saveSettings();
  }
  const vt = e.target.closest('#viewToggle button');
  if (vt) { S.settings.view = vt.dataset.view; return saveSettings(); }
});

document.addEventListener('change', (e) => {
  const t = e.target;
  if (t.dataset.set) {
    if (t.dataset.path === 'defaults') S.settings.defaults = { ...S.settings.defaults, [t.dataset.set]: t.checked };
    else if (t.dataset.path === 'share') { S.settings.share = { port: 5900, ask: true, trust_near: false, cmd_as_ctrl: true, ...S.settings.share, [t.dataset.set]: t.checked }; if (t.dataset.set === 'on') S.share = {}; }
    else if (t.dataset.path === 'web') { S.settings.web = { port: 8780, ...S.settings.web, [t.dataset.set]: t.checked }; S.web = {}; }
    else S.settings[t.dataset.set] = t.checked;
    saveSettings();
    // (Said once macOS took it: see 'login_item'.)
    if (MAC && t.dataset.set === 'login_start') S.loginAsked = t.checked;
  } else if (t.id === 'share-port') {
    const p = parseInt(t.value, 10);
    if (p >= 1024 && p < 65536) { S.settings.share = { ...S.settings.share, port: p }; S.share = {}; saveSettings(); }
    else { t.value = (S.settings.share || {}).port || 5900; toast(T('toast.portRange'), 'E7BA', true); }
  } else if (t.id === 'web-port') {
    const p = parseInt(t.value, 10);
    if (p >= 1024 && p < 65536) { S.settings.web = { ...S.settings.web, port: p }; S.web = {}; saveSettings(); }
    else { t.value = (S.settings.web || {}).port || 8780; toast(T('toast.portRange'), 'E7BA', true); }
  } else if (t.dataset.select) {
    S.settings[t.dataset.select] = t.value;
    saveSettings();
  }
});

$('#content').addEventListener('dblclick', (e) => {
  if (e.target.closest('button')) return;
  const c = e.target.closest('.card, .list-row:not(.list-head)');
  if (c) connect(c.dataset.id);
});

$('#content').addEventListener('contextmenu', (e) => {
  const c = e.target.closest('.card, .list-row:not(.list-head)');
  e.preventDefault();
  if (c) showCtx(c.dataset.id, e.clientX, e.clientY, c);
});
document.addEventListener('contextmenu', (e) => { if (!e.target.closest('input, textarea')) e.preventDefault(); });

$('#newBtn').onclick = () => openEdit();
const search = $('#search');
search.addEventListener('input', () => {
  S.query = search.value;
  if (['settings', 'help', 'nearby'].includes(S.page)) S.page = 'all';
  render();
});
search.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    if (S.page === 'files') { e.preventDefault(); return window.ItdaFiles?.browserSearch(); }
    const q = search.value.trim();
    if (!q) return;
    const list = pageHosts();
    if (list.length === 1) connect(list[0].id);
    else if (!list.length && looksLikeAddress(q)) quickConnect(q);
    else if (list.length) { const f = $('#content .card, #content .list-row:not(.list-head)'); if (f) f.focus(); }
  } else if (e.key === 'Escape') {
    search.value = '';
    S.query = '';
    render();
  } else if (e.key === 'ArrowDown') {
    const f = $('#content .card, #content .list-row:not(.list-head)');
    if (f) { e.preventDefault(); f.focus(); }
  }
});

document.addEventListener('keydown', (e) => {
  const typing = e.target.closest && e.target.closest('input, textarea, select');
  if (e.key === 'Escape') {
    if (!$('#ctx').hidden) { e.preventDefault(); return hideCtx(true); }
    const m = topModal();
    if (m) return closeModal(m.dataset.key);
  }
  const cmd = MAC ? e.metaKey : e.ctrlKey;
  if (cmd && !e.altKey && ['=', '+', '-', '0'].includes(e.key)) {
    e.preventDefault();
    return setZoom(e.key === '0' ? 100 : zoomStep(e.key === '-' ? -1 : 1));
  }
  if (topModal()) return;
  if (cmd && !e.altKey && (e.key === 'n' || e.key === 'N')) { e.preventDefault(); return openEdit(); }
  if ((cmd && (e.key === 'f' || e.key === 'F')) || (e.key === '/' && !typing)) { e.preventDefault(); search.focus(); search.select(); return; }
  if (e.key === 'F5') { e.preventDefault(); return scan(); }
  if (typing) return;
  const c = e.target.closest && e.target.closest('.card, .list-row:not(.list-head)');
  // Enter/Space activate an inner button itself; other device shortcuts still work there.
  const buttonActivation = e.target.closest?.('button') && (e.key === 'Enter' || e.key === ' ');
  if (c && !buttonActivation) {
    if (e.key === 'Enter') { e.preventDefault(); connect(c.dataset.id); }
    else if (e.key === 'Delete' || (MAC && e.metaKey && e.key === 'Backspace')) askDelete(c.dataset.id);
    else if (e.key === 'F2') openEdit({ id: c.dataset.id });
    else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) { const r = c.getBoundingClientRect(); showCtx(c.dataset.id, r.left + 20, r.top + 20, c); }
    else if (['ArrowRight', 'ArrowLeft', 'ArrowDown', 'ArrowUp'].includes(e.key)) {
      const all = $$('#content .card, #content .list-row:not(.list-head)');
      const i = all.indexOf(c);
      const cols = c.classList.contains('card') ? Math.max(1, Math.round($('#content .grid').clientWidth / c.offsetWidth)) : 1;
      const d = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols }[e.key];
      const n = all[Math.min(all.length - 1, Math.max(0, i + d))];
      if (n) { e.preventDefault(); n.focus(); }
    }
  }
});

window.addEventListener('blur', () => hideCtx());
// A language change redraws what is on screen; open dialogs keep their words until reopened.
I18N.onChange(() => { hideCtx(); render(); });
render();
send('ready');
