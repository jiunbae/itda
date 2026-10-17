// Itda help center: step-by-step guides with real Itda screenshots and drawn
// macOS / Windows settings screens (crisp, theme-aware, highlights where to click).
'use strict';

// ---------------------------------------------------------------- drawn OS screens

const MAC_ICON = {
  general: ['#8e8e93', '⚙'], share: ['#8e8e93', '⇪'], users: ['#8e8e93', '👥'], energy: ['#ffcc00', '⚡'], battery: ['#34c759', '▮'],
  wifi: ['#0a84ff', '◠'], network: ['#0a84ff', '◍'], privacy: ['#0a84ff', '✋'], display: ['#0a84ff', '▭'], lock: ['#8e8e93', '🔒'],
};

function macSidebar(active) {
  const items = [['wifi', 'Wi-Fi'], ['network', '네트워크'], ['general', '일반'], ['display', '디스플레이'], ['energy', '에너지'], ['battery', '배터리'], ['privacy', '개인정보 보호 및 보안'], ['users', '사용자 및 그룹']];
  return `<div class="mac-side"><div class="mac-search">⌕ 검색</div>${items.map(([k, t]) => {
    const [c, g] = MAC_ICON[k];
    return `<div class="mac-item${active === k ? ' on' : ''}"><span class="mac-ic" style="background:${c}">${g}</span>${t}</div>`;
  }).join('')}</div>`;
}

/** A macOS System Settings window. `body` is the right pane. */
function macWindow(active, crumb, body, extra = '') {
  return `<div class="os mac" role="img" aria-label="macOS 시스템 설정 화면">
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
  return `<div class="mac-h">공유</div>
    <div class="mac-group">
      ${r('화면 공유', 'screen')}
      ${macRow('파일 공유', sw(false))}
      ${macRow('미디어 공유', sw(false))}
      ${r('원격 로그인', 'ssh')}
      ${macRow('원격 관리', sw(false), { sub: '화면 공유와 함께 켜지 마세요' })}
    </div>
    <div class="mac-group">${macRow('로컬 호스트 이름', '<span class="mac-val">mac-mini.local</span>', { hl: highlight === 'name', n: highlight === 'name' ? 3 : '' })}</div>`;
}

function macScreenSheet() {
  return `<div class="mac-sheet"><div class="mac-h" style="margin-top:0">화면 공유</div>
    <div class="mac-group">
      ${macRow('화면 공유', sw(true))}
      ${macRow('다음 사용자에 대해 접근 허용', '<span class="mac-pop">다음 사용자만 ⌄</span>', { hl: true, n: 1 })}
      <div class="mac-users"><div>👤 alex <small>관리자</small></div><div class="mac-pm">＋ －</div></div>
      ${macRow('VNC 뷰어가 암호로 화면을 제어할 수 있음', sw(false), { sub: '켜지 않아도 됩니다 — Itda는 Mac 계정으로 로그인합니다' })}
    </div>
    <div class="mac-sheet-foot"><span class="mac-btn">완료</span></div></div>`;
}

function macEnergy() {
  return `<div class="mac-h">에너지</div><div class="mac-group">
    ${macRow('디스플레이가 꺼져 있을 때 자동으로 잠자기 방지', sw(true), { hl: true, n: 1 })}
    ${macRow('네트워크 접근으로 깨우기', '<span class="mac-pop">항상 ⌄</span>', { hl: true, n: 2 })}
    ${macRow('정전 후 자동으로 시동', sw(false))}</div>`;
}

const WIN_NAV = [['home', '⌂', '홈'], ['system', '▭', '시스템'], ['bt', '✱', 'Bluetooth 및 장치'], ['net', '◍', '네트워크 및 인터넷'], ['person', '✎', '개인 설정'], ['apps', '▦', '앱'], ['acct', '◉', '계정'], ['privacy', '⛨', '개인 정보 및 보안']];

/** A Windows 11 Settings window. */
function winWindow(active, title, body) {
  return `<div class="os win" role="img" aria-label="Windows 설정 화면">
    <div class="win-bar"><span>설정</span><span class="win-ctl">─ ▢ ✕</span></div>
    <div class="win-body"><div class="win-side"><div class="win-user"><span class="win-ava">A</span><div><b>alex</b><small>로컬 계정</small></div></div>
      <div class="win-find">설정 찾기 ⌕</div>${WIN_NAV.map(([k, g, t]) => `<div class="win-item${active === k ? ' on' : ''}"><span>${g}</span>${t}</div>`).join('')}</div>
      <div class="win-pane"><div class="win-title">${title}</div>${body}</div></div></div>`;
}

function winCard(label, right, { sub = '', hl = false, n = '', icon = '▭' } = {}) {
  return `<div class="win-card${hl ? ' hl' : ''}"><span class="win-ic">${icon}</span><div class="win-txt"><div>${label}${n ? mark(n) : ''}</div>${sub ? `<small>${sub}</small>` : ''}</div><div class="win-right">${right}</div></div>`;
}

const wsw = (on, hl) => `<span class="win-switch${on ? ' on' : ''}${hl ? ' hl' : ''}"><i></i></span><span class="win-sw-l">${on ? '켬' : '끔'}</span>`;

function winRemoteDesktop() {
  return winWindow('system', '시스템 › 원격 데스크톱', `
    ${winCard('원격 데스크톱', wsw(true, true), { sub: '원격 데스크톱 앱을 사용하여 다른 장치에서 이 PC에 연결하고 사용합니다', hl: true, n: 1, icon: '⇄' })}
    ${winCard('네트워크 수준 인증을 사용하여 연결하도록 요구', '☑', { sub: '권장', icon: '⛨' })}
    ${winCard('원격 데스크톱 포트', '<span class="win-val">3389</span>', { icon: '#' })}
    ${winCard('PC 이름', '<span class="win-val">LIVING-PC</span>', { sub: 'Itda에는 이 이름이나 Tailscale 주소를 넣습니다', hl: true, n: 2, icon: '▭' })}
    ${winCard('원격 데스크톱 사용자', '›', { sub: '관리자 계정은 기본으로 허용됩니다', icon: '◉' })}`);
}

function winFirewall() {
  return `<div class="os dlg" role="img" aria-label="Windows 보안 경고 창">
    <div class="dlg-bar">Windows 보안 경고<span>✕</span></div>
    <div class="dlg-body"><div class="dlg-head"><span class="dlg-shield">⛨</span><b>Windows Defender 방화벽에서 이 앱의 일부 기능을 차단했습니다.</b></div>
      <div class="dlg-grid"><span>이름:</span><b>Itda</b><span>게시자:</span><span>알 수 없음</span><span>경로:</span><span>F:\\…\\itda.exe</span></div>
      <p>Itda가 다음 네트워크에서 통신하도록 허용:</p>
      <label class="dlg-check hl">☑ 개인 네트워크(예: 홈 또는 회사 네트워크)${mark(1)}</label>
      <label class="dlg-check">☐ 공용 네트워크(예: 공항, 커피숍) — 권장하지 않음</label>
      <div class="dlg-foot"><span class="dlg-btn primary hl">액세스 허용${mark(2)}</span><span class="dlg-btn">취소</span></div></div></div>`;
}

function winAllowPrompt() {
  return `<div class="os dlg small" role="img" aria-label="Itda 화면 공유 요청 창">
    <div class="dlg-bar">Itda — 화면 공유 요청<span>✕</span></div>
    <div class="dlg-body"><div class="dlg-head"><span class="dlg-q">?</span><div>100.101.10.3에서 이 PC의 화면을 보고 조작하려고 합니다.<br><br>허용할까요?</div></div>
      <div class="dlg-foot"><span class="dlg-btn primary hl">예${mark(1)}</span><span class="dlg-btn">아니요</span></div></div></div>`;
}

function iosShare() {
  return `<div class="os ios" role="img" aria-label="iPhone Safari 공유 메뉴">
    <div class="ios-top"><span class="ios-share hl">⬆︎ 공유${mark(1)}</span></div>
    <div class="ios-row"><span class="ios-app">Itda</span><small>192.168.0.32</small></div>
    <div class="ios-list"><div>복사</div><div>읽기 목록에 추가</div><div>책갈피 추가</div><div class="hl">홈 화면에 추가 ⊞${mark(2)}</div></div></div>`;
}

// ---------------------------------------------------------------- real screenshots

function shot(name, caption, kind = 'desk') {
  const [w, h] = kind === 'phone' ? [780, 1215] : [1400, 834];
  return `<figure class="shot ${kind}"><img src="help/${name}.webp" alt="${esc(caption)}" width="${w}" height="${h}" decoding="async"><figcaption>${caption}</figcaption></figure>`;
}

function step(n, title, body, figure = '') {
  return `<li class="hstep"><div class="hstep-n">${n}</div><div class="hstep-txt"><h3>${title}</h3>${body}</div>${figure ? `<div class="hstep-fig">${figure}</div>` : ''}</li>`;
}

const path = (...xs) => `<span class="path">${xs.map(esc).join(` ${I('E76C')} `)}</span>`;
const note = (t, kind = 'info') => `<p class="hnote ${kind}">${I(kind === 'warn' ? 'E7BA' : 'E946')}<span>${t}</span></p>`;

// ---------------------------------------------------------------- scenarios
// The help center is organised by what people want to do. Each scenario says what each
// device needs, then the steps (where to click), a "do it now" button, how to tell it
// worked, and what to do when it doesn't.

// On a Mac some scenarios are the other way round (this Mac is shared with macOS's own
// Screen Sharing; "Mac에서 이 PC 보기" becomes "PC에서 이 Mac 보기").
const TOPICS = [
  ['start', 'E80F', '무엇을 하고 싶으세요?', 'main'],
  ['mac', 'E7F4', '다른 Mac에 접속하기', 'main'],
  ['windows', 'E7F8', MAC ? 'Windows PC에 접속하기' : '다른 Windows PC에 접속하기', 'main'],
  ['linux', 'E756', 'Linux·라즈베리파이에 접속하기', 'main'],
  ['share', 'E72D', MAC ? '이 Mac을 공유하기' : '이 PC를 공유하기', 'main'],
  ['phone', 'E8EA', MAC ? '휴대폰에서 내 기기 보기' : '휴대폰에서 이 PC·Mac 보기', 'main'],
  ['macpc', MAC ? 'E7F8' : 'E7F4', MAC ? 'Windows PC에서 이 Mac 보기' : 'Mac에서 이 PC 보기', 'main'],
  ['assist', 'E8F2', '다른 사람 PC 도와주기', 'main'],
  ['outside', 'E774', '집 밖에서 연결하기', 'main'],
  ['accounts', 'E77B', '계정과 암호화(SSH)', 'main'],
  ['settings', 'E713', '설정 자세히', 'ref'],
  ['words', 'E82D', '용어 풀이', 'ref'],
  ['trouble', 'E90F', '문제 해결', 'ref'],
  ['keys', 'E765', '단축키', 'ref'],
];

// Shorter labels for the topic list (the page title keeps the full sentence).
const NAV_SHORT = { start: '처음 화면', mac: 'Mac에 접속', windows: 'Windows PC에 접속', linux: 'Linux에 접속', share: MAC ? '이 Mac 공유하기' : '이 PC 공유하기', phone: '휴대폰에서 보기', macpc: MAC ? 'PC에서 이 Mac 보기' : 'Mac에서 이 PC 보기', assist: '다른 사람 도와주기', outside: '집 밖에서 연결', accounts: '계정과 암호화' };

const doBtn = (label, attrs, primary = false) => `<button class="btn${primary ? ' primary' : ''}" ${attrs}>${label}</button>`;
const goPage = (page, label, primary) => doBtn(label, `data-act="page" data-page="${page}"`, primary);
const addKind = (os, label, primary) => doBtn(`${I('E710')}${label}`, `data-act="new-os" data-os="${os}"`, primary);

/** One scenario page. */
function scenario({ lead, needs = [], actions = '', steps = [], check = '', trouble = [], more = '' }) {
  const need = needs.length
    ? `<div class="hneeds"><b>${I('E73E')} 준비물</b><ul>${needs.map(([icon, who, what]) => `<li>${I(icon)}<span><b>${who}</b> — ${what}</span></li>`).join('')}</ul></div>`
    : '';
  return `<p class="hlead">${lead}</p>${need}
    ${actions ? `<div class="hdo"><span>${I('E768')} 바로 하기</span>${actions}</div>` : ''}
    <ol class="hsteps">${steps.map((s, i) => step(i + 1, ...s)).join('')}</ol>
    ${check ? `<div class="hcheck">${I('E73E')}<div><b>잘 됐는지 확인</b><p>${check}</p></div></div>` : ''}
    ${trouble.length ? `<h3 class="hsec">안 될 때</h3><div class="faq">${trouble.map(([q, a]) => `<details><summary>${q}</summary><p>${a}</p></details>`).join('')}</div>` : ''}
    ${more}`;
}

function helpStart() {
  const tile = (t, icon, title, sub) => `<button class="htile" data-help="${t}">${I(icon)}<b>${title}</b><span>${sub}</span></button>`;
  return `<div class="hhero"><h2>무엇을 하고 싶으세요?</h2><p>하고 싶은 일을 고르면, 각 기기에서 무엇을 해야 하는지 처음부터 끝까지 안내합니다.</p></div>
    <h3 class="hsec">${HERE}에서 다른 기기 열기</h3>
    <div class="htiles">
      ${tile('mac', 'E7F4', '다른 Mac에 접속하기', `집이나 회사의 Mac 화면을 ${HERE}에서 보고 조작합니다.`)}
      ${tile('windows', 'E7F8', MAC ? 'Windows PC에 접속하기' : '다른 Windows PC에 접속하기', '다른 방이나 사무실의 Windows PC를 엽니다.')}
      ${tile('linux', 'E756', 'Linux·라즈베리파이에 접속하기', 'VNC 서버가 있는 기기를 엽니다.')}
    </div>
    <h3 class="hsec">다른 기기에서 ${HERE} 보기</h3>
    <div class="htiles">
      ${MAC ? `${tile('share', 'E72D', '이 Mac을 공유하기', 'macOS 화면 공유를 켜서 다른 기기가 이 Mac을 보고 조작하게 합니다.')}
      ${tile('phone', 'E8EA', '휴대폰에서 내 기기 보기', 'QR 하나로 iPhone·Android 브라우저에서 저장된 기기를 엽니다.')}
      ${tile('macpc', 'E7F8', 'Windows PC에서 이 Mac 보기', 'PC의 Itda에 이 Mac을 추가합니다.')}` : `${tile('share', 'E72D', '이 PC를 공유하기', '다른 PC·Mac·휴대폰이 이 PC를 보고 조작할 수 있게 합니다.')}
      ${tile('phone', 'E8EA', '휴대폰에서 이 PC·Mac 보기', 'QR 하나로 iPhone·Android 브라우저에서 엽니다.')}
      ${tile('macpc', 'E7F4', 'Mac에서 이 PC 보기', 'Mac의 Safari나 화면 공유 앱으로 이 PC를 엽니다.')}`}
      ${tile('assist', 'E8F2', '다른 사람 PC 도와주기', '가족·동료의 PC 화면을 보면서 대신 설정해 줍니다.')}
    </div>
    <h3 class="hsec">더 편하게</h3>
    <div class="htiles">
      ${tile('outside', 'E774', '집 밖에서 연결하기', '카페·회사·여행지에서도 집의 기기를 엽니다.')}
      ${tile('accounts', 'E77B', '계정과 암호화', '같은 로그인을 여러 기기에 쓰고, 연결을 암호화합니다.')}
      ${tile('settings', 'E713', '설정 자세히', '모든 설정을 쉬운 말로 풀어 둔 안내')}
    </div>
    ${shot('itda-home', 'Itda 홈 — 저장된 기기가 카드로 보입니다. 초록 점은 지금 연결할 수 있다는 뜻입니다.')}`;
}

function helpMac() {
  return scenario({
    lead: `집이나 회사에 있는 <b>Mac의 화면을 ${HERE}에서</b> 보고 마우스·키보드로 조작합니다. 상대 Mac에 따로 설치할 것은 없습니다.`,
    needs: [['E7F4', MAC ? '상대 Mac' : 'Mac', '화면 공유 켜기 (한 번만), Mac 로그인 이름과 암호'], [MAC ? 'E7F4' : 'E7F8', HERE, 'Itda'], ['E701', '네트워크', '같은 Wi-Fi (밖에서는 <a href="#" data-help="outside">Tailscale</a>)']],
    actions: goPage('nearby', `${I('E701')}주변 Mac 찾기`, true) + addKind('mac', '주소로 직접 추가'),
    steps: [
      ['Mac에서 화면 공유 켜기', `<p>Mac에서 ${path('Apple 메뉴', '시스템 설정', '일반', '공유')} 로 가서 <b>화면 공유</b>를 켭니다 ${mark(1)}.</p>
        ${note('<b>원격 관리</b>는 꺼 두세요. 둘 다 켜져 있으면 화면 공유가 동작하지 않습니다.', 'warn')}`, macWindow('general', '공유', macSharing('screen'))],
      ['내 계정이 허용돼 있는지 확인', `<p>화면 공유 옆 ⓘ ${mark(2)} 를 눌러 <b>다음 사용자에 대해 접근 허용</b>에 내 계정이 있는지 봅니다 ${mark(1)}. ‘모든 사용자’여도 됩니다.</p>`, macWindow('general', '공유', macSharing('screen-i'), macScreenSheet())],
      [`${HERE}의 Itda에서 Mac 추가`, `<p>Itda 왼쪽의 <b>주변 기기</b>를 엽니다. 같은 Wi-Fi의 Mac이 저절로 나타납니다. Mac 옆 <b>추가</b>를 누르세요.</p>
        <p>목록에 없으면 위의 <b>주소로 직접 추가</b>를 누르고, Mac 공유 화면 아래의 <b>로컬 호스트 이름</b>(예: <code>mac-mini.local</code>)을 넣습니다.</p>`, shot('itda-nearby', '주변 기기 — 같은 Wi-Fi의 Mac(위)과 Tailscale의 기기(아래)')],
      ['계정 이름과 암호 넣고 연결', `<p><b>Mac 계정 이름</b>(Mac 로그인할 때 쓰는 짧은 이름)과 <b>Mac 로그인 암호</b>를 넣고 <b>저장 후 연결</b>을 누릅니다.</p>
        ${note('Apple ID(이메일)가 아닙니다. 이름이 헷갈리면 Mac의 터미널에서 <code>whoami</code>를 입력해 보세요.')}`, shot('itda-edit', '연결 편집 — 종류는 Mac')],
    ],
    check: MAC ? '새 창에 Mac 화면이 나오고, 마우스를 움직이면 Mac의 커서가 따라 움직입니다. 홈의 카드에는 ‘연결됨’이 표시됩니다. 창 위쪽의 도구 막대에서 Mac 특수 키(⌘ Space 등), 화면 맞춤, 보기 전용, 전체 화면을 씁니다.' : '새 창에 Mac 화면이 나오고, 마우스를 움직이면 Mac의 커서가 따라 움직입니다. 홈의 카드에는 ‘연결됨’이 표시됩니다. 위쪽 가운데로 마우스를 가져가면 툴바(전체 화면, 맞춤, Mac 특수 키 등)가 나옵니다.',
    trouble: [
      ['‘Mac이 연결을 거부했습니다’', 'Mac의 화면 공유가 꺼져 있거나 원격 관리가 같이 켜져 있습니다. 1단계를 다시 확인하세요.'],
      ['‘로그인하지 못했습니다’', 'Mac <b>계정 이름(짧은 이름)</b>과 로그인 암호인지 확인하세요. 2단계의 접근 허용에 그 계정이 있어야 합니다. 여러 번 틀리면 Mac이 잠시 막으니 1분 뒤 다시 하세요.'],
      ['주변 기기에 Mac이 안 보여요', '같은 Wi-Fi인지 확인하세요. 회사·학교 Wi-Fi는 기기 찾기를 막기도 합니다. 그럴 땐 주소로 직접 추가하세요.'],
      ['Mac이 잠들면 연결이 안 돼요', `Mac의 ${path('시스템 설정', '에너지')} 에서 <b>잠자기 방지</b>와 <b>네트워크 접근으로 깨우기</b>를 켜세요.`],
      MAC ? ['⌘Tab 같은 단축키가 이 Mac에서 먼저 동작해요', '⌘Tab·⌘Space처럼 macOS가 먼저 받는 단축키는 연결 창 위쪽의 <b>특수 키</b> 버튼으로 보내세요.'] : ['한/영 전환이나 ⌘C/⌘V가 이상해요', '연결 편집 → <b>키보드</b> 탭에서 한/영 키와 <b>Ctrl ↔ ⌘ 바꾸기</b>를 맞추세요. <a href="#" data-help="settings#defaults">설정 자세히</a>'],
    ],
    more: `<h3 class="hsec">잠자는 Mac에도 연결되게 (선택)</h3>${macWindow('energy', '에너지', macEnergy())}`,
  });
}

function helpWindows() {
  return scenario({
    lead: `다른 방·사무실의 <b>Windows PC</b>를 ${HERE}에서 엽니다. 방법은 두 가지입니다. 잘 모르겠다면 <b>방법 A</b>를 쓰세요 — 어떤 Windows든 됩니다.`,
    needs: [['E7F8', '상대 PC', 'Itda 설치 (방법 A) 또는 Windows Pro의 원격 데스크톱 (방법 B)'], [MAC ? 'E7F4' : 'E7F8', HERE, 'Itda'], ['E701', '네트워크', '같은 Wi-Fi (밖에서는 Tailscale)']],
    actions: addKind('windows', 'Windows · Itda로 추가', true) + addKind('rdp', '원격 데스크톱으로 추가'),
    steps: [
      ['[방법 A] 상대 PC에 Itda 설치하고 공유 켜기', `<p>상대 PC에서 Itda를 받아 실행하고, 왼쪽 <b>공유</b> → <b>이 PC 공유</b>를 켭니다. 화면에 <b>주소</b>와 8글자 <b>공유 암호</b>가 나옵니다.</p>
        <p>처음 켤 때 Windows 방화벽 창이 뜨면 <b>개인 네트워크</b> ${mark(1)} 만 체크하고 <b>액세스 허용</b> ${mark(2)} 을 누릅니다.</p>`, shot('itda-share', '상대 PC의 공유 → 이 PC 공유 — 주소와 공유 암호') + winFirewall()],
      [`[방법 A] ${HERE}에서 추가하고 연결`, `<p>${HERE}의 Itda <b>주변 기기</b>에 그 PC가 <b>Windows · Itda</b>로 나타납니다. <b>추가</b>를 누르고 <b>공유 암호</b>만 넣으면 됩니다 (계정 이름은 필요 없습니다).</p>
        <p>연결하면 상대 PC에 ‘허용할까요?’ 창이 뜹니다. 상대가 <b>예</b> ${mark(1)} 를 눌러야 연결됩니다.</p>`, winAllowPrompt()],
      ['[방법 B] 원격 데스크톱 켜기 (Windows Pro)', `<p>상대 PC에서 ${path('설정', '시스템', '원격 데스크톱')} 을 켭니다 ${mark(1)}. 아래의 <b>PC 이름</b> ${mark(2)} 을 적어 둡니다.</p>
        ${note('Windows <b>Home</b>에는 원격 데스크톱이 없습니다. 방법 A를 쓰세요.', 'warn')}`, winRemoteDesktop()],
      [`[방법 B] ${HERE}에서 추가하고 연결`, `<p>위의 <b>원격 데스크톱으로 추가</b>를 누르고 주소에 PC 이름을 넣습니다. <b>계정</b>은 그 PC에 로그인할 때 쓰는 계정입니다.</p>
        <ul class="hkeys"><li><b>Microsoft 계정</b> (대부분의 Windows 11 가정용 PC) — 계정에 <b>이메일 주소</b>(예: <code>alex@outlook.com</code>), 암호에 <b>Microsoft 계정 암호</b></li>
          <li><b>로컬 계정</b> — 사용자 이름(예: <code>alex</code>)과 그 암호</li><li><b>회사 계정</b> — <code>회사\\이름</code> 형식</li></ul>
        ${note('로그인 화면의 <b>숫자 PIN·얼굴 인식은 원격 접속에 쓸 수 없습니다</b>. 어떤 계정인지 모르겠다면 그 PC의 <b>설정 → 계정 → 사용자 정보</b>를 보세요. 이메일이 보이면 Microsoft 계정입니다.', 'warn')}
        <p>${MAC ? '연결하면 Itda 창에 바로 열립니다.' : '연결하면 Windows의 원격 데스크톱 앱이 열립니다.'}</p>`, shot('itda-edit', '연결 편집 — 종류에서 Windows를 고릅니다')],
    ],
    check: MAC ? '두 방법 모두 Itda 창에 상대 PC 화면이 나옵니다. 방법 A에서는 상대 PC의 알림 영역에 ‘누군가 보고 있습니다’가 표시됩니다. 창 위쪽 도구 막대의 특수 키에서 <b>Ctrl Alt Del</b>, <b>Alt Tab</b> 등을 보낼 수 있습니다.' : '방법 A는 새 창에, 방법 B는 Windows 원격 데스크톱 창에 상대 PC 화면이 나옵니다. 방법 A에서는 상대 PC의 알림 영역에 ‘누군가 보고 있습니다’가 표시됩니다.',
    trouble: [
      ['‘연결을 거부했습니다’', '방법 A: 상대 PC의 Itda에서 <b>공유 → 이 PC 공유</b>가 켜져 있는지 확인하세요. 방법 B: 원격 데스크톱이 켜져 있는지, Windows Pro인지 확인하세요.'],
      ['‘로그인하지 못했습니다’', '방법 A는 상대 PC에 표시된 <b>공유 암호</b>, 방법 B는 그 PC의 <b>Windows 계정 암호</b>입니다. 공유 암호를 새로 만들었다면 새 암호를 넣으세요.'],
      ['원격 데스크톱 로그인이 계속 실패해요 (Microsoft 계정)', '계정에 <b>이메일 주소</b>, 암호에 <b>Microsoft 계정 암호</b>를 넣었는지 확인하세요 (PIN 아님). 그 PC의 <b>설정 → 계정 → 로그인 옵션</b>에서 ‘Microsoft 계정에 Windows Hello 로그인만 허용’이 켜져 있으면 끄고, 그 PC에서 한 번 암호로 로그인한 뒤 다시 시도하세요. PIN으로만 로그인해 온 PC는 이렇게 해야 원격 로그인이 됩니다.'],
      ['화면은 보이는데 조작이 안 돼요', '상대 PC가 잠금 화면이거나 관리자 권한 창(UAC)이 앞에 있으면 Windows가 입력을 막습니다. 상대 PC의 ‘보기만 허용’이 꺼져 있는지도 확인하세요.'],
      ['응답이 없어요', `두 기기가 같은 Wi-Fi인지, 상대 PC가 켜져 있는지 확인하세요. 다른 곳에 있다면 상대 PC와 ${HERE} 모두 Tailscale을 켜세요.`],
    ],
  });
}

function helpLinux() {
  return scenario({
    lead: 'Linux PC, 라즈베리파이 등 <b>VNC 서버가 있는 기기</b>를 엽니다.',
    needs: [['E756', '상대 기기', 'VNC 서버 (아래 표 참고) 와 VNC 암호'], [MAC ? 'E7F4' : 'E7F8', HERE, 'Itda']],
    actions: addKind('linux', 'Linux · VNC로 추가', true) + goPage('nearby', `${I('E701')}주변 기기 보기`),
    steps: [
      ['상대 기기에서 VNC 켜기', `<table class="shortcuts">
          <tr><td>라즈베리파이</td><td><code>sudo raspi-config</code> → Interface Options → <b>VNC</b> → 켜기</td></tr>
          <tr><td>Ubuntu (GNOME)</td><td>설정 → 시스템 → <b>원격 데스크톱</b> → 화면 공유·VNC 켜기 (버전에 따라 RDP만 있을 수 있습니다)</td></tr>
          <tr><td>그 밖의 Linux</td><td><code>x11vnc</code>, TigerVNC, <code>wayvnc</code>(Wayland) 중 하나를 설치하고 암호를 정합니다</td></tr></table>
        <p>대부분 포트는 <b>5900</b>이고, 두 번째 화면이면 <b>5901</b>입니다.</p>`],
      [`${HERE}에서 추가`, '<p>위의 <b>Linux · VNC로 추가</b>를 누르고 주소(예: <code>raspberrypi.local</code>, <code>192.168.0.30:5901</code>)와 <b>VNC 암호</b>를 넣습니다. 사용자 이름은 대부분 비워 둡니다.</p>', shot('itda-edit', '연결 편집 — 종류에서 Linux · 기타 VNC')],
      ['연결', '<p><b>저장 후 연결</b>을 누릅니다. 밖에서도 쓸 거라면 상대 기기에 SSH가 켜져 있을 때 <b>SSH로 암호화해서 연결</b>을 켜세요.</p>'],
    ],
    check: '새 창에 Linux 바탕화면이 나옵니다.',
    trouble: [
      ['‘연결을 거부했습니다’', 'VNC 서버가 실행 중인지, 포트(5900/5901)가 맞는지, 방화벽(ufw 등)이 막고 있지 않은지 확인하세요.'],
      ['‘로그인하지 못했습니다’', 'Linux 로그인 암호가 아니라 <b>VNC 서버에 정한 암호</b>입니다.'],
      ['검은 화면만 나와요', '모니터가 연결되지 않은 라즈베리파이는 가상 화면 해상도를 정해야 합니다 (raspi-config → Display Options).'],
    ],
  });
}

function helpShare() {
  return MAC ? helpShareMac() : scenario({
    lead: '<b>다른 기기가 이 PC를</b> 보고 조작할 수 있게 합니다. 다른 PC의 Itda, Mac의 화면 공유 앱, 휴대폰 브라우저 모두 됩니다.',
    needs: [['E7F8', '이 PC', 'Itda (지금 이 앱)'], ['E8EA', '볼 기기', 'Itda가 있는 PC, Mac, 또는 휴대폰 브라우저'], ['E701', '네트워크', '같은 Wi-Fi (밖에서는 Tailscale)']],
    actions: doBtn(`${I('E768')}모두 켜기 (공유 + 웹 뷰어)`, 'data-act="share-all-on"', true) + goPage('share', '공유 페이지 열기'),
    steps: [
      ['공유 켜기', `<p>왼쪽 <b>공유</b> 페이지에서 <b>모두 켜기</b>를 누릅니다. 이 PC의 <b>주소</b>, <b>공유 암호</b>, 휴대폰용 <b>QR 코드</b>와 <b>PIN</b>이 나옵니다.</p>`, shot('itda-web', '공유 페이지 — 웹 뷰어(위)와 이 PC 공유(아래)')],
      ['방화벽 허용', `<p>처음 켤 때 Windows가 묻습니다. <b>개인 네트워크</b> ${mark(1)} 만 체크하고 <b>액세스 허용</b> ${mark(2)} 을 누르세요. 놓쳤다면 Itda를 다시 시작하면 다시 묻습니다.</p>`, winFirewall()],
      ['볼 기기에서 접속', `<ul class="hkeys">
          <li><b>다른 PC의 Itda</b> — 주변 기기에 이 PC가 <b>Windows · Itda</b>로 나옵니다. 추가하고 공유 암호를 넣으세요. <a href="#" data-help="windows">자세히</a></li>
          <li><b>휴대폰·태블릿</b> — 카메라로 QR을 찍고 PIN을 넣으세요. <a href="#" data-help="phone">자세히</a></li>
          <li><b>Mac</b> — Safari로 웹 뷰어 주소를 열거나, Mac의 화면 공유 앱을 씁니다. <a href="#" data-help="macpc">자세히</a></li></ul>`],
      ['누가 들어오면 허락하기', `<p><b>연결할 때마다 묻기</b>가 켜져 있으면(추천), 누가 들어오려 할 때 이 PC에 이런 창이 뜹니다. <b>예</b> ${mark(1)} 를 눌러야 들어옵니다. 휴대폰 웹 뷰어는 PIN으로 이미 확인했으므로 묻지 않습니다.</p>`, winAllowPrompt()],
    ],
    check: '누군가 보고 있는 동안 알림 영역의 Itda에 표시되고, 공유 페이지에 ‘○명이 보는 중’이 나옵니다. <b>모두 끊기</b>로 언제든 끊을 수 있습니다.',
    trouble: [
      ['다른 기기에서 ‘응답이 없다’고 나와요', '방화벽에서 ‘개인 네트워크’를 허용했는지, 두 기기가 같은 Wi-Fi인지 확인하세요. 공유 페이지의 포트를 바꿔 볼 수도 있습니다.'],
      ['5900 주소를 브라우저에 넣었더니 안 열려요', '5900은 VNC 앱용입니다. 브라우저는 <b>웹 뷰어 주소</b>(:8780)를 쓰세요. 공유 페이지에서 웹 뷰어를 켜면 나옵니다.'],
      ['잠금 화면에서는 조작이 안 돼요', 'Windows 보안 때문에 잠금 화면과 관리자 권한 창은 원격으로 조작할 수 없습니다. 이 PC의 잠금을 먼저 풀어 두세요.'],
      ['다 쓰고 나면?', '공유 페이지에서 끄거나, 알림 영역의 Itda → 공유에서 끄세요. 쓰지 않을 때는 꺼 두는 것이 안전합니다.'],
    ],
  });
}

function helpPhone() {
  return MAC ? helpPhoneMac() : scenario({
    lead: '앱 설치 없이 <b>iPhone·Android 브라우저</b>로 이 PC와, 이 PC에 저장된 Mac·Windows PC를 봅니다. 암호는 이 PC에만 있고 휴대폰으로 가지 않습니다.',
    needs: [['E7F8', '이 PC', 'Itda, 공유 페이지의 웹 뷰어 켜기'], ['E8EA', '휴대폰', '카메라와 브라우저(Safari·Chrome)'], ['E701', '네트워크', '같은 Wi-Fi (밖에서는 두 기기 모두 Tailscale)']],
    actions: doBtn(`${I('E768')}모두 켜고 QR 보기`, 'data-act="share-all-on"', true),
    steps: [
      ['이 PC에서 켜기', `<p>위의 <b>모두 켜고 QR 보기</b>를 누르면 공유 페이지가 열리고 <b>QR 코드</b>와 6자리 <b>PIN</b>이 나옵니다. 휴대폰으로 <b>이 PC 자체</b>를 보려면 ‘이 PC 공유’도 켜져 있어야 합니다 (모두 켜기가 둘 다 켭니다).</p>`, shot('itda-web', '공유 → 웹 뷰어 — QR, 주소, PIN')],
      ['휴대폰 카메라로 QR 찍기 → PIN', '<p>iPhone 기본 카메라로 QR을 비추면 링크가 뜹니다. 열어서 PIN을 넣습니다. 12시간 동안 다시 묻지 않습니다.</p>', shot('phone-login', 'PIN 입력', 'phone')],
      ['보고 싶은 기기 고르기', '<p>맨 앞의 <b>○○ (이 PC)</b>가 지금 이 PC입니다. 그 뒤로 저장된 Mac과 PC가 있습니다. 암호가 없는 기기는 누르면 암호를 물어봅니다.</p>', shot('phone-list', '기기 목록 — 맨 앞이 이 PC', 'phone')],
      ['손가락으로 조작하기', `<ul class="hkeys"><li><b>탭</b> = 클릭, <b>두 손가락 탭</b> = 오른쪽 클릭, <b>두 손가락 끌기</b> = 스크롤</li><li><b>두 손가락 벌리기/오므리기</b> = 실제 크기 ↔ 화면에 맞춤</li>
          <li>위쪽 도구 막대: ‹ 목록 · ⌨ 키보드(한글 가능) · ⌘/⊞ 특수 키 · 📋 클립보드 · 👁 보기 전용 · ⤢ 전체 화면</li></ul>`, shot('phone-view', '원격 화면', 'phone') + shot('phone-keys', '특수 키', 'phone')],
      ['홈 화면에 추가 (앱처럼)', `<p>Safari의 공유 버튼 ${mark(1)} → <b>홈 화면에 추가</b> ${mark(2)}. 다음부터 아이콘으로 바로 열립니다. Android는 Chrome 메뉴 → <b>앱 설치</b>(https 주소에서).</p>`, iosShare()],
    ],
    check: '휴대폰에 PC 화면이 보이고, 탭하면 PC의 마우스가 움직입니다. 이 PC의 공유 페이지에는 보고 있는 기기가 표시됩니다.',
    trouble: [
      ['QR을 찍어도 페이지가 안 열려요', '휴대폰이 <b>같은 Wi-Fi</b>인지 확인하세요 (모바일 데이터면 안 됩니다). 이 PC의 방화벽에서 ‘개인 네트워크’를 허용했는지도 보세요. 밖이라면 Tailscale QR을 쓰세요.'],
      ['PIN을 계속 거부해요', '5번 틀리면 1분 동안 막힙니다. PIN을 새로 만들면 예전 PIN은 안 됩니다.'],
      ['목록에 ‘이 PC’가 없어요', '공유 페이지에서 <b>이 PC 공유</b>도 켜야 나옵니다.'],
      ['화면이 너무 작아요', '두 손가락으로 벌리면 실제 크기가 됩니다. 끌어서 보고 싶은 곳으로 옮기세요.'],
    ],
  });
}

function helpMacPc() {
  return MAC ? helpPcMac() : scenario({
    lead: '<b>Mac에서 이 Windows PC를</b> 엽니다. Mac에 따로 설치할 것은 없습니다 — Safari(웹 뷰어)나 Mac에 들어 있는 화면 공유 앱을 씁니다.',
    needs: [['E7F8', '이 PC', 'Itda, 공유 페이지에서 모두 켜기'], ['E7F4', 'Mac', 'Safari 또는 화면 공유 앱 (기본 포함)']],
    actions: doBtn(`${I('E768')}모두 켜기`, 'data-act="share-all-on"', true) + goPage('share', '공유 페이지 열기'),
    steps: [
      ['이 PC에서 공유 켜기', '<p>공유 페이지에서 <b>모두 켜기</b>를 누릅니다. <b>웹 뷰어 주소·PIN</b>과 <b>이 PC 공유 주소·공유 암호</b>가 나옵니다.</p>', shot('itda-web', '공유 페이지')],
      ['방법 1 — Safari로 (추천)', `<p>Mac의 Safari에 웹 뷰어 주소(예: <code>http://192.168.0.32:8780</code>)를 넣고 PIN을 입력합니다. 이 PC와 저장된 다른 기기도 여기서 열 수 있습니다.</p>
        <p>${path('Safari', '파일', 'Dock에 추가')} 를 하면 Mac 앱처럼 Dock에서 바로 열립니다.</p>`],
      ['방법 1-2 — Mac용 Itda 앱으로', `<p><a href="https://github.com/jiunbae/itda/releases/latest" data-ext>Mac용 Itda</a>(Itda-macos-universal.zip)를 받아 압축을 풀고, 처음 한 번은 <b>오른쪽 클릭 → 열기</b>로 실행합니다. 이 PC를 <b>Windows · Itda</b>로 추가하고 공유 암호를 넣으면 됩니다. 다른 Mac·Windows PC·Linux도 같은 앱에서 열 수 있습니다.</p>`],
      ['방법 2 — Mac의 화면 공유 앱으로', `<p>Finder에서 ${path('이동', '서버에 연결')} (⌘K)을 누르고 <code>vnc://192.168.0.32</code>처럼 <b>이 PC 공유 주소</b>를 넣습니다. 암호에는 <b>공유 암호</b>를 넣습니다.</p>
        <p>이 PC에 ‘허용할까요?’ 창이 뜨면 <b>예</b>를 누르세요. Mac의 ⌘C·⌘V가 이 PC의 Ctrl+C·Ctrl+V로 동작합니다.</p>`],
    ],
    check: 'Mac에 이 PC의 바탕화면이 보이고 조작됩니다.',
    trouble: [
      ['Safari에서 페이지가 안 열려요', '<b>웹 뷰어 주소</b>(:8780)를 썼는지 확인하세요. 같은 Wi-Fi인지, 이 PC 방화벽을 허용했는지 보세요.'],
      ['화면 공유 앱이 ‘인증 실패’라고 해요', 'Mac 계정 암호가 아니라 이 PC의 <b>공유 암호</b>(8글자)를 넣으세요.'],
      ['Mac용 Itda가 ‘로그인에 실패했습니다’라고 해요', '<b>Windows · Itda</b>로 추가한 PC는 Windows 계정·이메일·Windows 암호를 쓰지 않습니다. 이 PC의 Itda → <b>공유</b> 페이지에 표시된 <b>공유 암호</b>(8글자)를 넣으세요. Mac의 연결 목록에서 그 PC를 편집해 암호를 고칠 수 있습니다.'],
    ],
  });
}

// ---- Itda for Mac: this Mac is shared with macOS's own Screen Sharing.

function helpShareMac() {
  return scenario({
    lead: '<b>다른 기기가 이 Mac을</b> 보고 조작할 수 있게 합니다. macOS에 들어 있는 <b>화면 공유</b>를 켜면 됩니다 — Windows PC의 Itda, 다른 Mac의 화면 공유 앱, VNC 앱 모두 이 Mac에 들어올 수 있습니다.',
    needs: [['E7F4', '이 Mac', '화면 공유 켜기 (한 번만)'], ['E7F8', '볼 기기', 'Itda가 있는 PC·Mac, 또는 Mac의 화면 공유 앱'], ['E701', '네트워크', '같은 Wi-Fi (밖에서는 <a href="#" data-help="outside">Tailscale</a>)']],
    actions: doBtn(`${I('E8A7')}공유 설정 열기`, 'data-act="sys-sharing"', true) + goPage('share', '공유 페이지 열기'),
    steps: [
      ['화면 공유 켜기', `<p>${path('Apple 메뉴', '시스템 설정', '일반', '공유')} 에서 <b>화면 공유</b>를 켭니다 ${mark(1)}. 위의 <b>공유 설정 열기</b>를 누르면 바로 열립니다.</p>
        ${note('<b>원격 관리</b>는 꺼 두세요. 둘 다 켜져 있으면 화면 공유가 동작하지 않습니다.', 'warn')}`, macWindow('general', '공유', macSharing('screen'))],
      ['들어올 계정 정하기', `<p>화면 공유 옆 ⓘ ${mark(2)} → <b>다음 사용자에 대해 접근 허용</b> ${mark(1)} 에 들어올 계정이 있는지 봅니다. 다른 기기에서는 <b>이 Mac의 계정 이름과 로그인 암호</b>로 들어옵니다.</p>`, macWindow('general', '공유', macSharing('screen-i'), macScreenSheet())],
      ['이 Mac의 이름 확인', `<p>공유 화면 아래의 <b>로컬 호스트 이름</b> ${mark(3)} (예: <code>mac-mini.local</code>)이 이 Mac의 주소입니다. Itda의 왼쪽 <b>공유</b> 페이지에도 주소와 계정 이름이 나옵니다.</p>`, macWindow('general', '공유', macSharing('name'))],
      ['볼 기기에서 추가', `<ul class="hkeys">
          <li><b>Windows PC의 Itda</b> — 주변 기기에 이 Mac이 나옵니다. <b>추가</b>를 누르고 이 Mac의 계정 이름과 암호를 넣으세요. <a href="#" data-help="macpc">자세히</a></li>
          <li><b>다른 Mac</b> — Itda에서 Mac으로 추가하거나, Finder ${path('이동', '서버에 연결')} (⌘K)에 <code>vnc://mac-mini.local</code>처럼 넣습니다.</li>
          <li><b>휴대폰</b> — 이 Mac을 저장해 둔 다른 기기의 웹 뷰어로 엽니다.</li></ul>`],
    ],
    check: '누군가 이 Mac을 보고 있으면 메뉴 막대에 화면 공유 아이콘이 나타납니다. 눌러서 연결을 끊을 수 있습니다.',
    trouble: [
      ['다른 기기에서 ‘연결을 거부했습니다’', '화면 공유가 꺼져 있거나 <b>원격 관리</b>가 같이 켜져 있습니다. 1단계를 다시 확인하세요.'],
      ['‘로그인하지 못했습니다’', '이 Mac의 <b>계정 이름(짧은 이름)</b>과 로그인 암호입니다 (Apple ID 아님). 2단계의 접근 허용에 그 계정이 있어야 합니다.'],
      ['이 Mac이 잠들면 연결이 안 돼요', `${path('시스템 설정', '에너지')} 에서 <b>잠자기 방지</b>와 <b>네트워크 접근으로 깨우기</b>를 켜세요.`],
      ['다 쓰고 나면?', '공유 설정에서 화면 공유를 끄면 됩니다. 쓰지 않을 때는 꺼 두는 것이 안전합니다.'],
    ],
    more: `<h3 class="hsec">잠자는 Mac에도 연결되게 (선택)</h3>${macWindow('energy', '에너지', macEnergy())}`,
  });
}

function helpPhoneMac() {
  return scenario({
    lead: '앱 설치 없이 <b>iPhone·Android 브라우저</b>로 이 Mac에 저장된 Mac·Windows PC·Linux를 봅니다. 암호는 이 Mac에만 있고 휴대폰으로 가지 않습니다.',
    needs: [['E7F4', '이 Mac', 'Itda, 공유 페이지의 웹 뷰어 켜기'], ['E8EA', '휴대폰', '카메라와 브라우저(Safari·Chrome)'], ['E701', '네트워크', '같은 Wi-Fi (밖에서는 두 기기 모두 Tailscale)']],
    actions: doBtn(`${I('E768')}웹 뷰어 켜고 QR 보기`, 'data-act="web-on"', true),
    steps: [
      ['이 Mac에서 켜기', '<p>위의 버튼을 누르면 공유 페이지가 열리고 <b>QR 코드</b>와 6자리 <b>PIN</b>이 나옵니다. macOS가 <b>들어오는 네트워크 연결</b>을 허용할지 물으면 <b>허용</b>을 누르세요.</p>', shot('itda-web', '공유 → 웹 뷰어 — QR, 주소, PIN')],
      ['휴대폰 카메라로 QR 찍기 → PIN', '<p>iPhone 기본 카메라로 QR을 비추면 링크가 뜹니다. 열어서 PIN을 넣습니다. 12시간 동안 다시 묻지 않습니다.</p>', shot('phone-login', 'PIN 입력', 'phone')],
      ['보고 싶은 기기 고르기', '<p>이 Mac의 Itda에 저장된 기기가 나옵니다. 암호가 없는 기기는 누르면 암호를 물어봅니다.</p>', shot('phone-list', '기기 목록', 'phone')],
      ['손가락으로 조작하기', `<ul class="hkeys"><li><b>탭</b> = 클릭, <b>두 손가락 탭</b> = 오른쪽 클릭, <b>두 손가락 끌기</b> = 스크롤</li><li><b>두 손가락 벌리기/오므리기</b> = 실제 크기 ↔ 화면에 맞춤</li>
          <li>위쪽 도구 막대: ‹ 목록 · ⌨ 키보드(한글 가능) · ⌘/⊞ 특수 키 · 📋 클립보드 · 👁 보기 전용 · ⤢ 전체 화면</li></ul>`, shot('phone-view', '원격 화면', 'phone')],
      ['홈 화면에 추가 (앱처럼)', `<p>Safari의 공유 버튼 ${mark(1)} → <b>홈 화면에 추가</b> ${mark(2)}. 다음부터 아이콘으로 바로 열립니다.</p>`, iosShare()],
    ],
    check: '휴대폰에 고른 기기의 화면이 보이고, 탭하면 그 기기의 마우스가 움직입니다.',
    trouble: [
      ['QR을 찍어도 페이지가 안 열려요', '휴대폰이 <b>같은 Wi-Fi</b>인지 확인하세요 (모바일 데이터면 안 됩니다). <b>시스템 설정 → 네트워크 → 방화벽</b>이 켜져 있다면 Itda의 들어오는 연결을 허용하세요. 밖이라면 Tailscale QR을 쓰세요.'],
      ['PIN을 계속 거부해요', '5번 틀리면 1분 동안 막힙니다. PIN을 새로 만들면 예전 PIN은 안 됩니다.'],
      ['이 Mac 자체가 목록에 없어요', '웹 뷰어는 이 Mac에 저장된 다른 기기를 보여 줍니다. 휴대폰에서 이 Mac을 보려면 다른 기기의 Itda 웹 뷰어에 이 Mac을 추가하세요 (<a href="#" data-help="share">이 Mac을 공유하기</a>).'],
      ['Mac이 잠들면 웹 뷰어도 멈춰요', `잠자는 동안은 웹 뷰어도 쉬니 ${path('시스템 설정', '에너지')} 에서 잠자기 방지를 켜세요.`],
    ],
  });
}

function helpPcMac() {
  return scenario({
    lead: '<b>Windows PC에서 이 Mac을</b> 엽니다. 이 Mac은 macOS 화면 공유를 켜기만 하면 되고, PC에는 Itda를 설치합니다.',
    needs: [['E7F4', '이 Mac', '화면 공유 켜기 (<a href="#" data-help="share">이 Mac을 공유하기</a>)'], ['E7F8', 'Windows PC', 'Itda (Windows용)']],
    actions: doBtn(`${I('E8A7')}공유 설정 열기`, 'data-act="sys-sharing"', true) + goPage('share', '이 Mac 주소 보기'),
    steps: [
      ['이 Mac에서 화면 공유 켜기', `<p>${path('시스템 설정', '일반', '공유')} 에서 <b>화면 공유</b>를 켭니다. 왼쪽 <b>공유</b> 페이지에 이 Mac의 주소와 계정 이름이 나옵니다.</p>`, macWindow('general', '공유', macSharing('screen'))],
      ['PC에 Itda 설치', '<p>PC에서 <a href="https://github.com/jiunbae/itda/releases/latest" data-ext>Itda for Windows</a>(itda.exe)를 받아 실행합니다.</p>'],
      ['PC의 Itda에서 이 Mac 추가', '<p>PC Itda의 <b>주변 기기</b>에 이 Mac이 나옵니다. <b>추가</b>를 누르고 종류 <b>Mac</b>, 이 Mac의 <b>계정 이름</b>과 <b>로그인 암호</b>를 넣고 연결합니다.</p>', shot('itda-nearby', 'PC의 Itda — 주변 기기')],
    ],
    check: 'PC의 새 창에 이 Mac 화면이 나옵니다. 이 Mac의 메뉴 막대에는 화면 공유 아이콘이 나타납니다.',
    trouble: [
      ['PC에서 ‘연결을 거부했습니다’', '이 Mac의 화면 공유가 켜져 있는지, 원격 관리가 꺼져 있는지 확인하세요.'],
      ['PC의 주변 기기에 이 Mac이 없어요', '같은 Wi-Fi인지 확인하고, 없으면 공유 페이지에 나온 주소(예: <code>mac-mini.local</code>)를 직접 넣으세요.'],
    ],
  });
}

function helpAssist() {
  return scenario({
    lead: '부모님·동료의 PC 화면을 보면서 <b>대신 설정해 주거나 알려 줄 때</b> 씁니다. 상대가 허락해야만 연결되고, 언제든 끊을 수 있습니다.',
    needs: [['E7F8', '도움받는 사람의 PC', 'Itda 설치, 공유 켜기'], [MAC ? 'E7F4' : 'E7F8', '도와주는 사람(나)', MAC ? '이 Mac의 Itda' : 'Itda가 있는 PC, 또는 휴대폰'], ['E774', '멀리 떨어져 있다면', '두 PC 모두 Tailscale (같은 계정으로 로그인하거나 기기 공유)']],
    steps: [
      ['상대에게 이렇게 부탁하세요', `<div class="hscript"><b>상대에게 보낼 말</b><p>“Itda를 받아서 실행한 다음, 왼쪽 <b>공유</b>를 누르고 <b>이 PC 공유</b>를 켜 줘. 화면에 나오는 <b>주소</b>랑 <b>공유 암호</b>를 알려 줘. 내가 연결하면 ‘허용할까요?’ 창이 뜨는데 그때 <b>예</b>를 눌러 줘.”</p></div>`],
      ['내 Itda에서 연결', '<p><b>Windows · Itda로 추가</b>를 누르고 받은 주소와 공유 암호를 넣습니다. 같은 Wi-Fi라면 주변 기기에 바로 보입니다.</p>' + addKind('windows', 'Windows · Itda로 추가', true)],
      ['상대가 ‘예’를 누르면 시작', '<p>상대 화면에 허락 창이 뜹니다. 허락하면 연결됩니다. 상대는 알림 영역에서 언제든 <b>모두 끊기</b>로 끊을 수 있습니다.</p>', winAllowPrompt()],
      ['다 끝나면', '<p>연결을 끊고, 상대에게 <b>공유를 끄라고</b>알려 주세요. 다음에 또 도울 일이 있으면 다시 켜면 됩니다.</p>'],
    ],
    check: '상대 PC의 화면이 내 창에 보이고, 상대 PC의 알림 영역에 ‘누군가 보고 있습니다’가 표시됩니다.',
    trouble: [
      ['멀리 있어서 연결이 안 돼요', '같은 Wi-Fi가 아니면 직접 연결되지 않습니다. 두 PC에 Tailscale을 설치하고, 상대 PC를 내 Tailscale에 공유받거나 같은 계정으로 로그인하세요. <a href="#" data-help="outside">집 밖에서 연결하기</a>'],
      ['상대가 허락 창을 못 봤어요', '창이 다른 창 뒤에 있을 수 있습니다. 다시 연결하면 다시 뜹니다.'],
    ],
  });
}

function helpOutside() {
  return scenario({
    lead: '카페·회사·여행지에서 <b>집의 기기</b>를 엽니다. 가장 쉽고 안전한 방법은 무료 앱 <b>Tailscale</b>입니다. 공유기 설정은 필요 없습니다.',
    needs: [['E774', '모든 기기', 'Tailscale 설치 후 같은 계정으로 로그인 (개인은 무료)']],
    actions: goPage('nearby', `${I('E701')}Tailscale 기기 보기`, true),
    steps: [
      ['모든 기기에 Tailscale 설치', `<p>${HERE}, 연결할 Mac·PC, 휴대폰에 <a href="https://tailscale.com/download" data-ext>Tailscale</a>을 설치하고 <b>같은 계정</b>으로 로그인합니다.</p>`],
      ['Itda에서 Tailscale 기기 추가', '<p><b>주변 기기</b> 아래쪽 <b>Tailscale — 어디서나</b>에 기기들이 나타납니다. 여기서 추가한 연결은 밖에서도 그대로 됩니다.</p>', shot('itda-nearby', 'Tailscale — 어디서나')],
      [MAC ? '휴대폰에서 내 기기 보기' : '휴대폰에서 이 PC 보기', '<p>공유 페이지의 QR에서 <b>Tailscale</b>을 고르면 밖에서도 열리는 주소가 나옵니다. <b>HTTPS 주소 만들기</b>를 누르면 https 주소가 생겨 휴대폰에 앱처럼 설치할 수도 있습니다.</p>', shot('itda-web', '공유 → 웹 뷰어의 Tailscale 주소')],
    ],
    check: '휴대폰의 Wi-Fi를 끄고(모바일 데이터 + Tailscale 켬) 연결해 보세요. 그래도 열리면 성공입니다.',
    trouble: [
      ['Tailscale 기기가 안 보여요', 'Tailscale 앱이 켜져 있는지(연결됨), 같은 계정인지 확인하세요. Itda를 다시 시작하면 다시 찾습니다.'],
      ['HTTPS 주소 만들기가 안 돼요', `<a href="https://login.tailscale.com/admin/dns" data-ext>Tailscale 관리 콘솔 → DNS</a>에서 ‘HTTPS Certificates’를 켜야 합니다.`],
      ['Tailscale 없이 하고 싶어요', 'Mac에서 원격 로그인을 켜고 공유기에서 <b>SSH 포트(22)만</b> Mac으로 전달한 뒤 <b>SSH로 암호화해서 연결</b>을 쓰세요. 화면 공유(5900)나 웹 뷰어 포트를 인터넷에 직접 열면 안 됩니다.'],
    ],
  });
}

function helpAccounts() {
  return scenario({
    lead: '여러 기기에 같은 이름·암호로 로그인한다면 <b>계정</b>을 한 번만 저장해 두세요. 암호를 바꿀 때 한 곳만 고치면 됩니다. 밖에서 쓸 때는 <b>SSH 암호화</b>도 켜세요.',
    actions: goPage('accounts', `${I('E77B')}계정 열기`, true),
    steps: [
      ['계정 만들기', '<p>왼쪽 <b>계정</b> → <b>새 계정</b>. 이름, 계정 이름, 암호를 넣습니다. SSH 키를 쓴다면 키 파일과 키 암호도 넣습니다.</p>', shot('itda-account-edit', '계정 편집')],
      ['기기에서 계정 고르기', '<p>연결 편집의 <b>로그인</b>에서 계정을 고릅니다. 그 자리에서 <b>새 계정 만들기…</b>도 됩니다.</p>', shot('itda-edit', '연결 편집 — 로그인')],
      ['이미 만든 연결 묶기', '<p>같은 계정 이름을 쓰는 연결이 여러 개면 계정 화면에 <b>계정으로 묶기</b>가 뜹니다. 누르면 저장된 암호까지 한 계정으로 옮겨집니다.</p>', shot('itda-accounts', '계정 화면')],
      ['SSH로 암호화 (밖에서 쓸 때)', `<p>Mac에서 <b>원격 로그인</b> ${mark(1)} 을 켜고, 연결 편집에서 <b>SSH로 암호화해서 연결</b>을 켭니다. ${MAC ? 'macOS에 들어 있는 ssh' : 'Windows에 기본으로 있는 OpenSSH'}를 쓰므로 따로 설치할 것이 없습니다. 카드에 🔒가 보이면 암호화된 연결입니다.</p>`, macWindow('general', '공유', macSharing('ssh'))],
    ],
  });
}

const FAQ = [
  ['‘Mac이 연결을 거부했습니다’', 'Mac의 화면 공유가 꺼져 있습니다. <b>시스템 설정 → 일반 → 공유 → 화면 공유</b>를 켜세요. 원격 관리가 켜져 있다면 끄세요.', 'mac'],
  ['‘로그인하지 못했습니다’', 'Mac <b>계정 이름(짧은 이름)</b>과 로그인 암호인지 확인하세요 (Apple ID 아님). 화면 공유 ⓘ의 ‘접근 허용’에 그 계정이 있어야 합니다. 여러 번 틀리면 Mac이 잠시 막으니 1분 뒤 다시 하세요.', 'mac'],
  ['‘응답하지 않습니다’ / 회색 점', 'Mac이 꺼져 있거나 잠자기 중이거나, 다른 네트워크입니다. 밖이라면 Tailscale을 켜세요. 자주 잠든다면 <b>에너지 → 네트워크 접근으로 깨우기</b>를 켜세요.', 'outside'],
  ['주변 기기에 아무것도 안 보여요', '같은 Wi-Fi인지, 상대 기기에서 공유가 켜져 있는지 확인하세요. 회사·학교 Wi-Fi는 기기끼리 찾는 것을 막기도 합니다. 그럴 땐 주소로 직접 추가하거나 Tailscale을 쓰세요.', 'start'],
  ['한/영 전환이 안 돼요', '설정 → 새 연결 기본값(또는 연결 편집 → 키보드)에서 한/영 키를 Mac의 입력 소스 단축키(Ctrl+Space 또는 Caps Lock)와 맞추세요.', 'keys'],
  ['⌘C/⌘V가 Ctrl로 안 돼요', '연결 편집 → 키보드 → <b>Ctrl ↔ ⌘ 바꾸기</b>를 켜면 Ctrl+C/V가 Mac의 ⌘C/V로 동작합니다. Windows 키는 항상 ⌘입니다.', 'keys'],
  ['휴대폰에서 웹 뷰어가 안 열려요', MAC ? '휴대폰이 같은 Wi-Fi인지(또는 Tailscale이 켜져 있는지), macOS 방화벽이 Itda의 들어오는 연결을 막고 있지 않은지 확인하세요. 공유 페이지에서 웹 뷰어 포트를 바꿔 볼 수도 있습니다.' : '휴대폰이 같은 Wi-Fi인지(또는 Tailscale이 켜져 있는지), PC의 방화벽 창에서 ‘개인 네트워크’를 허용했는지 확인하세요. 설정에서 웹 뷰어 포트를 바꿔 볼 수도 있습니다.', 'phone'],
  ['Windows 원격 데스크톱에 어떤 계정을 넣나요?', 'Microsoft 계정으로 로그인하는 PC라면 <b>이메일 주소</b>와 <b>Microsoft 계정 암호</b>입니다. 숫자 PIN은 안 됩니다. 로컬 계정은 사용자 이름, 회사 계정은 <b>회사\\이름</b>입니다. Itda 공유(방법 A)는 계정 없이 <b>공유 암호</b>만 넣습니다.', 'windows'],
  ['Windows PC에 연결은 되는데 조작이 안 돼요', '상대 PC가 잠금 화면이거나 관리자 권한 창(UAC 등)이 앞에 있으면 Windows가 입력을 막습니다. 상대 PC의 ‘보기만 허용’이 꺼져 있는지도 확인하세요.', 'windows'],
];

// Keyboard options of the Windows viewer; the Mac's windows send keys as they are.
const PC_ONLY = ['한/영 전환이 안 돼요', '⌘C/⌘V가 Ctrl로 안 돼요'];

function helpTrouble() {
  return `<div class="faq">${FAQ.filter(([q]) => !(MAC && PC_ONLY.includes(q))).map(([q, a, t]) => `<details><summary>${q}</summary><p>${a}</p><a href="#" data-help="${t}">관련 안내 보기 →</a></details>`).join('')}</div>
    <p class="hnote info">${I('E946')}<span>계속 안 되면 ${path('설정', '정보', '로그')} 를 열어 마지막 줄들을 확인하세요. 연결마다 별도 로그가 남습니다.</span></p>`;
}

function helpKeysMac() {
  const row = (a, k) => `<tr><td>${a}</td><td><kbd>${k}</kbd></td></tr>`;
  return `<p class="hlead">연결 창에서는 키보드 입력이 그대로 원격 기기로 갑니다. macOS가 먼저 가로채는 조합(⌘Tab, ⌘Space 등)은 창 위쪽 도구 막대의 <b>특수 키</b>로 보내세요.</p>
    <h3 class="hsec">Itda 홈</h3>
    <table class="shortcuts">${row('새 연결', '⌘N')}${row('검색', '⌘F')}${row('설정', '⌘,')}${row('화면 크기', '⌘+ / ⌘− / ⌘0')}${row('Itda 홈 창 보기', '⌘1')}${row('종료', '⌘Q')}</table>
    <h3 class="hsec">연결 창의 도구 막대</h3>
    <table class="shortcuts"><tr><td>특수 키</td><td>Mac: ⌘ Space, ⌘ Tab, ⌥⌘ Esc, ⌃⌘ Q … · Windows: Ctrl Alt Del, Alt Tab, ⊞ …</td></tr>
      <tr><td>화면 맞춤 / 실제 크기</td><td>창에 맞춰 보기 ↔ 1:1로 보기</td></tr><tr><td>보기 전용</td><td>마우스·키보드를 보내지 않습니다</td></tr><tr><td>전체 화면</td><td>다시 누르면 돌아옵니다</td></tr></table>
    <h3 class="hsec">Windows PC를 열 때</h3>
    <table class="shortcuts"><tr><td>⌘ Command</td><td><kbd>⊞ Windows 키</kbd></td></tr><tr><td>⌥ Option</td><td><kbd>Alt</kbd></td></tr><tr><td>⌃ Control</td><td><kbd>Ctrl</kbd> — 복사는 ⌃C, 붙여넣기는 ⌃V</td></tr></table>`;
}

function helpKeys() {
  if (MAC) return helpKeysMac();
  const p = hk();
  const rows = [['전체 화면', 'Enter'], ['창에 맞춤 / 실제 크기', '0 / 1'], ['통계 표시', 'S'], ['보기 전용', 'V'], ['스크린샷 저장', 'P'], ['연결 정보', 'I'], ['툴바 고정', 'T'], ['다시 연결', 'R'], ['전체 메뉴', 'M']];
  return `<p class="hlead">접속 창에서 아래 조합은 Mac으로 보내지 않고 Itda가 처리합니다. 조합은 설정에서 Ctrl+Shift로 바꿀 수 있습니다.</p>
    <table class="shortcuts">${rows.map(([a, k]) => `<tr><td>${a}</td><td><kbd>${p}+${k}</kbd></td></tr>`).join('')}</table>
    <h3 class="hsec">Mac 키 대응</h3>
    <table class="shortcuts"><tr><td>Windows 키</td><td><kbd>⌘ Command</kbd></td></tr><tr><td>Alt</td><td><kbd>⌥ Option</kbd></td></tr><tr><td>Ctrl</td><td><kbd>⌃ Control</kbd> (Ctrl ↔ ⌘ 바꾸기를 켜면 ⌘)</td></tr><tr><td>한/영</td><td><kbd>Ctrl+Space</kbd> 또는 <kbd>Caps Lock</kbd></td></tr></table>`;
}

// One setting explained for someone who isn't technical: what it is, when to change it,
// and what we recommend.
const hset = (name, what, when, rec) => `<div class="hset"><div class="hset-head"><b>${name}</b>${rec ? `<span class="hset-rec">${rec}</span>` : ''}</div>
  <p>${what}</p>${when ? `<p class="hset-when">${I('E946')}<span>${when}</span></p>` : ''}</div>`;
const hgroup = (id, icon, title, where, body) => `<section class="hgroup" id="hs-${id}"><h3 class="hsec">${I(icon)} ${title}</h3>
  ${where ? `<p class="hwhere">찾는 곳: ${where}</p>` : ''}${body}</section>`;

function helpSettings() {
  const jump = [['look', '모양'], ...(MAC ? [] : [['session', '접속 창']]), ['defaults', '새 연결 기본값'], ['edit', '연결 편집'], ['share', '공유'], ['app', '앱']]
    .map(([k, t]) => `<a href="#" class="hjump" data-help="settings#${k}">${t}</a>`).join('');
  return `<p class="hlead">Itda의 모든 설정을 하나씩 풀어서 설명합니다. 잘 모르겠다면 <b>추천</b> 표시대로 두면 됩니다. 처음 설치한 상태가 이미 추천 설정입니다.</p>
    <div class="hjumps">${jump}</div>
    ${hgroup('look', 'E790', '모양', path('설정', '모양'), `
      ${hset('테마', `Itda 화면을 밝게(라이트) 또는 어둡게(다크) 보여 줍니다. <b>시스템</b>은 ${MAC ? 'macOS' : 'Windows'} 설정을 따라갑니다.`, '밤에 눈이 부시면 다크로 바꾸세요.', '추천: 시스템')}
      ${hset('주소록 보기', '저장한 기기를 <b>카드</b>(큰 미리보기)로 볼지 <b>목록</b>(한 줄씩)으로 볼지 고릅니다.', '기기가 10대 이상이면 목록이 찾기 쉽습니다.', '추천: 카드')}
      ${hset('화면 크기', MAC ? 'Itda 창 안의 글자와 버튼 크기입니다. 다른 프로그램에는 영향이 없습니다. <kbd>⌘</kbd> + <kbd>+</kbd> / <kbd>−</kbd>로 바꾸고, <kbd>⌘</kbd>+<kbd>0</kbd>으로 원래대로 돌립니다.' : 'Itda 창 안의 글자와 버튼 크기입니다. 다른 프로그램에는 영향이 없습니다. <kbd>Ctrl</kbd> + <kbd>+</kbd> / <kbd>−</kbd> 또는 <kbd>Ctrl</kbd>+마우스 휠로도 바꾸고, <kbd>Ctrl</kbd>+<kbd>0</kbd>으로 원래대로 돌립니다.', '글자가 작아 보이면 110~125%로 올리세요.', '추천: 100%')}`)}
    ${MAC ? '' : hgroup('session', 'E7C4', '접속 창', path('설정', '접속 창'), `
      ${hset('툴바', '원격 화면 위쪽 가운데에 나오는 버튼 막대입니다. <b>자동 숨김</b>은 마우스를 화면 맨 위로 가져가면 나타납니다. <b>항상 표시</b>는 계속 보이고, <b>숨김</b>은 단축키로만 씁니다.', '툴바가 어디 있는지 헷갈리면 ‘항상 표시’로 두세요.', '추천: 자동 숨김')}
      ${hset('단축키 조합', 'Mac으로 보내지 않고 Itda가 직접 처리하는 키 조합입니다 (예: <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Enter</kbd> = 전체 화면).', 'Mac에서 Ctrl+Alt 단축키를 자주 쓴다면 Ctrl+Shift로 바꾸세요.', '추천: Ctrl+Alt')}`)}
    ${hgroup('defaults', 'E9A6', '새 연결 기본값', path('설정', '새 연결 기본값'), `
      <p class="hwhere">여기서 고른 값은 <b>앞으로 새로 추가하는 기기</b>에만 적용됩니다. 이미 있는 기기는 각 기기의 ‘연결 편집’에서 바꾸세요.</p>
      ${hset('화면 배율', '<b>창에 맞춤</b>은 Mac 화면 전체를 창 크기에 맞춰 줄여 보여 줍니다. <b>실제 크기</b>는 1:1로 선명하게 보여 주지만, 화면이 크면 스크롤해야 합니다.', '글자가 작아 읽기 어려우면 실제 크기로 보세요. 연결 중에 툴바에서 바로 바꿀 수 있습니다.', '추천: 창에 맞춤')}
      ${hset('클립보드 공유', MAC ? '이 Mac에서 복사(<kbd>⌘</kbd>+<kbd>C</kbd>)한 글자를 연결 창을 앞으로 가져올 때 원격 기기로 보내고, 원격 기기에서 복사한 글자는 이 Mac에 붙여 넣을 수 있습니다.' : '이 PC에서 복사(<kbd>Ctrl</kbd>+<kbd>C</kbd>)한 글자를 Mac에서 붙여 넣고, 그 반대도 됩니다.', '비밀번호를 복사하는 일이 많고 다른 사람이 Mac을 보고 있다면 끄세요.', '추천: 켬')}
      ${hset('자동 재연결', 'Wi-Fi가 잠깐 끊기거나 Mac이 잠들었다 깨면 자동으로 다시 연결합니다.', '', '추천: 켬')}
      ${MAC ? '' : hset('Ctrl ↔ ⌘ Cmd 바꾸기', 'Mac은 복사가 <kbd>⌘</kbd>+<kbd>C</kbd>, Windows는 <kbd>Ctrl</kbd>+<kbd>C</kbd>입니다. 켜면 평소처럼 <kbd>Ctrl</kbd>+<kbd>C</kbd>/<kbd>V</kbd>를 눌러도 Mac에서 복사·붙여넣기가 됩니다.', 'Windows에 익숙하다면 켜세요. Mac 키보드 배치 그대로가 편하다면 끄세요 (그때는 Windows 키가 ⌘입니다).', '추천: 켬 (Windows에 익숙한 경우)')}
      ${MAC ? '' : hset('한/영 키', 'Mac에서 한글/영어를 바꾸는 키가 무엇인지 맞춥니다. Mac의 <b>시스템 설정 → 키보드 → 입력 소스</b>에서 쓰는 단축키와 같게 고르세요.', '한/영 키를 눌러도 한글이 안 되면 다른 쪽으로 바꿔 보세요.', '추천: Ctrl+Space')}`)}
    ${hgroup('edit', 'E70F', '연결 편집 (기기마다)', '기기 카드의 ⋯ → 편집', `
      ${hset('종류', '연결할 기기가 무엇인지 고릅니다. 고른 종류에 맞게 아래 입력 칸과 안내가 바뀝니다.<br><b>Mac</b> · <b>Windows · Itda</b>(그 PC에도 Itda가 있을 때) · <b>Windows 원격 데스크톱</b>(Windows Pro) · <b>Linux · 기타 VNC</b>', '')}
      ${hset('주소', '기기를 찾아가는 이름이나 번호입니다. 예: <code>mac-mini.local</code>, <code>192.168.0.10</code>. 대부분 <b>주변 기기</b>에서 ‘추가’를 누르면 자동으로 채워집니다.', '', '')}
      ${hset('로그인 / 계정', '그 기기에 들어갈 때 쓰는 이름과 암호입니다. 여러 기기에 같은 이름·암호를 쓴다면 <b>계정</b>을 만들어 고르면, 암호를 바꿀 때 한 번만 고치면 됩니다.', '', '')}
      ${hset('암호 저장', `켜 두면 다음부터 암호를 묻지 않습니다. 암호는 ${MAC ? '이 Mac의 <b>키체인</b>에' : '이 PC의 <b>Windows 자격 증명 관리자</b>에 암호화되어'} 저장되고, 파일이나 다른 기기로 나가지 않습니다.`, '여러 사람이 같이 쓰는 PC라면 끄세요.', '추천: 켬')}
      ${hset('SSH로 암호화해서 연결', '화면과 키보드 입력을 암호화해서 보냅니다. 상대 Mac의 <b>원격 로그인</b>이 켜져 있어야 합니다.', '카페·회사 Wi-Fi처럼 남과 같이 쓰는 네트워크나 인터넷으로 연결할 때 켜세요. 집에서만 쓴다면 꺼도 됩니다.', '추천: 밖에서 쓸 때 켬')}
      ${hset('화면 탭', `이 기기만의 시작 방식: 실제 크기로 시작, 전체 화면으로 시작, <b>창 크기에 맞춰 해상도 바꾸기</b>(Windows 원격 데스크톱의 해상도가 창을 따라감), <b>보기 전용</b>(내 마우스·키보드를 보내지 않음)${MAC ? '' : ', 통계 표시'}.`, '남의 화면을 지켜보기만 할 때는 보기 전용을 켜면 실수로 클릭하지 않습니다.', '')}
      ${MAC ? hset('연결 탭', '<b>그룹</b>(예: 집, 회사)으로 묶고 클립보드·자동 재연결을 기기마다 켜고 끕니다.', '', '') : hset('키보드 탭 · 연결 탭', 'Ctrl↔⌘, 한/영 키를 이 기기에만 다르게 둘 수 있습니다. 연결 탭에서는 <b>그룹</b>(예: 집, 회사)으로 묶고 클립보드·자동 재연결을 기기마다 켜고 끕니다.', '', '')}`)}
    ${hgroup('share', 'E72D', '공유 (다른 기기에서 보기)', path('왼쪽', '공유'), `
      ${hset('웹 뷰어', `${HERE}가 작은 웹사이트가 되어, 휴대폰·태블릿·${MAC ? 'PC' : 'Mac'}의 <b>브라우저</b>로 ${MAC ? '저장된 기기를' : '이 PC와 저장된 기기를'} 볼 수 있게 합니다. 화면에 나오는 <b>QR 코드</b>를 휴대폰 카메라로 찍으면 열립니다.`, '휴대폰으로 볼 일이 없다면 꺼 두세요.', '')}
      ${hset('접속 PIN', '웹 뷰어에 들어갈 때 넣는 6자리 숫자입니다. 다른 사람에게 알려 주지 마세요. 5번 틀리면 1분간 막힙니다.', '누군가 PIN을 알게 된 것 같으면 <b>새 PIN</b>을 누르세요. 들어와 있던 기기가 모두 로그아웃됩니다.', '')}
      ${hset('HTTPS 주소 만들기 (Tailscale)', 'Tailscale을 쓰면 <code>https://…ts.net</code> 주소를 만들어, 집 밖에서도 안전하게 열고 휴대폰에 앱처럼 설치할 수 있습니다.', 'Tailscale 관리 화면에서 ‘HTTPS Certificates’를 한 번 켜야 합니다.', '')}
      ${MAC ? hset('이 Mac 화면 공유', 'macOS의 <b>시스템 설정 → 일반 → 공유 → 화면 공유</b>입니다. 켜면 다른 기기의 Itda와 화면 공유 앱이 이 Mac의 계정 이름과 암호로 들어옵니다. Itda의 공유 페이지에서 설정을 바로 열 수 있습니다.', '쓰지 않을 때는 꺼 두는 것이 안전합니다.', '')
        + hset('주변에 이 Mac 알리기', 'Itda가 켜져 있으면 같은 Wi-Fi의 다른 Itda ‘주변 기기’에 이 Mac이 바로 나타납니다. 화면 공유가 꺼져 있으면 그렇다고 함께 알려 줍니다. 이름과 켜짐 여부만 알리고 주소·암호는 알리지 않습니다.', '남과 같이 쓰는 Wi-Fi에서는 끄세요. (설정 → 공유)', '추천: 켬')
        + hset('포트', '기기끼리 통하는 ‘문 번호’입니다. 웹 뷰어는 8780이 기본입니다.', '‘포트를 열 수 없습니다’라는 오류가 날 때만 다른 숫자(예: 8781)로 바꾸세요.', '추천: 그대로') : `
      ${hset('이 PC 공유', '이 PC의 화면을 다른 기기에서 보고 조작할 수 있게 합니다. 다른 PC의 Itda, Mac의 화면 공유 앱, 그리고 웹 뷰어 목록의 ‘이 PC’로 들어옵니다.', '쓰지 않을 때는 꺼 두는 것이 안전합니다.', '')}
      ${hset('공유 암호', '다른 기기가 이 PC에 들어올 때 넣는 8글자 암호입니다 (웹 뷰어는 PIN으로 들어오므로 필요 없습니다).', '', '')}
      ${hset('연결할 때마다 묻기', '누가 이 PC에 들어오려 하면 이 PC 화면에 ‘허용할까요?’ 창이 뜹니다. 여기 앉은 사람이 ‘예’를 눌러야 연결됩니다.', '혼자 쓰는 PC를 밖에서 열려면 끄세요. 그때는 암호만 맞으면 바로 들어옵니다.', '추천: 켬')}
      ${hset('Mac의 ⌘을 Ctrl로', 'Mac에서 이 PC에 연결했을 때 ⌘C·⌘V로 복사·붙여넣기가 되게 합니다.', '', '추천: 켬')}
      ${hset('보기만 허용', '들어온 사람이 화면을 보기만 하고 마우스·키보드는 쓰지 못하게 합니다.', '도움을 받는 게 아니라 화면만 보여 줄 때 켜세요.', '')}
      ${hset('주변에 이 PC 알리기', 'Itda가 켜져 있으면 같은 Wi-Fi의 다른 Itda ‘주변 기기’에 이 PC가 바로 나타납니다. 공유가 꺼져 있으면 ‘공유가 꺼져 있습니다’라고 함께 알려 줍니다. 이름과 켜짐 여부만 알리고 주소·암호는 알리지 않습니다.', '회사·카페 Wi-Fi처럼 남과 같이 쓰는 네트워크에서는 끄세요. (설정 → 공유)', '추천: 켬')}
      ${hset('포트', '기기끼리 통하는 ‘문 번호’입니다. 웹 뷰어 8780, 이 PC 공유 5900이 기본입니다.', '‘포트를 열 수 없습니다’라는 오류가 날 때만 다른 숫자(예: 8781, 5901)로 바꾸세요.', '추천: 그대로')}`}`)}
    ${hgroup('app', 'E8A5', '앱', path('설정', '앱'), `
      ${MAC ? hset('창을 닫아도 계속 실행', '홈 창의 빨간 버튼을 눌러도 Itda는 Dock에서 계속 실행되고, 웹 뷰어와 열린 연결도 그대로입니다. Dock의 Itda 아이콘을 누르면 홈이 다시 열립니다.', '완전히 끄려면 ⌘Q (메뉴 막대 Itda → 종료).', '') : hset('창을 닫으면 알림 영역에서 계속 실행', '창의 ✕를 눌러도 Itda가 꺼지지 않고, 화면 오른쪽 아래 알림 영역(시계 옆)에 남습니다. 공유와 웹 뷰어도 계속 켜져 있습니다.', '완전히 끄려면 알림 영역의 Itda 아이콘을 오른쪽 클릭 → 종료.', '추천: 켬')}
      ${hset('알림', MAC ? '연결이 끊기거나 다시 연결될 때 알림 센터에 알림을 띄웁니다.' : '연결이 끊기거나 다시 연결될 때, 누가 이 PC를 보기 시작할 때 오른쪽 아래에 알림을 띄웁니다.', '', '추천: 켬')}
      ${hset('미리보기 저장', `마지막으로 본 화면을 작게 저장해 주소록 카드에 보여 줍니다. ${HERE}에만 저장됩니다.`, '화면에 민감한 내용이 많다면 끄세요.', '추천: 켬')}
      ${hset('시작 안내 · 로그', '<b>시작 안내</b>는 처음 실행할 때 나온 안내를 다시 보여 줍니다. <b>로그</b>는 문제가 생겼을 때 원인을 찾는 기록입니다 (암호는 기록되지 않습니다).', '문제를 알릴 때 로그의 마지막 부분을 함께 보내 주세요.', '')}`)}`;
}

function helpWords() {
  const w = (term, desc) => `<div class="hword"><b>${term}</b><p>${desc}</p></div>`;
  return `<p class="hlead">설정과 안내에 나오는 말들을 쉽게 풀었습니다.</p><div class="hwords">
    ${w('주소 (IP 주소)', '네트워크 안에서 기기를 찾아가는 번호입니다. 예: <code>192.168.0.10</code>. 이름으로 된 주소(<code>mac-mini.local</code>)도 같은 역할을 합니다.')}
    ${w('포트', '한 기기 안에서 프로그램마다 쓰는 ‘문 번호’입니다. 주소 뒤에 <code>:5900</code>처럼 붙입니다. 보통은 신경 쓰지 않아도 됩니다.')}
    ${w('같은 Wi-Fi (같은 네트워크)', '집이나 회사에서 같은 공유기에 연결된 상태입니다. 같은 Wi-Fi에 있으면 주변 기기에 자동으로 나타납니다.')}
    ${w('화면 공유 / VNC', '다른 컴퓨터의 화면을 보고 조작하는 방식입니다. Mac의 ‘화면 공유’와 Itda의 ‘이 PC 공유’가 VNC를 씁니다.')}
    ${w('원격 데스크톱 (RDP)', 'Windows Pro에 들어 있는 원격 접속 기능입니다. Windows Home에는 없으니, 그때는 Itda의 ‘이 PC 공유’를 쓰세요.')}
    ${w('SSH (원격 로그인)', '두 컴퓨터 사이를 암호화된 통로로 잇는 방법입니다. Mac에서 ‘원격 로그인’을 켜면 쓸 수 있고, Itda가 화면 공유를 이 통로로 보냅니다.')}
    ${w('Tailscale', '집 밖에서도 내 기기들을 같은 Wi-Fi에 있는 것처럼 이어 주는 무료 앱입니다. 모든 기기에 설치하고 같은 계정으로 로그인하면 됩니다.')}
    ${w('웹 뷰어', `앱을 깔지 않고 브라우저(Safari, Chrome)로 Itda를 쓰는 방법입니다. ${HERE}의 Itda가 화면을 브라우저로 전달합니다.`)}
    ${w('PIN', `웹 뷰어에 들어가는 6자리 숫자 암호입니다. ${HERE}의 Itda → 공유 페이지에서 확인합니다.`)}
    ${MAC ? w('방화벽', 'macOS가 다른 기기의 접속을 막는 문지기입니다. 켜져 있다면 Itda가 처음 웹 뷰어를 열 때 허용할지 묻습니다. <b>허용</b>을 누르세요.') : w('방화벽', 'Windows가 다른 기기의 접속을 막는 문지기입니다. 처음 공유를 켤 때 물어보면 <b>개인 네트워크</b>만 허용하세요.')}
    ${MAC ? w('키체인', 'macOS가 암호를 안전하게 보관하는 금고입니다. Itda는 암호를 여기에만 저장합니다.') : w('자격 증명 관리자', 'Windows가 암호를 안전하게 보관하는 금고입니다. Itda는 암호를 여기에만 저장합니다.')}
    ${w('Microsoft 계정', 'Windows에 이메일 주소(outlook.com, hotmail.com 등)로 로그인하는 계정입니다. 원격 데스크톱에서는 이 <b>이메일 주소와 Microsoft 계정 암호</b>로 로그인합니다. 로그인 화면의 숫자 PIN은 그 PC 안에서만 쓰는 것이라 원격 접속에는 쓸 수 없습니다.')}
    ${w('계정 이름 (Mac 짧은 이름)', 'Mac에 로그인할 때 쓰는 이름입니다. Apple ID(이메일)가 아닙니다. Mac의 터미널에서 <code>whoami</code>를 입력하면 나옵니다.')}
    ${MAC ? w('Dock', '화면 아래의 앱 아이콘 줄입니다. 홈 창을 닫아도 Itda는 Dock에서 계속 실행되고, 아이콘을 누르면 다시 열립니다.') : w('알림 영역 (트레이)', '화면 오른쪽 아래 시계 옆의 작은 아이콘 모음입니다. 창을 닫아도 Itda가 여기서 계속 실행됩니다.')}
    ${w('PWA (앱처럼 설치)', '웹 뷰어를 휴대폰 홈 화면이나 Mac의 Dock에 아이콘으로 추가해, 앱처럼 전체 화면으로 여는 방법입니다.')}
  </div>`;
}

function renderHelp() {
  const t = S.helpTopic || 'start';
  const body = { start: helpStart, mac: helpMac, windows: helpWindows, linux: helpLinux, share: helpShare, phone: helpPhone, macpc: helpMacPc, assist: helpAssist, outside: helpOutside, accounts: helpAccounts, settings: helpSettings, words: helpWords, trouble: helpTrouble, keys: helpKeys }[t] || helpStart;
  const title = (TOPICS.find(([k]) => k === t) || TOPICS[0])[2];
  return `<div class="help">
    <nav class="hnav" aria-label="도움말 주제">${TOPICS.map(([k, icon, label, sec], i) => `${sec === 'ref' && TOPICS[i - 1][3] !== 'ref' ? '<div class="hnav-sec">참고</div>' : ''}<button class="hnav-item${k === t ? ' on' : ''}" data-help="${k}">${I(icon)}<span>${NAV_SHORT[k] || label}</span></button>`).join('')}</nav>
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
