// Icons outside Windows. The page writes Segoe Fluent Icons glyphs (`<span class="i">&#xE710;</span>`),
// a font only Windows has; elsewhere each glyph is swapped for a matching stroke icon.
'use strict';

(() => {
  if (/Windows/.test(navigator.userAgent)) return;
  const star = 'M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z';
  const eye = '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/>';
  const search = '<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.3-4.3"/>';
  const P = {
    E701: '<path d="M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0M2 9a15 15 0 0 1 20 0"/><circle class="f" cx="12" cy="19.5" r="1"/>',
    E703: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
    E70F: '<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>',
    E710: '<path d="M12 5v14M5 12h14"/>',
    E711: '<path d="M6 6l12 12M18 6L6 18"/>',
    E712: '<circle class="f" cx="5" cy="12" r="1.4"/><circle class="f" cx="12" cy="12" r="1.4"/><circle class="f" cx="19" cy="12" r="1.4"/>',
    E713: '<circle cx="12" cy="12" r="3"/><circle cx="12" cy="12" r="7"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9L7 7M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1"/>',
    E71E: search,
    E721: search,
    E722: '<path d="M4 8h3l2-2.5h6L17 8h3v11H4z"/><circle cx="12" cy="13" r="3.5"/>',
    E72B: '<path d="M15 5l-7 7 7 7"/>',
    E72C: '<path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7"/>',
    E72D: '<circle cx="18" cy="5.5" r="2.5"/><circle cx="6" cy="12" r="2.5"/><circle cx="18" cy="18.5" r="2.5"/><path d="M8.2 10.8l7.6-4.1M8.2 13.2l7.6 4.1"/>',
    E72E: '<rect x="5" y="10.5" width="14" height="9.5" rx="2"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/>',
    E734: `<path d="${star}"/>`,
    E735: `<path class="f" d="${star}"/>`,
    E73E: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
    E74D: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
    E756: '<rect x="3" y="4.5" width="18" height="15" rx="2"/><path d="M7 9.5l3 2.5-3 2.5M12.5 15H17"/>',
    E765: '<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6 10h.01M9.5 10h.01M13 10h.01M16.5 10h.01M7 14h10"/>',
    E768: '<path d="M7.5 4.5v15l12-7.5z"/>',
    E76C: '<path d="M9 5l7 7-7 7"/>',
    E774: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>',
    E77B: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
    E77F: '<rect x="6" y="4.5" width="12" height="16" rx="2"/><path d="M9.5 4.5h5v2.5h-5z"/>',
    E783: '<circle cx="12" cy="12" r="9"/><path d="M12 7.5V13M12 16.5h.01"/>',
    E790: '<path d="M12 3a9 9 0 0 0 0 18c1.2 0 1.8-.8 1.8-1.8 0-1.3-1-1.7-1-2.7 0-.9.7-1.5 1.6-1.5H17a4 4 0 0 0 4-4C21 6.6 17 3 12 3z"/><circle cx="7.5" cy="11" r="1"/><circle cx="10" cy="7" r="1"/><circle cx="15" cy="7" r="1"/>',
    E7B3: eye,
    E7BA: '<path d="M12 3.5l9.5 16.5h-19z"/><path d="M12 10v4.5M12 17.5h.01"/>',
    E7C4: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18M8 6.5h.01"/>',
    E7F4: '<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M9 20h6M12 16.5V20"/>',
    E7F8: '<rect x="4.5" y="5" width="15" height="10" rx="1.5"/><path d="M2.5 19h19"/>',
    E80F: '<path d="M4 11l8-7 8 7v9h-5v-6H9v6H4z"/>',
    E81C: '<path d="M3.5 12a8.5 8.5 0 1 0 2.5-6M3.5 4v4h4M12 7.5V12l3 2"/>',
    E82D: '<path d="M5 4.5h11a3 3 0 0 1 3 3V20H8a3 3 0 0 1-3-3z"/><path d="M5 17a3 3 0 0 1 3-3h11"/>',
    E890: eye,
    E897: '<circle cx="12" cy="12" r="9"/><path d="M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6v.6M12 17h.01"/>',
    E8A5: '<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4M9 12h6M9 16h6"/>',
    E8A7: '<path d="M14 4h6v6M20 4l-8 8M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
    E8AB: '<path d="M4 8h14l-3-3M20 16H6l3 3"/>',
    E8B7: '<path d="M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2h8.5A1.5 1.5 0 0 1 21 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"/>',
    E8C1: '<path d="M3.5 19l4.5-13 4.5 13M5.2 14h5.6M15 12.5a2.5 2.5 0 1 1 0 5 2.5 2.5 0 0 1 0-5zM17.5 11v8"/>',
    E8C8: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/>',
    E8D7: '<path d="M12 3l7.5 3v5.5c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5V6z"/>',
    E8D9: `<path d="${star}"/><path d="M4 4l16 16"/>`,
    E8EA: '<rect x="7" y="2.5" width="10" height="19" rx="2"/><path d="M11 18.5h2"/>',
    E8F2: '<path d="M4 5h16v11H9l-5 4z"/>',
    E8FD: '<path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01"/>',
    E90F: '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.8-3.8a6 6 0 0 1-7.9 7.9l-6.9 6.9a2.1 2.1 0 0 1-3-3l6.9-6.9a6 6 0 0 1 7.9-7.9z"/>',
    E91B: '<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="8.5" cy="10" r="1.5"/><path d="M21 16l-5-5-8 8"/>',
    E946: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.5h.01"/>',
    E968: '<rect x="9" y="3" width="6" height="5" rx="1"/><rect x="3" y="16" width="6" height="5" rx="1"/><rect x="15" y="16" width="6" height="5" rx="1"/><path d="M12 8v4M6 16v-4h12v4"/>',
    E9A6: '<path d="M4 9V5h4M20 9V5h-4M4 15v4h4M20 15v4h-4"/>',
    E9F9: '<path d="M5 20V11M11 20V5M17 20v-6M3 20h18"/>',
    EA8F: '<path d="M6 3.5h12V21l-6-4-6 4z"/>',
    F0E2: '<rect x="4" y="4" width="7" height="7" rx="1"/><rect x="13" y="4" width="7" height="7" rx="1"/><rect x="4" y="13" width="7" height="7" rx="1"/><rect x="13" y="13" width="7" height="7" rx="1"/>',
  };
  document.documentElement.classList.add('svg-icons');
  const swap = (el) => {
    const t = el.textContent;
    if (t.length !== 1) return;
    const code = t.codePointAt(0).toString(16).toUpperCase();
    if (P[code]) el.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true">${P[code]}</svg>`;
  };
  const scan = (root) => {
    if (root.nodeType !== 1) return;
    if (root.classList.contains('i')) swap(root);
    root.querySelectorAll('.i').forEach(swap);
  };
  new MutationObserver((ms) => {
    for (const m of ms) {
      if (m.type === 'characterData') { const p = m.target.parentElement; if (p && p.classList.contains('i')) swap(p); continue; }
      m.addedNodes.forEach(scan);
      if (m.target.nodeType === 1 && m.target.classList.contains('i')) swap(m.target);
    }
  }).observe(document.documentElement, { childList: true, subtree: true, characterData: true });
  document.addEventListener('DOMContentLoaded', () => scan(document.body));
})();
