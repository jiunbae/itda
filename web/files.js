/* Native-style file workspace. The Rust host owns credentials, filesystem operations and transfers. */
(() => {
  "use strict";
  window.ITDA_FILE_HOST ||= new URLSearchParams(location.search).get("files");
  if (window.ITDA_FILE_HOST) document.body.classList.add("file-window");
  let host,
    layer,
    setup,
    setupStarted = false,
    active = "remote",
    split = false,
    view = "list",
    hidden = false,
    search = "",
    clipboard = null,
    drag = null,
    networkMode = false,
    locationsPending = false;
  const NETWORK = "__network";
  const nativePending = new Set();
  let sequence = 0,
    typeAhead = "",
    typeAt = 0;
  const undo = [];
  const panes = { local: emptyPane(), remote: emptyPane() };
  const jobs = new Map();
  const extension = (name = "") => name.includes(".") ? name.split(".").pop().toLowerCase() : "";
  const kind = (e) => e.link ? "심볼릭 링크" : e.dir ? "폴더" : ({ txt: "텍스트 문서", md: "Markdown 문서", pdf: "PDF 문서", png: "PNG 이미지", jpg: "JPEG 이미지", jpeg: "JPEG 이미지", heic: "HEIF 이미지", zip: "ZIP 아카이브", mp4: "MPEG-4 동영상", mp3: "MP3 오디오" }[extension(e.name)] || (extension(e.name) ? `${extension(e.name).toUpperCase()} 문서` : "문서"));
  const icon = (dir, name = "") => {
    const ext = extension(name), color = /^(png|jpg|jpeg|gif|heic|webp|svg)$/.test(ext) ? "image" : /^(zip|gz|tar|7z)$/.test(ext) ? "archive" : ext === "pdf" ? "pdf" : "text";
    return `<svg class="${dir ? "folder" : `document ${color}`}" viewBox="0 0 24 24" aria-hidden="true">${dir ? '<path fill="#70c8f1" d="M2 5a2 2 0 0 1 2-2h5l3 3h8a2 2 0 0 1 2 2v11H2z"/><path fill="#40aee8" d="M2 8h20v11a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z"/><path stroke="#b0e8ff" d="M3 9h18"/>' : `<path fill="#fff" stroke="#9ba5af" stroke-width=".8" d="M5 2h9l5 5v15H5z"/><path fill="#e8edf2" stroke="#9ba5af" stroke-width=".8" d="M14 2v6h5"/><rect x="6" y="12" width="12" height="7" rx="1" fill="currentColor"/><text x="12" y="17.2" text-anchor="middle" fill="white" font-size="4.6" font-family="-apple-system, sans-serif" font-weight="600">${esc(ext.slice(0, 4).toUpperCase() || "DOC")}</text>`}</svg>`;
  };
  const columnWidths = {};
  function emptyPane() {
    return {
      path: "",
      entries: [],
      selected: new Set(),
      anchor: 0,
      focus: -1,
      history: [],
      at: -1,
      start: 0,
      scroll: 0,
      sort: "name",
      desc: false,
      seq: 0,
      busy: false,
    };
  }
  const call = (op, data = {}) => send("files", { id: host.id, op, ...data });
  const bytes = (n) =>
    n < 1024
      ? `${n} B`
      : n < 1048576
        ? `${(n / 1024).toFixed(1)} KB`
        : n < 1073741824
          ? `${(n / 1048576).toFixed(1)} MB`
          : `${(n / 1073741824).toFixed(1)} GB`;
  const nativeName = MAC ? "Finder" : "탐색기";
  function removeMenu() {
    document.querySelectorAll(".files-menu").forEach((e) => e.remove());
  }
  function fail(text) {
    if (setup) {
      setup.classList.remove("is-connecting");
      setup.querySelector(".files-dialog-error").textContent = text;
      setup.querySelector("[data-connect]").disabled = false;
    } else if (layer) {
      let el = layer.querySelector(".files-error");
      el.textContent = text || "";
      el.hidden = !text;
    } else toast(text, "E783", true);
  }
  function prompt(title, body, buttons) {
    return new Promise((resolve) => {
      const el = document.createElement("div");
      el.style.cssText =
        "position:fixed;inset:0;z-index:3700;background:#0005;display:flex;align-items:center;justify-content:center";
      el.className = "files-prompt";
      el.innerHTML = `<div class="dialog files-setup"><div class="body"><h2>${esc(title)}</h2>${body}</div><div class="foot">${buttons.map(([id, text, primary]) => `<button class="btn ${primary ? "primary" : ""}" data-choice="${id}">${esc(text)}</button>`).join("")}</div></div>`;
      document.body.append(el);
      el.addEventListener("click", (e) => {
        const b = e.target.closest("[data-choice]");
        if (!b) return;
        const fields = Object.fromEntries(
          [...el.querySelectorAll("input,select")].map((i) => [
            i.name,
            i.type === "checkbox" ? i.checked : i.value,
          ]),
        );
        el.remove();
        resolve({ choice: b.dataset.choice, ...fields });
      });
      el.addEventListener("keydown", (e) => {
        e.stopPropagation();
        if (e.key === "Escape") {
          el.querySelector("[data-choice]").click();
        }
        if (e.key === "Enter" && e.target.tagName === "INPUT") {
          el.querySelector("[data-choice]:last-child").click();
        }
      });
      const first = el.querySelector('input:not([type="checkbox"]),button');
      first?.focus();
      if (first?.tagName === "INPUT") first.select();
      el.addEventListener("keydown", (e) => {
        if (e.key !== "Tab") return;
        const fields = [
          ...el.querySelectorAll("input,select,button,summary"),
        ].filter((i) => !i.disabled && i.offsetParent !== null);
        const first = fields[0],
          last = fields[fields.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      });
    });
  }
  function network() {
    if (layer) {
      networkMode = true;
      search = "";
      layer.querySelector("[data-search]").value = "";
      renderPanes();
      return;
    }
    host = { id: NETWORK, name: "네트워크", files: {} };
    workspace({ path: "", entries: [], local_path: "", local_entries: [] });
  }
  const sortedDevices = () =>
    [...S.hosts].sort(
      (a, b) =>
        Number(b.favorite) - Number(a.favorite) ||
        label(a).localeCompare(label(b)),
    );
  function deviceState(h) {
    const states = [];
    if (S.sessions[h.id]?.state === "connected") states.push("화면 연결됨");
    else if (S.sessions[h.id]) states.push("화면 연결 중");
    if ((S.fileWindows || []).includes(h.id)) states.push("파일 창 열림");
    return states.join(" · ") || "저장된 기기";
  }
  function refreshNetwork() {
    if (!layer) return;
    layer.classList.toggle("is-network", networkMode);
    const sidebar = layer.querySelector("[data-network-sidebar]");
    const devices = sortedDevices();
    morph(sidebar, `<h4>네트워크</h4><button class="btn subtle ${networkMode ? "on" : ""}" data-action="network">${I("E774")}모든 기기</button>${devices.map((h) => `<button class="btn subtle files-device-link ${!networkMode && host.id === h.id ? "on" : ""}" data-device="${esc(h.id)}" data-id="file-device-${esc(h.id)}" ${MAC && host.id === h.id ? 'data-side="remote"' : ""} title="${esc(label(h))} · ${esc(deviceState(h))}">${I("E7F4")}<span>${esc(label(h))}</span>${(S.fileWindows || []).includes(h.id) ? '<span class="files-device-dot" aria-label="파일 창 열림"></span>' : ""}</button>`).join("")}`);
    const tabs = layer.querySelector(".files-locations");
    tabs.hidden = true; // Device navigation lives in the sidebar; these are separate windows, not tabs.
    if (networkMode) renderNetwork();
  }
  function networkData(query = search) {
    const matches = (text) =>
      !query || text.toLocaleLowerCase().includes(query.toLocaleLowerCase());
    const devices = sortedDevices().filter((h) =>
      matches(`${label(h)} ${h.host}`),
    );
    const found = S.found.filter(
      (f) => f.itda && !inBook(f) && matches(`${f.name} ${f.host}`),
    );
    const peers = tailPeers().filter(
      (p) => !peerInBook(p) && matches(`${p.name} ${p.dns} ${p.ip}`),
    );
    const discovered = (items, kind) =>
      items
        .map(
          (f, i) =>
            `<article class="files-network-device"><div class="files-network-machine">${I("E7F4")}</div><strong>${esc(f.name)}</strong><small>${esc(f.dns || f.host || f.ip)}</small><span class="files-network-state">${kind === "peer" ? "Tailscale 기기" : "주변에서 발견"}</span><button class="btn sm" data-found-device="${i}" data-found-kind="${kind}">추가</button></article>`,
        )
        .join("");
    return { html: `<section class="files-network" aria-label="네트워크 기기"><header><div><h2>네트워크</h2><p>파일과 화면에 접근할 기기를 선택하세요.</p></div><div class="files-network-actions"><button class="btn sm" data-action="scan">${I("E72C")}기기 찾기</button><button class="btn sm" data-action="add-device">${I("E710")}새 기기</button>${MAC ? '<button class="btn sm" data-action="locations">Finder 연결 폴더</button>' : ""}</div></header><h3>저장된 기기 <span>${devices.length}</span></h3><div class="files-network-grid">${devices.map((h) => `<article class="files-network-device" data-device-card="${esc(h.id)}" data-id="network-device-${esc(h.id)}"><button class="files-network-machine" data-device="${esc(h.id)}" aria-label="${esc(label(h))} 파일 열기">${I("E7F4")}</button><strong>${esc(label(h))}</strong><small>${esc(h.host)}</small><span class="files-network-state">${esc(deviceState(h))}</span><div class="files-network-buttons"><button class="btn sm" data-device="${esc(h.id)}">파일 열기</button><button class="btn sm" data-screen-device="${esc(h.id)}">화면</button><button class="btn sm" data-native-device="${esc(h.id)}">${nativeName}</button></div></article>`).join("") || '<p class="files-note">저장된 기기가 없습니다. 새 기기를 추가하거나 주변 기기를 찾아보세요.</p>'}</div>${found.length ? `<h3>주변 Itda 기기</h3><div class="files-network-grid" data-found-list>${discovered(found, "nearby")}</div>` : ""}${peers.length ? `<h3>Tailscale 기기</h3><div class="files-network-grid" data-peer-list>${discovered(peers, "peer")}</div>` : ""}<p class="files-network-hint">파일은 화면 공유를 켜지 않아도 연결할 수 있습니다. Finder·탐색기로 열려면 상대 기기의 파일 공유가 켜져 있어야 합니다.</p></section>`, devices, found, peers };
  }
  function renderBrowser() {
    return `<div class="files-home">${networkData(S.query).html}</div>`;
  }
  function renderNetwork() {
    const { html, devices, found, peers } = networkData();
    morph(layer.querySelector(".files-panes"), html);
    layer.querySelector("[data-status]").textContent =
      `저장된 기기 ${devices.length}대`;
    layer.querySelector("[data-location]").textContent = "네트워크";
    layer.querySelector(".files-folder-title").textContent = "네트워크";
    for (const a of [
      "copy",
      "cut",
      "paste",
      "rename",
      "trash",
      "undo",
      "mkdir",
      "up",
      "view",
      "split",
      "hidden",
      "settings",
      "more",
      "native",
      "forward",
    ])
      layer.querySelector(`[data-action="${a}"]`)?.setAttribute("disabled", "");
    if (host.id === NETWORK)
      layer.querySelector('[data-action="back"]').disabled = true;
    layer.querySelector(".files-jobs").hidden = true;
    layer.querySelector(".files-panes").ondblclick = (e) => {
      const card = e.target.closest("[data-device-card]");
      if (card && !e.target.closest("button"))
        launch(hostById(card.dataset.deviceCard));
    };
    layer.querySelectorAll("[data-found-device]").forEach(
      (b) =>
        (b.onclick = () => {
          const f = (b.dataset.foundKind === "peer" ? peers : found)[
            Number(b.dataset.foundDevice)
          ];
          openEdit({
            host: f.dns || f.host || f.ip,
            name: f.name,
            os: f.os || "mac",
            kind: f.os === "windows" && !f.itda ? "rdp" : "",
          });
        }),
    );
  }
  function browserClick(e) {
    if (S.page !== "files" || !e.target.closest(".files-home")) return;
    const device = e.target.closest("[data-device]");
    if (device) return launch(hostById(device.dataset.device));
    const native = e.target.closest("[data-native-device]");
    if (native) {
      nativePending.add(native.dataset.nativeDevice);
      send("files", { id: native.dataset.nativeDevice, op: "finder" });
      return;
    }
    const screen = e.target.closest("[data-screen-device]");
    if (screen) return connect(screen.dataset.screenDevice);
    const action = e.target.closest("[data-action]")?.dataset.action;
    if (action === "scan") return send("scan");
    if (action === "add-device") return openEdit();
    if (action === "locations") {
      locationsPending = true;
      return send("file_locations");
    }
    const found = e.target.closest("[data-found-device]");
    if (found) {
      const data = networkData(S.query);
      const f = (found.dataset.foundKind === "peer" ? data.peers : data.found)[Number(found.dataset.foundDevice)];
      if (f) openEdit({ host: f.dns || f.host || f.ip, name: f.name, os: f.os || "mac", kind: f.os === "windows" && !f.itda ? "rdp" : "" });
    }
  }
  function browserSearch() {
    const devices = networkData(S.query).devices;
    if (!devices.length) return toast("일치하는 기기가 없습니다. 새 기기를 추가하거나 검색어를 바꿔 주세요", "E946");
    if (devices.length === 1) launch(devices[0]);
    else document.querySelector('.files-home [data-device]')?.focus();
  }
  function open(h) {
    if (!h) return;
    if (host && host.id !== h.id) {
      call("disconnect");
      layer?.remove();
      layer = null;
      clipboard = null;
      undo.length = 0;
    }
    host = h;
    showSetup();
  }
  function showSetup(error = "") {
    setup?.remove();
    setupStarted = false;
    const f = host.files || {};
    const account = f.account || "";
    setup = document.createElement("div");
    setup.className = "files-prompt";
    setup.style.cssText =
      "position:fixed;inset:0;z-index:3700;background:#0004;display:flex;align-items:center;justify-content:center";
    const address = f.host || host.host.replace(/:\d+$/, "");
    setup.innerHTML = `<div class="dialog files-setup"><div class="body"><h2>${icon(true)} ${esc(label(host))}의 파일</h2><p class="lead">저장된 계정으로 파일에 접근합니다.</p>
      <div class="field"><label>사용할 계정</label><select class="input" name="account"><option value="">기기의 계정 사용${host.login_user ? ` · ${esc(host.login_user)}` : ""}</option>${S.accounts.map((a) => `<option value="${esc(a.id)}">${esc(a.name || a.user)} · ${esc(a.user)}</option>`).join("")}<option value="custom">다른 계정 직접 입력</option></select></div>
      <div class="field files-custom" style="margin-top:12px"><label>계정 이름</label><input class="input" name="user" value="${esc(f.user || "")}" autocomplete="username"></div>
      <details class="files-password"><summary>암호 입력 또는 변경</summary><div class="field"><label>계정 암호</label><input class="input" name="password" type="password" autocomplete="current-password" placeholder="비우면 저장된 암호·SSH 키 사용"></div><label class="check"><input name="save_password" type="checkbox" checked>암호를 ${MAC ? "키체인" : "자격 증명 관리자"}에 저장</label></details>
      <details class="files-advanced"><summary>연결 설정</summary><div class="field"><label>연결 방법</label><select class="input" name="method"><option value="auto">자동 선택</option><option value="sftp">SSH / SFTP</option><option value="smb">공유폴더 · ${nativeName}</option></select></div><div class="row"><div class="field" style="flex:1"><label>기기 주소</label><input class="input" name="host" value="${esc(address)}"></div><div class="field" style="width:90px"><label>포트</label><input class="input" name="port" type="number" min="1" max="65535" value="${f.port || host.ssh?.port || 22}"></div></div><div class="field"><label>시작 폴더</label><input class="input" name="path" value="${esc(f.path || "")}" placeholder="계정의 홈 폴더"></div><div class="field"><label>공유폴더 주소 · 선택 사항</label><input class="input" name="share" value="${esc(f.share || "")}" placeholder="smb://기기/공유폴더"></div><p class="hint">자동 선택은 저장한 연결을 우선 사용합니다. 공유폴더는 실제 ${nativeName}에서 열며 운영체제에서 로그인을 안내할 수 있습니다.</p></details>
      <p class="hint">화면 공유와 독립적으로 연결됩니다. 화면 공유 전용 암호는 기기 계정 암호와 다를 수 있습니다.</p><div class="files-dialog-error" role="alert">${esc(error)}</div></div><div class="foot"><button class="btn" data-close>취소</button><button class="btn primary" data-connect>파일 열기</button></div></div>`;
    document.body.append(setup);
    const progress = document.createElement("div");
    progress.className = "files-connect-progress";
    progress.setAttribute("role", "status");
    progress.innerHTML = `<span class="files-spinner" aria-hidden="true"></span><strong>${esc(label(host))}에 연결하는 중…</strong><span>저장된 계정을 사용하고 있습니다.</span>`;
    setup.querySelector(".body").prepend(progress);
    setup.querySelector('[name="account"]').value = account;
    setup.querySelector('[name="method"]').value = f.method || "auto";
    const refresh = () => {
      setup.querySelector(".files-custom").hidden =
        setup.querySelector('[name="account"]').value !== "custom";
    };
    setup.querySelector('[name="account"]').onchange = refresh;
    refresh();
    setup.querySelector("[data-close]").onclick = () => {
      if (setupStarted) {
        call("disconnect");
        layer?.remove();
        layer = null;
      }
      setup.remove();
      setup = null;
      if (!layer && window.ITDA_FILE_HOST)
        send("files_window", { id: host.id, close: true });
    };
    setup.querySelector("[data-connect]").onclick = () => {
      const value = (n) => setup.querySelector(`[name="${n}"]`).value.trim();
      const port = Number(value("port"));
      if (
        !value("host") ||
        !Number.isInteger(port) ||
        port < 1 ||
        port > 65535 ||
        (value("account") === "custom" && !value("user"))
      ) {
        fail("기기 주소와 계정 설정을 확인하세요");
        return;
      }
      host.files = {
        method: value("method"),
        account: value("account"),
        host: value("host"),
        port,
        user: value("account") === "custom" ? value("user") : "",
        path: value("path"),
        share: value("share"),
        engine: f.engine || "auto",
      };
      setupStarted = true;
      setup.querySelector("[data-connect]").disabled = true;
      setup.classList.add("is-connecting");
      setup.querySelector("[data-close]").focus();
      setup.querySelector(".files-dialog-error").textContent = "";
      call("connect", {
        options: host.files,
        password: setup.querySelector('[name="password"]').value,
        save_password: setup.querySelector('[name="save_password"]').checked,
      });
    };
    setup.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Escape") setup.querySelector("[data-close]").click();
      if (e.key === "Enter") setup.querySelector("[data-connect]").click();
    });
    setup.querySelector('[name="account"]').focus();
  }
  let booted = false;
  function boot() {
    if (!window.ITDA_FILE_HOST || booted) return;
    if (window.ITDA_FILE_HOST === NETWORK) {
      booted = true;
      network();
      return;
    }
    const h = hostById(window.ITDA_FILE_HOST);
    if (!h) return;
    booted = true;
    document.body.classList.add("file-window");
    document.title = `${label(h)} — 파일`;
    host = h;
    showSetup();
    const f = h.files || {};
    if (h.login_user || f.user || f.account || f.share || f.method === "smb") {
      setupStarted = true;
      setup.querySelector("[data-connect]").disabled = true;
      setup.classList.add("is-connecting");
      setup.querySelector("[data-close]").focus();
      setup.querySelector(".files-dialog-error").textContent = "";
      call("connect", {
        options: { ...f, method: f.method || "auto" },
        reuse: true,
      });
    }
  }
  function launch(h) {
    if (!h) return;
    if (location.protocol === "http:" && location.hostname === "127.0.0.1")
      return open(h);
    send("files_window", { id: h.id });
  }
  function workspace(data) {
    networkMode = host.id === NETWORK;
    setup?.remove();
    setup = null;
    layer?.remove();
    Object.assign(panes.local, emptyPane(), {
      path: data.local_path,
      home: data.local_path,
      entries: data.local_entries,
      history: [data.local_path],
      at: 0,
    });
    Object.assign(panes.remote, emptyPane(), {
      path: data.path,
      home: data.path,
      entries: data.entries,
      history: [data.path],
      at: 0,
    });
    active = "remote";
    split = false;
    search = "";
    clipboard = null;
    undo.length = 0;
    layer = document.createElement("div");
    layer.className = `files-layer ${MAC ? "mac" : "windows"}`;
    const tool = (action, glyph, text) =>
      `<button class="btn sm" data-action="${action}" title="${text}" aria-label="${text}">${I(glyph)}</button>`;
    layer.innerHTML = `<div class="files-title"><div class="files-nav">${tool("back", "E72B", "뒤로")}${tool("forward", "E76C", "앞으로")}</div>${icon(true)}<b>${esc(label(host))}</b><span class="files-folder-title"></span><span class="spacer"></span>${tool("view", "E80A", "보기")}${tool("mkdir", "E8F4", "새 폴더")}${tool("refresh", "E72C", "새로 고침")}<button class="btn sm" data-action="more" title="동작" aria-label="동작">${I("E712")}</button>${tool("settings", "E713", "연결 설정")}${tool("native", "E8A7", `${nativeName}에서 열기`)}<input class="input files-search" placeholder="검색" aria-label="파일 검색" data-search></div>
      <nav class="files-locations" aria-label="기기 위치"></nav><div class="files-tools">${tool("up", "E80F", "상위 폴더")}${tool("copy", "E8C8", "복사")}<button class="btn sm" data-action="cut">잘라내기</button><button class="btn sm" data-action="paste">붙여넣기</button>${tool("rename", "E70F", "이름 변경")}${tool("trash", "E74D", "보관함으로")}${tool("undo", "E81C", "되돌리기")}<span class="spacer"></span><button class="btn sm" data-action="split">분할 보기</button><button class="btn sm" data-action="hidden">숨김 파일</button><button class="btn sm" data-action="transfers">전송 내역</button></div>
      <div class="files-error" hidden role="alert"></div><div class="files-body"><aside class="files-sidebar"><div data-network-sidebar></div><div class="files-current-places"><h4>즐겨찾기</h4><button class="btn subtle" data-bookmark="home">${I("E80F")}홈 폴더</button><button class="btn subtle" data-bookmark="Downloads">${I("E8B7")}다운로드</button><button class="btn subtle" data-bookmark="Documents">${I("E8A5")}문서</button><button class="btn subtle" data-bookmark="Desktop">${I("E7F4")}${MAC ? "데스크탑" : "바탕 화면"}</button><h4>위치</h4><button class="btn subtle" data-side="remote">${I("E7F4")}${esc(label(host))}</button><button class="btn subtle" data-side="local">${I("E7F8")}${HERE}</button><h4>파일 관리</h4><button class="btn subtle" data-bookmark=".itda-trash">${I("E74D")}보관함</button></div></aside><div class="files-panes"></div></div><div class="files-jobs" hidden></div><div class="files-status"><span data-status></span><span data-location></span></div>`;
    document.body.append(layer);
    if (MAC) {
      const title = layer.querySelector(".files-title");
      const views = document.createElement("div");
      views.className = "files-view-switch";
      views.setAttribute("role", "group");
      views.setAttribute("aria-label", "보기 방식");
      views.innerHTML = `<button class="btn sm" data-mode="grid" title="아이콘 보기" aria-label="아이콘 보기">${I("F0E2")}</button><button class="btn sm" data-mode="list" title="목록 보기" aria-label="목록 보기">${I("E8FD")}</button>`;
      title.insertBefore(views, title.querySelector('[data-action="view"]'));
      layer.querySelector('[data-side="remote"]').remove();
      const resize = document.createElement("div");
      resize.className = "files-sidebar-resizer";
      resize.dataset.resizeSidebar = "";
      resize.tabIndex = 0;
      resize.setAttribute("role", "separator");
      resize.setAttribute("aria-label", "사이드바 너비");
      resize.setAttribute("aria-orientation", "vertical");
      resize.setAttribute("aria-valuemin", "150");
      resize.setAttribute("aria-valuemax", "320");
      resize.setAttribute("aria-valuenow", "200");
      layer.append(resize);
    }
    layer.querySelector(".files-title").addEventListener("pointerdown", (e) => {
      if (MAC && e.button === 0 && !e.target.closest("button,input")) send("files_drag");
    });
    layer.addEventListener("pointerdown", resizeStart, true);
    layer.addEventListener("click", click);
    layer.addEventListener("dblclick", dblclick);
    layer.addEventListener("contextmenu", context);
    layer.addEventListener("pointerdown", pointerDown);
    layer.querySelector("[data-search]").oninput = (e) => {
      search = e.target.value;
      for (const p of Object.values(panes)) {
        p.start = 0;
        p.scroll = 0;
      }
      renderPanes();
    };
    renderPanes();
    renderJobs();
    layer.querySelector(networkMode ? "[data-search]" : ".files-list")?.focus({ preventScroll: true });
  }
  function visible(side) {
    const p = panes[side],
      key = [hidden, search, p.sort, p.desc].join("|");
    if (p.cacheEntries === p.entries && p.cacheKey === key) return p.cache;
    p.cacheEntries = p.entries;
    p.cacheKey = key;
    return (p.cache = p.entries
      .filter(
        (e) =>
          (hidden || !e.name.startsWith(".")) &&
          e.name.toLocaleLowerCase().includes(search.toLocaleLowerCase()),
      )
      .sort((a, b) => {
        if (a.dir !== b.dir) return a.dir ? -1 : 1;
        let n =
          p.sort === "name"
            ? a.name.localeCompare(b.name, undefined, { numeric: true })
            : p.sort === "kind"
              ? kind(a).localeCompare(kind(b))
            : p.sort === "size"
              ? a.size - b.size
              : a.modified - b.modified;
        return p.desc ? -n : n;
      }));
  }
  const rowHeight = MAC ? 22 : 34;
  function resizeBy(handle, delta) {
    if (handle.hasAttribute("data-resize-sidebar")) {
      const width = layer.querySelector(".files-sidebar").getBoundingClientRect().width;
      const next = Math.max(150, Math.min(320, width + delta));
      layer.style.setProperty("--sidebar-width", `${next}px`);
      handle.setAttribute("aria-valuenow", String(Math.round(next)));
      return;
    }
    const table = handle.closest("table"), side = handle.closest("[data-pane]").dataset.pane;
    const widths = [...table.querySelectorAll("th")].map((th) => th.getBoundingClientRect().width);
    const index = Number(handle.dataset.resizeColumn), min = index === 0 ? 140 : 64;
    delta = Math.max(min - widths[index], Math.min(widths[index + 1] - 64, delta));
    widths[index] += delta;
    widths[index + 1] -= delta;
    columnWidths[side] = widths;
    table.querySelectorAll("col").forEach((col, i) => col.style.width = `${widths[i]}px`);
    handle.setAttribute("aria-valuenow", String(Math.round(widths[index])));
  }
  function resizeStart(e) {
    const handle = e.target.closest("[data-resize-column],[data-resize-sidebar]");
    if (!handle || e.button !== 0) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    handle.setPointerCapture(e.pointerId);
    let last = e.clientX;
    const move = (event) => { resizeBy(handle, event.clientX - last); last = event.clientX; };
    const done = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", done);
      handle.removeEventListener("pointercancel", done);
      handle.removeEventListener("lostpointercapture", done);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", done);
    handle.addEventListener("pointercancel", done);
    handle.addEventListener("lostpointercapture", done);
  }
  function rows(side) {
    const p = panes[side],
      items = visible(side),
      start = Math.min(p.start || 0, Math.max(0, items.length - 1)),
      end = Math.min(start + 160, items.length);
    const spacer = (n) =>
      n
        ? `<tr aria-hidden="true"><td colspan="4" style="height:${n * rowHeight}px;padding:0;border:0"></td></tr>`
        : "";
    return (
      spacer(start) +
      items
        .slice(start, end)
        .map(
          (e, i) =>
            `<tr data-index="${i + start}" class="files-item ${p.selected.has(e.name) ? "selected" : ""}" aria-selected="${p.selected.has(e.name)}"><td><span class="files-name">${icon(e.dir, e.name)}<span>${esc(e.name)}${e.link ? " ↗" : ""}</span></span></td><td>${e.modified ? esc(new Date(e.modified * 1000).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" })) : ""}</td><td>${e.dir ? "—" : bytes(e.size)}</td><td>${esc(kind(e))}</td></tr>`,
        )
        .join("") +
      spacer(items.length - end)
    );
  }
  function ensureVisible(side, index) {
    if (!layer || index < 0) return;
    const el = layer.querySelector(`[data-pane="${side}"] .files-list`);
    if (!el) return;
    if (
      view === "list" &&
      (index < (panes[side].start || 0) ||
        index >= (panes[side].start || 0) + 160)
    ) {
      panes[side].start = Math.max(0, index - 20);
      el.querySelector("tbody").innerHTML = rows(side);
      el.scrollTop = index * rowHeight;
    }
    el.querySelector(`[data-index="${index}"]`)?.scrollIntoView({
      block: "nearest",
    });
  }
  function crumbs(path, side) {
    const sep = path.includes("\\") ? "\\" : "/";
    let built = path.startsWith(sep + sep)
      ? sep + sep
      : path.startsWith(sep)
        ? sep
        : "";
    return path
      .split(/[\\/]/)
      .filter(Boolean)
      .map((part, i) => {
        built += (built && !built.endsWith(sep) ? sep : "") + part;
        const target = /^[A-Za-z]:$/.test(built) ? built + sep : built;
        return `<button data-crumb="${side}" data-folder="${esc(target)}">${i ? '<span aria-hidden="true">›</span>' : ""}${esc(part)}</button>`;
      })
      .join("");
  }
  function renderPanes() {
    if (!layer) return;
    const focusedSide = document.activeElement?.closest(".files-list")?.closest("[data-pane]")?.dataset.pane;
    refreshNetwork();
    if (networkMode) return;
    layer.querySelector(".files-panes").ondblclick = null;
    layer
      .querySelectorAll("[data-action]")
      .forEach((b) => b.removeAttribute("disabled"));
    const sides = split ? ["local", "remote"] : [active];
    layer.querySelector(".files-panes").innerHTML = sides
      .map((side) => {
        const p = panes[side];
        const list = visible(side);
        const cell = (e, i) =>
          `data-index="${i}" class="files-item ${p.selected.has(e.name) ? "selected" : ""}" aria-selected="${p.selected.has(e.name)}"`;
        return `<section class="files-pane ${side === active ? "active" : ""}" data-pane="${side}"><div class="files-path"><strong>${side === "local" ? HERE : esc(label(host))}</strong><nav class="files-breadcrumbs" aria-label="폴더 경로">${crumbs(p.path, side)}</nav><input aria-label="폴더 경로 입력" value="${esc(p.path)}" data-path="${side}"></div><div class="files-list" tabindex="0" role="grid" aria-label="${side === "local" ? "내 기기" : "원격 기기"} 파일">${p.busy ? '<div class="files-note">폴더를 읽는 중…</div>' : !list.length ? '<div class="files-note">표시할 항목이 없습니다</div>' : view === "grid" ? `<div class="files-grid">${list.map((e, i) => `<div ${cell(e, i)}>${icon(e.dir, e.name)}<span>${esc(e.name)}${e.link ? " ↗" : ""}</span></div>`).join("")}</div>` : `<table><colgroup>${[47, 25, 12, 16].map((w, i) => `<col style="width:${columnWidths[side]?.[i] ? `${columnWidths[side][i]}px` : `${w}%`}">`).join("")}</colgroup><thead><tr>${[["name", "이름"], ["modified", "수정한 날짜"], ["size", "크기"], ["kind", "종류"]].map(([key, title], i) => `<th data-sort="${key}">${title}${p.sort === key ? (p.desc ? " ▾" : " ▴") : ""}${i < 3 ? `<span data-resize-column="${i}" role="separator" aria-label="${title} 열 너비" aria-orientation="vertical" tabindex="0"></span>` : ""}</th>`).join("")}</tr></thead><tbody>${rows(side)}</tbody></table>`}</div></section>`;
      })
      .join("");
    layer
      .querySelectorAll("[data-side]")
      .forEach((b) => b.classList.toggle("on", b.dataset.side === active));
    layer.querySelectorAll("[data-pane]").forEach((section) => {
      const side = section.dataset.pane,
        el = section.querySelector(".files-list");
      section.querySelectorAll("[data-resize-column]").forEach((handle) => {
        handle.setAttribute("aria-valuemin", handle.dataset.resizeColumn === "0" ? "140" : "64");
        handle.setAttribute("aria-valuemax", "5000");
        handle.setAttribute("aria-valuenow", String(Math.round(handle.closest("th").getBoundingClientRect().width)));
      });
      el.scrollTop = panes[side].scroll || 0;
      el.addEventListener("scroll", () => {
        const p = panes[side];
        p.scroll = el.scrollTop;
        if (view !== "list" || !el.querySelector("tbody")) return;
        const start = Math.max(0, Math.floor(el.scrollTop / rowHeight) - 20);
        if (Math.abs(start - (p.start || 0)) > 10) {
          p.start = start;
          el.querySelector("tbody").innerHTML = rows(side);
        }
      });
    });
    layer.querySelectorAll("[data-mode]").forEach((b) => {
      b.classList.toggle("on", b.dataset.mode === view);
      b.setAttribute("aria-pressed", String(b.dataset.mode === view));
    });
    if (focusedSide) layer.querySelector(`[data-pane="${focusedSide}"] .files-list`)?.focus({ preventScroll: true });
    status();
  }
  function status() {
    if (!layer) return;
    const p = panes[active];
    layer.querySelector("[data-status]").textContent =
      `${visible(active).length}개 항목${p.selected.size ? ` · ${p.selected.size}개 선택됨` : ""}`;
    layer.querySelector("[data-location]").textContent = p.path;
    layer.querySelector(".files-folder-title").textContent =
      p.path.split(/[\\/]/).filter(Boolean).pop() || p.path;
    layer.querySelector('[data-action="paste"]').disabled = !clipboard;
    for (const a of ["copy", "cut", "rename", "trash"])
      layer.querySelector(`[data-action="${a}"]`).disabled = !p.selected.size;
  }
  function nav(side, path, history = true) {
    const p = panes[side];
    p.seq = ++sequence;
    p.busy = true;
    if (history) {
      p.start = 0;
      p.scroll = 0;
      p.focus = -1;
      p.selected.clear();
    }
    if (history) {
      p.history.splice(p.at + 1);
      p.history.push(path);
      p.at = p.history.length - 1;
    }
    renderPanes();
    call("list", { side, path, seq: p.seq });
  }
  function parent(path) {
    if (path === "/") return "/";
    const s = path.replace(/[\\/]+$/, "");
    const i = Math.max(s.lastIndexOf("/"), s.lastIndexOf("\\"));
    return i <= 0 ? "/" : s.slice(0, i) + (i === 2 && s[1] === ":" ? "\\" : "");
  }
  const child = (p, n) =>
    p.replace(/[\\/]+$/, "") + (p.includes("\\") ? "\\" : "/") + n;
  function select(side, index, e) {
    const p = panes[side];
    const items = visible(side);
    const item = items[index];
    if (!item) return;
    active = side;
    p.focus = index;
    if (e.shiftKey) {
      const lo = Math.min(p.anchor, index),
        hi = Math.max(p.anchor, index);
      if (!(e.metaKey || e.ctrlKey)) p.selected.clear();
      items.slice(lo, hi + 1).forEach((x) => p.selected.add(x.name));
    } else if (e.metaKey || e.ctrlKey) {
      p.selected.has(item.name)
        ? p.selected.delete(item.name)
        : p.selected.add(item.name);
      p.anchor = index;
    } else {
      p.selected = new Set([item.name]);
      p.anchor = index;
    }
    paintSelection();
    layer.querySelector(`[data-pane="${side}"] .files-list`)?.focus();
  }
  function paintSelection() {
    if (!layer) return;
    layer
      .querySelectorAll("[data-side]")
      .forEach((b) => b.classList.toggle("on", b.dataset.side === active));
    layer.querySelectorAll("[data-pane]").forEach((el) => {
      const side = el.dataset.pane,
        p = panes[side],
        items = visible(side);
      el.classList.toggle("active", side === active);
      el.querySelectorAll("[data-index]").forEach((row) => {
        const selected = p.selected.has(items[Number(row.dataset.index)]?.name);
        row.classList.toggle("selected", selected);
        row.setAttribute("aria-selected", String(selected));
      });
    });
    status();
  }
  function click(e) {
    const device = e.target.closest("[data-device]");
    if (device) {
      const h = hostById(device.dataset.device);
      if (h?.id === host.id && layer && host.id !== NETWORK) {
        active = "remote";
        networkMode = false;
        renderPanes();
      } else launch(h);
      return;
    }
    if (e.target.closest("[data-resize-column],[data-resize-sidebar]")) return;
    const mode = e.target.closest("[data-mode]");
    if (mode) {
      view = mode.dataset.mode;
      renderPanes();
      return;
    }
    const screen = e.target.closest("[data-screen-device]");
    if (screen) {
      const id = screen.dataset.screenDevice;
      if (S.sessions[id]?.state === "connected") send("focus", { id });
      else connect(id);
      return;
    }
    const native = e.target.closest("[data-native-device]");
    if (native) {
      nativePending.add(native.dataset.nativeDevice);
      send("files", { id: native.dataset.nativeDevice, op: "finder" });
      return;
    }

    const crumb = e.target.closest("[data-crumb]");
    if (crumb) {
      active = crumb.dataset.crumb;
      nav(active, crumb.dataset.folder);
      return;
    }

    removeMenu();
    if (e.target.closest("[data-cancel-job]")) return;
    const action = e.target.closest("[data-action]");
    if (action) {
      act(action.dataset.action);
      return;
    }
    const s = e.target.closest("[data-side]");
    if (s) {
      active = s.dataset.side;
      search = "";
      layer.querySelector("[data-search]").value = "";
      renderPanes();
      return;
    }
    const b = e.target.closest("[data-bookmark]");
    if (b) {
      nav(
        active,
        b.dataset.bookmark === "home"
          ? panes[active].home
          : child(panes[active].home, b.dataset.bookmark),
      );
      return;
    }
    const pane = e.target.closest("[data-pane]");
    if (!pane) return;
    active = pane.dataset.pane;
    const sort = e.target.closest("[data-sort]");
    if (sort) {
      const p = panes[active];
      p.desc = p.sort === sort.dataset.sort ? !p.desc : false;
      p.sort = sort.dataset.sort;
      p.start = 0;
      p.scroll = 0;
      renderPanes();
      return;
    }
    const item = e.target.closest("[data-index]");
    if (item) select(active, Number(item.dataset.index), e);
    else if (!e.target.closest("input")) {
      panes[active].selected.clear();
      paintSelection();
    }
  }
  function dblclick(e) {
    const item = e.target.closest("[data-index]");
    const pane = e.target.closest("[data-pane]");
    if (!item || !pane) return;
    const entry = visible(pane.dataset.pane)[Number(item.dataset.index)];
    if (entry.dir)
      nav(pane.dataset.pane, child(panes[pane.dataset.pane].path, entry.name));
    else if (entry.link)
      fail("심볼릭 링크는 원본을 확인한 뒤 직접 열어 주세요");
    else
      call("open", {
        side: pane.dataset.pane,
        path: child(panes[pane.dataset.pane].path, entry.name),
      });
  }
  async function act(action) {
    if (action === "network") return network();
    if (action === "scan") {
      send("scan");
      return;
    }
    if (action === "add-device") {
      openEdit();
      return;
    }
    if (action === "locations") {
      locationsPending = true;
      send("file_locations");
      return;
    }
    if (action === "native") {
      nativePending.add(host.id);
      call("finder");
      return;
    }
    if (networkMode && action === "refresh") {
      send("scan");
      return;
    }
    if (networkMode && action === "back" && host.id !== NETWORK) {
      networkMode = false;
      renderPanes();
      return;
    }
    if (
      networkMode &&
      !["close", "transfers", "back", "forward"].includes(action)
    )
      return;
    const p = panes[active];
    fail("");
    switch (action) {
      case "close":
        if (window.ITDA_FILE_HOST) {
          send("files_window", { id: host.id, close: true });
          return;
        }
        layer.remove();
        layer = null;
        removeMenu();
        return;
      case "transfers":
        layer.querySelector(".files-jobs").hidden =
          !layer.querySelector(".files-jobs").hidden;
        return;
      case "more": {
        const rect = layer.querySelector('[data-action="more"]').getBoundingClientRect();
        showMenu([
          ["open", "열기"], ["mkdir", "새 폴더"],
          ["copy", "복사"], ["cut", "잘라내기"], ["paste", "붙여넣기"],
          ["rename", "이름 변경"], ["trash", "보관함으로 이동"], ["undo", "되돌리기"],
          null, ["up", "상위 폴더"], ["goto", "폴더로 이동…"],
          ["split", split ? "분할 보기 끄기" : "분할 보기"],
          ["hidden", hidden ? "숨김 파일 가리기" : "숨김 파일 보기"],
          ["transfers", "전송 내역"], ["refresh", "새로 고침"],
          null, ["settings", "연결 설정…"], ["native", `${nativeName}에서 열기`],
        ], rect.right - 210, rect.bottom + 4);
        return;
      }
      case "open": {
        const entry = p.entries.find((e) => p.selected.has(e.name));
        if (!entry) return;
        if (entry.link) return fail("심볼릭 링크는 원본을 확인한 뒤 직접 열어 주세요");
        const path = child(p.path, entry.name);
        if (entry.dir) nav(active, path);
        else call("open", { side: active, path });
        return;
      }
      case "goto": {
        const r = await prompt("폴더로 이동", `<div class="field"><label>폴더 경로</label><input class="input" name="path" value="${esc(p.path)}"></div>`, [["cancel", "취소"], ["ok", "이동", true]]);
        if (r.choice === "ok" && r.path.trim()) nav(active, r.path.trim());
        return;
      }
      case "settings":
        return showSetup();
      case "undo": {
        const u = undo.pop();
        if (u) call("undo", u);
        return;
      }
      case "refresh":
        return nav(active, p.path, false);
      case "back":
        if (p.at > 0) nav(active, p.history[--p.at], false);
        return;
      case "forward":
        if (p.at + 1 < p.history.length) nav(active, p.history[++p.at], false);
        return;
      case "up":
        return nav(active, parent(p.path));
      case "hidden":
        hidden = !hidden;
        renderPanes();
        return;
      case "view":
        view = view === "list" ? "grid" : "list";
        renderPanes();
        return;
      case "split":
        split = !split;
        renderPanes();
        return;
      case "copy":
      case "cut":
        if (p.selected.size) {
          clipboard = {
            side: active,
            path: p.path,
            names: [...p.selected],
            move: action === "cut",
          };
          status();
        }
        return;
      case "paste":
        if (clipboard)
          await transfer(
            clipboard.side,
            clipboard.path,
            clipboard.names,
            active,
            p.path,
            clipboard.move,
          );
        return;
      case "mkdir":
      case "rename": {
        if (action === "rename" && p.selected.size !== 1) {
          fail("이름을 바꿀 파일 하나를 선택하세요");
          return;
        }
        const name = action === "mkdir" ? "새 폴더" : [...p.selected][0];
        const r = await prompt(
          action === "mkdir" ? "새 폴더" : "이름 변경",
          `<div class="field"><label>이름</label><input class="input" name="name" value="${esc(name)}"></div>`,
          [
            ["cancel", "취소"],
            ["ok", action === "mkdir" ? "만들기" : "변경", true],
          ],
        );
        if (r.choice !== "ok") return;
        call(action, {
          side: active,
          path: p.path,
          name: action === "mkdir" ? r.name : name,
          new_name: r.name,
        });
        return;
      }
      case "trash": {
        if (!p.selected.size) return;
        const r = await prompt(
          "선택한 항목을 보관함으로 옮길까요?",
          `<p>${p.selected.size}개 항목을 이 기기의 .itda-trash 폴더로 옮깁니다. 영구 삭제하지 않으며, 사이드바의 보관함에서 복원할 수 있습니다.</p>`,
          [
            ["cancel", "취소"],
            ["ok", "보관함으로 이동", true],
          ],
        );
        if (r.choice === "ok")
          for (const name of p.selected)
            call("trash", { side: active, path: p.path, name });
        return;
      }
    }
  }
  async function transfer(from, source, names, to, destination, move = false) {
    if (!names.length) return;
    const dest = panes[to];
    const collisions =
      dest.path === destination
        ? names.filter((n) => dest.entries.some((e) => e.name === n))
        : [];
    const r = await prompt(
      move ? "파일 이동" : "파일 복사",
      `<p><b>${esc(names.length === 1 ? names[0] : `${names.length}개 항목`)}</b></p><p class="hint">${esc(source)} → ${esc(destination)}</p>${collisions.length ? `<p>같은 이름의 항목 ${collisions.length}개가 있습니다.</p>` : ""}<div class="field"><label>같은 이름의 파일이 있으면</label><select class="input" name="policy"><option value="keep">둘 다 유지</option><option value="replace">변경된 파일 바꾸기 · 폴더는 합치기</option><option value="skip">기존 파일 건너뛰기</option></select></div><details><summary>전송 옵션 · 자동 선택</summary><div class="field"><label>전송 방식</label><select class="input" name="engine"><option value="auto">자동 · 가능한 경우 rsync 증분 전송</option><option value="sftp">일반 전송 · SFTP</option><option value="rsync">증분 전송 · rsync 필요</option></select></div></details>${move ? '<p class="hint">같은 파일시스템 안에서는 바로 이동하며, 장치 간 이동은 해시를 확인한 뒤 원본을 삭제합니다. 취소하거나 실패한 항목의 원본은 유지합니다.</p>' : ""}`,
      [
        ["cancel", "취소"],
        ["ok", move ? "이동" : "복사", true],
      ],
    );
    if (r.choice === "ok")
      call("transfer", {
        from,
        source,
        names,
        to,
        destination,
        move,
        policy: r.policy,
        engine: r.engine,
      });
  }
  function renderJobs() {
    if (!layer) return;
    const current = [...jobs.values()]
      .filter((j) => j.id === host.id)
      .slice(-12);
    if (current.some((j) => ["queued", "running", "failed"].includes(j.state)))
      layer.querySelector(".files-jobs").hidden = false;
    layer.querySelector(".files-jobs").innerHTML = current
      .map((j) => {
        const live = ["queued", "running"].includes(j.state);
        const text = {
          queued: "대기 중",
          running: j.detail || "전송 중",
          completed: "완료",
          failed: "실패",
          cancelled: "취소됨",
        }[j.state];
        return `<div class="files-job"><span>${j.engine === "rsync" ? "증분" : "파일"}</span><span class="job-label" title="${esc(j.error || j.file || "")}">${esc(j.file || "파일 전송")} · ${esc(text)}${j.error ? ` · ${esc(j.error)}` : ""}</span>${live ? `<progress ${j.total ? `max="${j.total}" value="${j.bytes || 0}"` : ""}></progress><button class="btn sm" data-cancel-job="${esc(j.job)}">취소</button>` : ""}</div>`;
      })
      .join("");
    layer
      .querySelectorAll("[data-cancel-job]")
      .forEach(
        (b) => (b.onclick = () => call("cancel", { job: b.dataset.cancelJob })),
      );
  }
  function context(e) {
    if (!layer) return;
    e.preventDefault();
    const item = e.target.closest("[data-index]");
    const pane = e.target.closest("[data-pane]");
    if (pane) active = pane.dataset.pane;
    if (item) {
      const n = visible(active)[Number(item.dataset.index)]?.name;
      if (n && !panes[active].selected.has(n))
        select(active, Number(item.dataset.index), {});
    }
    removeMenu();
    showMenu([
      ["open", "열기"], ["copy", "복사"], ["cut", "잘라내기"], ["paste", "붙여넣기"],
      ["rename", "이름 변경"], ["mkdir", "새 폴더"], ["trash", "보관함으로 이동"],
    ], e.clientX, e.clientY);
  }
  function showMenu(items, x, y) {
    removeMenu();
    const menu = document.createElement("div"), p = panes[active];
    menu.className = "files-menu";
    menu.setAttribute("role", "menu");
    const disabled = (a) => ["open", "copy", "cut", "trash"].includes(a) ? !p.selected.size : a === "rename" ? p.selected.size !== 1 : a === "paste" ? !clipboard : a === "undo" ? !undo.length : false;
    menu.innerHTML = items.map((item) => item ? `<button class="btn subtle" role="menuitem" data-action="${item[0]}" ${disabled(item[0]) ? "disabled" : ""}>${esc(item[1])}</button>` : "<hr>").join("");
    menu.onclick = (e) => {
      const button = e.target.closest("button:not(:disabled)");
      if (!button) return;
      removeMenu();
      layer.querySelector(".files-list")?.focus();
      act(button.dataset.action);
    };
    menu.onkeydown = (e) => {
      e.stopPropagation();
      if (e.key === "Escape") { removeMenu(); layer.querySelector('[data-action="more"]').focus(); }
      if (["ArrowUp", "ArrowDown", "Home", "End"].includes(e.key)) {
        e.preventDefault();
        const buttons = [...menu.querySelectorAll("button:not(:disabled)")];
        let i = buttons.indexOf(document.activeElement);
        i = e.key === "Home" ? 0 : e.key === "End" ? buttons.length - 1 : (i + (e.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
        buttons[i]?.focus();
      }
      if (e.key === "Tab") removeMenu();
    };
    document.body.append(menu);
    const rect = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(6, Math.min(x, innerWidth - rect.width - 6))}px`;
    menu.style.top = `${Math.max(6, Math.min(y, innerHeight - rect.height - 6))}px`;
    menu.querySelector("button:not(:disabled)")?.focus();
  }

  function key(e) {
    // WKWebView does not focus buttons after a click. Route file shortcuts even when
    // focus falls back to the document, while leaving dialogs and popup menus in charge.
    if (!layer || document.querySelector(".files-prompt,.files-menu") || topModal()) return;
    if (e.target.matches("[data-resize-column],[data-resize-sidebar]")) {
      if (["ArrowLeft", "ArrowRight"].includes(e.key)) {
        e.preventDefault();
        e.stopPropagation();
        resizeBy(e.target, e.key === "ArrowRight" ? 10 : -10);
      }
      return;
    }
    if (networkMode) {
      e.stopPropagation();
      if ((MAC ? e.metaKey : e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        layer.querySelector("[data-search]").focus();
      }
      if ((MAC ? e.metaKey : e.ctrlKey) && e.key.toLowerCase() === "w") {
        e.preventDefault();
        act("close");
      }
      return;
    }
    if (document.querySelector(".files-prompt")) return;
    e.stopPropagation();
    const input = e.target.closest("input");
    if (input) {
      if (e.key === "Enter" && input.dataset.path) {
        e.preventDefault();
        nav(input.dataset.path, input.value);
      }
      return;
    }
    const mod = MAC ? e.metaKey : e.ctrlKey;
    if (mod && ["a", "c", "x", "v"].includes(e.key.toLowerCase())) {
      e.preventDefault();
      if (e.key.toLowerCase() === "a") {
        panes[active].selected = new Set(visible(active).map((x) => x.name));
        paintSelection();
      } else act({ c: "copy", x: "cut", v: "paste" }[e.key.toLowerCase()]);
      return;
    }
    if (mod && e.key.toLowerCase() === "w") {
      e.preventDefault();
      act("close");
      return;
    }
    if (mod && e.key.toLowerCase() === "z") {
      e.preventDefault();
      act("undo");
      return;
    }
    if (mod && e.key.toLowerCase() === "f") {
      e.preventDefault();
      layer.querySelector("[data-search]").focus();
      return;
    }
    if (mod && e.shiftKey && e.key.toLowerCase() === "n") {
      e.preventDefault();
      act("mkdir");
      return;
    }
    if (e.key === "F2" || (MAC && e.key === "Enter" && !mod)) {
      e.preventDefault();
      act("rename");
      return;
    }
    if (
      (e.key === "Delete" && (mod || !MAC)) ||
      (MAC && mod && e.key === "Backspace")
    ) {
      e.preventDefault();
      act("trash");
      return;
    }
    if (e.key === "F5" || (mod && e.key.toLowerCase() === "r")) {
      e.preventDefault();
      act("refresh");
      return;
    }
    if (
      (e.altKey && e.key === "ArrowUp") ||
      (MAC && mod && e.key === "ArrowUp")
    ) {
      e.preventDefault();
      act("up");
      return;
    }
    if (e.altKey && e.key === "ArrowLeft") {
      e.preventDefault();
      act("back");
      return;
    }
    if (MAC && mod && ["[", "]"].includes(e.key)) {
      e.preventDefault();
      act(e.key === "[" ? "back" : "forward");
      return;
    }
    if (MAC && mod && ["1", "2"].includes(e.key)) {
      e.preventDefault();
      view = e.key === "1" ? "grid" : "list";
      renderPanes();
      return;
    }
    if (MAC && mod && e.shiftKey && e.key === ".") {
      e.preventDefault();
      act("hidden");
      return;
    }
    if (e.altKey && e.key === "ArrowRight") {
      e.preventDefault();
      act("forward");
      return;
    }
    if (
      (mod && e.key.toLowerCase() === "l") ||
      (MAC && mod && e.shiftKey && e.key.toLowerCase() === "g")
    ) {
      e.preventDefault();
      act("goto");
      return;
    }
    if (
      e.key === "Enter" ||
      (MAC && mod && (e.key === "ArrowDown" || e.key.toLowerCase() === "o"))
    ) {
      e.preventDefault();
      act("open");
      return;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const p = panes[active],
        items = visible(active);
      let i = p.focus;
      i = Math.max(
        0,
        Math.min(items.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)),
      );
      select(active, i, e);
      ensureVisible(active, i);
      return;
    }
    if (!mod && !e.altKey && e.key.length === 1) {
      const now = Date.now();
      typeAhead = (now - typeAt < 900 ? typeAhead : "") + e.key;
      typeAt = now;
      const i = visible(active).findIndex((x) =>
        x.name.toLocaleLowerCase().startsWith(typeAhead.toLocaleLowerCase()),
      );
      if (i >= 0) {
        e.preventDefault();
        select(active, i, {});
        ensureVisible(active, i);
      }
      return;
    }
    if (e.key === "Escape") {
      removeMenu();
      panes[active].selected.clear();
      paintSelection();
    }
  }
  function pointerDown(e) {
    if (networkMode) return;
    if (e.button !== 0) return;
    const item = e.target.closest("[data-index]"),
      pane = e.target.closest("[data-pane]");
    if (!item || !pane) return;
    const side = pane.dataset.pane,
      index = Number(item.dataset.index),
      entry = visible(side)[index];
    if (!entry) return;
    drag = {
      x: e.clientX,
      y: e.clientY,
      side,
      path: panes[side].path,
      names: panes[side].selected.has(entry.name)
        ? [...panes[side].selected]
        : [entry.name],
      live: false,
    };
  }
  document.addEventListener("pointermove", (e) => {
    if (!drag || !layer) return;
    if (!drag.live && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) > 7) {
      drag.live = true;
      const tag = document.createElement("div");
      tag.className = "files-drag";
      document.body.append(tag);
    }
    if (drag.live) {
      const tag = document.querySelector(".files-drag");
      tag.textContent = `${e.shiftKey ? "이동" : "복사"} · ${drag.names.length}개`;
      tag.style.left = `${e.clientX + 14}px`;
      tag.style.top = `${e.clientY + 12}px`;
    }
  });
  document.addEventListener(
    "pointerup",
    (e) => {
      if (!drag) return;
      const d = drag;
      drag = null;
      document.querySelector(".files-drag")?.remove();
      if (!d.live || !layer) return;
      const target = document.elementFromPoint(e.clientX, e.clientY),
        pane = target?.closest("[data-pane]");
      if (!pane) return;
      const side = pane.dataset.pane,
        item = target.closest("[data-index]"),
        entry = item ? visible(side)[Number(item.dataset.index)] : null;
      const to = entry?.dir
        ? child(panes[side].path, entry.name)
        : panes[side].path;
      if (d.side === side && d.path === to) return;
      transfer(d.side, d.path, d.names, side, to, e.shiftKey);
    },
    true,
  );
  function message(m) {
    if (m.type === "file_locations_result") {
      if (locationsPending) {
        locationsPending = false;
        if (m.ok) toast("Finder에서 연결 폴더를 열었습니다", "E8B7");
        else fail(m.error);
      }
      return true;
    }
    if (
      m.type === "files_result" &&
      m.op === "finder" &&
      nativePending.has(m.id)
    ) {
      nativePending.delete(m.id);
      if (m.ok) toast(`${nativeName}에 연결을 요청했습니다`, "E8B7");
      else fail(m.error);
      return true;
    }
    if (m.type === "files_job") {
      const previous = jobs.get(m.job) || {};
      jobs.set(m.job, { ...previous, ...m });
      if (jobs.size > 200) {
        const old = [...jobs].find(([, j]) =>
          ["completed", "failed", "cancelled"].includes(j.state),
        );
        if (old) jobs.delete(old[0]);
      }
      if (
        m.state === "completed" &&
        m.id === host?.id &&
        previous.moving &&
        clipboard?.move
      )
        clipboard = null;
      renderJobs();
      if (
        ["completed", "failed", "cancelled"].includes(m.state) &&
        layer &&
        m.id === host.id
      ) {
        for (const side of ["local", "remote"])
          nav(side, panes[side].path, false);
      }
      return true;
    }
    if (m.type === "files_drop") {
      if (
        !layer ||
        (m.id && m.id !== host.id) ||
        (window.ITDA_FILE_HOST && !m.id)
      )
        return true;
      if (networkMode) {
        toast("전송할 기기의 파일 창을 먼저 열어 주세요", "E8B7");
        return true;
      }
      const groups = new Map();
      for (const path of m.paths || []) {
        const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
        if (i < 0) continue;
        const parent = path.slice(0, i),
          name = path.slice(i + 1);
        if (!groups.has(parent)) groups.set(parent, []);
        groups.get(parent).push(name);
      }
      for (const [source, names] of groups)
        transfer("local", source, names, "remote", panes.remote.path, false);
      return true;
    }
    if (m.type !== "files_result") return false;
    if (!host || m.id !== host.id) return true;
    if (!m.ok) {
      if (m.op === "list") {
        const p = panes[m.side];
        if (m.seq !== p.seq) return true;
        p.busy = false;
        renderPanes();
      }
      fail(m.error || "파일 작업에 실패했습니다");
      return true;
    }
    if (m.op === "connect") {
      if (setup) workspace(m.data);
      return true;
    }
    if (m.op === "native") {
      if (window.ITDA_FILE_HOST)
        send("files_window", { id: host.id, close: true });
      setup?.remove();
      setup = null;
      toast(`${nativeName}에 파일 연결을 요청했습니다`, "E8B7");
      return true;
    }
    if (m.op === "list") {
      const p = panes[m.side];
      if (m.seq !== p.seq) return true;
      p.path = m.data.path;
      p.entries = m.data.entries;
      p.start = Math.min(p.start || 0, Math.max(0, p.entries.length - 160));
      p.scroll = Math.min(
        p.scroll || 0,
        Math.max(0, (p.entries.length - 1) * rowHeight),
      );
      p.busy = false;
      renderPanes();
      return true;
    }
    if (m.op === "open" && m.data?.preview)
      toast(
        "보기 전용 사본을 열었습니다. 원격 파일은 변경되지 않습니다",
        "E8B7",
      );
    if (m.data?.undo) {
      undo.push(m.data.undo);
      if (undo.length > 50) undo.shift();
    }
    if (["mkdir", "rename", "trash", "undo"].includes(m.op) && layer)
      nav(m.side, panes[m.side].path, false);
    return true;
  }
  window.ItdaFiles = {
    renderBrowser,
    browserSearch,
    open,
    launch,
    boot,
    message,
    network,
    refreshNetwork,
  };
  document.addEventListener("keydown", key, true);
  document.addEventListener("click", browserClick);
  document.addEventListener("dblclick", (e) => {
    if (S.page !== "files" || !e.target.closest(".files-home") || e.target.closest("button")) return;
    const card = e.target.closest("[data-device-card]");
    if (card) launch(hostById(card.dataset.deviceCard));
  });
  if (S.page === "files") render();
  boot();
})();
