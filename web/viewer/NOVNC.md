# noVNC in Itda

`novnc.min.js` is [noVNC](https://github.com/novnc/noVNC) 1.7.0 (MPL-2.0), bundled with
esbuild by `scripts/build-novnc.sh`, with one file replaced: `core/decoders/zrle.js` by
`novnc-src/zrle.js`.

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
with the one this replaced (1.7.0) for changes worth carrying over, and run the script.
