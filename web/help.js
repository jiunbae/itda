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

// ---------------------------------------------------------------- topics

const TOPICS = [
  ['start', 'E80F', '시작하기'],
  ['mac', 'E7F4', 'Mac에 연결'],
  ['windows', 'E7F8', 'Windows PC에 연결'],
  ['phone', 'E8EA', '휴대폰·Mac 브라우저로 보기'],
  ['outside', 'E774', '밖에서 연결 (Tailscale)'],
  ['accounts', 'E77B', '계정과 SSH'],
  ['settings', 'E713', '설정 자세히'],
  ['words', 'E82D', '용어 풀이'],
  ['trouble', 'E90F', '문제 해결'],
  ['keys', 'E765', '단축키'],
];

function helpStart() {
  const tile = (t, icon, title, sub) => `<button class="htile" data-help="${t}">${I(icon)}<b>${title}</b><span>${sub}</span></button>`;
  return `<div class="hhero"><h2>무엇을 연결할까요?</h2><p>어느 기기든 순서는 같습니다 — <b>① 상대 기기에서 공유를 켜고</b>, <b>② Itda에 추가해서</b>, <b>③ 연결</b>합니다.</p></div>
    <div class="htiles">
      ${tile('mac', 'E7F4', 'Mac', '화면 공유를 켜면 Itda에서 바로 연결됩니다. 5분이면 됩니다.')}
      ${tile('windows', 'E7F8', 'Windows PC', '상대 PC에 Itda를 설치해 ‘이 PC 공유’를 켜거나, 원격 데스크톱을 씁니다.')}
      ${tile('phone', 'E8EA', '휴대폰·Mac에서 보기', 'iPhone·Android·Mac의 브라우저로 Mac과 Windows PC를 열고, 앱처럼 설치해 씁니다.')}
      ${tile('outside', 'E774', '밖에서 연결', 'Tailscale을 쓰면 집·회사 밖에서도 그대로 연결됩니다.')}
      ${tile('accounts', 'E77B', '계정과 SSH', '계정 하나로 여러 Mac을 관리하고, 연결을 암호화합니다.')}
      ${tile('trouble', 'E90F', '문제 해결', '연결이 안 될 때 확인할 것')}
    </div>
    ${shot('itda-home', 'Itda 홈 — 저장된 연결이 카드로 보입니다. 초록 점은 지금 연결할 수 있다는 뜻입니다.')}`;
}

function helpMac() {
  return `<p class="hlead">Mac에는 화면 공유가 기본으로 들어 있습니다. 한 번만 켜 두면 됩니다.</p><ol class="hsteps">
    ${step(1, '화면 공유 켜기', `<p>Mac에서 ${path('Apple 메뉴', '시스템 설정', '일반', '공유')} 로 가서 <b>화면 공유</b>를 켭니다 ${mark(1)}.</p>
       ${note('<b>원격 관리</b>는 끄세요. 둘 다 켜져 있으면 화면 공유가 동작하지 않습니다.', 'warn')}`, macWindow('general', '공유', macSharing('screen')))}
    ${step(2, '연결할 계정 허용하기', `<p>화면 공유 옆 ⓘ ${mark(2)} 를 누르고, <b>다음 사용자에 대해 접근 허용</b>에 내 계정이 있는지 확인합니다 ${mark(1)}.</p>
       <p>Itda에는 이 <b>Mac 계정 이름(짧은 이름)</b>과 Mac 로그인 암호를 넣습니다. Apple ID가 아닙니다.</p>
       ${note('계정 이름이 헷갈리면 Mac의 터미널에서 <code>whoami</code>를 입력해 보세요.')}`, macWindow('general', '공유', macSharing('screen-i'), macScreenSheet()))}
    ${step(3, '주소 확인 (같은 Wi-Fi라면 건너뛰어도 됩니다)', `<p>같은 공유 화면 아래의 <b>로컬 호스트 이름</b> ${mark(3)} 이 Mac의 주소입니다 (예: <code>mac-mini.local</code>).</p>
       <p>같은 Wi-Fi에 있으면 Itda의 <b>주변 기기</b>에 자동으로 나타나니 주소를 몰라도 됩니다.</p>`, macWindow('general', '공유', macSharing('name')))}
    ${step(4, 'Itda에 추가하고 연결하기', `<p>Itda 왼쪽의 <b>주변 기기</b>에서 Mac 옆 <b>추가</b>를 누르고, 계정 이름과 암호를 넣은 뒤 <b>저장 후 연결</b>을 누릅니다.</p>
       <p>여러 Mac에 같은 계정을 쓴다면 먼저 <a href="#" data-help="accounts">계정</a>을 만들어 두면 편합니다.</p>`, shot('itda-nearby', '주변 기기 — 같은 Wi-Fi의 Mac(위)과 Tailscale의 기기(아래)'))}
    ${step(5, '밖에서도 쓸 거라면 — 원격 로그인 켜기 (선택)', `<p>같은 공유 화면에서 <b>원격 로그인</b> ${mark(1)} 을 켜면, Itda가 SSH로 연결을 암호화할 수 있습니다. 연결 편집에서 <b>SSH로 암호화해서 연결</b>을 켜세요.</p>`, macWindow('general', '공유', macSharing('ssh')))}
    ${step(6, '잠자는 Mac에도 연결되게 (선택)', `<p>${path('시스템 설정', '에너지')} 에서 <b>잠자기 방지</b> ${mark(1)} 와 <b>네트워크 접근으로 깨우기</b> ${mark(2)} 를 켭니다. MacBook은 ${path('배터리', '옵션')} 에 있습니다.</p>`, macWindow('energy', '에너지', macEnergy()))}
  </ol>`;
}

function helpWindows() {
  return `<p class="hlead">Windows PC는 두 가지 방법으로 연결합니다. 잘 모르겠다면 <b>방법 A</b>를 쓰세요 — 모든 Windows에서 됩니다.</p>
    <div class="hcompare">
      <div><b>A. Itda ‘이 PC 공유’</b><span>Windows 10/11 모든 버전 · 휴대폰 웹 뷰어·Mac 화면 공유 앱에서도 접속 · 연결할 때마다 허락</span></div>
      <div><b>B. 원격 데스크톱 (RDP)</b><span>Windows Pro/Enterprise만 · 이 PC에서는 Windows 원격 데스크톱 앱으로, 휴대폰·Mac에서는 웹 뷰어로 열림</span></div>
    </div>
    <h3 class="hsec">A. Itda로 이 PC 공유하기</h3><ol class="hsteps">
    ${step(1, '연결 받을 PC에 Itda를 설치하고 공유 켜기', `<p>그 PC의 Itda에서 ${path('공유', '이 PC 공유')} 를 켭니다. 주소와 <b>공유 암호</b>가 나타납니다.</p>
       <p><b>연결할 때마다 묻기</b>는 켜 두세요 — 누가 연결하려 하면 그 PC 화면에 허락 창이 뜹니다.</p>`, shot('itda-share', '공유 → 이 PC 공유 — 주소, 공유 암호, 지금 보고 있는 사람'))}
    ${step(2, '방화벽 허용', `<p>처음 켤 때 Windows가 묻습니다. <b>개인 네트워크</b> ${mark(1)} 만 체크하고 <b>액세스 허용</b> ${mark(2)} 을 누르세요.</p>`, winFirewall())}
    ${step(3, '내 PC의 Itda에서 추가', `<p>내 Itda의 <b>주변 기기</b>에 그 PC가 <b>Windows · Itda</b>로 나타납니다. <b>추가</b>를 누르고 <b>계정 이름은 비운 채</b> 공유 암호만 넣으면 됩니다.</p>
       ${note('다른 네트워크라면 그 PC에도 Tailscale을 설치하세요. 주소는 Tailscale 이름(예: living-pc.…ts.net)을 씁니다.')}`, shot('itda-nearby', '주변 기기의 STUDY-PC — Windows · Itda'))}
    ${step(4, '상대 PC에서 허락하기', `<p>연결하면 상대 PC에 이런 창이 뜹니다. <b>예</b> ${mark(1)} 를 누르면 연결됩니다. 보는 동안은 알림 영역에 표시되고, <b>모두 끊기</b>로 언제든 끊을 수 있습니다.</p>
       ${note('Windows 잠금 화면, 관리자 권한 확인 창(UAC), 관리자 권한으로 실행된 창은 Windows 보안 때문에 원격으로 조작할 수 없습니다.', 'warn')}`, winAllowPrompt())}
    </ol>
    <h3 class="hsec">B. 원격 데스크톱 쓰기 (Windows Pro)</h3><ol class="hsteps">
    ${step(1, '원격 데스크톱 켜기', `<p>연결 받을 PC에서 ${path('설정', '시스템', '원격 데스크톱')} 을 켭니다 ${mark(1)}. 아래의 <b>PC 이름</b> ${mark(2)} 을 적어 두세요.</p>
       ${note('Windows Home 에디션에는 원격 데스크톱이 없습니다. 방법 A를 쓰세요.', 'warn')}`, winRemoteDesktop())}
    ${step(2, 'Itda에서 종류를 ‘Windows 원격 데스크톱’으로', `<p>연결 편집 맨 위 <b>종류</b>에서 <b>Windows 원격 데스크톱 (RDP)</b>를 고르고, 주소에 PC 이름(또는 IP·Tailscale 주소), 계정에 그 PC의 Windows 계정을 넣습니다.</p>
       <p>연결하면 Itda가 계정을 넘겨 주고 Windows 원격 데스크톱 앱이 열립니다. <a href="#" data-help="phone">웹 뷰어</a>에서 열면 이 PC의 Itda가 직접 RDP로 접속해 휴대폰·Mac 브라우저에 화면을 보여 줍니다.</p>`, shot('itda-edit', '연결 편집 — 맨 위 ‘종류’에서 고릅니다'))}
    </ol>`;
}

function helpPhone() {
  return `<p class="hlead">따로 설치할 앱이 없습니다. 이 PC의 Itda가 작은 웹 서버가 되어 iPhone·Android·Mac·Linux의 브라우저에 Mac, Windows PC, 이 PC의 화면을 보여 줍니다. 암호는 이 PC에만 있고 브라우저로 가지 않습니다.</p>
    ${note('<b>Mac에서 다른 기기를 열 때</b>도 이 방법을 씁니다. Mac용 Itda 앱 대신, Mac의 Safari·Chrome으로 웹 뷰어를 열어 Dock에 추가하세요 (5단계).')}<ol class="hsteps">
    ${step(1, '이 PC에서 웹 뷰어 켜기', `<p>${path('공유', '웹 뷰어')} 를 켜면 QR 코드와 6자리 <b>접속 PIN</b>이 나옵니다. 처음 켤 때 방화벽이 물으면 <b>개인 네트워크</b>를 허용하세요.</p>
       <p>같은 Wi-Fi면 ‘같은 Wi-Fi’ QR을, 밖에서 쓸 거면 ‘Tailscale’ QR을 고릅니다.</p>`, shot('itda-web', '공유 → 웹 뷰어 — QR, 주소, PIN'))}
    ${step(2, '휴대폰 카메라로 QR 찍기 → PIN 입력', '<p>iPhone 기본 카메라로 QR을 비추면 링크가 뜹니다. 열어서 PIN을 넣습니다. 12시간 동안 다시 묻지 않습니다.</p>', shot('phone-login', 'PIN 입력', 'phone'))}
    ${step(3, '기기 고르기', `<p>PC의 Itda에 저장된 기기가 보입니다. <b>이 PC 공유</b>가 켜져 있으면 맨 앞에 이 PC도 나옵니다 — PIN으로 이미 확인했으니 허락 창 없이 바로 열립니다.</p>
       <p>브라우저에서도 <b>기기 추가</b>, 카드의 ✎ 로 <b>편집·삭제</b>, <b>계정</b> 탭에서 계정 관리를 할 수 있고, PC의 Itda에 바로 저장됩니다. 암호가 없는 기기는 누르면 암호를 물어봅니다.</p>`, shot('phone-list', '기기 목록', 'phone') + shot('phone-edit', '브라우저에서 기기 편집', 'phone'))}
    ${step(4, '보고 조작하기', `<p><b>탭</b> = 클릭, <b>두 손가락 탭</b> = 오른쪽 클릭, <b>두 손가락 끌기</b> = 스크롤입니다. 위쪽 도구 막대:</p>
       <ul class="hkeys"><li><b>‹</b> 목록으로</li><li><b>⌨</b> 휴대폰 키보드 (한글 입력 가능)</li><li><b>⌘</b> Mac 특수 키 — ⌘·⌥·⌃·⇧을 눌러 둔 채로 다음 키와 함께 보내기, ⌘Space·⌘Tab 등</li><li><b>📋</b> 클립보드 주고받기</li><li><b>⤢</b> 화면에 맞춤 / 실제 크기</li><li><b>👁</b> 보기 전용</li></ul>`, shot('phone-view', 'Mac 화면', 'phone') + shot('phone-keys', 'Mac 특수 키', 'phone'))}
    ${step(5, '앱처럼 설치하기', `<p><b>iPhone·iPad</b>: Safari의 공유 버튼 ${mark(1)} → <b>홈 화면에 추가</b> ${mark(2)} 를 누르면 Itda 아이콘이 생기고 전체 화면으로 열립니다.</p>
       <p><b>Mac</b>: Safari에서 ${path('파일', 'Dock에 추가')}, Chrome·Edge에서는 웹 뷰어 위쪽의 <b>앱으로 설치</b>를 누릅니다. <b>Android</b>: Chrome의 <b>앱으로 설치</b>.</p>
       ${note('Chrome·Edge·Android의 앱 설치는 <b>https://</b> 주소에서만 됩니다. 설정의 Tailscale 칸에서 <b>HTTPS 주소 만들기</b>를 누르고 그 주소(https://…ts.net)로 여세요.')}`, iosShare())}
  </ol>${note('브라우저와 PC 사이는 일반 HTTP입니다. 같은 Wi-Fi나 Tailscale에서만 쓰고, 공유기에서 이 포트를 인터넷에 열지 마세요.', 'warn')}`;
}

function helpOutside() {
  return `<p class="hlead">집·회사 밖에서 연결하는 가장 쉽고 안전한 방법은 <b>Tailscale</b>입니다. 기기들을 같은 계정으로 로그인해 두면, 어디에 있든 같은 네트워크처럼 연결됩니다. 공유기 설정은 필요 없습니다.</p><ol class="hsteps">
    ${step(1, '모든 기기에 Tailscale 설치·로그인', `<p>이 PC, 연결할 Mac·PC, 휴대폰에 <a href="https://tailscale.com/download" data-ext>Tailscale</a>을 설치하고 <b>같은 계정</b>으로 로그인합니다. 개인 사용은 무료입니다.</p>`)}
    ${step(2, 'Itda에서 Tailscale 기기 추가', '<p><b>주변 기기</b> 아래쪽 <b>Tailscale — 어디서나</b>에 기기들이 나타납니다. 여기서 추가한 연결은 밖에서도 그대로 됩니다.</p>', shot('itda-nearby', 'Tailscale — 어디서나'))}
    ${step(3, '휴대폰으로 볼 때는 Tailscale QR', '<p>웹 뷰어 설정의 QR 탭을 <b>Tailscale</b>로 바꿔 찍으면, 휴대폰이 밖에 있어도 열립니다 (휴대폰의 Tailscale 앱이 켜져 있어야 합니다).</p>', shot('itda-web', 'QR을 Tailscale로'))}
    ${step(4, 'https 주소 (선택)', `<p>Tailscale 관리 콘솔 ${path('DNS', 'HTTPS Certificates')} 를 켠 뒤, Itda에서 <b>HTTPS 주소 만들기</b>를 누르면 <code>https://내PC.…ts.net/</code> 주소가 생깁니다.</p>`)}
  </ol>
  <h3 class="hsec">Tailscale 없이 하려면</h3>
  <p>Mac에서 <b>원격 로그인</b>을 켜고, 공유기에서 <b>SSH 포트(22)만</b> Mac으로 전달한 뒤 연결 편집에서 <b>SSH로 암호화해서 연결</b>을 켜세요. 화면 공유 포트(5900)나 웹 뷰어 포트를 인터넷에 직접 열면 안 됩니다.</p>`;
}

function helpAccounts() {
  return `<p class="hlead">대부분 Mac 계정 하나로 여러 기기를 씁니다. <b>계정</b>을 만들어 두면 연결을 추가할 때 고르기만 하면 되고, 암호를 바꿔도 한 곳만 고치면 됩니다.</p><ol class="hsteps">
    ${step(1, '계정 만들기', '<p>왼쪽 <b>계정</b> → <b>새 계정</b>. 이름, Mac 계정 이름, 암호를 넣습니다. SSH 키를 쓴다면 키 파일과 키 암호도 넣습니다.</p>', shot('itda-account-edit', '계정 편집 — SSH 키(선택)'))}
    ${step(2, '연결에서 계정 고르기', '<p>연결 편집의 <b>로그인</b>에서 계정을 고릅니다. 그 자리에서 <b>새 계정 만들기…</b>도 됩니다.</p>', shot('itda-edit', '연결 편집 — 로그인'))}
    ${step(3, '이미 만든 연결 묶기', '<p>같은 계정 이름을 쓰는 연결이 여러 개면 계정 화면에 <b>계정으로 묶기</b>가 뜹니다. 누르면 저장된 암호까지 한 계정으로 옮겨집니다.</p>', shot('itda-accounts', '계정 화면'))}
  </ol>
  <h3 class="hsec">SSH로 암호화</h3>
  <p>일반 화면 공유(VNC)는 암호화되지 않습니다. 공용 Wi-Fi나 인터넷으로 연결한다면 Mac의 <b>원격 로그인</b>을 켜고 연결 편집에서 <b>SSH로 암호화해서 연결</b>을 켜세요. Windows에 기본으로 있는 OpenSSH를 쓰므로 따로 설치할 것이 없습니다. 카드에 🔒가 보이면 암호화된 연결입니다.</p>`;
}

const FAQ = [
  ['‘Mac이 연결을 거부했습니다’', 'Mac의 화면 공유가 꺼져 있습니다. <b>시스템 설정 → 일반 → 공유 → 화면 공유</b>를 켜세요. 원격 관리가 켜져 있다면 끄세요.', 'mac'],
  ['‘로그인하지 못했습니다’', 'Mac <b>계정 이름(짧은 이름)</b>과 로그인 암호인지 확인하세요 (Apple ID 아님). 화면 공유 ⓘ의 ‘접근 허용’에 그 계정이 있어야 합니다. 여러 번 틀리면 Mac이 잠시 막으니 1분 뒤 다시 하세요.', 'mac'],
  ['‘응답하지 않습니다’ / 회색 점', 'Mac이 꺼져 있거나 잠자기 중이거나, 다른 네트워크입니다. 밖이라면 Tailscale을 켜세요. 자주 잠든다면 <b>에너지 → 네트워크 접근으로 깨우기</b>를 켜세요.', 'outside'],
  ['주변 기기에 아무것도 안 보여요', '같은 Wi-Fi인지, 상대 기기에서 공유가 켜져 있는지 확인하세요. 회사·학교 Wi-Fi는 기기끼리 찾는 것을 막기도 합니다. 그럴 땐 주소로 직접 추가하거나 Tailscale을 쓰세요.', 'start'],
  ['한/영 전환이 안 돼요', '설정 → 새 연결 기본값(또는 연결 편집 → 키보드)에서 한/영 키를 Mac의 입력 소스 단축키(Ctrl+Space 또는 Caps Lock)와 맞추세요.', 'keys'],
  ['⌘C/⌘V가 Ctrl로 안 돼요', '연결 편집 → 키보드 → <b>Ctrl ↔ ⌘ 바꾸기</b>를 켜면 Ctrl+C/V가 Mac의 ⌘C/V로 동작합니다. Windows 키는 항상 ⌘입니다.', 'keys'],
  ['휴대폰에서 웹 뷰어가 안 열려요', '휴대폰이 같은 Wi-Fi인지(또는 Tailscale이 켜져 있는지), PC의 방화벽 창에서 ‘개인 네트워크’를 허용했는지 확인하세요. 설정에서 웹 뷰어 포트를 바꿔 볼 수도 있습니다.', 'phone'],
  ['Windows PC에 연결은 되는데 조작이 안 돼요', '상대 PC가 잠금 화면이거나 관리자 권한 창(UAC 등)이 앞에 있으면 Windows가 입력을 막습니다. 상대 PC의 ‘보기만 허용’이 꺼져 있는지도 확인하세요.', 'windows'],
];

function helpTrouble() {
  return `<div class="faq">${FAQ.map(([q, a, t]) => `<details><summary>${q}</summary><p>${a}</p><a href="#" data-help="${t}">관련 안내 보기 →</a></details>`).join('')}</div>
    <p class="hnote info">${I('E946')}<span>계속 안 되면 ${path('설정', '정보', '로그')} 를 열어 마지막 줄들을 확인하세요. 연결마다 별도 로그가 남습니다.</span></p>`;
}

function helpKeys() {
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
  const jump = [['look', '모양'], ['session', '접속 창'], ['defaults', '새 연결 기본값'], ['edit', '연결 편집'], ['share', '공유'], ['app', '앱']]
    .map(([k, t]) => `<a href="#" class="hjump" data-help="settings#${k}">${t}</a>`).join('');
  return `<p class="hlead">Itda의 모든 설정을 하나씩 풀어서 설명합니다. 잘 모르겠다면 <b>추천</b> 표시대로 두면 됩니다. 처음 설치한 상태가 이미 추천 설정입니다.</p>
    <div class="hjumps">${jump}</div>
    ${hgroup('look', 'E790', '모양', path('설정', '모양'), `
      ${hset('테마', 'Itda 화면을 밝게(라이트) 또는 어둡게(다크) 보여 줍니다. <b>시스템</b>은 Windows 설정을 따라갑니다.', '밤에 눈이 부시면 다크로 바꾸세요.', '추천: 시스템')}
      ${hset('주소록 보기', '저장한 기기를 <b>카드</b>(큰 미리보기)로 볼지 <b>목록</b>(한 줄씩)으로 볼지 고릅니다.', '기기가 10대 이상이면 목록이 찾기 쉽습니다.', '추천: 카드')}
      ${hset('화면 크기', 'Itda 창 안의 글자와 버튼 크기입니다. 다른 프로그램에는 영향이 없습니다. <kbd>Ctrl</kbd> + <kbd>+</kbd> / <kbd>−</kbd> 또는 <kbd>Ctrl</kbd>+마우스 휠로도 바꾸고, <kbd>Ctrl</kbd>+<kbd>0</kbd>으로 원래대로 돌립니다.', '글자가 작아 보이면 110~125%로 올리세요.', '추천: 100%')}`)}
    ${hgroup('session', 'E7C4', '접속 창', path('설정', '접속 창'), `
      ${hset('툴바', '원격 화면 위쪽 가운데에 나오는 버튼 막대입니다. <b>자동 숨김</b>은 마우스를 화면 맨 위로 가져가면 나타납니다. <b>항상 표시</b>는 계속 보이고, <b>숨김</b>은 단축키로만 씁니다.', '툴바가 어디 있는지 헷갈리면 ‘항상 표시’로 두세요.', '추천: 자동 숨김')}
      ${hset('단축키 조합', 'Mac으로 보내지 않고 Itda가 직접 처리하는 키 조합입니다 (예: <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Enter</kbd> = 전체 화면).', 'Mac에서 Ctrl+Alt 단축키를 자주 쓴다면 Ctrl+Shift로 바꾸세요.', '추천: Ctrl+Alt')}`)}
    ${hgroup('defaults', 'E9A6', '새 연결 기본값', path('설정', '새 연결 기본값'), `
      <p class="hwhere">여기서 고른 값은 <b>앞으로 새로 추가하는 기기</b>에만 적용됩니다. 이미 있는 기기는 각 기기의 ‘연결 편집’에서 바꾸세요.</p>
      ${hset('화면 배율', '<b>창에 맞춤</b>은 Mac 화면 전체를 창 크기에 맞춰 줄여 보여 줍니다. <b>실제 크기</b>는 1:1로 선명하게 보여 주지만, 화면이 크면 스크롤해야 합니다.', '글자가 작아 읽기 어려우면 실제 크기로 보세요. 연결 중에 툴바에서 바로 바꿀 수 있습니다.', '추천: 창에 맞춤')}
      ${hset('클립보드 공유', '이 PC에서 복사(<kbd>Ctrl</kbd>+<kbd>C</kbd>)한 글자를 Mac에서 붙여 넣고, 그 반대도 됩니다.', '비밀번호를 복사하는 일이 많고 다른 사람이 Mac을 보고 있다면 끄세요.', '추천: 켬')}
      ${hset('자동 재연결', 'Wi-Fi가 잠깐 끊기거나 Mac이 잠들었다 깨면 자동으로 다시 연결합니다.', '', '추천: 켬')}
      ${hset('Ctrl ↔ ⌘ Cmd 바꾸기', 'Mac은 복사가 <kbd>⌘</kbd>+<kbd>C</kbd>, Windows는 <kbd>Ctrl</kbd>+<kbd>C</kbd>입니다. 켜면 평소처럼 <kbd>Ctrl</kbd>+<kbd>C</kbd>/<kbd>V</kbd>를 눌러도 Mac에서 복사·붙여넣기가 됩니다.', 'Windows에 익숙하다면 켜세요. Mac 키보드 배치 그대로가 편하다면 끄세요 (그때는 Windows 키가 ⌘입니다).', '추천: 켬 (Windows에 익숙한 경우)')}
      ${hset('한/영 키', 'Mac에서 한글/영어를 바꾸는 키가 무엇인지 맞춥니다. Mac의 <b>시스템 설정 → 키보드 → 입력 소스</b>에서 쓰는 단축키와 같게 고르세요.', '한/영 키를 눌러도 한글이 안 되면 다른 쪽으로 바꿔 보세요.', '추천: Ctrl+Space')}`)}
    ${hgroup('edit', 'E70F', '연결 편집 (기기마다)', '기기 카드의 ⋯ → 편집', `
      ${hset('종류', '연결할 기기가 무엇인지 고릅니다. 고른 종류에 맞게 아래 입력 칸과 안내가 바뀝니다.<br><b>Mac</b> · <b>Windows · Itda</b>(그 PC에도 Itda가 있을 때) · <b>Windows 원격 데스크톱</b>(Windows Pro) · <b>Linux · 기타 VNC</b>', '')}
      ${hset('주소', '기기를 찾아가는 이름이나 번호입니다. 예: <code>mac-mini.local</code>, <code>192.168.0.10</code>. 대부분 <b>주변 기기</b>에서 ‘추가’를 누르면 자동으로 채워집니다.', '', '')}
      ${hset('로그인 / 계정', '그 기기에 들어갈 때 쓰는 이름과 암호입니다. 여러 기기에 같은 이름·암호를 쓴다면 <b>계정</b>을 만들어 고르면, 암호를 바꿀 때 한 번만 고치면 됩니다.', '', '')}
      ${hset('암호 저장', '켜 두면 다음부터 암호를 묻지 않습니다. 암호는 이 PC의 <b>Windows 자격 증명 관리자</b>에 암호화되어 저장되고, 파일이나 다른 기기로 나가지 않습니다.', '여러 사람이 같이 쓰는 PC라면 끄세요.', '추천: 켬')}
      ${hset('SSH로 암호화해서 연결', '화면과 키보드 입력을 암호화해서 보냅니다. 상대 Mac의 <b>원격 로그인</b>이 켜져 있어야 합니다.', '카페·회사 Wi-Fi처럼 남과 같이 쓰는 네트워크나 인터넷으로 연결할 때 켜세요. 집에서만 쓴다면 꺼도 됩니다.', '추천: 밖에서 쓸 때 켬')}
      ${hset('화면 탭', '이 기기만의 시작 방식: 실제 크기로 시작, 전체 화면으로 시작, <b>보기 전용</b>(내 마우스·키보드를 보내지 않음), 통계 표시.', '남의 화면을 지켜보기만 할 때는 보기 전용을 켜면 실수로 클릭하지 않습니다.', '')}
      ${hset('키보드 탭 · 연결 탭', 'Ctrl↔⌘, 한/영 키를 이 기기에만 다르게 둘 수 있습니다. 연결 탭에서는 <b>그룹</b>(예: 집, 회사)으로 묶고 클립보드·자동 재연결을 기기마다 켜고 끕니다.', '', '')}`)}
    ${hgroup('share', 'E72D', '공유 (다른 기기에서 보기)', path('왼쪽', '공유'), `
      ${hset('웹 뷰어', '이 PC가 작은 웹사이트가 되어, 휴대폰·태블릿·Mac의 <b>브라우저</b>로 이 PC와 저장된 기기를 볼 수 있게 합니다. 화면에 나오는 <b>QR 코드</b>를 휴대폰 카메라로 찍으면 열립니다.', '휴대폰으로 볼 일이 없다면 꺼 두세요.', '')}
      ${hset('접속 PIN', '웹 뷰어에 들어갈 때 넣는 6자리 숫자입니다. 다른 사람에게 알려 주지 마세요. 5번 틀리면 1분간 막힙니다.', '누군가 PIN을 알게 된 것 같으면 <b>새 PIN</b>을 누르세요. 들어와 있던 기기가 모두 로그아웃됩니다.', '')}
      ${hset('HTTPS 주소 만들기 (Tailscale)', 'Tailscale을 쓰면 <code>https://…ts.net</code> 주소를 만들어, 집 밖에서도 안전하게 열고 휴대폰에 앱처럼 설치할 수 있습니다.', 'Tailscale 관리 화면에서 ‘HTTPS Certificates’를 한 번 켜야 합니다.', '')}
      ${hset('이 PC 공유', '이 PC의 화면을 다른 기기에서 보고 조작할 수 있게 합니다. 다른 PC의 Itda, Mac의 화면 공유 앱, 그리고 웹 뷰어 목록의 ‘이 PC’로 들어옵니다.', '쓰지 않을 때는 꺼 두는 것이 안전합니다.', '')}
      ${hset('공유 암호', '다른 기기가 이 PC에 들어올 때 넣는 8자리 암호입니다 (웹 뷰어는 PIN으로 들어오므로 필요 없습니다).', '', '')}
      ${hset('연결할 때마다 묻기', '누가 이 PC에 들어오려 하면 이 PC 화면에 ‘허용할까요?’ 창이 뜹니다. 여기 앉은 사람이 ‘예’를 눌러야 연결됩니다.', '혼자 쓰는 PC를 밖에서 열려면 끄세요. 그때는 암호만 맞으면 바로 들어옵니다.', '추천: 켬')}
      ${hset('Mac의 ⌘을 Ctrl로', 'Mac에서 이 PC에 연결했을 때 ⌘C·⌘V로 복사·붙여넣기가 되게 합니다.', '', '추천: 켬')}
      ${hset('보기만 허용', '들어온 사람이 화면을 보기만 하고 마우스·키보드는 쓰지 못하게 합니다.', '도움을 받는 게 아니라 화면만 보여 줄 때 켜세요.', '')}
      ${hset('포트', '기기끼리 통하는 ‘문 번호’입니다. 웹 뷰어 8780, 이 PC 공유 5900이 기본입니다.', '‘포트를 열 수 없습니다’라는 오류가 날 때만 다른 숫자(예: 8781, 5901)로 바꾸세요.', '추천: 그대로')}`)}
    ${hgroup('app', 'E8A5', '앱', path('설정', '앱'), `
      ${hset('창을 닫으면 알림 영역에서 계속 실행', '창의 ✕를 눌러도 Itda가 꺼지지 않고, 화면 오른쪽 아래 알림 영역(시계 옆)에 남습니다. 공유와 웹 뷰어도 계속 켜져 있습니다.', '완전히 끄려면 알림 영역의 Itda 아이콘을 오른쪽 클릭 → 종료.', '추천: 켬')}
      ${hset('알림', '연결이 끊기거나 다시 연결될 때, 누가 이 PC를 보기 시작할 때 오른쪽 아래에 알림을 띄웁니다.', '', '추천: 켬')}
      ${hset('미리보기 저장', '마지막으로 본 화면을 작게 저장해 주소록 카드에 보여 줍니다. 이 PC에만 저장됩니다.', '화면에 민감한 내용이 많다면 끄세요.', '추천: 켬')}
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
    ${w('웹 뷰어', '앱을 깔지 않고 브라우저(Safari, Chrome)로 Itda를 쓰는 방법입니다. 이 PC의 Itda가 화면을 브라우저로 전달합니다.')}
    ${w('PIN', '웹 뷰어에 들어가는 6자리 숫자 암호입니다. 이 PC의 Itda → 공유 페이지에서 확인합니다.')}
    ${w('방화벽', 'Windows가 다른 기기의 접속을 막는 문지기입니다. 처음 공유를 켤 때 물어보면 <b>개인 네트워크</b>만 허용하세요.')}
    ${w('자격 증명 관리자', 'Windows가 암호를 안전하게 보관하는 금고입니다. Itda는 암호를 여기에만 저장합니다.')}
    ${w('계정 이름 (Mac 짧은 이름)', 'Mac에 로그인할 때 쓰는 이름입니다. Apple ID(이메일)가 아닙니다. Mac의 터미널에서 <code>whoami</code>를 입력하면 나옵니다.')}
    ${w('알림 영역 (트레이)', '화면 오른쪽 아래 시계 옆의 작은 아이콘 모음입니다. 창을 닫아도 Itda가 여기서 계속 실행됩니다.')}
    ${w('PWA (앱처럼 설치)', '웹 뷰어를 휴대폰 홈 화면이나 Mac의 Dock에 아이콘으로 추가해, 앱처럼 전체 화면으로 여는 방법입니다.')}
  </div>`;
}

function renderHelp() {
  const t = S.helpTopic || 'start';
  const body = { start: helpStart, mac: helpMac, windows: helpWindows, phone: helpPhone, outside: helpOutside, accounts: helpAccounts, settings: helpSettings, words: helpWords, trouble: helpTrouble, keys: helpKeys }[t] || helpStart;
  const title = (TOPICS.find(([k]) => k === t) || TOPICS[0])[2];
  return `<div class="help">
    <nav class="hnav" aria-label="도움말 주제">${TOPICS.map(([k, icon, label]) => `<button class="hnav-item${k === t ? ' on' : ''}" data-help="${k}">${I(icon)}<span>${label}</span></button>`).join('')}</nav>
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
