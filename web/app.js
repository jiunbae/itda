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
const I = (code) => `<span class="i">&#x${code};</span>`;

// Operating system marks (Segoe Fluent has no logos, so these are SVG everywhere).
const OS_SVG = {
  mac: '<path d="M16.37 12.73c-.02-2.1 1.72-3.12 1.8-3.17-.98-1.43-2.5-1.63-3.05-1.65-1.3-.13-2.53.76-3.19.76-.66 0-1.67-.74-2.75-.72-1.41.02-2.72.82-3.44 2.09-1.47 2.55-.38 6.32 1.05 8.39.7 1.01 1.53 2.15 2.62 2.11 1.05-.04 1.45-.68 2.72-.68s1.63.68 2.74.66c1.13-.02 1.85-1.03 2.54-2.05.8-1.17 1.13-2.31 1.15-2.37-.03-.01-2.2-.85-2.22-3.37zM14.29 6.55c.58-.7.97-1.68.86-2.65-.83.03-1.84.55-2.44 1.25-.53.62-1 1.61-.88 2.56.93.07 1.88-.47 2.46-1.16z"/>',
  windows: '<path d="M3.5 5.6l7-1v6.9h-7zM11.5 4.5l9-1.3v8.3h-9zM3.5 12.5h7v6.9l-7-1zM11.5 12.5h9v8.3l-9-1.3z"/>',
  linux: '<path d="M12 2.6c-2.3 0-3.6 1.9-3.6 4.4 0 1.3.3 2.2-.6 3.6-1.2 1.8-2.9 3.9-2.9 6.6 0 .6.1 1.1.3 1.6-.8.4-1.5.9-1.4 1.5.2.9 2.1.6 3.4 1.1 1.1.4 2 .6 2.6-.2.5-.1 1.4-.2 2.2-.2s1.7.1 2.2.2c.6.8 1.5.6 2.6.2 1.3-.5 3.2-.2 3.4-1.1.1-.6-.6-1.1-1.4-1.5.2-.5.3-1 .3-1.6 0-2.7-1.7-4.8-2.9-6.6-.9-1.4-.6-2.3-.6-3.6 0-2.5-1.3-4.4-3.6-4.4z"/><path class="cut" d="M12 10.2c-1.9 0-3.2 2.6-3.2 5.6 0 1.7.6 3.2 1.3 4h3.8c.7-.8 1.3-2.3 1.3-4 0-3-1.3-5.6-3.2-5.6z"/><circle class="cut" cx="10.6" cy="6.6" r=".9"/><circle class="cut" cx="13.4" cy="6.6" r=".9"/><path class="cut" d="M10.9 8.4h2.2L12 9.6z"/>',
};

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
const HERE = MAC ? '이 Mac' : '이 PC';
const MOD = MAC ? '⌘' : 'Ctrl';
const VAULT = MAC ? '이 Mac의 키체인' : '이 PC의 Windows 자격 증명 관리자';
if (MAC) {
  document.documentElement.classList.add('mac');
  document.querySelector('.search kbd').textContent = '⌘F';
  document.getElementById('newBtn').title = '새 연결 (⌘N)';
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
  if (!sec) return '연결한 적 없음';
  const d = Date.now() / 1000 - sec;
  if (d < 60) return '방금 전';
  if (d < 3600) return `${Math.floor(d / 60)}분 전`;
  if (d < 86400) return `${Math.floor(d / 3600)}시간 전`;
  if (d < 172800) return '어제';
  if (d < 86400 * 30) return `${Math.floor(d / 86400)}일 전`;
  const t = new Date(sec * 1000);
  return `${t.getFullYear()}. ${t.getMonth() + 1}. ${t.getDate()}.`;
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
  zoomToast = setTimeout(() => toast(`화면 크기 ${z}% — ${MOD}+0으로 원래대로`, 'E71E'), 120);
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
      Object.assign(S, { accounts: m.accounts || [], hosts: m.hosts, settings: m.settings, sessions: m.sessions || {}, fileWindows: m.file_windows || [], online: m.online || {}, found: m.found || [], version: m.version, sysDark: !!m.dark, sshOk: m.ssh_ok, tailnet: m.tailnet, update: m.update || null });
      applyTheme();
      render();
      window.ItdaFiles?.boot();
      if (window.ITDA_FILE_HOST) break;
      if (m.update_failed) toast(`지난 업데이트를 마치지 못했습니다 — ${m.update_failed}`, 'E7BA', true);
      if (m.quick) quickConnect(m.quick);
      else if (!S.settings.onboarded && !S.hosts.length && !topModal()) showOnboarding();
      break;
    case 'apple_screen_sharing_result':
      toast(m.ok ? 'Apple 화면 공유 앱에 연결을 요청했습니다' : m.error || 'Apple 화면 공유 앱을 열 수 없습니다', 'E7F4', !m.ok);
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
        toast(`${h ? label(h) : 'Mac'}에 연결했습니다`, 'E73E');
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
      if (m.grouped) toast(`연결 ${m.grouped}개를 계정 하나로 묶었습니다`, 'E73E');
      const em = S.pickAccountFor;
      S.pickAccountFor = null;
      if (em && em.isConnected) {
        const sel = em.querySelector('#f-account');
        const acc = acctById(m.id);
        const text = acc ? `계정: ${acc.name || acc.user} (${acc.user})` : '계정';
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
    case 'update':
      // A newer release: offered in the sidebar and Settings; installing always asks first.
      if (m.stage) { S.updateStage = m.stage; toast(`업데이트: ${m.stage}`, 'E896'); }
      else if (m.error) { S.updateStage = ''; toast(`업데이트하지 못했습니다 — ${m.error}`, 'E7BA', true); }
      else {
        S.update = m.available || null;
        if (m.found && S.update) toast(`Itda ${S.update.version}을(를) 설치할 수 있습니다 — 설정에서 업데이트하세요`, 'E896');
        if (m.checked) toast(S.update ? `Itda ${S.update.version}을(를) 설치할 수 있습니다` : '최신 버전을 사용하고 있습니다', S.update ? 'E896' : 'E73E');
      }
      render();
      break;
    case 'hp': {
      // ⚡ high-performance mode being set up from a connection's editor (or window).
      const h = S.hosts.find((x) => x.id === m.id);
      const name = h ? label(h) : '';
      const guided=window.ItdaHpSetup?.message(m);
      if(m.helper){
        if(!guided && m.error) toast(`${name} 권한 도우미를 열지 못했습니다 — ${m.error}`, 'E7BA');
        return;
      }
      // Only the open editor of this very connection reflects it.
      const ed = document.querySelector(`#modals [data-key="edit"][data-host="${CSS.escape(m.id || '')}"]`);
      const btn = ed && ed.querySelector('#hp-enable');
      if (m.stage && !guided) toast(`${name} 고성능 모드: ${m.stage}`, 'E72C');
      else if (m.error && !m.status) {
        if(!guided) toast(`${name} 고성능 모드를 켜지 못했습니다 — ${m.error}`, 'E7BA');
        if (btn) { btn.disabled = false; btn.textContent = '켜기'; }
      } else if (m.ready) {
        if(!guided) toast(m.screen && m.access ? `${name} 고성능 모드를 켰습니다 — 다음 연결부터 사용합니다` : `${name} 고성능 모드를 켰습니다. 그 Mac에서 Itda의 화면 기록·손쉬운 사용 권한을 허용하세요.`, 'E73E');
        const st = ed && ed.querySelector('#hp-state');
        if (st) st.textContent = '켜짐 — 이 Mac의 Itda 고성능 공유로 연결합니다 (SSH 터널).';
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
  return `<button class="nav-item${on}" data-page="${esc(page)}">${I(icon)}<span class="label">${esc(text)}</span>${c}</button>`;
}

function renderNav() {
  const fav = S.hosts.filter((h) => h.favorite).length;
  const live = Object.keys(S.sessions).length;
  let html = navItem('all', 'E80F', '모든 연결', S.hosts.length);
  html += navItem('favorites', 'E734', '즐겨찾기', fav || null);
  html += navItem('recent', 'E81C', '최근 사용', null);
  html += navItem('files', 'E8B7', '파일 브라우저', null);
  html += navItem('accounts', 'E77B', '계정', S.accounts.length || null);
  if (live) html += navItem('live', 'E703', '연결 중', live);
  // Only devices that could still be added: the rest are in the book already.
  const addable = S.found.filter((f) => !inBook(f)).length + tailPeers().filter((p) => p.online && !peerInBook(p)).length;
  html += navItem('nearby', 'E701', '주변 기기', addable || null);
  const sharing = (S.settings.web || {}).on || (S.settings.share || {}).on;
  html += navItem('share', 'E72D', '공유', null).replace('</button>', sharing ? '<span class="dot-on" title="켜짐"></span></button>' : '</button>');
  const gs = groups();
  if (gs.length) {
    html += `<div class="nav-section">그룹</div>`;
    for (const g of gs) html += navItem(`group:${g}`, 'E8B7', g, S.hosts.filter((h) => hostGroups(h).includes(g)).length);
  }
  morph($('#nav'), html);
  $$('.nav-bottom .nav-item').forEach((b) => b.classList.toggle('active', b.dataset.page === S.page));
}

const TITLES = { all: '모든 연결', favorites: '즐겨찾기', recent: '최근 사용', files: '파일 브라우저', live: '연결 중', nearby: '주변 기기', accounts: '계정', settings: '설정', help: '도움말', share: '공유 — 다른 기기에서 보기' };

function renderTop() {
  const page = S.page;
  $('#title').textContent = page.startsWith('group:') ? page.slice(6) : TITLES[page] || '';
  const listy = !['settings', 'help', 'nearby', 'accounts', 'share'].includes(page);
  $('.search').style.visibility = listy ? '' : 'hidden';
  $('#viewToggle').style.visibility = listy && page !== 'files' ? '' : 'hidden';
  $('#search').placeholder = page === 'files' ? '폴더 또는 위치 검색' : '검색하거나 주소를 입력해 바로 연결';
  $('#newBtn .label-text').textContent = page === 'files' ? '새 기기' : '새 연결';
  $('#newBtn').title = page === 'files' ? '새 기기' : `새 연결 (${MOD}N)`;
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
  return { start: '시작하는 중…', ssh: '암호화 연결 중…', resolve: '주소 찾는 중…', tcp: '연결하는 중…', auth: '로그인 중…', init: '화면 준비 중…' }[st] || '연결하는 중…';
}

function badge(h) {
  const s = S.sessions[h.id];
  if (!s) return '';
  if (s.state === 'connected') return `<div class="badge ok">${I('E73E')}연결됨</div>`;
  if (s.state === 'reconnecting') return `<div class="badge warn"><span class="spin"></span>재연결 중</div>`;
  return `<div class="badge"><span class="spin"></span>${esc(stageText(s.stage))}</div>`;
}

function dot(h) {
  if (S.sessions[h.id] && S.sessions[h.id].state === 'connected') return `<span class="dot on" title="연결됨"></span>`;
  const o = S.online[h.id];
  if (o === true) return `<span class="dot on" title="화면 공유 응답함"></span>`;
  if (o === false) return `<span class="dot off" title="응답 없음 (꺼져 있거나 다른 네트워크)"></span>`;
  return `<span class="dot unknown" title="확인 중"></span>`;
}

function thumb(h) {
  const img = h.thumb ? `<img data-thumb src="thumb/${esc(h.id)}.png?v=${h.thumb}" alt="" loading="lazy" onerror="this.remove()">` : '';
  const initial = esc((label(h)[0] || '?').toUpperCase());
  return `<div class="thumb" style="--h:${hue(h.id)}">
    <span class="ph">${osIcon(h)}</span><span class="ph-initial">${initial}</span>${img}`;
}

function primaryAction(h) {
  const s = S.sessions[h.id];
  if (!s) return `<button class="btn primary" data-act="connect" data-id="${esc(h.id)}">${I('E768')}화면 연결</button>`;
  if (s.state === 'connecting') return `<button class="btn" data-act="cancel" data-id="${esc(h.id)}">${I('E711')}취소</button>`;
  return `<button class="btn primary" data-act="focus" data-id="${esc(h.id)}">${I('E8A7')}창으로 이동</button>`;
}

function card(h) {
  const live = S.sessions[h.id] ? ' live' : '';
  const lu = h.login_user || h.user;
  const who = (h.ssh && h.ssh.on ? `<span class="i lock" title="SSH로 암호화">&#xE72E;</span>` : '') + (lu ? `${esc(lu)}@${esc(h.host)}` : esc(h.host));
  const extra = [h.kind === 'rdp' ? '원격 데스크톱' : '', ago(h.last_used), h.remote_size, hostGroups(h).join(', ')].filter(Boolean).map(esc).join(' · ');
  return `<div class="card${live}" tabindex="0" data-id="${esc(h.id)}">
    ${thumb(h)}
      ${badge(h)}
      <button class="star i${h.favorite ? ' on' : ''}" data-act="fav" data-id="${esc(h.id)}" title="${h.favorite ? '즐겨찾기 해제' : '즐겨찾기'}">${h.favorite ? '&#xE735;' : '&#xE734;'}</button>
      <div class="connect-overlay">${primaryAction(h)}</div>
    </div>
    <div class="meta">
      <div class="row1">${dot(h)}<span class="name" title="${esc(label(h))}">${esc(label(h))}</span>
        <button class="more i" data-act="more" data-id="${esc(h.id)}" title="더보기">&#xE712;</button></div>
      <div class="sub">${who}</div>
      <div class="sub2">${extra}</div>
    </div>
  </div>`;
}

function row(h) {
  const s = S.sessions[h.id];
  const state = s
    ? s.state === 'connected'
      ? `<span class="pill ok">${I('E73E')}연결됨</span>`
      : s.state === 'reconnecting'
        ? `<span class="pill warn">재연결 중</span>`
        : `<span class="pill mute"><span class="spin" style="width:10px;height:10px;border-width:2px"></span>${esc(stageText(s.stage))}</span>`
    : S.online[h.id] === true
      ? `<span class="pill ok">응답함</span>`
      : S.online[h.id] === false
        ? `<span class="pill mute">응답 없음</span>`
        : '';
  return `<div class="list-row" tabindex="0" data-id="${esc(h.id)}">
    ${thumb(h)}</div>
    <div class="nm"><b>${dot(h)}${esc(label(h))}${h.favorite ? ` <span class="i" style="color:#ffc83d;font-size:12px">&#xE735;</span>` : ''}</b><small>${h.ssh && h.ssh.on ? `<span class="i lock" title="SSH로 암호화">&#xE72E;</span>` : ''}${esc(h.host)}</small></div>
    <span class="col c-user">${h.user ? esc(h.user) : '<span style="color:var(--text-3)">VNC 암호</span>'}</span>
    <span class="col c-last">${esc(ago(h.last_used))}</span>
    <span class="col">${state}</span>
    <div class="actions">${primaryAction(h).replace('class="btn', 'class="btn sm')}
      <button class="btn subtle icon" data-act="more" data-id="${esc(h.id)}" title="더보기">${I('E712')}</button></div>
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
  const list = names.slice(0, 3).map(esc).join(', ') + (names.length > 3 ? ` 외 ${names.length - 3}대` : '');
  return `<div class="banner">${I('E701')}<div class="txt"><b>추가할 수 있는 기기를 찾았습니다</b><span>${list}${macs.length ? ' — Tailscale 기기는 어디서나 연결됩니다' : ''}</span></div>
    <button class="btn" data-act="page" data-page="nearby">보기</button></div>`;
}
function renderHosts() {
  const list = pageHosts();
  const view = S.settings.view || 'grid';
  if (!S.hosts.length) return emptyBook();
  let html = foundBanner();
  if (!list.length) {
    if (S.query) {
      const addr = looksLikeAddress(S.query.trim());
      html += `<div class="empty"><div class="art">${I('E721')}</div><h2>'${esc(S.query)}'와 일치하는 연결이 없습니다</h2>
        ${addr ? `<p>Enter를 누르면 <b>${esc(S.query.trim())}</b>에 새로 연결합니다.</p><div class="actions"><button class="btn primary" data-act="quick">${I('E768')}${esc(S.query.trim())}에 연결</button></div>` : '<p>이름, 주소, 사용자 이름으로 검색할 수 있습니다.</p>'}</div>`;
    } else {
      const msg = { favorites: ['즐겨찾기한 연결이 없습니다', '카드의 별표를 눌러 자주 쓰는 Mac을 모아 두세요.'], recent: ['최근에 연결한 Mac이 없습니다', '연결하면 여기에 순서대로 표시됩니다.'], live: ['연결 중인 Mac이 없습니다', ''] }[S.page] || ['연결이 없습니다', ''];
      html += `<div class="empty"><div class="art">${I('E7F4')}</div><h2>${msg[0]}</h2><p>${msg[1]}</p></div>`;
    }
    return html;
  }
  if (view === 'list') {
    html += `<div class="list"><div class="list-row list-head"><span></span><span>이름</span><span class="c-user">사용자</span><span class="c-last">마지막 연결</span><span>상태</span><span></span></div>${list.map(row).join('')}</div>`;
  } else {
    html += `<div class="grid">${list.map(card).join('')}${S.page === 'all' && !S.query ? `<button class="add-card" data-act="new">${I('E710')}<span>새 연결</span></button>` : ''}</div>`;
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

function showOnboarding(step = 'welcome') {
  const choice = (key, icon, title, sub) => `<button class="ob-choice" data-ob="${key}">${I(icon)}<b>${title}</b><span>${sub}</span></button>`;
  const check = (n, html) => `<li><span class="ob-n">${n}</span><span>${html}</span></li>`;
  const back = `<button class="btn subtle left" data-ob="welcome">${I('E72B')}처음으로</button>`;
  const steps = {
    welcome: () => `<div class="ob-hero"><div class="logo ob-logo"></div><h2>Itda에 오신 것을 환영합니다</h2>
        <p>${MAC ? '이 Mac에서 다른 Mac과 Windows PC, Linux를 열고, 휴대폰 브라우저로 내 기기들을 볼 수 있습니다.' : '이 PC에서 Mac과 다른 PC를 열고, 휴대폰이나 Mac의 브라우저로 이 PC와 내 기기들을 볼 수 있습니다.'}<br>무엇부터 할까요?</p></div>
      <div class="ob-choices">
        ${choice('mac', 'E7F4', 'Mac 화면 열기', 'Mac의 화면 공유로 연결합니다')}
        ${choice('win', 'E7F8', '다른 Windows PC 열기', 'Itda 공유 또는 원격 데스크톱')}
        ${choice('phone', 'E8EA', MAC ? '휴대폰에서 보기' : '휴대폰·Mac에서 보기', 'QR 하나로 브라우저에서 엽니다')}
        ${choice('linux', 'E756', 'Linux · 기타 VNC', 'VNC 서버가 있는 기기')}
      </div>
      <div class="foot"><span class="ob-dots"><i class="on"></i><i></i><i></i></span><button class="btn subtle" data-ob="skip">건너뛰기</button></div>`,
    mac: () => `<div class="ob-body"><h2>${I('E7F4')} Mac 화면 열기</h2>
        <div class="ob-split"><ol class="ob-list">
          ${check(1, `Mac에서 <b>시스템 설정 → 일반 → 공유</b>를 열고 <b>화면 공유</b>를 켭니다. <b>원격 관리</b>가 켜져 있다면 그대로 두어도 연결됩니다.`)}
          ${check(2, `화면 공유 옆 ⓘ → <b>접근 허용</b>에 내 계정이 있는지 확인합니다. 원격 관리라면 ⓘ → <b>옵션</b>에서 내 계정에 <b>관찰</b>과 <b>제어</b>를 켭니다.`)}
          ${check(3, `Itda에는 <b>Mac 계정 이름(짧은 이름)</b>과 로그인 암호를 넣습니다. Apple ID가 아닙니다.`)}
          ${check(4, `밖에서도 쓸 거라면 같은 화면의 <b>원격 로그인</b>도 켜 두세요 (SSH 암호화).`)}
        </ol><div class="ob-art">${typeof macWindow === 'function' ? macWindow('general', '공유', macSharing('screen')) : ''}</div></div></div>
      <div class="foot">${back}<span class="ob-dots"><i></i><i class="on"></i><i></i></span>
        <button class="btn" data-ob="mac-manual">주소로 직접 추가</button><button class="btn primary" data-ob="mac-nearby">${I('E701')}주변 Mac 찾기</button></div>`,
    win: () => `<div class="ob-body"><h2>${I('E7F8')} 다른 Windows PC 열기</h2>
        <div class="ob-two">
          <div class="ob-card"><b>A. Itda ‘이 PC 공유’ <span class="pill ok">모든 Windows</span></b>
            <ol class="ob-list">${check(1, '그 PC에도 Itda를 설치합니다.')}${check(2, '그 PC의 Itda → <b>공유 → 이 PC 공유</b>를 켭니다.')}${check(3, '표시된 <b>공유 암호</b>로 여기서 추가합니다.')}</ol>
            <button class="btn primary" data-ob="win-itda">${I('E710')}Windows · Itda로 추가</button></div>
          <div class="ob-card"><b>B. 원격 데스크톱 <span class="pill mute">Pro·Enterprise</span></b>
            <ol class="ob-list">${check(1, '그 PC의 <b>설정 → 시스템 → 원격 데스크톱</b>을 켭니다.')}${check(2, '그 PC의 Windows 계정과 암호로 추가합니다. <b>Microsoft 계정이면 이메일 주소</b>와 그 계정 암호를 넣습니다 (숫자 PIN은 안 됩니다).')}</ol>
            <button class="btn" data-ob="win-rdp">${I('E710')}원격 데스크톱으로 추가</button></div>
        </div></div>
      <div class="foot">${back}<span class="ob-dots"><i></i><i class="on"></i><i></i></span></div>`,
    phone: () => MAC ? `<div class="ob-body"><h2>${I('E8EA')} 휴대폰 브라우저에서 보기</h2>
        <ol class="ob-list">
          ${check(1, '아래 버튼을 누르면 <b>웹 뷰어</b>가 켜지고 QR 코드가 나옵니다.')}
          ${check(2, 'macOS가 <b>들어오는 네트워크 연결</b>을 허용할지 물으면 <b>허용</b>을 누르세요.')}
          ${check(3, '휴대폰 카메라로 QR을 찍고 <b>6자리 PIN</b>을 넣으면 이 Mac에 저장된 기기가 보입니다.')}
          ${check(4, 'iPhone은 공유 → <b>홈 화면에 추가</b>로 앱처럼 씁니다.')}
        </ol>${note('집 밖에서도 쓰려면 이 Mac과 휴대폰에 <b>Tailscale</b>을 설치해 같은 계정으로 로그인하세요.')}</div>
      <div class="foot">${back}<span class="ob-dots"><i></i><i class="on"></i><i></i></span><button class="btn primary" data-ob="phone-on">${I('E768')}켜고 QR 보기</button></div>`
    : `<div class="ob-body"><h2>${I('E8EA')} 휴대폰·Mac 브라우저에서 보기</h2>
        <ol class="ob-list">
          ${check(1, '아래 버튼을 누르면 <b>웹 뷰어</b>와 <b>이 PC 공유</b>가 켜지고 QR 코드가 나옵니다.')}
          ${check(2, 'Windows 방화벽이 물으면 <b>개인 네트워크</b>를 허용하세요.')}
          ${check(3, '휴대폰 카메라로 QR을 찍고 <b>6자리 PIN</b>을 넣으면 이 PC와 저장된 기기가 보입니다.')}
          ${check(4, 'iPhone은 공유 → <b>홈 화면에 추가</b>, Mac은 Safari <b>파일 → Dock에 추가</b>로 앱처럼 씁니다.')}
        </ol>${note('집 밖에서도 쓰려면 이 PC와 휴대폰에 <b>Tailscale</b>을 설치해 같은 계정으로 로그인하세요.')}</div>
      <div class="foot">${back}<span class="ob-dots"><i></i><i class="on"></i><i></i></span><button class="btn primary" data-ob="phone-on">${I('E768')}켜고 QR 보기</button></div>`,
  };
  const m = openModal('onboard', `<div class="dialog onboard">${steps[step]()}</div>`, { sticky: true });
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
    <h2>Mac에 연결할 준비가 되었습니다</h2>
    <p>Mac에서 화면 공유를 켜거나 다른 PC에서 Itda 공유를 켜고, 주소와 계정으로 연결하세요. 같은 네트워크의 기기는 자동으로 찾아 드립니다.</p>
    <div class="actions"><button class="btn primary" data-act="new">${I('E710')}새 연결</button><button class="btn" data-act="onboard">${I('E768')}시작 안내</button><button class="btn" data-help="start">${I('E897')}도움말</button></div>
  </div>`;
  if (found.length) {
    html += `<div class="page" style="margin:36px auto 0"><div class="section-head">${I('E701')}주변에서 찾은 기기<span class="line"></span></div><div class="found-list">${found.map(foundRow).join('')}</div></div>`;
  }
  return html;
}

// A device found on this Wi-Fi: what it is, how it would open, and what is missing.
// Windows PCs announce themselves while Itda runs, even with sharing off.
function foundRow(f) {
  const win = f.os === 'windows';
  const rdp = win && !f.share && f.rdp;
  const addr = [f.hostname, f.ip].filter(Boolean).filter((v, i, a) => a.indexOf(v) === i).join(' · ');
  const act = `<button class="btn sm primary" data-act="add-found" data-host="${esc(f.host)}" data-name="${esc(f.name)}" data-os="${esc(f.os || '')}" data-itda="${win && !rdp ? '1' : ''}">${I('E710')}추가</button>`;
  const pill = win
    ? ` <span class="pill ${f.share || rdp ? 'ok' : 'mute'}">${f.share ? 'Windows · Itda 공유 중' : rdp ? 'Windows · 원격 데스크톱' : 'Windows · Itda'}</span>`
    : f.itda ? ` <span class="pill ${f.share ? 'ok' : 'mute'}">Mac · Itda</span>` : '';
  const off = !f.itda || f.share || rdp ? ''
    : win ? '<span class="off">공유가 꺼져 있습니다 — 그 PC의 Itda에서 <strong>공유 → 이 PC 공유</strong>를 켜면 연결됩니다.</span>'
      : '<span class="off">화면 공유가 꺼져 있습니다 — 그 Mac의 <strong>시스템 설정 → 일반 → 공유 → 화면 공유</strong>(또는 원격 관리)를 켜면 연결됩니다.</span>';
  return `<div class="found"><div class="ico">${osIcon({ os: win ? 'windows' : 'mac' })}</div><div class="txt"><b>${esc(f.name)}${pill}</b><span>${esc(addr)}</span>${off}</div>${act}</div>`;
}

function peerRow(p) {
  const os = { macOS: 'Mac', windows: 'Windows', linux: 'Linux' }[p.os] || p.os;
  const act = `<button class="btn sm primary" data-act="add-found" data-host="${esc(p.dns || p.ip)}" data-name="${esc(p.name)}" data-os="${esc(p.os)}">${I('E710')}추가</button>`;
  return `<div class="found"><div class="ico">${osIcon(p)}</div><div class="txt"><b>${p.online ? '<span class="dot on"></span> ' : '<span class="dot off"></span> '}${esc(p.name)} <span class="pill mute">${esc(os)}</span></b><span>${esc(p.dns)} · ${esc(p.ip)}</span></div>${act}</div>`;
}

function renderNearby() {
  const head = `<div class="banner">${I('E946')}<div class="txt"><b>연결할 수 있는 기기</b><span>같은 Wi-Fi에서는 화면 공유가 켜진 Mac과 Itda가 실행 중인 PC·Mac을 찾고(30초마다 새로 고침), 어디에 있든 연결되는 기기는 Tailscale로 찾습니다.</span></div>
    <button class="btn" data-act="scan" ${S.scanning ? 'disabled' : ''}>${S.scanning ? '<span class="spin"></span>검색 중…' : `${I('E72C')}다시 검색`}</button></div>`;
  // Devices already in the address book are left out: they're on the home page.
  const fresh = S.found.filter((f) => !inBook(f));
  const lan = fresh.length
    ? `<div class="found-list">${fresh.map(foundRow).join('')}</div>`
    : S.found.length
      ? `<p class="note-line">같은 Wi-Fi에서 찾은 기기는 모두 주소록에 있습니다.</p>`
      : `<p class="note-line">${S.scanning ? '찾는 중…' : '같은 Wi-Fi에서 찾은 기기가 없습니다. 화면 공유가 켜진 Mac과 Itda가 켜진 PC·Mac이 여기에 나타납니다. 원격 데스크톱만 켠 PC는 주소로 직접 추가하세요.'}</p>`;
  const all = tailPeers();
  const peers = all.filter((p) => !peerInBook(p));
  const t = S.tailnet;
  const ts = !t
    ? `<p class="note-line">Tailscale을 설치하면 집 밖의 Mac에도 VPN 설정 없이 연결할 수 있습니다. <a href="https://tailscale.com/download" data-ext>Tailscale 받기</a></p>`
    : !t.running
      ? `<p class="note-line">Tailscale이 꺼져 있습니다. Tailscale을 켜면 어디서나 연결되는 기기가 여기에 나타납니다.</p>`
      : peers.length
        ? `<div class="found-list">${peers.map(peerRow).join('')}</div>`
        : all.length
          ? `<p class="note-line">Tailscale의 Mac·PC는 모두 주소록에 있습니다.</p>`
          : `<p class="note-line">같은 Tailscale 네트워크에 열 수 있는 Mac·PC가 없습니다.</p>`;
  return `<div class="page">${head}
    <div class="section-head">${I('E701')}같은 Wi-Fi<span class="line"></span></div>${lan}
    <div class="section-head">${I('E774')}Tailscale — 어디서나${t && t.running ? ` · ${esc(t.me.name)}` : ''}<span class="line"></span></div>${ts}
    <p class="note-line" style="margin-top:18px">주소를 알면 <a href="#" data-act="new">직접 추가</a>할 수도 있습니다.</p></div>`;
}
function toggle(key, on, path = 'settings') {
  return `<label class="switch"><input type="checkbox" data-set="${esc(key)}" data-path="${path}" ${on ? 'checked' : ''}><span></span></label>`;
}

function seg(key, value, options, path = 'settings') {
  return `<div class="seg">${options.map(([v, t]) => `<button data-seg="${esc(key)}" data-path="${path}" data-v="${esc(v)}" class="${String(value) === String(v) ? 'on' : ''}">${esc(t)}</button>`).join('')}</div>`;
}

function setting(icon, title, desc, control, last = false) {
  return `<div class="setting${last ? ' last' : ''}">${I(icon)}<div class="txt"><b>${title}</b>${desc ? `<span>${desc}</span>` : ''}</div>${control}</div>`;
}

function sharePanel(sh) {
  const info = S.share || {};
  if (info.error) {
    return `<div class="setting last">${I('E7BA')}<div class="txt"><b>공유를 시작하지 못했습니다</b><span>${esc(info.error)} — 다른 VNC 서버가 이 포트를 쓰고 있다면 포트를 바꾸세요.</span></div>
      <input class="input port" id="share-port" value="${esc(sh.port)}" inputmode="numeric" aria-label="포트"></div>`;
  }
  if (!info.running) return `<div class="setting last"><span class="spin"></span><div class="txt"><b>시작하는 중…</b></div></div>`;
  const addrs = (info.addrs || []).filter((a) => !(info.tailscale && a.kind === 'tailscale'))
    .map((a) => `<div class="url-row"><code class="url">${esc(a.addr)}</code><span class="pill mute">같은 Wi-Fi</span></div>`).join('');
  const ts = info.tailscale ? `<div class="url-row"><code class="url">${esc(info.tailscale)}</code><span class="pill mute">Tailscale · 어디서나</span></div>` : '';
  const viewers = (info.viewers || []).length
    ? `<div class="viewers">${I('E7B3')}<b>${info.viewers.length}명이 보는 중</b><span>${info.viewers.map((v) => esc(v.peer)).join(', ')}</span><button class="btn sm danger" data-act="share-kick">모두 끊기</button></div>`
    : '';
  return `<div class="setting">${I('E8EA')}<div class="txt">
        <b>다른 PC의 Itda나 VNC 앱에서 아래 주소와 암호로 연결하세요</b>
        <span>브라우저로는 열리지 않습니다 — 휴대폰·Mac 브라우저는 위의 웹 뷰어 주소를 쓰세요.</span>
        <div class="urls">${addrs}${ts}</div>
        <div class="pin-row"><span>공유 암호</span><code class="pin" id="share-pw" data-pin="${esc(info.password)}">••••••••</code>
          <button class="btn sm" data-act="share-show">${I('E7B3')}보기</button>
          <button class="btn sm subtle" data-act="share-reset">${I('E72C')}새 암호</button></div>
        <span>다른 Itda에서는 ‘주변 기기’에 ${esc(info.name)}(Windows · Itda)로 나타납니다. 계정 이름은 비우고 이 암호만 넣으면 됩니다.</span>
        ${viewers}
      </div></div>
    ${setting('E8D7', '연결할 때마다 묻기', '누가 연결하려 하면 이 PC에 허용 여부를 묻습니다 (권장)', toggle('ask', sh.ask, 'share'))}
    ${sh.ask ? setting('E7F8', '내 기기는 묻지 않기', '같은 Wi-Fi·유선 네트워크와 내 Tailscale 기기는 공유 암호만 맞으면 바로 연결합니다. 그 밖에서 오는 연결만 묻습니다.', toggle('trust_near', sh.trust_near !== false, 'share')) : ''}
    ${setting('E765', 'Mac의 ⌘을 Ctrl로', 'Mac에서 연결했을 때 ⌘C·⌘V로 복사·붙여넣기', toggle('cmd_as_ctrl', sh.cmd_as_ctrl, 'share'))}
    ${setting('E890', '보기만 허용', '마우스·키보드 입력은 받지 않습니다', toggle('view_only', sh.view_only, 'share'))}
    <div class="setting last">${I('E968')}<div class="txt"><b>포트</b><span>기본 5900 (VNC 표준)</span></div>
      <input class="input port" id="share-port" value="${esc(sh.port)}" inputmode="numeric" aria-label="포트"></div>`;
}

function webPanel(w) {
  const info = S.web || {};
  if (info.error) {
    return `<div class="setting last">${I('E7BA')}<div class="txt"><b>웹 뷰어를 시작하지 못했습니다</b><span>${esc(info.error)} — 다른 포트를 써 보세요.</span></div>
      <input class="input port" id="web-port" value="${esc(w.port)}" inputmode="numeric" aria-label="포트"></div>`;
  }
  if (!info.running) return `<div class="setting last"><span class="spin"></span><div class="txt"><b>시작하는 중…</b></div></div>`;
  const urls = (info.urls || []).filter((u) => !(info.tailscale && u.kind === 'tailscale')).map((u) => `<div class="url-row"><code class="url">${esc(u.url)}</code>${u.kind === 'tailscale' ? '<span class="pill mute">Tailscale · 밖에서</span>' : '<span class="pill mute">같은 Wi-Fi</span>'}</div>`).join('') || `<span>${HERE}의 네트워크 주소를 찾지 못했습니다</span>`;
  const ts = info.tailscale;
  const useTs = ts && S.qrMode === 'ts';
  const qrSwitch = ts ? `<div class="seg qr-seg"><button data-act="qr-lan" class="${useTs ? '' : 'on'}">같은 Wi-Fi</button><button data-act="qr-ts" class="${useTs ? 'on' : ''}">Tailscale</button></div>` : '';
  const tsBlock = !ts ? '' : `<div class="ts-box">
      <div class="url-row"><code class="url">${esc(ts.url)}</code><span class="pill mute">Tailscale · 어디서나</span></div>
      <span>휴대폰에서 Tailscale 앱을 켜 두면 집 밖에서도 이 주소로 열립니다.</span>
      <div class="pin-row">${ts.serving
        ? `${I('E72E')}<span>HTTPS로 제공 중</span><button class="btn sm subtle" data-act="ts-serve-off">끄기</button>`
        : ts.https
          ? `<button class="btn sm" data-act="ts-serve-on">${I('E72E')}HTTPS 주소 만들기</button><span>https://… 주소로 바꿔 브라우저 경고 없이 씁니다</span>`
          : `<span>HTTPS 주소를 쓰려면 <a href="https://login.tailscale.com/admin/dns" data-ext>Tailscale 관리 콘솔 → DNS</a>에서 ‘HTTPS Certificates’를 켜세요.</span>`}</div>
      ${ts.error ? `<span class="err">${esc(ts.error)}</span>` : ''}
    </div>`;
  return `<div class="setting web-panel">
      <div class="qr-col"><div class="qr" aria-label="QR 코드">${(useTs ? ts.qr : info.qr) || ''}</div>${qrSwitch}</div>
      <div class="txt">
        <b>휴대폰 카메라로 QR을 찍거나 아래 주소를 여세요</b>
        <div class="urls">${urls}</div>${tsBlock}
        <div class="pin-row"><span>접속 PIN</span><code class="pin" id="web-pin" data-pin="${esc(info.pin)}">••••••</code>
          <button class="btn sm" data-act="pin-show">${I('E7B3')}보기</button>
          <button class="btn sm subtle" data-act="pin-reset">${I('E72C')}새 PIN</button></div>
        <span>같은 Wi-Fi에서 쓰세요. 밖에서는 Tailscale 같은 VPN으로 ${HERE}에 접속하고, 인터넷에 이 포트를 직접 열지 마세요. ${MAC ? '처음 켤 때 macOS가 들어오는 연결을 허용할지 물으면 ‘허용’을 누르세요.' : '처음 켤 때 Windows 방화벽이 허용을 물으면 ‘개인 네트워크’를 허용하세요.'}</span>
      </div></div>
    <div class="setting last">${I('E968')}<div class="txt"><b>포트</b><span>다른 앱과 겹치면 바꾸세요</span></div>
      <input class="input port" id="web-port" value="${esc(w.port)}" inputmode="numeric" aria-label="포트"></div>`;
}

// One place for everything other devices use: the web viewer (browsers) and this PC's
// VNC server (other Itdas, VNC apps). Turning on "this PC from a browser" needs both.
function renderShare() {
  if (MAC) return renderShareMac();
  const st = S.settings;
  const web = st.web || {}, sh = st.share || {};
  const both = !web.on && !sh.on
    ? `<div class="share-hero">${I('E8EA')}<div class="txt"><b>휴대폰이나 Mac 브라우저로 이 PC와 내 기기들을 보세요</b>
        <span>한 번 켜면 QR 코드가 나옵니다. 휴대폰 카메라로 찍고 PIN을 넣으면 끝입니다.</span></div>
        <button class="btn primary" data-act="share-all-on">${I('E768')}모두 켜기</button></div>`
    : '';
  const nudge = sh.on && !web.on
    ? `<div class="banner warn">${I('E7BA')}<div class="txt"><b>브라우저로 이 PC를 보려면 웹 뷰어도 켜세요</b><span>이 PC 공유 주소(포트 ${esc(sh.port)})는 VNC 앱용이라 브라우저로는 열리지 않습니다.</span></div>
        <button class="btn primary" data-act="web-on">웹 뷰어 켜기</button></div>`
    : '';
  return `<div class="page share-page">
    ${both}${nudge}
    <div class="group-title" id="web-viewer">${I('E774')} 브라우저로 보기 — 웹 뷰어 <span class="pill ok">추천</span><a href="#" class="help-link" data-help="phone">자세히</a></div>
    ${setting('E8EA', '웹 뷰어', 'iPhone·Android·Mac의 브라우저로 저장된 Mac·Windows PC와 이 PC를 엽니다. 설치할 앱이 없고, 암호는 이 PC 밖으로 나가지 않습니다.', toggle('on', web.on, 'web'), !web.on)}
    ${web.on ? webPanel(web) : ''}
    <div class="group-title" id="share-pc">${I('E7F8')} 이 PC 화면 공유 — VNC<a href="#" class="help-link" data-help="settings#share">설명 보기</a></div>
    ${setting('E7F8', '이 PC 공유', '다른 PC의 Itda나 VNC 앱(Mac의 화면 공유 등)에서 이 PC를 보고 조작합니다. 켜 두면 웹 뷰어 목록에도 이 PC가 나옵니다.', toggle('on', sh.on, 'share'), !sh.on)}
    ${sh.on ? sharePanel(sh) : ''}
  </div>`;
}

// macOS: the web viewer as on Windows; this Mac itself is shared with the system's own
// Screen Sharing, so the page shows how to turn it on and what others should type.
function renderShareMac() {
  const web = S.settings.web || {};
  const info = S.share || {};
  const hero = !web.on
    ? `<div class="share-hero">${I('E8EA')}<div class="txt"><b>휴대폰이나 다른 기기의 브라우저로 내 기기들을 보세요</b>
        <span>한 번 켜면 QR 코드가 나옵니다. 휴대폰 카메라로 찍고 PIN을 넣으면 이 Mac에 저장된 기기가 보입니다.</span></div>
        <button class="btn primary" data-act="web-on">${I('E768')}웹 뷰어 켜기</button></div>`
    : '';
  const addr = info.local_name || 'mac.local';
  const ts = info.tailscale ? `<div class="url-row"><code class="url">${esc(info.tailscale)}</code><span class="pill mute">Tailscale · 어디서나</span></div>` : '';
  return `<div class="page share-page">
    ${hero}
    <div class="group-title" id="web-viewer">${I('E774')} 브라우저로 보기 — 웹 뷰어 <span class="pill ok">추천</span><a href="#" class="help-link" data-help="phone">자세히</a></div>
    ${setting('E8EA', '웹 뷰어', 'iPhone·Android·PC의 브라우저로 이 Mac에 저장된 Mac·Windows PC·Linux를 엽니다. 브라우저에 앱을 설치할 필요가 없고, 저장된 기기 암호를 브라우저에 전달하지 않습니다.', toggle('on', web.on, 'web'), !web.on)}
    ${web.on ? webPanel(web) : ''}
    <div class="group-title">${I('E945')} 고성능 모드 권한</div>
    <div class="setting last">${I('E73E')}<div class="txt"><b>이 Mac의 화면 보기·키보드·마우스 조작 권한</b><span>권한 도우미에서 설정을 열고, 허용된 항목을 한곳에서 확인할 수 있어요.</span></div><button class="btn" data-act="permission-helper">권한 도우미 열기</button></div>
    <div class="group-title" id="share-pc">${I('E7F4')} 이 Mac 화면 공유 — macOS 화면 공유<a href="#" class="help-link" data-help="share">자세히</a></div>
    <div class="setting">${I('E7F4')}<div class="txt"><b>이 Mac은 macOS에 들어 있는 ‘화면 공유’로 공유합니다</b>
        <span><strong>시스템 설정 → 일반 → 공유 → 화면 공유</strong>를 켜면 다른 기기의 Itda, Mac의 화면 공유 앱, VNC 앱에서 이 Mac을 보고 조작할 수 있습니다.</span></div>
      <button class="btn" data-act="sys-sharing">${I('E8A7')}공유 설정 열기</button></div>
    <div class="setting last">${I('E946')}<div class="txt"><b>다른 기기에서는 이렇게 추가하세요</b>
        <span>종류는 <strong>Mac</strong>, 주소는 아래 중 하나, 계정은 이 Mac의 사용자 이름${info.user ? `(<strong>${esc(info.user)}</strong>)` : '(짧은 이름)'}과 로그인 암호입니다.</span>
        <div class="urls"><div class="url-row"><code class="url">${esc(addr)}</code><span class="pill mute">같은 Wi-Fi</span></div>${ts}</div>
        <span>같은 화면의 <strong>원격 로그인</strong>도 켜 두면 밖에서 SSH로 암호화해 연결할 수 있습니다.</span></div></div>
  </div>`;
}

function renderSettings() {
  const st = S.settings;
  const d = st.defaults || {};
  const p = hk();
  const keys = [
    ['전체 화면', 'Enter'], ['창에 맞춤 / 실제 크기', '0 / 1'], ['통계 표시', 'S'], ['보기 전용', 'V'], ['스크린샷 저장', 'P'],
    ['연결 정보', 'I'], ['툴바 고정', 'T'], ['다시 연결', 'R'], ['전체 메뉴', 'M'],
  ];
  const more = (k) => `<a href="#" class="help-link" data-help="settings#${k}">설명 보기</a>`;
  return `<div class="page">
    <div class="banner">${I('E897')}<div class="txt"><b>설정이 무엇인지 모르겠다면</b><span>모든 설정을 쉬운 말로 풀어 둔 안내가 있습니다. 처음 설치한 상태가 추천 설정입니다.</span></div>
      <button class="btn" data-help="settings">${I('E713')}설정 자세히</button></div>
    <div class="group-title">모양${more('look')}</div>
    ${setting('E790', '테마', `${MAC ? 'macOS' : 'Windows'} 설정을 따르거나 직접 고릅니다`, seg('theme', st.theme, [['system', '시스템'], ['light', '라이트'], ['dark', '다크']]))}
    ${setting('F0E2', '주소록 보기', '', seg('view', st.view, [['grid', '카드'], ['list', '목록']]))}
    ${setting('E71E', '화면 크기', MAC ? '⌘ + / ⌘ − 로도 바꿀 수 있습니다 (⌘0 = 100%)' : 'Ctrl + / Ctrl − / Ctrl+휠로도 바꿀 수 있습니다 (Ctrl+0 = 100%)', seg('zoom', st.zoom || 100, [[90, '90%'], [100, '100%'], [110, '110%'], [125, '125%'], [150, '150%']]), true)}

    ${MAC ? `<div class="group-title">접속 창</div>
    ${setting('E962', '다른 Mac 스크롤 속도', '트랙패드나 휠로 민 만큼 다른 Mac 화면이 움직이는 양. ‘빠르게’면 이 Mac에서와 거의 같습니다', seg('mac_scroll', st.mac_scroll || 'fast', [['normal', '보통'], ['fast', '빠르게'], ['faster', '아주 빠르게']]), true)}` : ''}
    ${MAC ? '' : `<div class="group-title">접속 창${more('session')}<a href="#" class="help-link" data-help="keys">단축키 전체 보기</a></div>
    ${setting('E7C4', '툴바', '화면 위쪽 가운데의 툴바 표시 방식', `<select class="input" data-select="toolbar"><option value="auto" ${st.toolbar === 'auto' ? 'selected' : ''}>자동 숨김</option><option value="pinned" ${st.toolbar === 'pinned' ? 'selected' : ''}>항상 표시</option><option value="hidden" ${st.toolbar === 'hidden' ? 'selected' : ''}>숨김 (단축키만)</option></select>`)}
    ${setting('E765', '단축키 조합', 'Mac으로 보내지 않고 Itda가 처리하는 키 조합', seg('hotkey', st.hotkey, [['ctrl_alt', 'Ctrl+Alt'], ['ctrl_shift', 'Ctrl+Shift']]))}
    ${setting('E962', 'Mac 스크롤 속도', '마우스 휠 한 칸에 Mac 화면이 움직이는 양. 보통은 Windows의 ‘한 번에 스크롤할 줄 수’만큼입니다', seg('mac_scroll', st.mac_scroll || 'fast', [['normal', '보통'], ['fast', '빠르게'], ['faster', '아주 빠르게']]))}
    ${setting('E945', '빠르게 굴리면 가속', 'Mac 마우스처럼, 휠을 빠르게 연달아 굴리면 더 멀리 스크롤합니다', toggle('scroll_accel', st.scroll_accel !== false))}
    <div class="setting last" style="align-items:flex-start">${I('E8AB')}<div class="txt"><b>단축키</b><table class="shortcuts">${keys.map(([a, k]) => `<tr><td>${a}</td><td><kbd>${p}+${k}</kbd></td></tr>`).join('')}</table></div></div>`}

    <div class="group-title">새 연결 기본값${more('defaults')}</div>
    ${setting('E9A6', '화면 배율', '', seg('actual', d.actual, [[false, '창에 맞춤'], [true, '실제 크기']], 'defaults'))}
    ${setting('E77F', '클립보드 공유', MAC ? '복사한 텍스트를 이 Mac과 원격 기기가 주고받습니다' : '복사한 텍스트를 Windows와 Mac이 주고받습니다', toggle('clipboard', d.clipboard, 'defaults'))}
    ${setting('E72C', '자동 재연결', '네트워크가 끊기면 자동으로 다시 연결합니다', toggle('reconnect', d.reconnect, 'defaults'), MAC)}
    ${MAC ? '' : `${setting('E765', 'Ctrl ↔ ⌘ Cmd 바꾸기', '켜면 Ctrl+C/V가 Mac의 ⌘C/⌘V로 동작합니다 (Windows 키는 Ctrl)', toggle('swap', d.swap, 'defaults'))}
    ${setting('E8C1', '한/영 키', 'Mac의 입력 소스 전환 단축키에 맞추세요', seg('hangul_caps', d.hangul_caps, [[false, 'Ctrl+Space'], [true, 'Caps Lock']], 'defaults'), true)}`}

    <div class="group-title">공유</div>
    ${setting('E701', `주변에 ${HERE} 알리기`, `같은 Wi-Fi의 다른 Itda ‘주변 기기’에 ${HERE}가 바로 나타납니다. 이름과 공유·원격 데스크톱이 켜져 있는지만 알리고, 주소나 암호는 알리지 않습니다.`, toggle('announce', st.announce !== false))}
    ${setting('E72D', MAC ? '다른 기기에서 보기' : '다른 기기에서 보기 · 이 PC 공유', MAC ? '웹 뷰어(QR·PIN)와 이 Mac의 화면 공유 안내는 왼쪽의 ‘공유’ 페이지에 있습니다' : '웹 뷰어(QR·PIN)와 이 PC 공유는 왼쪽의 ‘공유’ 페이지에서 켭니다', `<button class="btn" data-act="page" data-page="share">${I('E72D')}공유 열기</button>`, true)}

    <div class="group-title">앱${more('app')}</div>
    ${MAC ? '' : setting('E8A5', '창을 닫으면 알림 영역에서 계속 실행', '알림 영역 아이콘으로 빠르게 다시 연결할 수 있습니다', toggle('tray', st.tray))}
    ${setting('EA8F', '알림', '연결이 끊기거나 다시 연결될 때 알려 줍니다', toggle('notify', st.notify))}
    ${setting('E91B', '미리보기 저장', `마지막 화면을 ${HERE}에만 작게 저장해 주소록에 표시합니다`, toggle('thumbnails', st.thumbnails), true)}

    <div class="group-title">업데이트</div>
    ${setting('E896', '원격 기기에 설치·설정 공유', 'SSH로 Itda를 설치하고 선택한 연결 프로필과 계정 설정을 전달합니다', `<button class="btn" data-act="remote-setup">원격 설치 열기</button>`)}
    ${setting('E896', S.update ? `새 버전 ${esc(S.update.version)}` : `Itda ${esc(S.version)}`, S.updateStage ? esc(S.updateStage) : S.update ? `지금 버전은 ${esc(S.version)}입니다. 업데이트하면 Itda가 다시 시작되고 열린 연결이 끊깁니다.${S.update.page ? ' <a href="' + esc(S.update.page) + '" data-ext>변경 사항</a>' : ''}` : '새 버전이 있는지 확인합니다', S.update ? `<button class="btn primary" data-act="update-install" ${S.updateStage ? 'disabled' : ''}>${I('E896')}업데이트</button>` : `<button class="btn" data-act="update-check">${I('E72C')}업데이트 확인</button>`)}
    ${setting('E823', '새 버전 자동 확인', '실행할 때와 하루에 한 번 GitHub 릴리스를 확인합니다. 설치는 항상 직접 고릅니다', toggle('auto_update', st.auto_update !== false), true)}

    <div class="group-title">정보</div>
    <div class="setting last"><div class="logo" style="width:36px;height:36px"></div><div class="txt"><b>Itda ${esc(S.version)}</b><span>내 기기를 잇다 — 가볍고 빠른 원격 데스크톱</span></div>
      <button class="btn" data-act="onboard">${I('E768')}시작 안내</button>
      <button class="btn" data-act="open" data-what="log">${I('E9F9')}로그</button>
      <button class="btn" data-act="open" data-what="folder">${I('E8B7')}데이터 폴더</button>
      ${MAC ? '' : `<button class="btn" data-act="open" data-what="screenshots">${I('E722')}스크린샷</button>`}</div>
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
  if (nu) { nu.hidden = !S.update; nu.title = S.update ? `Itda ${S.update.version} 설치 가능` : ''; }
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
  render();
}

// ---------------------------------------------------------------- modals

function openModal(key, html, opts = {}) {
  closeModal(key);
  const scrim = document.createElement('div');
  scrim.className = 'scrim';
  scrim.dataset.key = key;
  scrim.innerHTML = html;
  scrim.addEventListener('mousedown', (e) => {
    if (e.target === scrim && !opts.sticky) closeModal(key);
  });
  $('#modals').appendChild(scrim);
  const first = scrim.querySelector('[autofocus]') || scrim.querySelector('input, .btn.primary');
  if (first) setTimeout(() => first.focus(), 30);
  return scrim;
}

function closeModal(key) {
  $$(`#modals .scrim${key ? `[data-key="${key}"]` : ''}`).forEach((m) => {m.dispatchEvent(new Event('itda-modal-close'));m.remove();});
}

function topModal() {
  const all = $$('#modals .scrim');
  return all[all.length - 1];
}

function confirmDialog(title, text, okText, danger, onOk) {
  const m = openModal('confirm', `<div class="dialog small"><div class="body"><h2>${title}</h2><p class="lead">${text}</p></div>
    <div class="foot"><button class="btn" data-x="cancel">취소</button><button class="btn ${danger ? 'danger' : 'primary'}" data-x="ok" autofocus>${okText}</button></div></div>`);
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
  return [a.user, a.has_pw ? '암호 저장됨' : '암호 없음 — 계정에서 입력하세요', a.ssh_key ? 'SSH 키' : ''].filter(Boolean).join(' · ');
}

/// The connection editor shows its own name/password only without an account.
function syncAccount(m) {
  const id = m.querySelector('#f-account').value;
  m.querySelector('#own-creds').hidden = !!id;
  m.querySelector('#acct-hint').textContent = id ? acctHint(id) : '여러 기기에 같은 계정을 쓴다면 계정을 만들어 두면 암호를 한 번만 입력합니다.';
}

/// Same sign-in name on several connections without an account: offer to group them.
function accountSuggestions() {
  const by = {};
  for (const h of S.hosts) if (!h.account && h.user) (by[h.user] = by[h.user] || []).push(h);
  return Object.entries(by).filter(([, hs]) => hs.length >= 2);
}

function renderAccounts() {
  const sugg = accountSuggestions().map(([user, hs]) => `<div class="banner">${I('E77B')}<div class="txt"><b>‘${esc(user)}’ 계정을 쓰는 연결이 ${hs.length}개 있습니다</b>
      <span>${hs.slice(0, 3).map((h) => esc(label(h))).join(', ')}${hs.length > 3 ? ` 외 ${hs.length - 3}개` : ''} — 계정 하나로 묶으면 암호를 한 곳에서 관리합니다.</span></div>
      <button class="btn primary" data-act="group" data-user="${esc(user)}">계정으로 묶기</button></div>`).join('');
  const list = S.accounts.length
    ? `<div class="found-list">${S.accounts.map((a) => `<div class="found acct">
        <div class="ico">${I('E77B')}</div>
        <div class="txt"><b>${esc(a.name || a.user)}</b><span>${esc(a.user)} · 연결 ${a.uses}개${a.has_pw ? ' · 암호 저장됨' : ' · <em class="warn-text">암호 없음</em>'}${a.ssh_key ? ` · SSH 키 ${esc(a.ssh_key.split(/[\\/]/).pop())}` : ''}</span></div>
        <button class="btn sm" data-act="acct-edit" data-id="${esc(a.id)}">${I('E70F')}편집</button>
        <button class="btn sm subtle icon" data-act="acct-del" data-id="${esc(a.id)}" title="삭제">${I('E74D')}</button></div>`).join('')}</div>`
    : `<div class="empty" style="margin-top:3vh"><div class="art">${I('E77B')}</div><h2>아직 계정이 없습니다</h2>
        <p>대부분 계정 하나로 여러 기기를 씁니다. 계정을 만들어 두면 연결을 추가할 때 고르기만 하면 되고, 암호를 바꿀 때도 한 곳만 고치면 됩니다.</p>
        <div class="actions"><button class="btn primary" data-act="acct-new">${I('E710')}새 계정</button></div></div>`;
  return `<div class="page">${sugg}
    ${S.accounts.length ? `<div class="section-head">${I('E77B')}계정 ${S.accounts.length}개<span class="line"></span><button class="btn sm" data-act="acct-new">${I('E710')}새 계정</button></div>` : ''}
    ${list}
    <p class="note-line" style="margin-top:16px">암호와 SSH 키 암호는 ${VAULT}에만 저장됩니다. 웹 뷰어로 접속한 기기에는 전달되지 않습니다.</p></div>`;
}

function openAccount(init = {}) {
  const ex = init.id ? acctById(init.id) : null;
  const a = ex ? { ...ex } : { id: '', name: '', user: init.user || '', ssh_key: '' };
  const m = openModal('account', `<div class="dialog"><div class="body">
      <h2>${ex ? '계정 편집' : '새 계정'}</h2>
      <p class="lead">${ex && ex.uses ? `이 계정을 쓰는 연결 ${ex.uses}개에 모두 적용됩니다.` : '이 계정을 여러 연결에서 함께 쓸 수 있습니다.'}</p>
      <div class="row">
        <div class="field"><label>이름</label><input class="input" id="a-name" value="${esc(a.name)}" placeholder="예: 내 Mac 계정"></div>
        <div class="field"><label>계정 이름 (로그인 이름)</label><input class="input" id="a-user" value="${esc(a.user)}" placeholder="예: alex" autofocus><span class="err" id="e-user"></span></div>
      </div>
      <div class="field"><label>암호</label><div class="input-wrap"><input class="input" id="a-pass" type="password" placeholder="${ex && ex.has_pw ? '저장된 암호 사용 (바꾸려면 입력)' : 'Mac 로그인 암호'}"><button class="reveal i" tabindex="-1" title="암호 보기">&#xE7B3;</button></div>
        <span class="hint">화면 공유와 원격 로그인(SSH)에 같이 씁니다.</span></div>
      <div class="field"><label>SSH 키 (선택)</label>
        <div class="row" style="gap:8px"><input class="input" id="a-key" value="${esc(a.ssh_key)}" placeholder="비우면 기본 키(~/.ssh)와 에이전트 사용"><button class="btn" data-x="browse" style="flex:none">${I('E8B7')}찾아보기</button></div></div>
      <div class="field" id="keypass-field" ${a.ssh_key ? '' : 'hidden'}><label>키 암호 (있다면)</label><input class="input" id="a-keypass" type="password" placeholder="${ex && ex.has_keypass ? '저장됨 (바꾸려면 입력)' : '키에 암호가 없으면 비워 두세요'}"></div>
    </div><div class="foot">${ex ? `<button class="btn subtle left" data-x="delete" style="color:var(--danger)">${I('E74D')}삭제</button>` : ''}
      <button class="btn" data-x="cancel">취소</button><button class="btn primary" data-x="save">저장</button></div></div>`);
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
    if (!user) { m.querySelector('#e-user').textContent = '계정 이름을 입력하세요'; m.querySelector('#a-user').focus(); return; }
    const acc = { ...a, name: m.querySelector('#a-name').value.trim(), user, ssh_key: m.querySelector('#a-key').value.trim() };
    delete acc.has_pw; delete acc.has_keypass; delete acc.uses;
    send('save_account', { account: acc, password: pass.value, key_pass: m.querySelector('#a-keypass').value, clear_key_pass: !acc.ssh_key });
    closeModal('account');
    toast(ex ? '계정을 저장했습니다' : `${acc.name || user} 계정을 만들었습니다`, 'E73E');
  };
  m.querySelector('[data-x="save"]').onclick = save;
  m.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target.tagName === 'INPUT') { e.preventDefault(); save(); } });
}

function askDeleteAccount(id) {
  const a = acctById(id);
  if (!a) return;
  confirmDialog(`${esc(a.name || a.user)} 계정을 삭제할까요?`, a.uses ? `이 계정을 쓰던 연결 ${a.uses}개는 계정 이름만 남고, 다음 연결 때 암호를 다시 묻습니다.` : '저장된 암호와 키 암호도 함께 삭제됩니다.', '삭제', true, () => {
    send('delete_account', { id });
    toast('계정을 삭제했습니다', 'E74D');
  });
}

// What the connection editor asks for, per kind of device.
const KINDS = {
  mac: { kind: '', os: 'mac', label: 'Mac', hint: 'Mac의 화면 공유 (시스템 설정 → 일반 → 공유 → 화면 공유)', hostPh: '192.168.0.10, mac-mini.local',
    user: 'Mac 계정 이름', userPh: '예: alex', userHint: 'Mac 로그인에 쓰는 짧은 이름 (Apple ID 아님)', pass: 'Mac 로그인 암호', ssh: true },
  windows: { kind: '', os: 'windows', label: 'Windows · Itda', hint: '그 PC의 Itda에서 ‘공유 → 이 PC 공유’를 켜 두세요. 어떤 Windows든 됩니다.', hostPh: 'STUDY-PC.local, 192.168.0.40',
    user: null, pass: '공유 암호', passHint: '그 PC의 Itda → 공유 페이지에 표시된 8글자 암호 (Windows 로그인 암호나 이메일 아님)', ssh: false },
  rdp: { kind: 'rdp', os: 'windows', label: 'Windows 원격 데스크톱', hint: MAC ? 'Windows Pro의 원격 데스크톱(설정 → 시스템 → 원격 데스크톱)이 켜진 PC. Itda 창에서 바로 엽니다.' : 'Windows Pro의 원격 데스크톱(설정 → 시스템 → 원격 데스크톱)이 켜진 PC. 이 PC에서는 Windows 원격 데스크톱 앱으로, 웹 뷰어에서는 브라우저로 엽니다.', hostPh: 'OFFICE-PC, 192.168.0.40',
    user: 'Windows 계정 (Microsoft 계정은 이메일)', userPh: '예: alex@outlook.com 또는 alex', userHint: 'Microsoft 계정으로 로그인하는 PC는 이메일 주소, 로컬 계정은 사용자 이름, 회사 계정은 회사\\이름입니다.',
    pass: 'Windows 암호 (PIN 아님)', passHint: 'Microsoft 계정이면 그 계정의 암호입니다. 로그인 화면의 숫자 PIN이나 얼굴 인식은 원격 접속에 쓸 수 없습니다.', ssh: false },
  linux: { kind: '', os: 'linux', label: 'Linux · 기타 VNC', hint: 'Linux(x11vnc, TigerVNC, wayvnc), Raspberry Pi 등 VNC 서버가 있는 기기', hostPh: 'raspberrypi.local, 192.168.0.30:5901',
    user: '사용자 이름 (선택)', userPh: '대부분 비워 둡니다', userHint: 'VNC 암호만 쓰는 서버는 비워 두세요', pass: 'VNC 암호', ssh: true },
};
// Which editor kind a saved host is; older hosts without `os` were Macs (or Itda PCs with no user).
function kindOf(h) {
  if (h.kind === 'rdp') return 'rdp';
  if (KINDS[h.os]) return h.os;
  return 'mac';
}
// Words for a host's device in prompts and errors.
const devWord = (h) => (!h ? 'Mac' : { mac: 'Mac', windows: 'PC', rdp: 'PC', linux: '기기' }[kindOf(h)]);

function openEdit(init = {}, mode = {}) {
  const existing = init.id ? hostById(init.id) : null;
  const h = existing ? JSON.parse(JSON.stringify(existing)) : { id: '', name: init.name || '', host: init.host || '', user: init.user || '', group: '', favorite: false, kind: init.kind || '', os: init.os || '', opts: { ...(S.settings.defaults || {}) } };
  const K0 = kindOf(h);
  const KD = KINDS[K0];
  const quick = !!mode.quick;
  const hasPw = existing && existing.has_pw;
  const ssh = h.ssh || { on: false, user: '', port: 0 };
  const title = quick ? `${esc(h.host)}에 연결` : existing ? '연결 편집' : '새 연결';
  const tabs = quick ? '' : `<div class="tabs" role="tablist">
      <button class="on" data-tab="general">일반</button><button data-tab="display">화면</button>${MAC ? '' : '<button data-tab="input">키보드</button>'}<button data-tab="conn">연결</button></div>`;
  const groupsList = groups().map((g) => `<option value="${esc(g)}">`).join('');
  const chipList = (gs) => gs.map((g) => `<span class="chip" data-chip="${esc(g)}">${esc(g)}<button type="button" class="chip-x" data-chip-x="${esc(g)}" aria-label="${esc(g)} 그룹 빼기">×</button></span>`).join('');
  const groupField = quick ? '' : `<div class="field"><label for="f-group">그룹</label>
      <div class="chips input" id="group-chips">${chipList(hostGroups(h))}<input id="f-group" list="groups" placeholder="${hostGroups(h).length ? '' : '없음 — 예: 집, 회사'}" aria-label="그룹 추가"></div>
      <datalist id="groups">${groupsList}</datalist><span class="hint">Enter나 쉼표로 추가합니다. 같은 그룹의 연결은 왼쪽 목록에서 모아 볼 수 있습니다.</span></div>`;
  const sshBox = `<div class="ssh-box" ${KD.ssh ? '' : 'hidden'}>
        <div class="opt"><span class="i ssh-ico">&#xE72E;</span><div class="txt"><b>SSH로 암호화해서 연결 <a href="#" class="help-link" data-help="accounts">자세히</a></b><span>인터넷·공용 Wi-Fi에서 켜세요. 상대 기기에 SSH(Mac은 ‘원격 로그인’)가 켜져 있어야 하고 암호는 위와 같습니다.</span></div>
          <label class="switch"><input type="checkbox" id="f-ssh" ${ssh.on ? 'checked' : ''}><span></span></label></div>
        <div class="row" id="ssh-fields" ${ssh.on ? '' : 'hidden'}>
          <div class="field"><label>SSH 사용자</label><input class="input" id="f-ssh-user" value="${esc(ssh.user)}" placeholder="위 계정 이름과 같음"></div>
          <div class="field"><label>SSH 포트</label><input class="input" id="f-ssh-port" value="${ssh.port || ''}" placeholder="22" inputmode="numeric"><span class="err" id="e-ssh"></span></div>
        </div>
        ${S.sshOk === false ? `<p class="warn-note" id="ssh-missing" ${ssh.on ? '' : 'hidden'}>${I('E7BA')} 이 PC에 OpenSSH 클라이언트가 없습니다. 설정 → 시스템 → 선택적 기능에서 ‘OpenSSH 클라이언트’를 추가하세요.</p>` : ''}
      </div>`;
  const hp = h.hp || {};
  const hpBox = MAC ? `<div class="field hp-box" ${h.kind !== 'rdp' && (h.os || 'mac') === 'mac' ? '' : 'hidden'}><label>성능</label>
      <div class="opt"><span class="hp-ico" aria-hidden="true">⚡</span><div class="txt"><b>고성능 모드 (H.264)</b><span id="hp-state">${hp.on ? '켜짐 — 이 Mac의 Itda 고성능 공유로 연결합니다 (SSH 터널). 연결할 수 없으면 기본 화면 공유를 씁니다.' : '꺼짐 — 기본 화면 공유(ZRLE)로 연결합니다. 켜면 SSH로 이 Mac에 Itda를 설치하고 고성능 공유를 켭니다.'}</span></div>
        ${hp.on ? '<label class="switch"><input type="checkbox" id="f-hp" checked><span></span></label>' : (existing ? '<button type="button" class="btn sm" id="hp-enable">켜기</button>' : '')}</div>
      <span class="hint">원격 로그인(SSH)이 켜져 있고, 그 Mac 화면에 로그인한 계정으로 연결해야 합니다. 그 Mac의 권한 도우미에서 화면 보기와 조작 권한을 한 항목씩 안내합니다.</span></div>` : '';
  const general = `<div data-pane="general">
      ${quick ? '' : `<div class="field"><label>종류</label><div class="seg kind-seg">
          ${Object.entries(KINDS).map(([k, d]) => `<button type="button" data-kind="${k}" class="${k === K0 ? 'on' : ''}">${d.label}</button>`).join('')}</div>
        <span class="hint" id="kind-hint">${KD.hint}</span></div>`}
      ${quick ? '' : `<div class="field"><label>이름</label><input class="input" id="f-name" value="${esc(h.name)}" placeholder="예: 사무실 Mac mini (비우면 주소 사용)"></div>`}
      ${quick ? '' : `<div class="field"><label>주소</label><input class="input" id="f-host" value="${esc(h.host)}" placeholder="${KD.hostPh}" ${existing ? '' : 'autofocus'}><span class="err" id="e-host"></span></div>`}
      ${groupField}
      <div class="field"><label>로그인</label>
        <select class="input" id="f-account">
          <option value="">이 연결에만 입력</option>
          ${S.accounts.map((a) => `<option value="${esc(a.id)}" ${h.account === a.id ? 'selected' : ''}>계정: ${esc(a.name || a.user)} (${esc(a.user)})</option>`).join('')}
          <option value="__new">+ 새 계정 만들기…</option>
        </select>
        <span class="hint" id="acct-hint">${h.account ? acctHint(h.account) : '여러 기기에 같은 계정을 쓴다면 계정을 만들어 두면 암호를 한 번만 입력합니다.'}</span></div>
      <div id="own-creds" ${h.account ? 'hidden' : ''}>
        <div class="row">
          <div class="field" id="user-field" ${KD.user ? '' : 'hidden'}><label id="user-label">${KD.user || ''}</label><input class="input" id="f-user" value="${esc(h.user)}" placeholder="${KD.userPh || ''}" ${quick || existing ? 'autofocus' : ''}><span class="hint" id="user-hint">${KD.userHint || ''}</span></div>
          <div class="field"><label id="pass-label">${KD.pass}</label><div class="input-wrap"><input class="input" id="f-pass" type="password" placeholder="${hasPw && !h.account ? '저장된 암호 사용 (바꾸려면 입력)' : KD.pass}"><button class="reveal i" tabindex="-1" title="암호 보기">&#xE7B3;</button></div></div>
        </div>
        <label class="check"><input type="checkbox" id="f-save" ${hasPw || !existing ? 'checked' : ''}>암호 저장 <span style="color:var(--text-3);font-size:12px">— ${MAC ? 'Mac 키체인에' : 'Windows 자격 증명 관리자에 암호화되어'} 저장됩니다</span></label>
      </div>
      <span class="hint" id="pass-hint" ${KD.passHint ? '' : 'hidden'}>${KD.passHint || ''}</span>
${quick ? sshBox : ''}
    </div>`;
  const o = h.opts || {};
  const display = `<div data-pane="display" hidden>
      <div class="field"><label>다중 모니터 표시</label><select class="input" data-opt-number="monitor_mode">${[[0,'한 창에서 모니터 선택'],[1,'전체 모니터 한눈에 보기'],[2,'모니터마다 별도 창']].map(([v,t])=>`<option value="${v}" ${v===(o.monitor_mode||0)?'selected':''}>${t}</option>`).join('')}</select></div>
      <div class="field"><label>원격 화면 구성</label><select class="input" data-opt-number="monitor_layout">${[[0,'서버의 모니터 정보 사용'],[1,'수동: 가로 2영역'],[2,'수동: 세로 2영역'],[3,'수동: 가로 3영역'],[4,'수동: 2 × 2영역']].map(([v,t])=>`<option value="${v}" ${v===(o.monitor_layout||0)?'selected':''}>${t}</option>`).join('')}</select><span class="hint">서버가 모니터 배치를 제공하지 않을 때만 수동 분할을 선택하세요. RDP는 로그인 세션의 가상 화면을 보여줍니다.</span></div>
      ${optRow('monitor_fullscreen', '분리한 창을 로컬 모니터에 전체 화면으로 배치', '로컬 모니터가 부족하면 남는 창은 겹치지 않게 배치합니다.', o.monitor_fullscreen)}
      ${optRow('actual', '실제 크기 (1:1)로 시작', '끄면 창 크기에 맞춰 축소해서 보여 줍니다', o.actual)}
      ${optRow('fullscreen', '전체 화면으로 시작', '', o.fullscreen)}
      ${optRow('resize', '창 크기에 맞춰 해상도 바꾸기 (원격 데스크톱)', MAC ? 'Windows 원격 데스크톱의 해상도가 연결 창 크기를 따라갑니다.' : '웹 뷰어(PC·Mac 브라우저)에서 Windows 원격 데스크톱의 해상도가 창 크기를 따라갑니다. 이 PC의 원격 데스크톱 앱은 원래 창 크기를 따라갑니다.', o.resize !== false)}
      ${optRow('view_only', '보기 전용', '마우스와 키보드를 Mac으로 보내지 않습니다', o.view_only)}
      ${MAC ? '' : optRow('stats', '통계 표시', 'fps, 전송 속도, 응답 시간', o.stats)}
    </div>`;
  const input = `<div data-pane="input" hidden>
      ${optRow('swap', 'Ctrl ↔ ⌘ Cmd 바꾸기', '켜면 Ctrl+C/V가 ⌘C/⌘V로 동작합니다. Windows 키는 Ctrl이 됩니다.', o.swap)}
      ${optRow('hangul_caps', '한/영 키를 Caps Lock으로 보내기', '끄면 Ctrl+Space를 보냅니다 (Mac의 입력 소스 단축키에 맞추세요)', o.hangul_caps)}
    </div>`;
  const conn = `<div data-pane="conn" hidden>
      <div class="field"><label>연결 방법</label></div>
      ${sshBox}
      ${hpBox}
      ${optRow('clipboard', '클립보드 공유', '복사한 텍스트를 양쪽에서 붙여 넣을 수 있습니다', o.clipboard)}
      ${optRow('reconnect', '자동 재연결', '네트워크가 끊기면 다시 연결합니다', o.reconnect)}
    </div>`;
  const foot = quick
    ? `<button class="btn" data-x="cancel">취소</button><button class="btn primary" data-x="connect">${I('E768')}연결</button>`
    : `${existing ? `<button class="btn subtle left" data-x="delete" style="color:var(--danger)">${I('E74D')}삭제</button>` : ''}
       <button class="btn" data-x="cancel">취소</button><button class="btn" data-x="save">저장</button><button class="btn primary" data-x="connect">${I('E768')}저장 후 연결</button>`;
  const m = openModal('edit', `<div class="dialog wide"><div class="body"><h2>${title}</h2>
      <p class="lead">${quick ? '새 연결로 저장되어 다음부터 바로 연결할 수 있습니다.' : '연결할 기기의 종류와 주소, 로그인 정보를 입력하세요.'}</p>
      ${tabs}${general}${quick ? '' : display + input + conn}</div><div class="foot">${foot}</div></div>`);

  m.dataset.host = h.id || '';
  m.querySelectorAll('[data-tab]').forEach((b) => (b.onclick = () => {
    m.querySelectorAll('[data-tab]').forEach((x) => x.classList.toggle('on', x === b));
    m.querySelectorAll('[data-pane]').forEach((p) => (p.hidden = p.dataset.pane !== b.dataset.tab));
  }));
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
      gin.placeholder = current().length ? '' : '없음 — 예: 집, 회사';
    };
    gin.addEventListener('keydown', (e) => {
      // Enter adds a group here; it must not also save the dialog. (Empty Enter still saves.)
      // Enter that finishes Korean composition only commits the syllable.
      if (e.key === 'Enter' && (e.isComposing || e.keyCode === 229)) { e.stopPropagation(); return; }
      if ((e.key === 'Enter' && gin.value.trim() || e.key === ',') && !e.isComposing) { e.preventDefault(); e.stopPropagation(); add(gin.value); }
      else if (e.key === 'Backspace' && !gin.value && !e.isComposing && e.keyCode !== 229) { const last = [...chips.querySelectorAll('[data-chip]')].pop(); if (last) { last.remove(); gin.placeholder = current().length ? '' : '없음 — 예: 집, 회사'; } }
    });
    gin.addEventListener('input', () => { if (gin.value.includes(',')) add(gin.value); });
    // Picking a suggestion from the list adds it right away.
    gin.addEventListener('change', () => { if (gin.value.trim()) add(gin.value); });
    chips.addEventListener('click', (e) => {
      const x = e.target.closest('[data-chip-x]');
      if (x) { x.closest('[data-chip]').remove(); gin.focus(); } else gin.focus();
    });
    m._groups = () => { if (gin.value.trim()) add(gin.value); return current(); };
  }
  const hpEnable = m.querySelector('#hp-enable');
  if (hpEnable) hpEnable.onclick = () => {
    hpEnable.disabled = true;
    hpEnable.textContent = '준비 중…';
    window.ItdaHpSetup.begin({id:h.id,name:label(h),send:(op,extra={})=>send(`hp_${op}`,{id:h.id,...extra}),done:()=>toast(`${label(h)} 고성능 모드가 준비됐습니다`, 'E73E'),cancel:()=>{if(hpEnable.isConnected){hpEnable.disabled=false;hpEnable.textContent='다시 확인';}}});
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
    if (!host.host) msg = '주소를 입력하세요';
    else if (/\s/.test(host.host)) msg = '주소에 공백이 있을 수 없습니다';
    else {
      const port = host.host.match(/^[^\[\]]+:(\d*)$/) || host.host.match(/^\[.*\]:(\d*)$/);
      if (port && !(+port[1] > 0 && +port[1] < 65536)) msg = '포트 번호가 올바르지 않습니다';
    }
    if (e) { e.textContent = msg; m.querySelector('#f-host').classList.toggle('bad', !!msg); }
    const raw = m.querySelector('#f-ssh-port').value.trim();
    const sshBad = host.ssh.on && raw !== '' && !(+raw > 0 && +raw < 65536 && /^\d+$/.test(raw));
    m.querySelector('#e-ssh').textContent = sshBad ? '1–65535 사이의 숫자' : '';
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
    if (!then) toast(existing ? '저장했습니다' : `${label(host)}을(를) 추가했습니다`, 'E73E');
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
  confirmDialog(`${esc(label(h))}을(를) 삭제할까요?`, '저장된 암호와 미리보기도 함께 삭제됩니다.', '삭제', true, () => {
    send('delete_host', { id });
    toast(`${label(h)}을(를) 삭제했습니다`, 'E74D');
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

const STAGES = [['resolve', '주소 찾기'], ['tcp', 'Mac에 연결'], ['auth', '로그인'], ['init', '화면 준비']];
const SSH_STAGES = [['ssh', '암호화 연결 (SSH)'], ['tcp', '화면 공유에 연결'], ['auth', '로그인'], ['init', '화면 준비']];

function stepsHtml(stage, ssh) {
  const list = ssh ? SSH_STAGES : STAGES;
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
      <h2>${esc(label(h))}에 연결하는 중</h2><div class="addr">${esc(h.user ? `${h.user}@${h.host}` : h.host)}</div>
      <div class="progress-steps">${stepsHtml(s ? s.stage : (h.ssh && h.ssh.on ? 'ssh' : 'resolve'), h.ssh && h.ssh.on)}</div></div>
      <div class="foot"><button class="btn subtle left" data-x="hide">백그라운드에서 계속</button><button class="btn" data-x="cancel">취소</button></div></div>`, { sticky: true });
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
      <h2>${esc(label(h))}에 로그인</h2>
      <p class="lead">${retry ? '암호가 맞지 않았습니다. 다시 입력하세요.' : `${KINDS[kindOf(h)].pass}를 입력하세요.`}</p>
      <div class="field" ${KINDS[kindOf(h)].user ? '' : 'hidden'}><label>${KINDS[kindOf(h)].user || ''}</label><input class="input" id="p-user" value="${esc(h.user)}" placeholder="${KINDS[kindOf(h)].userPh || ''}"></div>
      <div class="field"><label>${KINDS[kindOf(h)].pass}</label><div class="input-wrap"><input class="input" id="p-pass" type="password" autofocus><button class="reveal i" tabindex="-1" title="암호 보기">&#xE7B3;</button></div></div>
      <label class="check"><input type="checkbox" id="p-save" checked>암호 저장</label>
    </div><div class="foot"><button class="btn" data-x="cancel">취소</button><button class="btn primary" data-x="ok">${I('E768')}연결</button></div></div>`);
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

const ERRORS = {
  auth: ['로그인하지 못했습니다', 'Mac이 계정 이름 또는 암호를 받아들이지 않았습니다.', [
    'Mac 로그인 화면에서 쓰는 <b>계정 이름(짧은 이름)</b>과 암호를 입력하세요. Apple ID가 아닙니다.',
    'Mac의 <b>화면 공유 ⓘ → 접근 허용</b>에 이 계정이 포함되어 있는지 확인하세요.',
    'Mac이 <b>원격 관리</b>를 쓰고 있다면 원격 관리 ⓘ → <b>옵션</b>에서 이 계정에 <b>관찰</b>과 <b>제어</b>가 켜져 있는지 확인하세요.',
  ]],
  ssh_auth: ['SSH 로그인에 실패했습니다', 'Mac이 원격 로그인(SSH) 계정 이름 또는 암호를 받아들이지 않았습니다.', [
    'Mac 계정의 <b>짧은 이름</b>과 로그인 암호를 입력하세요. SSH 사용자를 따로 지정했다면 그 이름을 확인하세요.',
    'Mac의 <b>원격 로그인 ⓘ → 접근 허용</b>에 이 계정이 포함되어 있는지 확인하세요.',
  ]],
  ssh_hostkey: ['Mac의 보안 키가 바뀌었습니다', '이전에 이 주소로 접속했을 때와 다른 SSH 키가 왔습니다. 다른 기기이거나 누군가 연결을 가로채고 있을 수 있습니다.', [
    'Mac을 새로 설치했거나 교체했다면 정상입니다. 확인한 뒤 <b>새 키를 신뢰</b>하고 연결하세요.',
    '이유를 모른다면 연결하지 마세요. 특히 공용 Wi-Fi에서는 주의하세요.',
  ]],
  ssh_refused: ['원격 로그인(SSH)이 꺼져 있습니다', 'Mac이 SSH 연결을 받지 않았습니다.', [
    'Mac의 <b>시스템 설정 → 일반 → 공유 → 원격 로그인</b>을 켜세요.',
    '외부에서 연결한다면 공유기에서 SSH 포트(기본 22)가 Mac으로 전달되는지, 또는 VPN(예: Tailscale)이 켜져 있는지 확인하세요.',
  ]],
  ssh_missing: ['OpenSSH 클라이언트가 없습니다', '암호화 연결에는 Windows에 기본 포함된 OpenSSH 클라이언트가 필요합니다.', [
    'Windows <b>설정 → 시스템 → 선택적 기능 → 기능 추가</b>에서 ‘OpenSSH 클라이언트’를 설치하세요.',
  ]],
  declined: ['상대 PC에서 연결을 거절했습니다', '그 PC 앞에 있는 사람이 화면 공유 요청에 ‘아니요’를 눌렀습니다.', ['상대에게 알린 뒤 다시 연결하세요. 요청 창이 뜨면 ‘예’를 눌러 달라고 하세요.']],
  need_user: ['Mac 계정 이름이 필요합니다', '이 Mac은 VNC 암호 대신 Mac 계정으로 로그인하도록 설정되어 있습니다.', ['연결 편집에서 <b>Mac 계정 이름</b>을 입력하세요.']],
  refused: ['Mac이 연결을 거부했습니다', '화면 공유가 꺼져 있는 것 같습니다.', [
    'Mac의 <b>시스템 설정 → 일반 → 공유 → 화면 공유</b>를 켜세요. <b>원격 관리</b>를 켜도 연결됩니다.',
    '이미 켜져 있다면 껐다가 다시 켜 보세요. 다른 원격 앱이 같은 포트(5900)를 쓰고 있을 수도 있습니다.',
  ]],
  timeout: ['Mac이 응답하지 않습니다', '주소로 연결을 시도했지만 응답이 없었습니다.', [
    'Mac이 켜져 있고 <b>같은 네트워크</b>에 있는지 확인하세요. 외부에서는 VPN이나 SSH 터널이 필요합니다.',
    'Mac이 잠자기 중이라면 <b>에너지 → 네트워크 접근 시 깨우기</b>를 켜 두세요.',
    'Mac의 IP 주소가 바뀌었을 수 있습니다. 주변 Mac 검색으로 다시 찾아보세요.',
  ]],
  unreachable: ['Mac에 도달할 수 없습니다', `${HERE}에서 그 주소로 가는 네트워크 경로가 없습니다.`, [
    'Wi-Fi 또는 유선 네트워크에 연결되어 있는지 확인하세요.',
    '다른 네트워크의 Mac이라면 VPN을 먼저 연결하세요.',
  ]],
  dns: ['주소를 찾을 수 없습니다', '입력한 이름을 IP 주소로 바꾸지 못했습니다.', [
    '주소에 오타가 없는지 확인하세요.',
    '<b>이름.local</b> 주소가 안 되면 Mac의 화면 공유 설정에 표시된 IP 주소를 쓰세요.',
  ]],
  reset: ['Mac이 연결을 끊었습니다', '연결은 되었지만 Mac이 바로 끊었습니다.', [
    '잠시 후 다시 시도하세요. 로그인을 여러 번 실패하면 Mac이 잠시 연결을 막습니다.',
    'Mac의 화면 공유 설정에서 이 계정의 접근이 허용되어 있는지 확인하세요.',
  ]],
  not_vnc: ['화면 공유 서버가 아닙니다', '그 주소와 포트에서 다른 서비스가 응답했습니다.', ['주소 뒤의 포트 번호를 확인하세요. Mac 화면 공유는 기본적으로 5900을 씁니다.']],
  lost: ['연결이 끊어졌습니다', 'Mac과의 연결이 끊겼고 다시 연결하지 못했습니다.', ['네트워크 상태를 확인한 뒤 다시 연결하세요.']],
  other: ['연결 중 문제가 생겼습니다', '예상하지 못한 오류가 발생했습니다.', ['다시 시도해 보고, 계속되면 설정 → 로그에서 내용을 확인하세요.']],
};

function showError(id, kind, msg) {
  const h = hostById(id);
  let [title, lead, tips] = ERRORS[kind] || ERRORS.other;
  const k = h ? kindOf(h) : 'mac';
  if (k !== 'mac') {
    // The tables speak about Macs; reword for PCs and other VNC servers.
    const w = devWord(h);
    title = title.replace(/Mac이/g, `${w}이(가)`).replace(/Mac에/g, `${w}에`).replace(/Mac /g, `${w} `);
    lead = lead.replace(/Mac이/g, `${w}이(가)`).replace(/Mac /g, `${w} `);
    const OTHER = {
      auth: { windows: ['그 PC의 Itda → <b>공유</b> 페이지에 표시된 <b>8글자 공유 암호</b>를 입력하세요. Windows 로그인 암호나 Microsoft 계정 암호가 <b>아닙니다</b>.', '공유 암호를 새로 만들었다면 새 암호가 필요합니다.'],
        rdp: ['Microsoft 계정으로 로그인하는 PC(대부분의 Windows 11 가정용 PC)라면 계정에 <b>이메일 주소</b>(예: alex@outlook.com), 암호에 그 <b>Microsoft 계정 암호</b>를 넣으세요. 로그인 화면의 <b>숫자 PIN·얼굴 인식은 쓸 수 없습니다</b>.',
          '로컬 계정이면 사용자 이름과 그 암호, 회사 계정이면 <b>회사\\이름</b> 형식입니다. 그 PC의 <b>설정 → 계정 → 사용자 정보</b>에 이메일이 보이면 Microsoft 계정입니다.',
          '그 PC의 <b>설정 → 계정 → 로그인 옵션</b>에서 ‘Microsoft 계정에 Windows Hello 로그인만 허용’이 켜져 있으면 끄고, 그 PC에서 한 번 <b>암호로</b> 로그인한 뒤 다시 시도하세요.',
          '그 계정이 <b>원격 데스크톱 사용자</b>로 허용되어 있는지 확인하세요 (관리자 계정은 기본으로 허용).'],
        linux: ['VNC 서버에 설정한 <b>VNC 암호</b>를 입력하세요 (리눅스 로그인 암호와 다를 수 있습니다).', '사용자 이름이 필요 없는 서버라면 사용자 이름을 비워 두세요.'] },
      refused: { windows: ['그 PC의 Itda에서 <b>공유 → 이 PC 공유</b>가 켜져 있는지 확인하세요.', '포트를 바꿨다면 주소 뒤에 <b>:포트</b>를 붙이세요.'],
        rdp: ['그 PC의 <b>설정 → 시스템 → 원격 데스크톱</b>을 켜세요. Windows Home에는 없으니 대신 Itda의 ‘이 PC 공유’를 쓰세요.'],
        linux: ['VNC 서버가 실행 중인지, 포트(보통 5900 또는 5901)가 맞는지 확인하세요.', '방화벽이 VNC 포트를 막고 있지 않은지 확인하세요.'] },
    };
    tips = (OTHER[kind] && OTHER[kind][k]) || tips.map((t) => t.replace(/Mac/g, w));
  }
  const name = h ? label(h) : 'Mac';
  const primary = kind === 'auth' || kind === 'ssh_auth'
    ? `<button class="btn primary" data-x="password" autofocus>${I('E72E')}암호 다시 입력</button>`
    : kind === 'ssh_hostkey'
      ? `<button class="btn danger" data-x="trust">${I('E8D7')}새 키 신뢰하고 연결</button>`
      : kind === 'ssh_missing'
        ? ''
        : kind === 'need_user'
      ? `<button class="btn primary" data-x="edit" autofocus>${I('E70F')}연결 편집</button>`
      : `<button class="btn primary" data-x="retry" autofocus>${I('E72C')}다시 시도</button>`;
  const secondary = kind !== 'need_user' ? `<button class="btn" data-x="edit">${I('E70F')}연결 편집</button>` : '';
  const topic = { refused: 'mac', auth: 'mac', need_user: 'mac', ssh_auth: 'accounts', ssh_refused: 'mac', ssh_hostkey: 'accounts', timeout: 'outside', unreachable: 'outside', dns: 'trouble', declined: 'windows' }[kind] || 'trouble';
  const help = `<button class="btn subtle left" data-x="help">${I('E897')}해결 방법 보기</button>`;
  const m = openModal('error', `<div class="dialog"><div class="body">
      <div class="state-icon err">${I(kind === 'auth' || kind.startsWith('ssh') ? 'E72E' : 'E783')}</div>
      <h2>${esc(title)}</h2><p class="lead"><b>${esc(name)}</b> — ${esc(lead)}</p>
      <ul class="tips">${tips.map((t) => `<li>${I('E946')}<span>${t}</span></li>`).join('')}</ul>
      ${msg ? `<details class="raw"><summary>자세한 정보</summary><code>${esc(msg)}</code></details>` : ''}
    </div><div class="foot">${help}<button class="btn" data-x="close">닫기</button>${secondary}${primary}</div></div>`);
  const on = (x, f) => { const b = m.querySelector(`[data-x="${x}"]`); if (b) b.onclick = () => { closeModal('error'); f(); }; };
  on('close', () => {});
  on('retry', () => connect(id));
  on('password', () => showPassword(id, true));
  on('trust', () => { showConnecting(id); send('trust_new_key', { id }); });
  on('edit', () => openEdit({ id }));
  on('help', () => helpGo(topic));
}

// ---------------------------------------------------------------- context menu

function showCtx(id, x, y) {
  const h = hostById(id);
  if (!h) return;
  const s = S.sessions[id];
  const items = [];
  if (!s) items.push(['connect', 'E768', '연결']);
  else if (s.state === 'connecting') items.push(['cancel', 'E711', '연결 취소']);
  else items.push(['focus', 'E8A7', '창으로 이동'], ['disconnect', 'E711', '연결 끊기']);
  if (MAC && ['mac', 'macos'].includes(h.os) && h.kind !== 'rdp') items.push(['apple-screen-sharing', 'E7F4', 'Apple 화면 공유 앱에서 열기']);
  items.push(['files', 'E8B7', '파일 열기']);
  if(h.os!=='linux')items.push(['remote-setup','E896','Itda 설치·설정 공유…']);
  items.push('-', ['edit', 'E70F', '편집'], ['dup', 'E8C8', '복제'], ['fav', h.favorite ? 'E8D9' : 'E734', h.favorite ? '즐겨찾기 해제' : '즐겨찾기에 추가']);
  items.push(['thumb-pick', 'E91B', '미리보기 이미지 지정…']);
  if (h.custom_thumb) items.push(['thumb-reset', 'E72C', '미리보기를 마지막 화면으로']);
  if (h.has_pw) items.push(['forget', 'E8D7', '저장된 암호 지우기']);
  items.push('-', ['del', 'E74D', '삭제', 'danger']);
  const el = $('#ctx');
  el.innerHTML = items.map((it) => (it === '-' ? '<hr>' : `<button data-act="${it[0]}" data-id="${esc(id)}" class="${it[3] || ''}">${I(it[1])}${it[2]}</button>`)).join('');
  el.hidden = false;
  const r = el.getBoundingClientRect();
  el.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  el.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
  const first = el.querySelector('button');
  if (first) first.focus();
}

function hideCtx() {
  $('#ctx').hidden = true;
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
    case 'remote-setup': return window.ItdaRemoteSetup.open({id,hosts:S.hosts,accounts:S.accounts,send,modal:openModal,close:closeModal,toast,esc});
    case 'connect': return connect(id);
    case 'focus': return send('focus', { id });
    case 'cancel': return send('cancel', { id });
    case 'disconnect': return send('disconnect', { id });
    case 'edit': return openEdit({ id });
    case 'dup': send('duplicate', { id }); return toast('복제했습니다', 'E8C8');
    case 'fav': {
      const h = hostById(id);
      if (h) { h.favorite = !h.favorite; render(); send('favorite', { id, on: h.favorite }); }
      return;
    }
    case 'forget': send('forget_password', { id }); return toast('저장된 암호를 지웠습니다', 'E8D7');
    case 'thumb-pick': return pickThumb(id);
    case 'thumb-reset': send('set_thumb', { id, png: null }); return toast('마지막 화면을 미리보기로 씁니다', 'E72C');
    case 'del': return askDelete(id);
    case 'more': {
      const r = el.getBoundingClientRect();
      return showCtx(id, r.left, r.bottom + 4);
    }
    case 'new': return openEdit();
    case 'quick': return quickConnect(S.query);
    case 'page': return go(el.dataset.page);
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
    case 'ts-serve-on': toast('Tailscale HTTPS 주소를 만드는 중…', 'E72E'); return send('web_tailscale', { on: true });
    case 'ts-serve-off': return send('web_tailscale', { on: false });
    case 'share-show': {
      const p = document.getElementById('share-pw');
      p.textContent = p.textContent === '••••••••' ? p.dataset.pin : '••••••••';
      return;
    }
    case 'share-reset':
      return confirmDialog('새 공유 암호를 만들까요?', '지금 이 PC를 보고 있는 기기는 연결이 끊기고, 다음부터 새 암호가 필요합니다.', '새 암호 만들기', false, () => { S.share = {}; send('share_reset_pw'); toast('새 공유 암호를 만들었습니다', 'E72C'); });
    case 'share-kick': return send('share_kick');
    case 'onboard': return showOnboarding();
    case 'update-check': toast('새 버전을 확인하는 중…', 'E72C'); return send('update_check');
    case 'update-install': {
      if (!S.update) return send('update_check');
      const open = Object.keys(S.sessions || {}).length;
      return confirmDialog(`Itda ${esc(S.update.version)}로 업데이트할까요?`, `내려받아 확인한 뒤 Itda를 다시 시작합니다.${open ? ` 열려 있는 연결 ${open}개가 끊깁니다.` : ''}`, '업데이트', false, () => { S.updateStage = '준비 중…'; render(); send('update_install'); });
    }
    case 'permission-helper': return send('permission_helper');
    case 'sys-sharing': return send('url', { url: 'x-apple.systempreferences:com.apple.Sharing-Settings.extension' });
    case 'share-all-on':
      if (MAC) return act('web-on', el);
      S.settings.share = { port: 5900, ask: true, trust_near: true, cmd_as_ctrl: true, ...S.settings.share, on: true };
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
      return confirmDialog('새 PIN을 만들까요?', '지금 웹 뷰어에 들어와 있는 기기는 모두 로그아웃됩니다.', '새 PIN 만들기', false, () => { S.web = {}; send('web_reset_pin'); toast('새 PIN을 만들었습니다', 'E72C'); });
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
      toast('미리보기 이미지를 바꿨습니다', 'E91B');
    };
    img.onerror = () => { URL.revokeObjectURL(url); toast('이 이미지는 열 수 없습니다', 'E783', true); };
    img.src = url;
  };
  f.click();
}

function saveSettings() {
  send('settings', { settings: S.settings });
  applyTheme();
  render();
}

document.addEventListener('click', (e) => {
  const ext = e.target.closest('a[data-ext]');
  if (ext) { e.preventDefault(); return send('url', { url: ext.href }); }
  const inCtx = e.target.closest('#ctx');
  if (!inCtx) hideCtx();
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
    else if (t.dataset.path === 'share') { S.settings.share = { port: 5900, ask: true, trust_near: true, cmd_as_ctrl: true, ...S.settings.share, [t.dataset.set]: t.checked }; if (t.dataset.set === 'on') S.share = {}; }
    else if (t.dataset.path === 'web') { S.settings.web = { port: 8780, ...S.settings.web, [t.dataset.set]: t.checked }; S.web = {}; }
    else S.settings[t.dataset.set] = t.checked;
    saveSettings();
  } else if (t.id === 'share-port') {
    const p = parseInt(t.value, 10);
    if (p >= 1024 && p < 65536) { S.settings.share = { ...S.settings.share, port: p }; S.share = {}; saveSettings(); }
    else { t.value = (S.settings.share || {}).port || 5900; toast('1024–65535 사이의 포트를 쓰세요', 'E7BA', true); }
  } else if (t.id === 'web-port') {
    const p = parseInt(t.value, 10);
    if (p >= 1024 && p < 65536) { S.settings.web = { ...S.settings.web, port: p }; S.web = {}; saveSettings(); }
    else { t.value = (S.settings.web || {}).port || 8780; toast('1024–65535 사이의 포트를 쓰세요', 'E7BA', true); }
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
  if (c) showCtx(c.dataset.id, e.clientX, e.clientY);
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
    if (!$('#ctx').hidden) return hideCtx();
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
  if (c) {
    if (e.key === 'Enter') { e.preventDefault(); connect(c.dataset.id); }
    else if (e.key === 'Delete' || (MAC && e.metaKey && e.key === 'Backspace')) askDelete(c.dataset.id);
    else if (e.key === 'F2') openEdit({ id: c.dataset.id });
    else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) { const r = c.getBoundingClientRect(); showCtx(c.dataset.id, r.left + 20, r.top + 20); }
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

window.addEventListener('blur', hideCtx);
render();
send('ready');
