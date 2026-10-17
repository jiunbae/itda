// Itda web viewer service worker: makes the page installable as an app and opens the
// shell instantly. The app shell is network-first (the PC's Itda may have been updated)
// with the cached copy as a fallback; the API, thumbnails and WebSockets are never cached.
'use strict';

const CACHE = 'itda-shell-v1';
const SHELL = ['./', 'viewer.css', 'viewer.js', 'novnc.min.js', 'icon.svg', 'icon-192.png', 'icon-512.png', 'manifest.webmanifest'];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))).then(() => self.clients.claim()));
});

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET' || url.origin !== location.origin || url.pathname.startsWith('/api/') || url.pathname.startsWith('/thumb/') || url.pathname.startsWith('/ws/')) return;
  e.respondWith(
    fetch(e.request)
      .then((r) => {
        if (r.ok) { const copy = r.clone(); caches.open(CACHE).then((c) => c.put(e.request, copy)); }
        return r;
      })
      .catch(() => caches.match(e.request, { ignoreSearch: true }).then((r) => r || Response.error())),
  );
});
