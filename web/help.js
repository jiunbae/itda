// Itda help center: step-by-step guides with real Itda screenshots and drawn
// macOS / Windows settings screens (crisp, theme-aware, highlights where to click).
// Text comes from the help.* messages (web/locales); every page is built when it is shown,
// so a language change only needs a redraw.
'use strict';

// ---------------------------------------------------------------- drawn OS screens

const MAC_ICON = {
  general: ['#8e8e93', '⚙'], share: ['#8e8e93', '⇪'], users: ['#8e8e93', '👥'], energy: ['#ffcc00', '⚡'], battery: ['#34c759', '▮'],
  wifi: ['#0a84ff', '◠'], network: ['#0a84ff', '◍'], privacy: ['#0a84ff', '✋'], display: ['#0a84ff', '▭'], lock: ['#8e8e93', '🔒'],
};

function macSidebar(active) {
  const items = [['wifi', 'Wi-Fi'], ['network', T('help.os.mac.side.network')], ['general', T('help.os.mac.side.general')], ['display', T('help.os.mac.side.display')], ['energy', T('help.os.mac.side.energy')], ['battery', T('help.os.mac.side.battery')], ['privacy', T('help.os.mac.side.privacy')], ['users', T('help.os.mac.side.users')]];
  return `<div class="mac-side"><div class="mac-search">${T('help.os.mac.search')}</div>${items.map(([k, t]) => {
    const [c, g] = MAC_ICON[k];
    return `<div class="mac-item${active === k ? ' on' : ''}"><span class="mac-ic" style="background:${c}">${g}</span>${t}</div>`;
  }).join('')}</div>`;
}

/** A macOS System Settings window. `body` is the right pane. */
function macWindow(active, crumb, body, extra = '') {
  return `<div class="os mac" role="img" aria-label="${T('help.os.mac.label')}">
    <div class="mac-bar"><span class="tl r"></span><span class="tl y"></span><span class="tl g"></span><span class="mac-crumb">‹ ›&nbsp;&nbsp;${crumb}</span></div>
    <div class="mac-body">${macSidebar(active)}<div class="mac-pane">${body}</div></div>${extra}</div>`;
}

const sw = (on, hl) => `<span class="os-switch${on ? ' on' : ''}${hl ? ' hl' : ''}"></span>`;
const mark = (n) => `<b class="mark">${n}</b>`;

function macRow(label, right, { hl = false, sub = '', n = '' } = {}) {
  return `<div class="mac-row${hl ? ' hl' : ''}"><div><div>${label}${n ? mark(n) : ''}</div>${sub ? `<small>${sub}</small>` : ''}</div><div class="mac-right">${right}</div></div>`;
}

const info = (hl, n) => `<span class="mac-info${hl ? ' hl' : ''}">i</span>${n ? mark(n) : ''}`;

function macSharing(highlight) {
  const r = (name, key) => {
    const on = key === 'screen' || (key === 'ssh' && highlight === 'ssh');
    const infoHl = highlight === key + '-i';
    return macRow(name, `${info(infoHl, infoHl ? 2 : '')}${sw(on, highlight === key)}`, { hl: highlight === key, n: highlight === key ? 1 : '' });
  };
  return `<div class="mac-h">${T('help.os.mac.sharing')}</div>
    <div class="mac-group">
      ${r(T('help.os.mac.screenSharing'), 'screen')}
      ${macRow(T('help.os.mac.fileSharing'), sw(false))}
      ${macRow(T('help.os.mac.mediaSharing'), sw(false))}
      ${r(T('help.os.mac.remoteLogin'), 'ssh')}
      ${macRow(T('help.os.mac.remoteMgmt'), sw(false), { sub: T('help.os.mac.remoteMgmt.sub') })}
    </div>
    <div class="mac-group">${macRow(T('help.os.mac.hostname'), '<span class="mac-val">mac-mini.local</span>', { hl: highlight === 'name', n: highlight === 'name' ? 3 : '' })}</div>`;
}

function macScreenSheet() {
  return `<div class="mac-sheet"><div class="mac-h" style="margin-top:0">${T('help.os.mac.screenSharing')}</div>
    <div class="mac-group">
      ${macRow(T('help.os.mac.screenSharing'), sw(true))}
      ${macRow(T('help.os.mac.allowFor'), `<span class="mac-pop">${T('help.os.mac.onlyThese')}</span>`, { hl: true, n: 1 })}
      <div class="mac-users"><div>👤 alex <small>${T('help.os.mac.admin')}</small></div><div class="mac-pm">＋ －</div></div>
      ${macRow(T('help.os.mac.vncPassword'), sw(false), { sub: T('help.os.mac.vncPassword.sub') })}
    </div>
    <div class="mac-sheet-foot"><span class="mac-btn">${T('help.os.mac.done')}</span></div></div>`;
}

function macEnergy() {
  return `<div class="mac-h">${T('help.os.mac.energy')}</div><div class="mac-group">
    ${macRow(T('help.os.mac.preventSleep'), sw(true), { hl: true, n: 1 })}
    ${macRow(T('help.os.mac.wake'), `<span class="mac-pop">${T('help.os.mac.always')}</span>`, { hl: true, n: 2 })}
    ${macRow(T('help.os.mac.powerFailure'), sw(false))}</div>`;
}

const WIN_NAV = [['home', '⌂', 'help.os.win.nav.home'], ['system', '▭', 'help.os.win.nav.system'], ['bt', '✱', 'help.os.win.nav.bt'], ['net', '◍', 'help.os.win.nav.net'], ['person', '✎', 'help.os.win.nav.person'], ['apps', '▦', 'help.os.win.nav.apps'], ['acct', '◉', 'help.os.win.nav.acct'], ['privacy', '⛨', 'help.os.win.nav.privacy']];

/** A Windows 11 Settings window. */
function winWindow(active, title, body) {
  return `<div class="os win" role="img" aria-label="${T('help.os.win.label')}">
    <div class="win-bar"><span>${T('help.os.win.settings')}</span><span class="win-ctl">─ ▢ ✕</span></div>
    <div class="win-body"><div class="win-side"><div class="win-user"><span class="win-ava">A</span><div><b>alex</b><small>${T('help.os.win.localAccount')}</small></div></div>
      <div class="win-find">${T('help.os.win.find')}</div>${WIN_NAV.map(([k, g, t]) => `<div class="win-item${active === k ? ' on' : ''}"><span>${g}</span>${T(t)}</div>`).join('')}</div>
      <div class="win-pane"><div class="win-title">${title}</div>${body}</div></div></div>`;
}

function winCard(label, right, { sub = '', hl = false, n = '', icon = '▭' } = {}) {
  return `<div class="win-card${hl ? ' hl' : ''}"><span class="win-ic">${icon}</span><div class="win-txt"><div>${label}${n ? mark(n) : ''}</div>${sub ? `<small>${sub}</small>` : ''}</div><div class="win-right">${right}</div></div>`;
}

const wsw = (on, hl) => `<span class="win-switch${on ? ' on' : ''}${hl ? ' hl' : ''}"><i></i></span><span class="win-sw-l">${on ? T('help.os.win.on') : T('help.os.win.off')}</span>`;

function winRemoteDesktop() {
  return winWindow('system', T('help.os.win.rd.title'), `
    ${winCard(T('help.os.win.rd'), wsw(true, true), { sub: T('help.os.win.rd.sub'), hl: true, n: 1, icon: '⇄' })}
    ${winCard(T('help.os.win.rd.nla'), '☑', { sub: T('help.os.win.rd.nla.sub'), icon: '⛨' })}
    ${winCard(T('help.os.win.rd.port'), '<span class="win-val">3389</span>', { icon: '#' })}
    ${winCard(T('help.os.win.rd.name'), '<span class="win-val">LIVING-PC</span>', { sub: T('help.os.win.rd.name.sub'), hl: true, n: 2, icon: '▭' })}
    ${winCard(T('help.os.win.rd.users'), '›', { sub: T('help.os.win.rd.users.sub'), icon: '◉' })}`);
}

function winFirewall() {
  return `<div class="os dlg" role="img" aria-label="${T('help.os.fw.label')}">
    <div class="dlg-bar">${T('help.os.fw.title')}<span>✕</span></div>
    <div class="dlg-body"><div class="dlg-head"><span class="dlg-shield">⛨</span><b>${T('help.os.fw.head')}</b></div>
      <div class="dlg-grid"><span>${T('help.os.fw.name')}</span><b>Itda</b><span>${T('help.os.fw.publisher')}</span><span>${T('help.os.fw.unknown')}</span><span>${T('help.os.fw.path')}</span><span>F:\\…\\itda.exe</span></div>
      <p>${T('help.os.fw.allow')}</p>
      <label class="dlg-check hl">${T('help.os.fw.private')}${mark(1)}</label>
      <label class="dlg-check">${T('help.os.fw.public')}</label>
      <div class="dlg-foot"><span class="dlg-btn primary hl">${T('help.os.fw.allowAccess')}${mark(2)}</span><span class="dlg-btn">${T('common.cancel')}</span></div></div></div>`;
}

function winAllowPrompt() {
  return `<div class="os dlg small" role="img" aria-label="${T('help.os.ask.label')}">
    <div class="dlg-bar">${T('help.os.ask.title')}<span>✕</span></div>
    <div class="dlg-body"><div class="dlg-head"><span class="dlg-q">?</span><div>${T('help.os.ask.text')}</div></div>
      <div class="dlg-foot"><span class="dlg-btn primary hl">${T('help.os.ask.yes')}${mark(1)}</span><span class="dlg-btn">${T('help.os.ask.no')}</span></div></div></div>`;
}

function iosShare() {
  return `<div class="os ios" role="img" aria-label="${T('help.os.ios.label')}">
    <div class="ios-top"><span class="ios-share hl">${T('help.os.ios.share')}${mark(1)}</span></div>
    <div class="ios-row"><span class="ios-app">Itda</span><small>192.168.0.32</small></div>
    <div class="ios-list"><div>${T('help.os.ios.copy')}</div><div>${T('help.os.ios.readingList')}</div><div>${T('help.os.ios.bookmark')}</div><div class="hl">${T('help.os.ios.homeScreen')}${mark(2)}</div></div></div>`;
}

// ---------------------------------------------------------------- real screenshots

// English shots in web/help/en/ (scripts/site-capture.mjs --help-images); phone-view has no words.
const EN_SHOT = new Set(['itda-home', 'itda-nearby', 'itda-accounts', 'itda-account-edit', 'itda-edit', 'itda-share', 'itda-web', 'phone-login', 'phone-list', 'phone-keys']);
const shotSrc = (name) => `help/${I18N.lang === 'en' && EN_SHOT.has(name) ? 'en/' : ''}${name}.webp`;

function shot(name, caption, kind = 'desk') {
  const [w, h] = name === 'phone-list' ? [780, 1440] : kind === 'phone' ? [780, 1215] : [1400, 834];
  return `<figure class="shot ${kind}"><img ${name === 'phone-list' ? 'style="aspect-ratio:780 / 1440" ' : ''}src="${shotSrc(name)}" alt="${esc(caption)}" width="${w}" height="${h}" decoding="async"><figcaption>${caption}</figcaption></figure>`;
}

function step(n, title, body, figure = '') {
  return `<li class="hstep"><div class="hstep-n">${n}</div><div class="hstep-txt"><h3>${title}</h3>${body}</div>${figure ? `<div class="hstep-fig">${figure}</div>` : ''}</li>`;
}

const path = (...xs) => `<span class="path">${xs.map(esc).join(` ${I('E76C')} `)}</span>`;
const note = (t, kind = 'info') => `<p class="hnote ${kind}">${I(kind === 'warn' ? 'E7BA' : 'E946')}<span>${t}</span></p>`;
const hlink = (topic, label) => `<a href="#" data-help="${topic}">${label}</a>`;
const xlink = (url, label) => `<a href="${url}" data-ext>${label}</a>`;
// Places in the apps, for path(): System Settings, Settings and so on.
const P = (k) => T(`help.place.${k}`);

// ---------------------------------------------------------------- scenarios
// The help center is organised by what people want to do. Each scenario says what each
// device needs, then the steps (where to click), a "do it now" button, how to tell it
// worked, and what to do when it doesn't.

// On a Mac some scenarios are the other way round (this Mac is shared with macOS's own
// Screen Sharing; "Mac에서 이 PC 보기" becomes "PC에서 이 Mac 보기").
const TOPICS = [
  ['start', 'E80F', 'help.start.title', 'main'],
  ['mac', 'E7F4', 'help.topic.mac', 'main'],
  ['windows', 'E7F8', MAC ? 'help.topic.windows.mac' : 'help.topic.windows.pc', 'main'],
  ['linux', 'E756', 'help.topic.linux', 'main'],
  ['share', 'E72D', MAC ? 'help.topic.share.mac' : 'help.topic.share.pc', 'main'],
  ['phone', 'E8EA', MAC ? 'help.topic.phone.mac' : 'help.topic.phone.pc', 'main'],
  ['macpc', MAC ? 'E7F8' : 'E7F4', MAC ? 'help.topic.macpc.mac' : 'help.topic.macpc.pc', 'main'],
  ['assist', 'E8F2', 'help.topic.assist', 'main'],
  ['outside', 'E774', 'help.topic.outside', 'main'],
  ['accounts', 'E77B', 'help.topic.accounts', 'main'],
  ['settings', 'E713', 'help.topic.settings', 'ref'],
  ['words', 'E82D', 'help.topic.words', 'ref'],
  ['trouble', 'E90F', 'help.topic.trouble', 'ref'],
  ['keys', 'E765', 'help.topic.keys', 'ref'],
];

// Shorter labels for the topic list (the page title keeps the full sentence).
const NAV_SHORT = { start: 'help.nav.start', mac: 'help.nav.mac', windows: 'help.nav.windows', linux: 'help.nav.linux', share: MAC ? 'help.nav.share.mac' : 'help.nav.share.pc', phone: 'help.nav.phone', macpc: MAC ? 'help.nav.macpc.mac' : 'help.nav.macpc.pc', assist: 'help.nav.assist', outside: 'help.nav.outside', accounts: 'help.accounts.short' };

const doBtn = (label, attrs, primary = false) => `<button class="btn${primary ? ' primary' : ''}" ${attrs}>${label}</button>`;
const goPage = (page, label, primary) => doBtn(label, `data-act="page" data-page="${page}"`, primary);
const addKind = (os, label, primary) => doBtn(`${I('E710')}${label}`, `data-act="new-os" data-os="${os}"`, primary);

// Who needs what, for the "what you need" box.
const hereWho = () => (MAC ? T('help.who.here.mac') : T('help.who.here.pc'));
const hereIcon = () => (MAC ? 'E7F4' : 'E7F8');

/** One scenario page. */
function scenario({ lead, needs = [], actions = '', steps = [], check = '', trouble = [], more = '' }) {
  const need = needs.length
    ? `<div class="hneeds"><b>${I('E73E')} ${T('help.needs')}</b><ul>${needs.map(([icon, who, what]) => `<li>${I(icon)}<span><b>${who}</b> — ${what}</span></li>`).join('')}</ul></div>`
    : '';
  return `<p class="hlead">${lead}</p>${need}
    ${actions ? `<div class="hdo"><span>${I('E768')} ${T('help.doNow')}</span>${actions}</div>` : ''}
    <ol class="hsteps">${steps.map((s, i) => step(i + 1, ...s)).join('')}</ol>
    ${check ? `<div class="hcheck">${I('E73E')}<div><b>${T('help.check')}</b><p>${check}</p></div></div>` : ''}
    ${trouble.length ? `<h3 class="hsec">${T('help.trouble')}</h3><div class="faq">${trouble.map(([q, a]) => `<details><summary>${q}</summary><p>${a}</p></details>`).join('')}</div>` : ''}
    ${more}`;
}

function helpStart() {
  const tile = (t, icon, title, sub) => `<button class="htile" data-help="${t}">${I(icon)}<b>${title}</b><span>${sub}</span></button>`;
  return `<div class="hhero"><h2>${T('help.start.title')}</h2><p>${T('help.start.text')}</p></div>
    <h3 class="hsec">${MAC ? T('help.start.fromHere.mac') : T('help.start.fromHere.pc')}</h3>
    <div class="htiles">
      ${tile('mac', 'E7F4', T('help.topic.mac'), MAC ? T('help.start.mac.sub.mac') : T('help.start.mac.sub.pc'))}
      ${tile('windows', 'E7F8', MAC ? T('help.topic.windows.mac') : T('help.topic.windows.pc'), T('help.start.windows.sub'))}
      ${tile('linux', 'E756', T('help.topic.linux'), T('help.start.linux.sub'))}
    </div>
    <h3 class="hsec">${MAC ? T('help.start.toHere.mac') : T('help.start.toHere.pc')}</h3>
    <div class="htiles">
      ${MAC ? `${tile('share', 'E72D', T('help.topic.share.mac'), T('help.start.share.sub.mac'))}
      ${tile('phone', 'E8EA', T('help.topic.phone.mac'), T('help.start.phone.sub.mac'))}
      ${tile('macpc', 'E7F8', T('help.topic.macpc.mac'), T('help.start.macpc.sub.mac'))}` : `${tile('share', 'E72D', T('help.topic.share.pc'), T('help.start.share.sub.pc'))}
      ${tile('phone', 'E8EA', T('help.topic.phone.pc'), T('help.start.phone.sub.pc'))}
      ${tile('macpc', 'E7F4', T('help.topic.macpc.pc'), T('help.start.macpc.sub.pc'))}`}
      ${tile('assist', 'E8F2', T('help.topic.assist'), T('help.start.assist.sub'))}
    </div>
    <h3 class="hsec">${T('help.start.more')}</h3>
    <div class="htiles">
      ${tile('outside', 'E774', T('help.topic.outside'), T('help.start.outside.sub'))}
      ${tile('accounts', 'E77B', T('help.accounts.short'), T('help.start.accounts.sub'))}
      ${tile('settings', 'E713', T('help.topic.settings'), T('help.start.settings.sub'))}
    </div>
    ${shot('itda-home', T('help.start.shot'))}`;
}

// The optional high-performance and sleep sections of the Mac pages.
const hpMore = () => `<h3 class="hsec">${T('help.hp.title')}</h3><p>${T('help.hp.p1')}</p><p>${T('help.hp.p2')}</p><h3 class="hsec">${T('help.sleep.title')}</h3>${macWindow('energy', T('help.os.mac.energy'), macEnergy())}`;

function helpMac() {
  return scenario({
    lead: MAC ? T('help.mac.lead.mac') : T('help.mac.lead.pc'),
    needs: [['E7F4', MAC ? T('help.who.otherMac') : 'Mac', T('help.mac.need.mac')], [hereIcon(), hereWho(), 'Itda'], ['E701', T('help.who.network'), T('help.need.wifi', { ts: hlink('outside', 'Tailscale') })]],
    actions: goPage('nearby', `${I('E701')}${T('onboard.mac.nearby')}`, true) + addKind('mac', T('onboard.mac.manual')),
    steps: [
      [T('help.mac.s1.t'), `<p>${T('help.mac.s1.p', { path: path(P('appleMenu'), P('systemSettings'), P('general'), P('sharing')), m1: mark(1) })}</p>
        ${note(T('help.note.remoteMgmt'), 'info')}`, macWindow('general', T('help.os.mac.sharing'), macSharing('screen'))],
      [T('help.mac.s2.t'), `<p>${T('help.mac.s2.p', { m1: mark(1), m2: mark(2) })}</p>`, macWindow('general', T('help.os.mac.sharing'), macSharing('screen-i'), macScreenSheet())],
      [MAC ? T('help.mac.s3.t.mac') : T('help.mac.s3.t.pc'), `<p>${T('help.mac.s3.p1')}</p>
        <p>${T('help.mac.s3.p2')}</p>`, shot('itda-nearby', T('help.shot.nearby'))],
      [T('help.mac.s4.t'), `<p>${T('help.mac.s4.p')}</p>
        ${note(T('help.mac.s4.note'))}`, shot('itda-edit', T('help.shot.edit.mac'))],
    ],
    check: MAC ? T('help.mac.check.mac') : T('help.mac.check.pc'),
    trouble: [
      [T('help.q.macRefused'), T('help.mac.t1.a')],
      [T('help.q.loginFailed'), T('help.mac.t2.a')],
      [T('help.mac.t3.q'), T('help.mac.t3.a')],
      [T('help.mac.t4.q'), T('help.mac.t4.a', { path: path(P('systemSettings'), P('energy')) })],
      MAC ? [T('help.mac.t5.q.mac'), T('help.mac.t5.a.mac')] : [T('help.mac.t5.q.pc'), T('help.mac.t5.a.pc', { more: hlink('settings#defaults', T('help.topic.settings')) })],
    ],
    more: hpMore(),
  });
}

function helpWindows() {
  return scenario({
    lead: MAC ? T('help.win.lead.mac') : T('help.win.lead.pc'),
    needs: [['E7F8', T('help.who.otherPc'), T('help.win.need.pc')], [hereIcon(), hereWho(), 'Itda'], ['E701', T('help.who.network'), T('help.need.wifi', { ts: 'Tailscale' })]],
    actions: addKind('windows', T('onboard.win.a.add'), true) + addKind('rdp', T('onboard.win.b.add')),
    steps: [
      [T('help.win.a1.t'), `<p>${T('help.win.a1.p')}</p>
        <p>${T('help.win.a1.firewall', { m1: mark(1), m2: mark(2) })}</p>`, shot('itda-share', T('help.shot.shareOther')) + winFirewall()],
      [MAC ? T('help.win.a2.t.mac') : T('help.win.a2.t.pc'), `<p>${MAC ? T('help.win.a2.p1.mac') : T('help.win.a2.p1.pc')}</p>
        <p>${T('help.win.a2.p2', { m1: mark(1) })}</p>`, winAllowPrompt()],
      [T('help.win.b1.t'), `<p>${T('help.win.b1.p', { path: path(P('settings'), P('system'), P('remoteDesktop')), m1: mark(1), m2: mark(2) })}</p>
        ${note(T('help.win.b1.note'), 'warn')}`, winRemoteDesktop()],
      [MAC ? T('help.win.b2.t.mac') : T('help.win.b2.t.pc'), `<p>${T('help.win.b2.p')}</p>
        <ul class="hkeys"><li>${T('help.win.b2.msa')}</li>
          <li>${T('help.win.b2.local')}</li><li>${T('help.win.b2.work')}</li></ul>
        ${note(T('help.win.b2.note'), 'warn')}
        <p>${MAC ? T('help.win.b2.opens.mac') : T('help.win.b2.opens.pc')}</p>`, shot('itda-edit', T('help.shot.edit.win'))],
    ],
    check: MAC ? T('help.win.check.mac') : T('help.win.check.pc'),
    trouble: [
      [T('help.q.refused'), T('help.win.t1.a')],
      [T('help.q.loginFailed'), T('help.win.t2.a')],
      [T('help.win.t3.q'), T('help.win.t3.a')],
      [T('help.win.t4.q'), T('help.win.t4.a')],
      [T('help.win.t5.q'), MAC ? T('help.win.t5.a.mac') : T('help.win.t5.a.pc')],
    ],
  });
}

function helpLinux() {
  return scenario({
    lead: T('help.linux.lead'),
    needs: [['E756', T('help.who.otherDevice'), T('help.linux.need')], [hereIcon(), hereWho(), 'Itda']],
    actions: addKind('linux', T('help.linux.add'), true) + goPage('nearby', `${I('E701')}${T('help.linux.nearby')}`),
    steps: [
      [T('help.linux.s1.t'), `<table class="shortcuts">
          <tr><td>${T('help.linux.s1.pi')}</td><td>${T('help.linux.s1.pi.how')}</td></tr>
          <tr><td>Ubuntu (GNOME)</td><td>${T('help.linux.s1.ubuntu.how')}</td></tr>
          <tr><td>${T('help.linux.s1.other')}</td><td>${T('help.linux.s1.other.how')}</td></tr></table>
        <p>${T('help.linux.s1.port')}</p>`],
      [MAC ? T('help.linux.s2.t.mac') : T('help.linux.s2.t.pc'), `<p>${T('help.linux.s2.p')}</p>`, shot('itda-edit', T('help.shot.edit.linux'))],
      [T('common.connect'), `<p>${T('help.linux.s3.p')}</p>`],
    ],
    check: T('help.linux.check'),
    trouble: [
      [T('help.q.refused'), T('help.linux.t1.a')],
      [T('help.q.loginFailed'), T('help.linux.t2.a')],
      [T('help.linux.t3.q'), T('help.linux.t3.a')],
    ],
  });
}

function helpShare() {
  return MAC ? helpShareMac() : scenario({
    lead: T('help.share.lead'),
    needs: [['E7F8', T('help.who.here.pc'), T('help.share.need.here')], ['E8EA', T('help.who.viewer'), T('help.share.need.viewer')], ['E701', T('help.who.network'), T('help.need.wifi', { ts: 'Tailscale' })]],
    actions: doBtn(`${I('E768')}${T('help.share.allOn')}`, 'data-act="share-all-on"', true) + goPage('share', T('help.openSharePage')),
    steps: [
      [T('help.share.s1.t'), `<p>${T('help.share.s1.p')}</p>`, shot('itda-web', T('help.shot.sharePage'))],
      [T('help.share.s2.t'), `<p>${T('help.share.s2.p', { m1: mark(1), m2: mark(2) })}</p>`, winFirewall()],
      [T('help.share.s3.t'), `<ul class="hkeys">
          <li>${T('help.share.s3.itda', { more: hlink('windows', T('common.learnMore')) })}</li>
          <li>${T('help.share.s3.phone', { more: hlink('phone', T('common.learnMore')) })}</li>
          <li>${T('help.share.s3.mac', { more: hlink('macpc', T('common.learnMore')) })}</li></ul>`],
      [T('help.share.s4.t'), `<p>${T('help.share.s4.p1', { m1: mark(1) })}</p>
        <p>${T('help.share.s4.p2')}</p>`, winAllowPrompt()],
    ],
    check: T('help.share.check'),
    trouble: [
      [T('help.share.t1.q'), T('help.share.t1.a')],
      [T('help.share.t2.q'), T('help.share.t2.a')],
      [T('help.share.t3.q'), T('help.share.t3.a')],
      [T('help.q.whenDone'), T('help.share.t4.a')],
    ],
  });
}

function helpPhone() {
  return MAC ? helpPhoneMac() : scenario({
    lead: T('help.phone.lead.pc'),
    needs: [['E7F8', T('help.who.here.pc'), T('help.phone.need.here')], ['E8EA', T('help.who.phone'), T('help.phone.need.phone')], ['E701', T('help.who.network'), T('help.phone.need.network')]],
    actions: doBtn(`${I('E768')}${T('help.phone.allOn')}`, 'data-act="share-all-on"', true),
    steps: [
      [T('help.phone.s1.t.pc'), `<p>${T('help.phone.s1.p.pc')}</p>`, shot('itda-web', T('help.shot.web'))],
      [T('help.phone.qr.t'), `<p>${T('help.phone.qr.p')}</p>`, shot('phone-login', T('help.shot.pin'), 'phone')],
      [T('help.phone.pick.t'), `<p>${T('help.phone.pick.p.pc')}</p>`, shot('phone-list', T('help.shot.list.pc'), 'phone')],
      [T('help.phone.touch.t'), `<ul class="hkeys"><li>${T('help.phone.touch.tap')}</li><li>${T('help.phone.touch.pinch')}</li>
          <li>${T('help.phone.touch.toolbar')}</li></ul>`, shot('phone-view', T('help.shot.view'), 'phone') + shot('phone-keys', T('help.shot.keys'), 'phone')],
      [T('help.phone.home.t'), `<p>${T('help.phone.home.p.pc', { m1: mark(1), m2: mark(2) })}</p>`, iosShare()],
    ],
    check: T('help.phone.check.pc'),
    trouble: [
      [T('help.q.qr'), T('help.phone.t1.a.pc')],
      [T('help.q.pin'), T('help.a.pin')],
      [T('help.phone.t3.q.pc'), T('help.phone.t3.a.pc')],
      [T('help.phone.t4.q'), T('help.phone.t4.a')],
    ],
  });
}

function helpMacPc() {
  return MAC ? helpPcMac() : scenario({
    lead: T('help.macpc.lead'),
    needs: [['E7F8', T('help.who.here.pc'), T('help.macpc.need.here')], ['E7F4', 'Mac', T('help.macpc.need.mac')]],
    actions: doBtn(`${I('E768')}${T('share.hero.pc.allOn')}`, 'data-act="share-all-on"', true) + goPage('share', T('help.openSharePage')),
    steps: [
      [T('help.macpc.s1.t'), `<p>${T('help.macpc.s1.p')}</p>`, shot('itda-web', T('help.shot.sharePageOnly'))],
      [T('help.macpc.s2.t'), `<p>${T('help.macpc.s2.p1')}</p>
        <p>${T('help.macpc.s2.p2', { path: path('Safari', P('file'), P('addToDock')) })}</p>`],
      [T('help.macpc.s3.t'), `<p>${T('help.macpc.s3.p', { link: xlink('https://github.com/jiunbae/itda/releases/latest', T('help.macpc.s3.link')), path: path(P('systemSettings'), P('privacy')) })}</p>`],
      [T('help.macpc.s4.t'), `<p>${T('help.macpc.s4.p1', { path: path(P('go'), P('connectServer')) })}</p>
        <p>${T('help.macpc.s4.p2')}</p>`],
    ],
    check: T('help.macpc.check'),
    trouble: [
      [T('help.macpc.t1.q'), T('help.macpc.t1.a')],
      [T('help.macpc.t2.q'), T('help.macpc.t2.a')],
      [T('help.macpc.t3.q'), T('help.macpc.t3.a')],
    ],
  });
}

// ---- Itda for Mac: this Mac is shared with macOS's own Screen Sharing.

function helpShareMac() {
  return scenario({
    lead: T('help.shareMac.lead'),
    needs: [['E7F4', T('help.who.here.mac'), T('help.shareMac.need.here')], ['E7F8', T('help.who.viewer'), T('help.shareMac.need.viewer')], ['E701', T('help.who.network'), T('help.need.wifi', { ts: hlink('outside', 'Tailscale') })]],
    actions: doBtn(`${I('E8A7')}${T('share.mac.ss.open')}`, 'data-act="sys-sharing"', true) + goPage('share', T('help.openSharePage')),
    steps: [
      [T('help.shareMac.s1.t'), `<p>${T('help.shareMac.s1.p', { path: path(P('appleMenu'), P('systemSettings'), P('general'), P('sharing')), m1: mark(1) })}</p>
        ${note(T('help.note.remoteMgmt'), 'info')}`, macWindow('general', T('help.os.mac.sharing'), macSharing('screen'))],
      [T('help.shareMac.s2.t'), `<p>${T('help.shareMac.s2.p', { m1: mark(1), m2: mark(2) })}</p>`, macWindow('general', T('help.os.mac.sharing'), macSharing('screen-i'), macScreenSheet())],
      [T('help.shareMac.s3.t'), `<p>${T('help.shareMac.s3.p', { m3: mark(3) })}</p>`, macWindow('general', T('help.os.mac.sharing'), macSharing('name'))],
      [T('help.shareMac.s4.t'), `<ul class="hkeys">
          <li>${T('help.shareMac.s4.pc', { more: hlink('macpc', T('common.learnMore')) })}</li>
          <li>${T('help.shareMac.s4.mac', { path: path(P('go'), P('connectServer')) })}</li>
          <li>${T('help.shareMac.s4.phone')}</li></ul>`],
    ],
    check: T('help.shareMac.check'),
    trouble: [
      [T('help.shareMac.t1.q'), T('help.shareMac.t1.a')],
      [T('help.q.loginFailed'), T('help.shareMac.t2.a')],
      [T('help.shareMac.t3.q'), T('help.shareMac.t3.a', { path: path(P('systemSettings'), P('energy')) })],
      [T('help.q.whenDone'), T('help.shareMac.t4.a')],
    ],
    more: hpMore(),
  });
}

function helpPhoneMac() {
  return scenario({
    lead: T('help.phone.lead.mac'),
    needs: [['E7F4', T('help.who.here.mac'), T('help.phone.need.here')], ['E8EA', T('help.who.phone'), T('help.phone.need.phone')], ['E701', T('help.who.network'), T('help.phone.need.network')]],
    actions: doBtn(`${I('E768')}${T('help.phone.webOn')}`, 'data-act="web-on"', true),
    steps: [
      [T('help.phone.s1.t.mac'), `<p>${T('help.phone.s1.p.mac')}</p>`, shot('itda-web', T('help.shot.web'))],
      [T('help.phone.qr.t'), `<p>${T('help.phone.qr.p')}</p>`, shot('phone-login', T('help.shot.pin'), 'phone')],
      [T('help.phone.pick.t'), `<p>${T('help.phone.pick.p.mac')}</p>`, shot('phone-list', T('help.shot.list.mac'), 'phone')],
      [T('help.phone.touch.t'), `<ul class="hkeys"><li>${T('help.phone.touch.tap')}</li><li>${T('help.phone.touch.pinch')}</li>
          <li>${T('help.phone.touch.toolbar')}</li></ul>`, shot('phone-view', T('help.shot.view'), 'phone')],
      [T('help.phone.home.t'), `<p>${T('help.phone.home.p.mac', { m1: mark(1), m2: mark(2) })}</p>`, iosShare()],
    ],
    check: T('help.phone.check.mac'),
    trouble: [
      [T('help.q.qr'), T('help.phone.t1.a.mac')],
      [T('help.q.pin'), T('help.a.pin')],
      [T('help.phone.t3.q.mac'), T('help.phone.t3.a.mac', { link: hlink('share', T('help.topic.share.mac')) })],
      [T('help.phone.t5.q'), T('help.phone.t5.a', { path: path(P('systemSettings'), P('energy')) })],
    ],
  });
}

function helpPcMac() {
  return scenario({
    lead: T('help.pcmac.lead'),
    needs: [['E7F4', T('help.who.here.mac'), T('help.pcmac.need.here', { link: hlink('share', T('help.topic.share.mac')) })], ['E7F8', 'Windows PC', T('help.pcmac.need.pc')]],
    actions: doBtn(`${I('E8A7')}${T('share.mac.ss.open')}`, 'data-act="sys-sharing"', true) + goPage('share', T('help.pcmac.address')),
    steps: [
      [T('help.pcmac.s1.t'), `<p>${T('help.pcmac.s1.p', { path: path(P('systemSettings'), P('general'), P('sharing')) })}</p>`, macWindow('general', T('help.os.mac.sharing'), macSharing('screen'))],
      [T('help.pcmac.s2.t'), `<p>${T('help.pcmac.s2.p', { link: xlink('https://github.com/jiunbae/itda/releases/latest', 'Itda for Windows') })}</p>`],
      [T('help.pcmac.s3.t'), `<p>${T('help.pcmac.s3.p')}</p>`, shot('itda-nearby', T('help.shot.nearbyPc'))],
    ],
    check: T('help.pcmac.check'),
    trouble: [
      [T('help.pcmac.t1.q'), T('help.pcmac.t1.a')],
      [T('help.pcmac.t2.q'), T('help.pcmac.t2.a')],
    ],
  });
}

function helpAssist() {
  return scenario({
    lead: T('help.assist.lead'),
    needs: [['E7F8', T('help.assist.who.them'), T('help.assist.need.them')], [hereIcon(), T('help.assist.who.me'), MAC ? T('help.assist.need.me.mac') : T('help.assist.need.me.pc')], ['E774', T('help.assist.who.far'), T('help.assist.need.far')]],
    steps: [
      [T('help.assist.s1.t'), `<div class="hscript"><b>${T('help.assist.s1.label')}</b><p>${T('help.assist.s1.script')}</p></div>`],
      [T('help.assist.s2.t'), `<p>${T('help.assist.s2.p')}</p>` + addKind('windows', T('onboard.win.a.add'), true)],
      [T('help.assist.s3.t'), `<p>${T('help.assist.s3.p')}</p>`, winAllowPrompt()],
      [T('help.assist.s4.t'), `<p>${T('help.assist.s4.p')}</p>`],
    ],
    check: T('help.assist.check'),
    trouble: [
      [T('help.assist.t1.q'), T('help.assist.t1.a', { link: hlink('outside', T('help.topic.outside')) })],
      [T('help.assist.t2.q'), T('help.assist.t2.a')],
    ],
  });
}

function helpOutside() {
  return scenario({
    lead: T('help.outside.lead'),
    needs: [['E774', T('help.outside.who'), T('help.outside.need')]],
    actions: goPage('nearby', `${I('E701')}${T('help.outside.nearby')}`, true),
    steps: [
      [T('help.outside.s1.t'), `<p>${MAC ? T('help.outside.s1.p.mac', { link: xlink('https://tailscale.com/download', 'Tailscale') }) : T('help.outside.s1.p.pc', { link: xlink('https://tailscale.com/download', 'Tailscale') })}</p>`],
      [T('help.outside.s2.t'), `<p>${T('help.outside.s2.p')}</p>`, shot('itda-nearby', T('help.shot.ts'))],
      [MAC ? T('help.topic.phone.mac') : T('help.outside.s3.t.pc'), `<p>${T('help.outside.s3.p')}</p>`, shot('itda-web', T('help.shot.webTs'))],
    ],
    check: T('help.outside.check'),
    trouble: [
      [T('help.outside.t1.q'), T('help.outside.t1.a')],
      [T('help.outside.t2.q'), T('help.outside.t2.a', { link: xlink('https://login.tailscale.com/admin/dns', T('help.outside.t2.link')) })],
      [T('help.outside.t3.q'), T('help.outside.t3.a')],
    ],
  });
}

function helpAccounts() {
  return scenario({
    lead: T('help.accounts.lead'),
    actions: goPage('accounts', `${I('E77B')}${T('help.accounts.open')}`, true),
    steps: [
      [T('help.accounts.s1.t'), `<p>${T('help.accounts.s1.p')}</p>`, shot('itda-account-edit', T('help.shot.accountEdit'))],
      [T('help.accounts.s2.t'), `<p>${T('help.accounts.s2.p')}</p>`, shot('itda-edit', T('help.shot.edit.login'))],
      [T('help.accounts.s3.t'), `<p>${T('help.accounts.s3.p')}</p>`, shot('itda-accounts', T('help.shot.accounts'))],
      [T('help.accounts.s4.t'), `<p>${MAC ? T('help.accounts.s4.p.mac', { m1: mark(1) }) : T('help.accounts.s4.p.pc', { m1: mark(1) })}</p>`, macWindow('general', T('help.os.mac.sharing'), macSharing('ssh'))],
    ],
  });
}

// [id, related topic]: the question and answer are help.faq.<id>.q / .a.
const FAQ = [
  ['refused', 'mac'],
  ['login', 'mac'],
  ['noResponse', 'outside'],
  ['nearbyEmpty', 'start'],
  ['hangul', 'keys'],
  ['copyPaste', 'keys'],
  ['webPhone', 'phone'],
  ['rdpAccount', 'windows'],
  ['noControl', 'windows'],
  ['loginItems', 'settings'],
];
const MAC_ONLY = ['loginItems'];

// Keyboard options of the Windows viewer; the Mac's windows send keys as they are.
const PC_ONLY = ['hangul', 'copyPaste'];

const faqQ = (id) => ({ refused: T('help.q.macRefused'), login: T('help.q.loginFailed') })[id] || T(`help.faq.${id}.q`);
const faqA = (id) => (id === 'webPhone' ? (MAC ? T('help.faq.webPhone.a.mac') : T('help.faq.webPhone.a.pc')) : T(`help.faq.${id}.a`));

function helpTrouble() {
  return `<div class="faq">${FAQ.filter(([id]) => !(MAC ? PC_ONLY : MAC_ONLY).includes(id)).map(([id, t]) => `<details><summary>${faqQ(id)}</summary><p>${faqA(id)}</p><a href="#" data-help="${t}">${T('help.faq.related')}</a></details>`).join('')}</div>
    <p class="hnote info">${I('E946')}<span>${T('help.faq.log', { path: path(P('settings'), P('about'), P('log')) })}</span></p>`;
}

function helpKeysMac() {
  const row = (a, k) => `<tr><td>${a}</td><td><kbd>${k}</kbd></td></tr>`;
  return `<p class="hlead">${T('help.keys.lead.mac')}</p>
    <h3 class="hsec">${T('help.keys.home')}</h3>
    <table class="shortcuts">${row(T('help.keys.new'), '⌘N')}${row(T('help.keys.search'), '⌘F')}${row(T('nav.settings'), '⌘,')}${row(T('settings.zoom'), '⌘+ / ⌘− / ⌘0')}${row(T('help.keys.showHome'), '⌘1')}${row(T('help.keys.quit'), '⌘Q')}</table>
    <h3 class="hsec">${T('help.keys.toolbar')}</h3>
    <table class="shortcuts"><tr><td>${T('help.keys.special')}</td><td>Mac: ⌘ Space, ⌘ Tab, ⌥⌘ Esc, ⌃⌘ Q … · Windows: Ctrl Alt Del, Alt Tab, ⊞ …</td></tr>
      <tr><td>${T('help.keys.fit')}</td><td>${T('help.keys.fit.desc')}</td></tr><tr><td>${T('settings.keys.viewOnly')}</td><td>${T('help.keys.viewOnly.desc')}</td></tr><tr><td>${T('settings.keys.fullscreen')}</td><td>${T('help.keys.fullscreen.desc')}</td></tr></table>
    <h3 class="hsec">${T('help.keys.windows')}</h3>
    <table class="shortcuts"><tr><td>⌘ Command</td><td><kbd>${T('help.keys.winKey')}</kbd></td></tr><tr><td>⌥ Option</td><td><kbd>Alt</kbd></td></tr><tr><td>⌃ Control</td><td><kbd>Ctrl</kbd> ${T('help.keys.ctrlCopy')}</td></tr></table>`;
}

function helpKeys() {
  if (MAC) return helpKeysMac();
  const p = hk();
  const rows = [['settings.keys.fullscreen', 'Enter'], ['settings.keys.fit', '0 / 1'], ['settings.keys.stats', 'S'], ['settings.keys.viewOnly', 'V'], ['settings.keys.screenshot', 'P'], ['settings.keys.info', 'I'], ['settings.keys.pinToolbar', 'T'], ['settings.keys.reconnect', 'R'], ['settings.keys.menu', 'M']];
  return `<p class="hlead">${T('help.keys.lead.pc')}</p>
    <table class="shortcuts">${rows.map(([a, k]) => `<tr><td>${T(a)}</td><td><kbd>${p}+${k}</kbd></td></tr>`).join('')}</table>
    <h3 class="hsec">${T('help.keys.macKeys')}</h3>
    <table class="shortcuts"><tr><td>${T('help.keys.winKeyName')}</td><td><kbd>⌘ Command</kbd></td></tr><tr><td>Alt</td><td><kbd>⌥ Option</kbd></td></tr><tr><td>Ctrl</td><td><kbd>⌃ Control</kbd> ${T('help.keys.ctrlSwap')}</td></tr><tr><td>${T('help.keys.hangul')}</td><td>${T('help.keys.hangul.keys')}</td></tr></table>`;
}

// One setting explained for someone who isn't technical: what it is, when to change it,
// and what we recommend.
const hset = (name, what, when, rec) => `<div class="hset"><div class="hset-head"><b>${name}</b>${rec ? `<span class="hset-rec">${rec}</span>` : ''}</div>
  <p>${what}</p>${when ? `<p class="hset-when">${I('E946')}<span>${when}</span></p>` : ''}</div>`;
const hgroup = (id, icon, title, where, body) => `<section class="hgroup" id="hs-${id}"><h3 class="hsec">${I(icon)} ${title}</h3>
  ${where ? `<p class="hwhere">${T('help.set.where', { where })}</p>` : ''}${body}</section>`;

function helpSettings() {
  const jump = [['look', T('settings.group.look')], ...(MAC ? [] : [['session', T('settings.group.session')]]), ['defaults', T('settings.group.defaults')], ['edit', T('help.set.jump.edit')], ['share', T('nav.share')], ['app', T('settings.group.app')]]
    .map(([k, t]) => `<a href="#" class="hjump" data-help="settings#${k}">${t}</a>`).join('');
  const on = T('help.rec.on');
  return `<p class="hlead">${T('help.set.lead')}</p>
    <div class="hjumps">${jump}</div>
    ${hgroup('look', 'E790', T('settings.group.look'), path(P('settings'), P('look')), `
      ${hset(T('settings.theme'), MAC ? T('help.set.theme.mac') : T('help.set.theme.pc'), T('help.set.theme.when'), T('help.set.theme.rec'))}
      ${hset(T('settings.view'), T('help.set.view'), T('help.set.view.when'), T('help.set.view.rec'))}
      ${hset(T('settings.zoom'), MAC ? T('help.set.zoom.mac') : T('help.set.zoom.pc'), T('help.set.zoom.when'), T('help.set.zoom.rec'))}`)}
    ${MAC ? '' : hgroup('session', 'E7C4', T('settings.group.session'), path(P('settings'), P('session')), `
      ${hset(T('settings.toolbar'), T('help.set.toolbar'), T('help.set.toolbar.when'), T('help.set.toolbar.rec'))}
      ${hset(T('settings.hotkey'), T('help.set.hotkey'), T('help.set.hotkey.when'), T('help.set.hotkey.rec'))}
      ${hset(T('settings.macScroll.pc'), T('help.set.macScroll'), T('help.set.macScroll.when'), T('help.set.macScroll.rec'))}
      ${hset(T('settings.scrollAccel'), T('help.set.accel'), T('help.set.accel.when'), T('help.set.accel.rec'))}`)}
    ${hgroup('defaults', 'E9A6', T('settings.group.defaults'), path(P('settings'), P('defaults')), `
      <p class="hwhere">${T('help.set.defaults.note')}</p>
      ${hset(T('settings.scale'), T('help.set.scale'), T('help.set.scale.when'), T('help.set.scale.rec'))}
      ${hset(T('settings.clipboard'), MAC ? T('help.set.clipboard.mac') : T('help.set.clipboard.pc'), T('help.set.clipboard.when'), on)}
      ${hset(T('settings.reconnect'), T('help.set.reconnect'), '', on)}
      ${MAC ? '' : hset(T('settings.swap'), T('help.set.swap'), T('help.set.swap.when'), T('help.set.swap.rec'))}
      ${MAC ? '' : hset(T('settings.hangulKey'), T('help.set.hangul'), T('help.set.hangul.when'), T('help.set.hangul.rec'))}`)}
    ${hgroup('edit', 'E70F', T('help.set.edit.title'), T('help.set.edit.where'), `
      ${hset(T('editor.kind'), T('help.set.kind'), '')}
      ${hset(T('editor.host'), T('help.set.host'), '', '')}
      ${hset(T('help.set.login'), T('help.set.login.what'), '', '')}
      ${hset(T('common.savePassword'), MAC ? T('help.set.savePw.mac') : T('help.set.savePw.pc'), T('help.set.savePw.when'), on)}
      ${hset(T('editor.ssh'), T('help.set.ssh'), T('help.set.ssh.when'), T('help.set.ssh.rec'))}
      ${hset(T('help.set.display'), MAC ? T('help.set.display.what.mac') : T('help.set.display.what.pc'), T('help.set.display.when'), '')}
      ${MAC ? hset(T('help.set.connTab'), T('help.set.connTab.what'), '', '') : hset(T('help.set.kbConnTab'), T('help.set.kbConnTab.what'), '', '')}`)}
    ${hgroup('share', 'E72D', T('help.set.share.title'), path(P('sidebar'), P('share')), `
      ${hset(T('share.web.title'), MAC ? T('help.set.web.mac') : T('help.set.web.pc'), T('help.set.web.when'), '')}
      ${hset(T('help.set.pin'), T('help.set.pin.what'), T('help.set.pin.when'), '')}
      ${hset(T('help.set.https'), T('help.set.https.what'), T('help.set.https.when'), '')}
      ${MAC ? hset(T('help.set.macShare'), T('help.set.macShare.what'), T('help.set.offWhenUnused'), '')
        + hset(T('settings.announce.mac'), T('help.set.announce.mac'), T('help.set.announce.when.mac'), on)
        + hset(T('share.port'), T('help.set.port.mac'), T('help.set.port.when.mac'), T('help.rec.keep')) : `
      ${hset(T('share.pc.title'), T('help.set.pcShare'), T('help.set.offWhenUnused'), '')}
      ${hset(T('share.vnc.password'), T('help.set.sharePw'), '', '')}
      ${hset(T('share.vnc.ask'), T('help.set.ask'), T('help.set.ask.when'), on)}
      ${hset(T('share.vnc.trustNear'), T('help.set.trust'), T('help.set.trust.when'), T('help.set.trust.rec'))}
      ${hset(T('share.vnc.cmdAsCtrl'), T('help.set.cmdAsCtrl'), '', on)}
      ${hset(T('share.vnc.viewOnly'), T('help.set.viewOnly'), T('help.set.viewOnly.when'), '')}
      ${hset(T('settings.announce.pc'), T('help.set.announce.pc'), T('help.set.announce.when.pc'), on)}
      ${hset(T('share.port'), T('help.set.port.pc'), T('help.set.port.when.pc'), T('help.rec.keep'))}`}`)}
    ${hgroup('app', 'E8A5', T('settings.group.app'), path(P('settings'), P('app')), `
      ${MAC ? hset(T('help.set.keepRunning.mac'), T('help.set.keepRunning.mac.what'), T('help.set.keepRunning.mac.when'), '') : hset(T('settings.tray'), T('help.set.tray'), T('help.set.tray.when'), on)}
      ${hset(T('settings.notify'), MAC ? T('help.set.notify.mac') : T('help.set.notify.pc'), '', on)}
      ${hset(T('settings.thumbnails'), MAC ? T('help.set.thumbs.mac') : T('help.set.thumbs.pc'), T('help.set.thumbs.when'), on)}
      ${hset(T('settings.login'), T('help.set.login.app'), T('help.set.login.app.when'), '')}
      ${hset(T('settings.remoteSetup'), T('help.set.remoteSetup'), T('help.set.remoteSetup.when'), '')}
      ${hset(T('help.set.guideLog'), T('help.set.guideLog.what'), T('help.set.guideLog.when'), '')}`)}`;
}

function helpWords() {
  const w = (term, desc) => `<div class="hword"><b>${term}</b><p>${desc}</p></div>`;
  return `<p class="hlead">${T('help.words.lead')}</p><div class="hwords">
    ${w(T('help.words.address'), T('help.words.address.desc'))}
    ${w(T('share.port'), T('help.words.port.desc'))}
    ${w(T('help.words.wifi'), T('help.words.wifi.desc'))}
    ${w(T('help.words.vnc'), T('help.words.vnc.desc'))}
    ${w(T('help.words.rdp'), T('help.words.rdp.desc'))}
    ${w(T('help.words.ssh'), T('help.words.ssh.desc'))}
    ${w('Tailscale', T('help.words.tailscale.desc'))}
    ${w(T('share.web.title'), MAC ? T('help.words.web.desc.mac') : T('help.words.web.desc.pc'))}
    ${w('PIN', MAC ? T('help.words.pin.desc.mac') : T('help.words.pin.desc.pc'))}
    ${MAC ? w(T('help.words.firewall'), T('help.words.firewall.mac')) : w(T('help.words.firewall'), T('help.words.firewall.pc'))}
    ${MAC ? w(T('help.words.keychain'), T('help.words.keychain.desc')) : w(T('help.words.credman'), T('help.words.credman.desc'))}
    ${w(T('help.words.msa'), T('help.words.msa.desc'))}
    ${w(T('help.words.shortName'), T('help.words.shortName.desc'))}
    ${MAC ? w('Dock', T('help.words.dock.desc')) : w(T('help.words.tray'), T('help.words.tray.desc'))}
    ${w(T('help.words.pwa'), T('help.words.pwa.desc'))}
  </div>`;
}

function renderHelp() {
  const t = S.helpTopic || 'start';
  const body = { start: helpStart, mac: helpMac, windows: helpWindows, linux: helpLinux, share: helpShare, phone: helpPhone, macpc: helpMacPc, assist: helpAssist, outside: helpOutside, accounts: helpAccounts, settings: helpSettings, words: helpWords, trouble: helpTrouble, keys: helpKeys }[t] || helpStart;
  const title = T((TOPICS.find(([k]) => k === t) || TOPICS[0])[2]);
  return `<div class="help">
    <nav class="hnav" aria-label="${T('help.nav.label')}">${TOPICS.map(([k, icon, label, sec], i) => `${sec === 'ref' && TOPICS[i - 1][3] !== 'ref' ? `<div class="hnav-sec">${T('help.nav.ref')}</div>` : ''}<button class="hnav-item${k === t ? ' on' : ''}" data-help="${k}">${I(icon)}<span>${T(NAV_SHORT[k] || label)}</span></button>`).join('')}</nav>
    <article class="hbody">${t === 'start' ? '' : `<h2 class="htitle">${title}</h2>`}${body()}</article></div>`;
}

/** Open the help center at a topic (from anywhere: settings, errors, empty states). */
function helpGo(target) {
  const [topic, part] = target.split('#');
  closeModal();
  S.helpTopic = topic;
  if (S.page !== 'help') go('help');
  else render();
  const c = document.getElementById('content');
  if (c) c.scrollTop = 0;
  const el = part && document.getElementById(`hs-${part}`);
  if (el) el.scrollIntoView({ block: 'start' });
}

document.addEventListener('click', (e) => {
  const h = e.target.closest('[data-help]');
  if (!h) return;
  e.preventDefault();
  helpGo(h.dataset.help);
});
