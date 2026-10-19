# noVNC in Itda

`novnc.min.js` is [noVNC](https://github.com/novnc/noVNC) 1.7.0 (MPL-2.0), bundled with
esbuild by `scripts/build-novnc.sh`, with `core/decoders/zrle.js` and `core/decoders/raw.js` replaced by
the versions in `novnc-src/`. The checked `scripts/patch-novnc.py` transformation preserves all ExtendedDesktopSize screen records and emits geometry events. `novnc-src/itda.js` exposes the pinned viewport/absolute-pointer adapter, a frame version and Keyboard for sibling views; the viewer imports the bundle explicitly before it boots.

## Why

noVNC's ZRLE decoder asks pako for one byte at a time for every palette index and run length
(`inflate(1)`), which never reaches pako's fast path, and draws every 64×64 tile with its own
`putImageData`. Itda's servers send ZRLE with palette tiles, so text and UI screens decoded
slowly: a full 1080p text frame took about 73 ms in WKWebView (the Mac app's windows).

The replacement inflates a rectangle's data in one call (it ends with a sync flush), decodes
it flat into one RGBA buffer and draws the rectangle once. On recorded 1080p streams it gives
pixel-identical output; per 10 frames in node: text 851 → 75 ms, UI 553 → 32 ms, RLE text
671 → 165 ms. It also has the palette index bound check of noVNC PR #2099 (`>=`).

## Updating noVNC

Change `VERSION` in `scripts/build-novnc.sh`, compare upstream's `core/decoders/zrle.js`
with the one this replaced (1.7.0) for changes worth carrying over, review the checked patch and adapter against the new RFB/Display internals, and run the script. Then run `scripts/monitors-e2e.mjs` and the CU test fixture before shipping.

## Scroll and burst responsiveness

`interaction.js` shares wheel delta accumulation between the owner and monitor windows.
The adapter sends each event's complete press/release pairs in one WebSocket packet,
without a timer or a queued scroll tail. Excess whole clicks above the per-event cap are
discarded; fractional deltas expire after an idle gesture. Ordinary mouse moves and keys
still use noVNC's original path.

Raw decoding draws up to 128 available scanlines at once instead of one canvas operation
per row, preserving partial-line buffering, 8-bit conversion and opaque alpha. The RFB
receive loop yields between protocol messages after an 8 ms work budget; one large
rectangle can still take longer. Updates, CopyRect and fence ordering are preserved.

Run `node scripts/raw-decoder-test.mjs`, the CU fixture, and
`MAC_FIXTURE=1 node scripts/monitors-e2e.mjs` (Playwright) after changes. The latter checks
wheel packet counts/coordinates, remote Mac shortcut key pairs, view-only suppression,
fragmented Raw pixels and a 12-frame burst under CPU throttling. Synthetic checks do not
measure the frame rate or latency of a user's real Mac server/network.
