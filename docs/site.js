// Itda website: KO/EN switch, nav shadow, reveal-on-scroll, guide TOC highlight. No dependencies.
(function () {
  'use strict';
  var root = document.documentElement;
  root.classList.add('js');

  // ---- language: ?lang= > saved choice > browser language (Korean first)
  function saved() { try { return localStorage.getItem('itda.lang'); } catch (e) { return null; } }
  function save(v) { try { localStorage.setItem('itda.lang', v); } catch (e) {} }
  var q = (location.search.match(/[?&]lang=(ko|en)/) || [])[1];
  var lang = q || saved() || (/^ko\b/i.test(navigator.language || 'ko') ? 'ko' : 'en');
  function apply(l) {
    lang = l;
    root.dataset.lang = l;
    root.lang = l;
    document.querySelectorAll('[data-' + l + '-title]').forEach(function (el) { el.title = el.getAttribute('data-' + l + '-title'); });
    document.querySelectorAll('[data-' + l + '-alt]').forEach(function (el) { el.alt = el.getAttribute('data-' + l + '-alt'); });
    document.querySelectorAll('[data-' + l + '-label]').forEach(function (el) { el.setAttribute('aria-label', el.getAttribute('data-' + l + '-label')); });
    var t = document.querySelector('meta[name="title-' + l + '"]');
    if (t) document.title = t.content;
    document.querySelectorAll('.lang button').forEach(function (b) { b.setAttribute('aria-pressed', String(b.dataset.set === l)); });
  }
  apply(lang);
  document.addEventListener('click', function (e) {
    var b = e.target.closest && e.target.closest('.lang button');
    if (!b) return;
    save(b.dataset.set);
    apply(b.dataset.set);
  });

  // ---- nav border once scrolled
  var nav = document.querySelector('.nav');
  function onScroll() { if (nav) nav.classList.toggle('scrolled', window.scrollY > 8); }
  window.addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  // ---- reveal on scroll
  var items = document.querySelectorAll('.reveal');
  if ('IntersectionObserver' in window) {
    var io = new IntersectionObserver(function (es) {
      es.forEach(function (en) { if (en.isIntersecting) { en.target.classList.add('in'); io.unobserve(en.target); } });
    }, { rootMargin: '0px 0px -8% 0px', threshold: 0.08 });
    items.forEach(function (el) { io.observe(el); });
  } else {
    items.forEach(function (el) { el.classList.add('in'); });
  }

  // ---- guide: highlight the step in view
  var links = document.querySelectorAll('.toc a[href^="#"]');
  if (links.length && 'IntersectionObserver' in window) {
    var map = {};
    links.forEach(function (a) { map[a.getAttribute('href').slice(1)] = a; });
    var spy = new IntersectionObserver(function (es) {
      es.forEach(function (en) {
        if (!en.isIntersecting) return;
        links.forEach(function (a) { a.classList.remove('on'); });
        var a = map[en.target.id];
        if (a) a.classList.add('on');
      });
    }, { rootMargin: '-30% 0px -60% 0px' });
    Object.keys(map).forEach(function (id) { var s = document.getElementById(id); if (s) spy.observe(s); });
  }
})();
