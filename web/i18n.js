// Translations for the web UI (home window, file windows, help, web viewer).
// Catalogs: web/locales/{ko,en}.js, flat maps of stable message IDs. Korean is the source
// language; scripts/i18n-check.mjs keeps both catalogs in step and the UI free of literals.
//
// Language, first match wins:
//   ?lang=ko|en in the page URL (tests and recordings)
//   the person's choice: Settings → Language in the app (S.settings.lang), or the web
//     viewer's own picker on a phone (localStorage), passed in with I18N.setPref()
//   window.ITDA_LANG: what the app resolved when it opened this window
//   the system: window.ITDA_OS_LANG (the app's OS language), else the browser's
// Korean when that starts with "ko", otherwise English.
//
//   T('home.connected', { name })            '{name}에 연결했습니다'
//   T('files.items', { count: 3 })           { one: '{count} item', other: '{count} items' }
// Values are trusted markup where the caller puts them in innerHTML; escape what you pass in.
(function () {
  'use strict';
  const LANGS = ['ko', 'en'];
  const CATALOG = { ko: Object.create(null), en: Object.create(null) };
  const norm = (l) => (String(l || '').toLowerCase().startsWith('ko') ? 'ko' : 'en');
  const param = (() => {
    try { const v = new URLSearchParams(location.search).get('lang'); return LANGS.includes(v) ? v : null; } catch (_) { return null; }
  })();
  const system = () => norm(window.ITDA_OS_LANG || (navigator.languages && navigator.languages[0]) || navigator.language);
  let pref = 'auto';
  let lang = param || (LANGS.includes(window.ITDA_LANG) ? window.ITDA_LANG : system());
  let plural = null;
  const listeners = [];

  function T(key, vars) {
    let v = CATALOG[lang][key];
    if (v === undefined) v = CATALOG.ko[key];
    if (v === undefined) { if (window.console) console.warn('i18n: missing', key); return key; }
    if (typeof v === 'object') {
      const n = vars && Number(vars.count);
      if (!plural) plural = new Intl.PluralRules(lang === 'ko' ? 'ko-KR' : 'en-US');
      v = v[plural.select(Number.isFinite(n) ? n : 0)] ?? v.other;
    }
    return vars ? v.replace(/\{(\w+)\}/g, (m, k) => (k in vars && vars[k] != null ? String(vars[k]) : m)) : v;
  }

  // Static markup: data-i18n="key" (text), data-i18n-html="key" (markup),
  // data-i18n-attr="title:key;aria-label:key2;placeholder:key3". Only on markup that scripts
// never rewrite: a language change re-applies these. Pages call I18N.apply() once at start.
  function apply(root) {
    root = root || document;
    if (root === document && document.documentElement) document.documentElement.lang = lang;
    root.querySelectorAll('[data-i18n]').forEach((e) => { e.textContent = T(e.dataset.i18n); });
    root.querySelectorAll('[data-i18n-html]').forEach((e) => { e.innerHTML = T(e.dataset.i18nHtml); });
    root.querySelectorAll('[data-i18n-attr]').forEach((e) => {
      for (const pair of e.dataset.i18nAttr.split(';')) {
        const i = pair.indexOf(':');
        if (i > 0) e.setAttribute(pair.slice(0, i).trim(), T(pair.slice(i + 1).trim()));
      }
    });
  }

  function setLang(next) {
    if (next === lang) return false;
    lang = next;
    plural = null;
    apply(document);
    for (const f of listeners.slice()) { try { f(lang); } catch (e) { if (window.console) console.error(e); } }
    return true;
  }

  window.T = T;
  window.I18N = {
    LANGS,
    get lang() { return lang; },
    get pref() { return pref; },
    /** BCP 47 tag for Intl / toLocaleString. */
    get locale() { return lang === 'ko' ? 'ko-KR' : 'en-US'; },
    /** The language "auto" picks here. */
    get system() { return system(); },
    /** The URL forces a language: the choice can't change it. */
    get forced() { return !!param; },
    add(l, dict) { Object.assign(CATALOG[l], dict); },
    has(key) { return CATALOG.ko[key] !== undefined; },
    /** "auto" | "ko" | "en"; true when the shown language changed. */
    setPref(p) { pref = LANGS.includes(p) ? p : 'auto'; return setLang(param || (pref === 'auto' ? system() : pref)); },
    onChange(f) { listeners.push(f); },
    apply,
    T,
  };
})();
