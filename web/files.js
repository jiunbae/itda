/* Native-style file workspace. The Rust host owns credentials, filesystem operations and transfers. */
(() => {
  "use strict";
  window.ITDA_FILE_HOST ||= new URLSearchParams(location.search).get("files");
  window.ITDA_FILE_WINDOW ||= new URLSearchParams(location.search).get("fileWindow");
  const fileWindow = () => window.ITDA_FILE_WINDOW || window.ITDA_FILE_HOST;
  const inNewWindow = (e) => MAC ? e.metaKey : e.ctrlKey;
  if (window.ITDA_FILE_HOST) document.body.classList.add("file-window");
  const pageToken = crypto.randomUUID();
  let pendingFolder = null;
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
  let previewOn = false, previewSeq = 0, previewTimer, previewKey = "", previewModal = false;
  let previewInFlight = false, queuedPreview = null;
  let tabId = 0, currentTab = 0, suppressClick = false;
  const fileTabs = [], folderCache = new Map(), columnRequests = new Map();
  const viewModes = ["grid", "list", "columns", "gallery"];
  const viewLabels = ["아이콘 보기", "목록 보기", "열 보기", "갤러리 보기"];
  const cacheKey = (side, path) => `${side}:${path}`;
  function recentFolders() {
    try { return JSON.parse(localStorage.getItem("itda.file.folders") || "[]").filter(x => hostById(x.id)).slice(0, 16); } catch { return []; }
  }
  function rememberFolder() {
    if (!host || host.id === "__network" || active !== "remote" || !panes.remote.path) return;
    const item = { id: host.id, path: panes.remote.path };
    try { localStorage.setItem("itda.file.folders", JSON.stringify([item, ...recentFolders().filter(x => x.id !== item.id || x.path !== item.path)].slice(0, 16))); } catch {}
  }
  function saveTab() {
    const t = fileTabs.find(t => t.id === currentTab);
    if (t) Object.assign(t, { panes: structuredClone(panes), active, view, search, hidden, split });
  }
  function tabBar() {
    if (!layer) return;
    const bar = layer.querySelector(".files-tabs");
    bar.hidden = host.id === NETWORK;
    bar.innerHTML = fileTabs.map(t => `<div role="tab" aria-selected="${t.id === currentTab}" class="files-tab ${t.id === currentTab ? "on" : ""}" data-tab="${t.id}" tabindex="${t.id === currentTab ? 0 : -1}"><span>${icon(true)}${esc((t.id === currentTab ? panes[active].path : t.panes[t.active].path).split(/[\\/]/).filter(Boolean).pop() || "/")}</span><button data-close-tab="${t.id}" aria-label="탭 닫기">×</button></div>`).join("") + `<button class="files-tab-add" data-action="new-tab" aria-label="새 탭" title="새 탭 (${MAC ? "⌘T" : "Ctrl+T"})"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"/></svg></button>`;
  }
  function newTab(path, side) {
    if (host.id === NETWORK) return;
    networkMode = false;
    saveTab();
    currentTab = ++tabId;
    fileTabs.push({ id: currentTab });
    for (const side of ["local", "remote"]) panes[side] = structuredClone(panes[side]);
    if (side) active = side;
    saveTab();
    if (path) nav(active, path); else renderPanes();
  }
  function switchTab(id) {
    if (id === currentTab) { if (networkMode && host.id !== NETWORK) { networkMode = false; renderPanes(); } return; }
    networkMode = false;
    saveTab();
    const t = fileTabs.find(t => t.id === id);
    if (!t) return;
    currentTab = id;
    Object.assign(panes, structuredClone(t.panes));
    ({ active, view, search, hidden, split } = t);
    layer.querySelector("[data-search]").value = search;
    previewKey = "";
    renderPanes();
    // A tab switched during a list request must not wait for a now-stale response.
    for (const side of ["local", "remote"]) if (panes[side].busy || side === active) nav(side, panes[side].busy ? panes[side].history[panes[side].at] || panes[side].path : panes[side].path, false);
  }
  function closeTab(id) {
    if (fileTabs.length === 1) return act("close");
    const index = fileTabs.findIndex(t => t.id === id);
    if (id === currentTab) switchTab(fileTabs[index === 0 ? 1 : index - 1].id);
    fileTabs.splice(index, 1);
    tabBar();
  }
  function columnHTML(side) {
    const p = panes[side], target = (p.busy && p.pendingPath) || p.path, paths = [target];
    while (paths.length < 8 && paths[0] !== p.home && parent(paths[0]) !== paths[0]) paths.unshift(parent(paths[0]));
    return `<div class="files-columns">${paths.map((path, n) => {
      const loading = p.busy && path === target;
      const current = path === p.path && !p.busy, k = cacheKey(side, path);
      const entries = loading ? null : current ? visible(side) : folderCache.get(k);
      const shown = entries?.filter(e => hidden || !e.name.startsWith("."));
      if (!loading && !entries && ![...columnRequests.values()].some(r => r.key === k)) {
        const seq = ++sequence;
        columnRequests.set(seq, { key: k, side });
        call("list", { side, path, seq });
      }
      const next = paths[n + 1];
      return `<section class="files-column" data-column="${esc(path)}"><header>${esc(path.split(/[\\/]/).filter(Boolean).pop() || "/")}</header>${shown?.length ? shown.map((e, i) => current ? `<div class="files-item ${p.selected.has(e.name) ? "selected" : ""}" data-index="${i}">${icon(e.dir,e.name)}<span>${esc(e.name)}</span>${e.dir ? "<b>›</b>" : ""}</div>` : `<button class="files-column-entry ${child(path,e.name) === next ? "on" : ""}" data-column-folder="${esc(e.dir ? child(path,e.name) : path)}">${icon(e.dir,e.name)}<span>${esc(e.name)}</span>${e.dir ? "<b>›</b>" : ""}</button>`).join("") : `<p class="files-note">${loading || !entries ? "폴더를 읽는 중…" : "표시할 항목이 없습니다"}</p>`}</section>`;
    }).join("")}</div>`;
  }
  function previewTarget() {
    const p = panes[active];
    return visible(active).find(e => p.selected.has(e.name));
  }
  function updatePreview() {
    if (!layer) return;
    const el = layer.querySelector(".files-preview"), entry = previewTarget();
    const show = previewOn || view === "gallery" || previewModal;
    el.hidden = !show;
    el.classList.toggle("quick-look", previewModal);
    if (!show) { clearTimeout(previewTimer); queuedPreview = null; previewKey = ""; ++previewSeq; return; }
    const identity = `${currentTab}:${active}:${panes[active].path}:${entry?.name || ""}`;
    if (previewKey === identity) return;
    previewKey = identity;
    clearTimeout(previewTimer);
    previewSeq = ++sequence;
    zoom = null;
    el.innerHTML = `<header><strong>미리보기</strong><span class="spacer"></span><button class="btn sm subtle files-zoom" data-action="zoom-fit" title="확대/축소: 핀치·${MAC ? "⌘" : "Ctrl+"}+/−, 두 번 클릭 또는 ${MAC ? "⌘" : "Ctrl+"}0으로 맞춤" hidden></button><button class="btn sm" data-action="close-preview" aria-label="미리보기 닫기">×</button></header><div class="files-preview-content">${entry ? icon(entry.dir,entry.name) : ""}<p>${entry ? esc(entry.name) : "미리볼 파일을 선택하세요"}</p></div>${entry ? `<footer>${esc(kind(entry))} · ${entry.dir ? "폴더" : bytes(entry.size)}</footer>` : ""}`;
    if (!entry || entry.dir || entry.link) return;
    const seq = previewSeq, side = active, path = child(panes[active].path, entry.name);
    el.querySelector(".files-preview-content").insertAdjacentHTML("beforeend", '<small>미리보기 불러오는 중…</small>');
    previewTimer = setTimeout(() => {
      const request = { side, path, seq };
      if (previewInFlight) queuedPreview = request;
      else { previewInFlight = true; call("preview", request); }
    }, 180);
  }
  function previewResult(m) {
    if (m.seq !== previewSeq || !layer) return;
    const el = layer.querySelector(".files-preview-content");
    if (!el) return;
    const d = m.data || {};
    const entry = previewTarget();
    if (!m.ok || d.unsupported || d.too_large) {
      el.innerHTML = `${entry ? icon(entry.dir,entry.name) : ""}<p>${esc(entry?.name || "")}</p><small>${esc(!m.ok ? m.error : d.too_large ? "미리보기 크기 제한을 넘었습니다 (텍스트 512KB, 미디어 16MB)" : "이 형식은 기본 앱에서 열어 확인할 수 있습니다")}</small><button class="btn sm" data-action="open">기본 앱으로 열기</button>`;
    } else if (d.mime === "text/plain") {
      el.innerHTML = '<pre></pre>';
      el.querySelector("pre").textContent = d.text;
    } else {
      const src = `data:${d.mime};base64,${d.base64}`;
      if (d.first_page) layer.querySelector(".files-preview footer").textContent += " · 첫 페이지";
      el.innerHTML = d.mime.startsWith("image/") ? `<img alt="${esc(entry?.name || "미리보기")}" src="${src}">` : `<${d.mime.startsWith("video/") ? "video" : "audio"} controls src="${src}"></${d.mime.startsWith("video/") ? "video" : "audio"}>`;
    }
    zoom = null;
    const img = el.querySelector("img");
    if (img) img.onload = () => applyZoom();
    applyZoom();
    if (!zoomWatch) {
      zoomWatch = new ResizeObserver(() => { if (zoom === null) applyZoom(); });
    }
    zoomWatch.disconnect();
    zoomWatch.observe(el);
  }

  // Preview zoom, like Quick Look: images open fitted (never enlarged past actual size);
  // a pinch (ctrl+wheel in the webview), ⌘+/−/0 or a double-click zooms the picture or
  // the text — never the whole window. zoom: null = fitted, else the scale (1 = actual
  // size; for text, 1 = the normal font size).
  let zoom = null, zoomWatch = null;
  function zoomTarget() {
    const c = layer?.querySelector(".files-preview:not([hidden]) .files-preview-content");
    const img = c?.querySelector("img"), pre = c?.querySelector("pre");
    return c && (img?.naturalWidth || pre) ? { c, img, pre } : null;
  }
  function fitScale(t) {
    if (!t.img) return 1;
    const cs = getComputedStyle(t.c), padX = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight), padY = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
    return Math.min(1, (t.c.clientWidth - padX) / t.img.naturalWidth, (t.c.clientHeight - padY) / t.img.naturalHeight);
  }
  function applyZoom() {
    const t = zoomTarget(), label = layer?.querySelector(".files-preview .files-zoom");
    if (label) label.hidden = !t;
    if (!t) return;
    t.c.classList.toggle("zoomed", zoom !== null && !!t.img);
    if (t.img) {
      // Fitted too gets an explicit size: WebKit doesn't always resolve max-height:100% in
      // the flex column, which left tall pictures cut off instead of fitted.
      const scale = zoom ?? fitScale(t);
      t.img.style.width = `${Math.max(1, Math.round(t.img.naturalWidth * scale))}px`;
      t.img.style.height = `${Math.max(1, Math.round(t.img.naturalHeight * scale))}px`;
    } else t.pre.style.fontSize = zoom === null ? "" : `${(12 * zoom).toFixed(1)}px`;
    if (label) label.textContent = zoom === null ? (t.img ? "맞춤" : "100%") : `${Math.round(zoom * 100)}%`;
  }
  /** Zoom by `factor` (or to `to`) keeping the point under (x, y) where it is. */
  function zoomBy(factor, x, y, to) {
    const t = zoomTarget();
    if (!t) return false;
    const fit = fitScale(t), cur = zoom ?? fit;
    let next = to ?? cur * factor;
    next = Math.min(t.img ? 8 : 4, Math.max(t.img ? fit : 0.5, next));
    const r = t.c.getBoundingClientRect();
    x ??= r.left + r.width / 2; y ??= r.top + r.height / 2;
    const box = (t.img || t.pre).getBoundingClientRect();
    const fx = box.width ? (x - box.left) / box.width : 0.5, fy = box.height ? (y - box.top) / box.height : 0.5;
    zoom = t.img && next <= fit * 1.001 ? null : (Math.abs(next - 1) < 0.02 && !t.img ? null : next);
    applyZoom();
    if (t.img && zoom !== null) {
      t.c.scrollLeft = t.img.offsetLeft + fx * t.img.offsetWidth - (x - r.left);
      t.c.scrollTop = t.img.offsetTop + fy * t.img.offsetHeight - (y - r.top);
    }
    return true;
  }
  const NETWORK = "__network";
  const nativePending = new Set();
  let sequence = 0,
    typeAhead = "",
    typeAt = 0;
  const undo = [];
  const panes = { local: emptyPane(), remote: emptyPane() };
  const jobs = new Map();
  let showTransferHistory = false;
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
      pendingPath: null,
    };
  }
  const call = (op, data = {}) => send("files", { id: host.id, window: pageToken, op, ...data });
  const nativeFiles = () => MAC && window.itdaNativeFiles === true;
  // Dragging past the window's edge hands the files to the system (Finder, or Explorer).
  const nativeDrag = () => nativeFiles() || (!MAC && window.itdaNativeDrag === true);
  const exportItems = (side, names) => panes[side].entries.filter(e => names.includes(e.name)).map(e => ({name:e.name,dir:e.dir}));
  function clearDrag() { drag=null; document.querySelector(".files-drag")?.remove(); layer?.querySelectorAll(".files-drop-target").forEach(el=>el.classList.remove("files-drop-target")); }
  function importPaths(paths, side, destination) {
    const groups=new Map();
    for(const path of paths||[]) { const i=Math.max(path.lastIndexOf("/"),path.lastIndexOf("\\")); if(i<0)continue; const parent=path.slice(0,i)||"/",name=path.slice(i+1);if(!groups.has(parent))groups.set(parent,[]);groups.get(parent).push(name); }
    for(const [source,names]of groups)transfer("local",source,names,side,destination,false);
  }
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
    morph(sidebar, `<h4>위치</h4><button class="btn subtle ${networkMode ? "on" : ""}" data-action="network">${I("E774")}모든 기기</button>${devices.map((h) => `<button class="btn subtle files-device-link ${!networkMode && host.id === h.id ? "on" : ""}" data-device="${esc(h.id)}" data-id="file-device-${esc(h.id)}" ${MAC && host.id === h.id ? 'data-side="remote"' : ""} title="${esc(label(h))} · ${esc(deviceState(h))} · ${MOD}+클릭: 새 창">${I("E7F4")}<span>${esc(label(h))}</span>${(S.fileWindows || []).includes(h.id) ? '<span class="files-device-dot" aria-label="파일 창 열림"></span>' : ""}</button>`).join("")}`);
    layer.querySelector(".files-locations").hidden = true;
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
    return { html: `<section class="files-network" aria-label="네트워크 기기"><header><div><h2>네트워크</h2><p>파일과 화면에 접근할 기기를 선택하세요.</p></div><div class="files-network-actions"><button class="btn sm" data-action="scan">${I("E72C")}기기 찾기</button><button class="btn sm" data-action="add-device">${I("E710")}새 기기</button>${MAC ? '<button class="btn sm" data-action="locations">Finder 연결 폴더</button>' : ""}</div></header><section aria-label="최근 폴더"><h3>최근 폴더</h3>${recentFolderRows(query)}</section><h3>저장된 기기 <span>${devices.length}</span></h3><div class="files-network-grid">${devices.map((h) => `<article class="files-network-device" data-device-card="${esc(h.id)}" data-id="network-device-${esc(h.id)}"><button class="files-network-machine" data-device="${esc(h.id)}" aria-label="${esc(label(h))} 파일 열기">${I("E7F4")}</button><strong>${esc(label(h))}</strong><small>${esc(h.host)}</small><span class="files-network-state">${esc(deviceState(h))}</span><div class="files-network-buttons"><button class="btn sm" data-device="${esc(h.id)}">파일 열기</button><button class="btn sm" data-screen-device="${esc(h.id)}">화면</button><button class="btn sm" data-native-device="${esc(h.id)}">${nativeName}</button></div></article>`).join("") || '<p class="files-note">저장된 기기가 없습니다. 새 기기를 추가하거나 주변 기기를 찾아보세요.</p>'}</div>${found.length ? `<h3>주변 Itda 기기</h3><div class="files-network-grid" data-found-list>${discovered(found, "nearby")}</div>` : ""}${peers.length ? `<h3>Tailscale 기기</h3><div class="files-network-grid" data-peer-list>${discovered(peers, "peer")}</div>` : ""}<p class="files-network-hint">파일은 화면 공유를 켜지 않아도 연결할 수 있습니다. Finder·탐색기로 열려면 상대 기기의 파일 공유가 켜져 있어야 합니다.</p></section>`, devices, found, peers };
  }
  const matchingFolders = query => recentFolders().filter(x => !query || `${x.path} ${label(hostById(x.id))}`.toLowerCase().includes(query.toLowerCase()));
  function recentFolderRows(query) {
    const folders = matchingFolders(query);
    const row = (f, i) => {
      const name = f.path.split(/[\\/]/).filter(Boolean).pop() || "/";
      return `<button class="files-home-recent" data-recent="${i}" title="${esc(f.path)}">${icon(true)}<strong>${esc(name)}</strong><span>${esc(label(hostById(f.id)))}</span><small>${esc(f.path)}</small></button>`;
    };
    return folders.length ? `<div class="files-home-recents">${folders.map(row).join("")}</div>` : `<p class="files-home-empty">${query ? "검색어와 일치하는 최근 폴더가 없습니다." : "아직 연 폴더가 없습니다. 기기를 고르면 저장된 계정으로 파일을 엽니다."}</p>`;
  }
  function renderBrowser() {
    // Finder-like start: the devices as tiles, then recent folders as a list.
    const devices = networkData(S.query).devices;
    const status = (h) => S.online[h.id] === true ? '<i class="files-dot on" title="응답함"></i>' : S.online[h.id] === false ? '<i class="files-dot" title="응답 없음"></i>' : "";
    const tile = (h) => `<button class="files-home-device" data-device="${esc(h.id)}" title="${esc(label(h))} 파일 열기 · ${MOD}+클릭: 새 창"><span class="files-home-os">${osIcon(h)}</span><span class="files-home-name">${status(h)}<span>${esc(label(h))}</span></span><small>${esc(deviceState(h) === "저장된 기기" ? h.host : deviceState(h))}</small></button>`;
    return `<div class="files-home files-start">
      <section><h3>기기</h3><div class="files-home-devices">${devices.map(tile).join("")}<button class="files-home-device add" data-action="add-device"><span class="files-home-os">${I("E710")}</span><span class="files-home-name">위치 추가</span><small>Mac · Windows · Linux</small></button></div></section>
      <section><h3>최근 폴더</h3>${recentFolderRows(S.query)}</section>
    </div>`;
  }
  function renderNetwork() {
    previewModal = false; previewKey = ""; queuedPreview = null; ++previewSeq;
    layer.querySelector(".files-preview").hidden = true;
    layer.classList.remove("has-preview", "gallery-view");
    const { html, devices, found, peers } = networkData();
    morph(layer.querySelector(".files-panes"), html);
    layer.querySelector("[data-status]").textContent =
      `저장된 기기 ${devices.length}대`;
    layer.querySelector("[data-location]").textContent = "네트워크";
    layer.querySelector("[data-footer-path]").replaceChildren();
    delete layer.querySelector("[data-footer-path]").dataset.footerLocation;
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
    if (S.page !== "files" || e.target.closest(".files-layer") || !e.target.closest(".files-home")) return;
    const recent = e.target.closest("[data-recent]");
    if (recent) {
      const f = recentFolders().filter(x => !S.query || `${x.path} ${label(hostById(x.id))}`.toLowerCase().includes(S.query.toLowerCase()))[Number(recent.dataset.recent)];
      if (f) launch(hostById(f.id), inNewWindow(e), f.path);
      return;
    }
    const device = e.target.closest("[data-device]");
    if (device) return launch(hostById(device.dataset.device), inNewWindow(e));
    const native = e.target.closest("[data-native-device]");
    if (native) {
      nativePending.add(native.dataset.nativeDevice);
      send("files", { id: native.dataset.nativeDevice, window: pageToken, op: "finder" });
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
  function open(h, reuse = false) {
    if (!h) return;
    if (host && host.id !== h.id) {
      if (host.id !== NETWORK) call("release");
      layer?.remove();
      layer = null;
      clipboard = null;
      undo.length = 0;
    }
    host = h;
    if (fileWindow()) send("files_window_context", {window:fileWindow(),id:h.id,page:pageToken});
    document.title = `${label(h)} — 파일`;
    showSetup();
    const f = h.files || {};
    if (reuse && (h.login_user || f.user || f.account || f.share || f.method === "smb")) {
      setupStarted = true;
      setup.querySelector("[data-connect]").disabled = true;
      setup.classList.add("is-connecting");
      setup.querySelector("[data-close]").focus();
      call("connect", { options: { ...f, method: f.method || "auto" }, reuse: true });
    }
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
      if (!layer && window.ITDA_FILE_HOST) network();
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
    pendingFolder = window.ITDA_FILE_PATH || null;
    open(h, true);
  }
  function launch(h, newWindow = false, path = null) {
    if (!h) return;
    if (newWindow) return send("files_window", { id: h.id, new: true, path: path || (host?.id === h.id && layer && !networkMode && active === "remote" ? panes.remote.path : null) });
    if (host?.id === h.id && layer) {
      active = "remote"; networkMode = false; if (path) nav("remote", path); else renderPanes(); return;
    }
    pendingFolder = path;
    open(h, true);
  }
  function workspace(data) {
    if (fileWindow()) send("files_window_context", { window: fileWindow(), id: host.id, page: pageToken });
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
    previewKey = ""; queuedPreview = null; ++previewSeq;
    fileTabs.length = 0; currentTab = ++tabId; fileTabs.push({ id: currentTab }); folderCache.clear(); columnRequests.clear();
    layer.className = `files-layer ${MAC ? "mac" : "windows"}`;
    const tool = (action, glyph, text) =>
      `<button class="btn sm" data-action="${action}" title="${text}" aria-label="${text}">${I(glyph)}</button>`;
    layer.innerHTML = `<div class="files-title"><div class="files-nav">${tool("back", "E72B", "뒤로")}${tool("forward", "E76C", "앞으로")}</div>${icon(true)}<b>${esc(label(host))}</b><span class="files-folder-title"></span><span class="spacer"></span>${tool("view", "E80A", "보기")}${tool("mkdir", "E8F4", "새 폴더")}${tool("refresh", "E72C", "새로 고침")}<button class="btn sm" data-action="more" title="동작" aria-label="동작">${I("E712")}</button>${tool("settings", "E713", "연결 설정")}${tool("native", "E8A7", `${nativeName}에서 열기`)}<input class="input files-search" placeholder="검색" aria-label="파일 검색" data-search></div>
      <nav class="files-tabs" role="tablist" aria-label="폴더 탭"></nav><aside class="files-preview" hidden></aside><nav class="files-locations" aria-label="기기 위치"></nav><div class="files-tools">${tool("up", "E80F", "상위 폴더")}${tool("copy", "E8C8", "복사")}<button class="btn sm" data-action="cut">잘라내기</button><button class="btn sm" data-action="paste">붙여넣기</button>${tool("rename", "E70F", "이름 변경")}${tool("trash", "E74D", "보관함으로")}${tool("undo", "E81C", "되돌리기")}<span class="spacer"></span><button class="btn sm" data-action="split">분할 보기</button><button class="btn sm" data-action="hidden">숨김 파일</button><button class="btn sm" data-action="transfers">전송 내역</button></div>
      <div class="files-error" hidden role="alert"></div><div class="files-body"><aside class="files-sidebar"><div data-network-sidebar></div><div class="files-current-places"><h4>즐겨찾기</h4><button class="btn subtle" data-bookmark="home">${I("E80F")}홈 폴더</button><button class="btn subtle" data-bookmark="Downloads">${I("E8B7")}다운로드</button><button class="btn subtle" data-bookmark="Documents">${I("E8A5")}문서</button><button class="btn subtle" data-bookmark="Desktop">${I("E7F4")}${MAC ? "데스크탑" : "바탕 화면"}</button><h4>위치</h4><button class="btn subtle" data-side="remote">${I("E7F4")}${esc(label(host))}</button><button class="btn subtle" data-side="local">${I("E7F8")}${HERE}</button><h4>파일 관리</h4><button class="btn subtle" data-bookmark=".itda-trash">${I("E74D")}보관함</button></div></aside><div class="files-panes"></div></div><div class="files-jobs" hidden></div><div class="files-status"><div data-footer-path></div><span data-status></span><span data-location></span></div>`;
    document.body.append(layer);
    {
      const title = layer.querySelector(".files-title");
      const views = document.createElement("div");
      views.className = "files-view-switch";
      views.setAttribute("role", "group");
      views.setAttribute("aria-label", "보기 방식");
      views.innerHTML = viewModes.map((mode,i) => `<button class="btn sm" data-mode="${mode}" title="${viewLabels[i]}" aria-label="${viewLabels[i]}">${["<svg viewBox='0 0 20 20' aria-hidden='true'><path d='M3 3h5v5H3zM12 3h5v5h-5zM3 12h5v5H3zM12 12h5v5h-5z'/></svg>","<svg viewBox='0 0 20 20' aria-hidden='true'><path d='M3 4h2M8 4h9M3 10h2M8 10h9M3 16h2M8 16h9'/></svg>","<svg viewBox='0 0 20 20' aria-hidden='true'><path d='M2 3h16v14H2zM7 3v14M13 3v14'/></svg>","<svg viewBox='0 0 20 20' aria-hidden='true'><path d='M2 3h16v10H2zM2 16h4M8 16h4M14 16h4'/></svg>"][i]}</button>`).join("") + tool("preview", "E890", "미리보기 패널");
      title.insertBefore(views, title.querySelector('[data-action="view"]'));
      if (MAC) layer.querySelector('[data-side="remote"]').remove();
      if (MAC) {
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
    }
    layer.querySelector(".files-title").addEventListener("pointerdown", (e) => {
      if (MAC && e.button === 0 && !e.target.closest("button,input")) send("files_drag");
    });
    layer.addEventListener("pointerdown", resizeStart, true);
    layer.addEventListener("click", click);
    layer.addEventListener("dblclick", dblclick);
    // A pinch arrives as ctrl+wheel: zoom the preview under it, and never the whole window
    // (the home's own ctrl+wheel zoom stays outside the file browser).
    let wheelPinch = 0, gestureScale = 1;
    layer.addEventListener("wheel", e => {
      if (!e.ctrlKey) return;
      e.preventDefault(); e.stopPropagation();
      wheelPinch = performance.now();
      if (e.target.closest(".files-preview-content")) zoomBy(Math.exp(-e.deltaY * (e.deltaMode ? 0.05 : 0.01)), e.clientX, e.clientY);
    }, { passive: false });
    // WebKit may report a pinch as gesture events instead (used only when no ctrl+wheel came).
    layer.addEventListener("gesturestart", e => { e.preventDefault(); gestureScale = 1; });
    layer.addEventListener("gesturechange", e => {
      e.preventDefault();
      if (performance.now() - wheelPinch < 200 || !e.target.closest?.(".files-preview-content")) return;
      zoomBy(e.scale / gestureScale, e.clientX, e.clientY);
      gestureScale = e.scale;
    });
    layer.addEventListener("gestureend", e => e.preventDefault());
    layer.addEventListener("contextmenu", context);
    layer.addEventListener("pointerdown", pointerDown);
    layer.addEventListener("dragstart", e => { if (!e.target.closest("input,textarea")) e.preventDefault(); });
    layer.addEventListener("selectstart", e => { if (!e.target.closest("input,textarea,pre")) e.preventDefault(); });
    layer.querySelector("[data-search]").oninput = (e) => {
      search = e.target.value;
      for (const p of Object.values(panes)) {
        p.start = 0;
        p.scroll = 0;
      }
      renderPanes();
    };
    saveTab();
    rememberFolder();
    renderPanes();
    renderJobs();
    if (pendingFolder) { const path = pendingFolder; pendingFolder = null; nav("remote", path); }
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
  function pathHTML(side) {
    const p = panes[side];
    return `<div class="files-path"><strong>${side === "local" ? HERE : esc(label(host))}</strong><nav class="files-breadcrumbs" aria-label="폴더 경로">${crumbs(p.path, side)}</nav><input aria-label="폴더 경로 입력" value="${esc(p.path)}" data-path="${side}"></div>`;
  }
  function renderPanes() {
    if (!layer) return;
    const focusedSide = document.activeElement?.closest(".files-list")?.closest("[data-pane]")?.dataset.pane;
    refreshNetwork();
    tabBar();
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
        return `<section class="files-pane ${side === active ? "active" : ""}" data-pane="${side}">${MAC ? "" : pathHTML(side)}<div class="files-list" tabindex="0" role="grid" aria-label="${side === "local" ? "내 기기" : "원격 기기"} 파일">${view === "columns" ? columnHTML(side) : p.busy ? '<div class="files-note">폴더를 읽는 중…</div>' : !list.length ? '<div class="files-note">표시할 항목이 없습니다</div>' : ["grid", "gallery"].includes(view) ? `<div class="files-grid ${view === "gallery" ? "files-gallery-strip" : ""}">${list.map((e, i) => `<div ${cell(e, i)}>${icon(e.dir, e.name)}<span>${esc(e.name)}${e.link ? " ↗" : ""}</span></div>`).join("")}</div>` : `<table><colgroup>${[47, 25, 12, 16].map((w, i) => `<col style="width:${columnWidths[side]?.[i] ? `${columnWidths[side][i]}px` : `${w}%`}">`).join("")}</colgroup><thead><tr>${[["name", "이름"], ["modified", "수정한 날짜"], ["size", "크기"], ["kind", "종류"]].map(([key, title], i) => `<th data-sort="${key}">${title}${p.sort === key ? (p.desc ? " ▾" : " ▴") : ""}${i < 3 ? `<span data-resize-column="${i}" role="separator" aria-label="${title} 열 너비" aria-orientation="vertical" tabindex="0"></span>` : ""}</th>`).join("")}</tr></thead><tbody>${rows(side)}</tbody></table>`}</div></section>`;
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
    layer.classList.toggle("has-preview", previewOn || view === "gallery");
    layer.classList.toggle("gallery-view", view === "gallery");
    layer.querySelectorAll(".files-columns").forEach(el => { el.scrollLeft = el.scrollWidth; });
    status();
    updatePreview();
  }
  function status() {
    if (!layer) return;
    const p = panes[active];
    if (MAC) {
      const footer = layer.querySelector("[data-footer-path]"), key = `${host.id}:${active}:${p.path}`;
      if (footer.dataset.footerLocation !== key) { footer.innerHTML = pathHTML(active); footer.dataset.footerLocation = key; }
    }
    layer.querySelector("[data-status]").textContent =
      `${visible(active).length}개 항목${p.selected.size ? ` · ${p.selected.size}개 선택됨` : ""}`;
    layer.querySelector("[data-location]").textContent = p.path;
    layer.querySelector(".files-folder-title").textContent =
      p.path.split(/[\\/]/).filter(Boolean).pop() || p.path;
    layer.querySelector('[data-action="paste"]').disabled = !clipboard && !nativeFiles();
    for (const a of ["copy", "cut", "rename", "trash"])
      layer.querySelector(`[data-action="${a}"]`).disabled = !p.selected.size;
  }
  function nav(side, path, history = true) {
    const p = panes[side];
    folderCache.set(cacheKey(side, p.path), p.entries);
    p.seq = ++sequence;
    p.busy = true;
    p.pendingPath = path;
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
    if (/^[A-Za-z]:[\\/]?$/.test(path)) return path.slice(0,2) + "\\";
    if (/^\\\\[^\\]+\\[^\\]+[\\]?$/.test(path)) return path.replace(/[\\]+$/, "");
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
    updatePreview();
  }
  function click(e) {
    if (suppressClick) { suppressClick = false; e.preventDefault(); e.stopPropagation(); return; }
    const close = e.target.closest("[data-close-tab]");
    if (close) return closeTab(Number(close.dataset.closeTab));
    const recent = e.target.closest("[data-recent]");
    if (recent) {
      const folder = matchingFolders(search)[Number(recent.dataset.recent)];
      if (folder) launch(hostById(folder.id), inNewWindow(e), folder.path);
      return;
    }
    const tab = e.target.closest("[data-tab]");
    if (tab) return switchTab(Number(tab.dataset.tab));
    const column = e.target.closest("[data-column-folder]");
    if (column) return nav(column.closest("[data-pane]").dataset.pane, column.dataset.columnFolder);
    const device = e.target.closest("[data-device]");
    if (device) {
      const h = hostById(device.dataset.device);
      if (!inNewWindow(e) && h?.id === host.id && layer && host.id !== NETWORK) {
        active = "remote";
        networkMode = false;
        renderPanes();
      } else launch(h, inNewWindow(e));
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
      send("files", { id: native.dataset.nativeDevice, window: pageToken, op: "finder" });
      return;
    }

    const crumb = e.target.closest("[data-crumb]");
    if (crumb) {
      active = crumb.dataset.crumb;
      nav(active, crumb.dataset.folder);
      return;
    }

    removeMenu();
    if (e.target.closest("[data-cancel-job], [data-dismiss-job]")) return;
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
    if (item) {
      select(active, Number(item.dataset.index), e);
      const entry = visible(active)[Number(item.dataset.index)];
      if (view === "columns" && entry?.dir && !e.metaKey && !e.ctrlKey && !e.shiftKey) nav(active, child(panes[active].path, entry.name));
    }
    else if (!e.target.closest("input")) {
      panes[active].selected.clear();
      paintSelection();
    }
  }
  function dblclick(e) {
    if (e.target.closest(".files-preview-content img")) { zoomBy(1, e.clientX, e.clientY, zoom === null ? 1 : 0); return; }
    const item = e.target.closest("[data-index]");
    const pane = e.target.closest("[data-pane]");
    if (!item || !pane || panes[pane.dataset.pane].busy) return;
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
    if (action === "new-window") return launch(host, true);
    if (action === "new-tab") return newTab();
    if (action === "zoom-fit") { if (zoom === null) zoomBy(1, undefined, undefined, 1); else { zoom = null; applyZoom(); } return; }
    if (action === "preview") { previewOn = !previewOn; previewKey = ""; renderPanes(); return; }
    if (action === "close-preview") { previewOn = false; previewModal = false; if (view === "gallery") view = "list"; previewKey = ""; renderPanes(); return; }
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
          send("files_window", { id: host.id, window: fileWindow(), close: true });
          return;
        }
        send("files_release_page", {page:pageToken});
        layer.remove();
        layer = null;
        removeMenu();
        document.title = "Itda";
        render();
        return;
      case "transfers":
        showTransferHistory = !showTransferHistory;
        renderJobs();
        return;
      case "more": {
        const rect = layer.querySelector('[data-action="more"]').getBoundingClientRect();
        showMenu([
          ["open", "열기"], ["preview", "미리보기 패널"], ["new-tab", "새 탭"], ["new-window", `새 창 (${MOD}N)`], ["mkdir", "새 폴더"],
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
          if(nativeFiles()) {
            clipboard.seq=crypto.randomUUID();
            send("files_native_copy",{id:host.id,window:pageToken,seq:clipboard.seq,side:active,path:p.path,items:exportItems(active,clipboard.names)});
            toast("Finder에서 붙여넣을 파일을 준비하고 있습니다", "E8B7");
          }
        }
        return;
      case "paste":
        if(nativeFiles()) { send("files_native_paste",{id:host.id,window:pageToken,stamp:clipboard?.nativeStamp,side:active,path:p.path}); return; }
      case "paste-internal":
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
      .filter((j) => j.id === host.id && (showTransferHistory || ["queued", "running", "failed"].includes(j.state)))
      .slice(-12);
    layer.querySelector(".files-jobs").hidden = current.length === 0 && !showTransferHistory;
    layer.querySelectorAll('[data-action="transfers"]').forEach(b => b.setAttribute("aria-pressed", String(showTransferHistory)));
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
        return `<div class="files-job"><span>${j.engine === "rsync" ? "증분" : "파일"}</span><span class="job-label" title="${esc(j.error || j.file || "")}">${esc(j.file || "파일 전송")} · ${esc(text)}${j.error ? ` · ${esc(j.error)}` : ""}</span>${live ? `<progress ${j.total ? `max="${j.total}" value="${j.bytes || 0}"` : ""}></progress><button class="btn sm" data-cancel-job="${esc(j.job)}">취소</button>` : `<button class="btn sm" data-dismiss-job="${esc(j.job)}" aria-label="전송 알림 닫기">닫기</button>`}</div>`;
      })
      .join("");
    layer
      .querySelectorAll("[data-cancel-job]")
      .forEach(
        (b) => (b.onclick = () => call("cancel", { job: b.dataset.cancelJob })),
      );
    layer.querySelectorAll("[data-dismiss-job]").forEach(b => b.onclick = () => { jobs.delete(b.dataset.dismissJob); renderJobs(); });
    if (!current.length && showTransferHistory) layer.querySelector(".files-jobs").textContent = "최근 전송 내역이 없습니다";
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
    const disabled = (a) => ["open", "copy", "cut", "trash"].includes(a) ? !p.selected.size : a === "rename" ? p.selected.size !== 1 : a === "paste" ? !clipboard && !nativeFiles() : a === "undo" ? !undo.length : false;
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
    if ((MAC ? e.metaKey : e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "n" && (layer || S.page === "files") && !document.querySelector(".files-prompt,.files-menu") && !topModal()) {
      e.preventDefault(); e.stopImmediatePropagation();
      const device = layer ? null : e.target.closest("[data-device]");
      launch(device ? hostById(device.dataset.device) : host || {id:NETWORK}, true);
      return;
    }
    // WKWebView does not focus buttons after a click. Route file shortcuts even when
    // focus falls back to the document, while leaving dialogs and popup menus in charge.
    if (!layer || document.querySelector(".files-prompt,.files-menu") || topModal()) return;
    if (e.target.matches("[data-resize-column],[data-resize-sidebar]")) {
      if (["ArrowLeft", "ArrowRight"].includes(e.key)) {
        e.preventDefault();
        e.stopPropagation();
        resizeBy(e.target, e.key === "ArrowRight" ? 10 : -10);
        return;
      }
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
    const focusedTab = e.target.closest("[data-tab]");
    if (focusedTab && ["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) {
      e.preventDefault();
      const i = fileTabs.findIndex(t => t.id === currentTab);
      const next = e.key === "Home" ? 0 : e.key === "End" ? fileTabs.length - 1 : (i + (e.key === "ArrowRight" ? 1 : fileTabs.length - 1)) % fileTabs.length;
      switchTab(fileTabs[next].id);
      layer.querySelector(`[data-tab="${currentTab}"]`)?.focus(); return;
    }
    const modKey = MAC ? e.metaKey : e.ctrlKey;
    if (modKey && e.key.toLowerCase() === "t") { e.preventDefault(); newTab(); return; }
    if (modKey && e.key.toLowerCase() === "w" && fileTabs.length > 1) { e.preventDefault(); closeTab(currentTab); return; }
    if (e.ctrlKey && e.key === "Tab") { e.preventDefault(); const i = fileTabs.findIndex(t => t.id === currentTab); switchTab(fileTabs[(i + (e.shiftKey ? fileTabs.length - 1 : 1)) % fileTabs.length].id); return; }
    if (e.key === "Escape" && previewModal) { e.preventDefault(); previewModal = false; previewKey = ""; updatePreview(); return; }
    const input = e.target.closest("input,textarea");
    if (input) {
      if (e.key === "Enter" && input.dataset.path) {
        e.preventDefault();
        nav(input.dataset.path, input.value);
      }
      return;
    }
    const zoomKey = (MAC ? e.metaKey : e.ctrlKey) && !e.altKey && ["=", "+", "-", "0"].includes(e.key);
    if (zoomKey && zoomTarget()) {
      e.preventDefault();
      if (e.key === "0") { zoom = null; applyZoom(); } else zoomBy(e.key === "-" ? 1 / 1.25 : 1.25);
      return;
    }
    if (e.code === "Space" || e.key === " ") { e.preventDefault(); previewModal = !previewModal; previewKey = ""; updatePreview(); return; }
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
    if (mod && ["1", "2", "3", "4"].includes(e.key)) {
      e.preventDefault();
      view = viewModes[Number(e.key) - 1];
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
      drag = null;
      document.querySelector(".files-drag")?.remove();
      layer.querySelectorAll(".files-drop-target").forEach(el => el.classList.remove("files-drop-target"));
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
    e.preventDefault();
    try { item.setPointerCapture(e.pointerId); } catch {}
    window.getSelection()?.removeAllRanges();
    layer.querySelector(`[data-pane="${side}"] .files-list`)?.focus({ preventScroll: true });
    drag = {
      x: e.clientX,
      y: e.clientY,
      side,
      path: panes[side].path,
      names: panes[side].selected.has(entry.name)
        ? [...panes[side].selected]
        : [entry.name],
      live: false, capture:item, pointer:e.pointerId,
    };
  }
  document.addEventListener("pointermove", (e) => {
    if (!drag || !layer) return;
    if (!drag.live && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) > 7) {
      drag.live = true;
      window.getSelection()?.removeAllRanges();
      panes[drag.side].selected = new Set(drag.names);
      paintSelection();
      const tag = document.createElement("div");
      tag.className = "files-drag";
      document.body.append(tag);
    }
    if (drag.live && nativeDrag() && (e.clientX<=1 || e.clientY<=1 || e.clientX>=innerWidth-1 || e.clientY>=innerHeight-1)) {
      const d=drag,items=exportItems(d.side,d.names);
      try{d.capture.releasePointerCapture(d.pointer);}catch{}
      clearDrag(); suppressClick=true; setTimeout(()=>{suppressClick=false;},0);
      send("files_native_drag",{id:host.id,window:pageToken,side:d.side,path:d.path,items});
      return;
    }
    if (drag.live) {
      const tag = document.querySelector(".files-drag");
      tag.textContent = `${e.shiftKey ? "이동" : "복사"} · ${drag.names.length}개`;
      tag.style.left = `${e.clientX + 14}px`;
      tag.style.top = `${e.clientY + 12}px`;
      layer.querySelectorAll(".files-drop-target").forEach(el => el.classList.remove("files-drop-target"));
      const target = document.elementFromPoint(e.clientX,e.clientY);
      const row = target?.closest("[data-index]");
      const side = row?.closest("[data-pane]")?.dataset.pane;
      if (side && visible(side)[Number(row.dataset.index)]?.dir) row.classList.add("files-drop-target");
      else target?.closest("[data-bookmark],[data-tab]")?.classList.add("files-drop-target");
      const scroller = target?.closest(".files-list,.files-column");
      if (scroller) {
        const r = scroller.getBoundingClientRect();
        if (e.clientY < r.top + 28) scroller.scrollTop -= 20;
        else if (e.clientY > r.bottom - 28) scroller.scrollTop += 20;
      }
    }
  });
  function finishDrag(e) {
      if (!drag) return;
      const d = drag;
      drag = null;
      document.querySelector(".files-drag")?.remove();
      layer?.querySelectorAll(".files-drop-target").forEach(el => el.classList.remove("files-drop-target"));
      if (!d.live || !layer) return;
      suppressClick = true;
      setTimeout(() => { suppressClick = false; }, 0);
      const target = document.elementFromPoint(e.clientX, e.clientY),
        pane = target?.closest("[data-pane]");
      if (!pane) {
        const bookmark = target?.closest("[data-bookmark]");
        const tab = target?.closest("[data-tab]");
        if (bookmark) {
          const p = panes[active];
          const to = bookmark.dataset.bookmark === "home" ? p.home : child(p.home, bookmark.dataset.bookmark);
          if (d.side !== active || d.path !== to) transfer(d.side, d.path, d.names, active, to, e.shiftKey);
        } else if (tab) {
          const t = fileTabs.find(t => t.id === Number(tab.dataset.tab));
          const side = t.id === currentTab ? active : t.active;
          const to = t.id === currentTab ? panes[side].path : t.panes[side].path;
          if (d.side !== side || d.path !== to) transfer(d.side, d.path, d.names, side, to, e.shiftKey);
        }
        return;
      }
      const side = pane.dataset.pane,
        item = target.closest("[data-index]"),
        entry = item ? visible(side)[Number(item.dataset.index)] : null;
      const to = entry?.dir
        ? child(panes[side].path, entry.name)
        : panes[side].path;
      if (d.side === side && d.path === to) return;
      transfer(d.side, d.path, d.names, side, to, e.shiftKey);
  }
  document.addEventListener("pointerup", finishDrag, true);
  // Some native WebKit input paths deliver a mouse-up without a matching pointer-up.
  document.addEventListener("mouseup", finishDrag, true);
  document.addEventListener("pointercancel", clearDrag);
  window.addEventListener("blur", clearDrag);
  function message(m) {
    if (["files_result", "files_job", "files_native_paste", "files_native_drag_end"].includes(m.type) && m.window !== pageToken) return true;
    if (m.type === "files_native_drag_end") { if(m.id===host?.id){clearDrag();if(m.error)fail(m.error);}return true; }
    if (m.type === "files_native_paste") {
      if(m.id!==host?.id)return true;
      if(m.internal && clipboard)act("paste-internal");
      else if(m.paths?.length)importPaths(m.paths,m.side,m.path);
      else toast("클립보드에 붙여넣을 파일이 없습니다", "E8B7");
      return true;
    }
    if(m.type==="files_result" && ["native_copy_started","native_copy"].includes(m.op)) {
      if(m.id!==host?.id || m.seq!==clipboard?.seq)return true;
      if(m.ok){clipboard.nativeStamp=m.data.stamp;if(m.op==="native_copy")toast(clipboard.move ? "Finder에서는 복사됩니다 — Itda 안의 붙여넣기는 이동합니다" : "파일 복사 준비 완료 — Finder에서 붙여넣으세요", "E8B7");}
      else fail(m.error||"파일 복사를 준비하지 못했습니다");
      return true;
    }
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
      if (m.state === "completed" && previous.state !== "completed") {
        const device = hostById(m.id);
        toast(`${device ? `${label(device)} · ` : ""}${previous.kind === "export" || m.kind === "export" ? "파일 내보내기 완료" : previous.moving || m.moving ? "파일 이동 완료" : "파일 복사 완료"}`, "E8B7");
      }
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
        ("window" in m ? m.window !== (fileWindow() || null) : ((m.id && m.id !== host.id) || (window.ITDA_FILE_HOST && !m.id)))
      )
        return true;
      if (networkMode) {
        toast("전송할 기기의 파일 창을 먼저 열어 주세요", "E8B7");
        return true;
      }
      let side=active,destination=panes[active].path;
      if(m.position){const target=document.elementFromPoint(...m.position),pane=target?.closest("[data-pane]");if(pane){side=pane.dataset.pane;const item=target.closest("[data-index]"),entry=item?visible(side)[Number(item.dataset.index)]:null;destination=entry?.dir?child(panes[side].path,entry.name):panes[side].path;}}
      importPaths(m.paths,side,destination);
      return true;
    }
    if (m.type !== "files_result") return false;
    if (!host || m.id !== host.id) return true;
    if (m.op === "preview") {
      previewInFlight = false;
      previewResult(m);
      if (queuedPreview) { const request = queuedPreview; queuedPreview = null; if (request.seq === previewSeq) { previewInFlight = true; call("preview", request); } }
      return true;
    }
    if (m.op === "list" && columnRequests.has(m.seq)) {
      const request = columnRequests.get(m.seq); columnRequests.delete(m.seq);
      folderCache.set(request.key, m.ok ? m.data.entries : []);
      if (view === "columns") renderPanes();
      return true;
    }
    if (!m.ok) {
      if (m.op === "list") {
        const p = panes[m.side];
        if (m.seq !== p.seq) return true;
        p.busy = false;
        p.pendingPath = null;
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
        send("files_window", { id: host.id, window: fileWindow(), close: true });
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
      folderCache.set(cacheKey(m.side, p.path), p.entries);
      if (folderCache.size > 64) folderCache.delete(folderCache.keys().next().value);
      if (m.side === "remote") rememberFolder();
      p.start = Math.min(p.start || 0, Math.max(0, p.entries.length - 160));
      p.scroll = Math.min(
        p.scroll || 0,
        Math.max(0, (p.entries.length - 1) * rowHeight),
      );
      p.busy = false;
      p.pendingPath = null;
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
  function nativeEdit(action) {
    const selection=window.getSelection();
    const text=document.activeElement?.closest("input,textarea") || (!selection?.isCollapsed && selection?.anchorNode?.parentElement?.closest(".files-preview pre"));
    if(!layer || networkMode || text || document.querySelector(".files-prompt") || topModal())send("files_system_edit",{action});
    else act(action);
  }
  function nativeNew(force = false) {
    if (document.querySelector(".files-prompt,.files-menu") || topModal()) return;
    if (force || layer || S.page === "files") launch(host || {id: NETWORK}, true);
    else openEdit();
  }
  window.ItdaFiles = {
    windowToken: pageToken,
    nativeNew,
    nativeEdit,
    renderBrowser,
    browserSearch,
    open,
    launch,
    boot,
    message,
    network,
    refreshNetwork,
  };
  for(const type of ["copy","cut","paste"])document.addEventListener(type,e=>{
    if(!layer || networkMode || document.querySelector(".files-prompt,.files-menu") || topModal())return;
    const selected=window.getSelection();
    if(e.target.closest?.("input,textarea") || document.activeElement?.closest("input,textarea") || (!selected?.isCollapsed && selected?.anchorNode?.parentElement?.closest(".files-preview pre")))return;
    e.preventDefault();e.stopPropagation();act(type);
  },true);
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
