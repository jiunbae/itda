# Mac HP resolution and quality measurements (2026-10-22)

High-performance sharing keeps **points** as its speed default. The opt-in
**native** mode preserves Retina detail, with an even-sized framebuffer bounded
at 4096 pixels on its longest edge. It does not replace the protocol or raise the
bitrate globally. The marker `<data-dir>/hp-quality` accepts `native` or `points`;
missing/unrecognised values mean points. Diagnostic servers may use
`itda --hp-server PORT --native-pixels`.

The first viewer creates the shared capture and fixes its resolution until the
last viewer leaves. `hp-status` reports `quality`, `quality_active` (whether a
capture exists), `capture_width` and `capture_height`. A saved preference is not
proof an already-running shared capture has switched. Older hosts without these
fields do not establish support. Mouse coordinates map framebuffer pixels back
to desktop points; monitor metadata uses framebuffer pixels.

Native geometry uses the display mode's backing-pixel density applied to desktop
bounds, including portrait orientation. `CGDisplayPixelsWide/High` returned
1920×1080 on the 3840×2160 HiDPI test display, so those functions were not adequate
for selecting native resolution. Live trials must check negotiated framebuffer
dimensions and valid timestamp samples, rather than infer a mode from its flag.

## Offscreen text benchmark

The same CoreText 12-point glyph fixture is rendered at point and backing-pixel
resolution, scrolled through eight phases, then encoded through VideoToolbox
with production real-time/low-latency/no-reordering/High-profile settings.
180 frames are scheduled at 60 Hz. Both encoder and supplied-frame times advance
at that rate; slower runs fall behind instead of hiding dropped work. Bitrate
is `width * height * rate`, clamped to 4–40 Mbps as in production.

The last accepted H.264 frame is decoded with ffmpeg 9.0.2. RGB PSNR/SSIM compare
it with its exact source. To compare resolution choices at the **same size**, the
point result is bilinearly enlarged to native pixels; the native text ROI is
1800×800. Most blank margins are excluded. This is a synthetic text comparison,
not a universal perceptual-quality score or a physical screen capture.

| Device / capture | Encode p50 / p95, ms (two repeats) | Encoded cadence | Native text ROI SSIM / PSNR |
| --- | --- | --- | --- |
| M1 Pro / 1512×982 points | 7.79–8.32 / 9.24–12.81 | ~60 fps | 0.8938 / 20.84 dB |
| M1 Pro / 3024×1964 native | 19.30–19.47 / 21.46–24.25 | 48.8–49.8 fps | 0.9920 / 35.92 dB |
| M1 Max / 1920×1080 points | 11.06–11.68 / 13.28–14.55 | ~60 fps | 0.8940 / 20.88 dB |
| M1 Max / 3840×2160 native | 28.92–29.38 / 32.25–32.51 | 34.6–34.8 fps | 0.9920 / 35.91 dB |

At point resolution, doubling/tripling the target bitrate changes native-ROI
SSIM by less than 0.0005. The actual text stream stays about 4.7–5.2 Mbps.
Higher bitrate alone does not recover pixels discarded during downscaling, and
is not supported as a blanket fix by this fixture. Native mode's much better
text score comes with a measurable encoding cost, so it remains an explicit
choice. `UsingHardwareAcceleratedVideoEncoder` queries returned OSStatus -12900;
this is **unknown**, not evidence of a software encoder or proof of hardware use.

Full numerical results: [hp-quality-results-2026-10-22.json](hp-quality-results-2026-10-22.json).

```sh
swiftc -O scripts/hp-quality-bench.swift -o /tmp/itda-hp-quality-bench
# Use <device>-<scale>-<rate> subdirectories; include a native -2-6 baseline.
/tmp/itda-hp-quality-bench /tmp/quality/m1pro-1-6 1512 982 1 6 180
/tmp/itda-hp-quality-bench /tmp/quality/m1pro-2-6 1512 982 2 6 180
python3 scripts/hp-quality-score.py /tmp/quality
```

## Still-frame refinement

An opt-in test feeds a fixed checkerboard to the **production pump and real
VideoToolbox encoder**, receives the first H.264 frame over local TCP, requests
the next incremental update, independently inflates/reads the ZRLE solid tiles,
and verifies every pixel of the entire exact repaint. It invokes no screen
capture, input events, or permission prompts.

Three optimized-build trials per size measured receipt after the last video:

| Device / capture | Exact repaint receipt, ms |
| --- | --- |
| M1 Pro / 1512×982 | 513–525 |
| M1 Pro / 3024×1964 | 516–550 |
| M1 Max / 1920×1080 | 521–531 |
| M1 Max / 3840×2160 | 534–553 |

This confirms the 500 ms producer-side refinement path, including the real
encoder, pump scheduling and local TCP receiver. The solid-tile fixture is highly
compressible; these timings do not predict compression or network cost for an
arbitrary desktop. An ordinary desktop trial kept producing H.264 even after
its test panel froze: changes elsewhere on the screen can postpone a whole-screen
still repaint. That trial does **not** establish that the screen was fully still.

```sh
ITDA_REFINE_DIMS=3840x2160 cargo test --release fixed_frame_refines -- --ignored --nocapture
```

## Live viewer and speed interpretation

The live viewer is WKWebView on an M5 receiver, the current bundled noVNC decoder,
and a temporary, nonactivating click-through timestamp workload on the sending
Mac. It sends no remote input. Sender/receiver clock offset is measured before
and after each group. Content age is sampled at canvas presentation, not at
physical scanout. The sender runs an isolated signed, view-only, loopback server;
SSH encrypts transport. The production installation and permission settings are
unchanged.

The unlocked M1 Max baseline (0.9.21) produced 52.3–53.2 decoded presentations/s,
content age p50 48–49 ms and p95 106–130 ms in two 13-second runs. A locked M1 Pro
trial produced only the initial screen and was rejected (no warm timestamp
samples); its results are not a valid speed comparison. Initial supposed-native
runs that negotiated 1920×1080 were also rejected.

A paired **points → native → native → points** run on the same signed candidate
(commit `3bfac70`, server SHA-256 recorded in the JSON) checked negotiated
1920×1080 / 3840×2160 dimensions and rejected any invalid timestamp samples:

| Mode | Presentations/s | Content age p50 / p95, ms | Received Mbps | Valid warm samples |
| --- | --- | --- | --- | --- |
| Points, first | 56.1 | 45 / 113 | 10.9 | 121 |
| Native, first | 29.8 | 78 / 152 | 6.8 | 66 |
| Native, repeat | 29.7 | 80 / 153 | 6.9 | 66 |
| Points, repeat | 54.0 | 48 / 117 | 10.4 | 117 |

The clock midpoint estimate was -1 ms with ±3.04 ms minimum-RTT uncertainty;
the before/after clock exchanges are retained with the raw run. All four runs
had zero invalid barcode samples, no decoder errors and no disconnect. The
fixture is a 1280×800 logical-point panel; native mode captures the whole display
at 4K, so both source pixels and encoder workload change. The two results are a
user-visible resolution trade-off, not an isolated codec or transport speedup.
Traffic is content- and cadence-dependent: native mode sent fewer bytes here
while presenting about half as many pictures; native resolution does not imply
a fixed bandwidth multiplier. The final source also adds portrait-density
handling and the pointer cache, which do not change this view-only landscape
measurement path.

The current measurements justify a resolution/quality choice. They do not show
that the transport must be replaced or that bitrate increases improve speed.
Frame delivery and source-content age both matter; a larger queue must not be
accepted merely because it raises FPS. The pointer path now caches the fixed
frame dimensions once per connection, avoiding a capture-mutex wait on every
mouse event; end-to-end input latency has not been measured by injecting events.

The capture size is fixed during a connection. If display resolution/scaling is
changed mid-session, reconnect before relying on the new geometry. A general
live display-mode reconfiguration path is not added here.

A separate traced point-mode run (~56.2 presentations/s) found capture snapshot
copy p50 1.7–1.8 ms / p95 5.2–5.3 ms; encoder p50 9.6–10.0 ms / p95 12.0–12.1 ms;
whole update p50 11.7–12.1 ms / p95 15.7–15.8 ms. Capture-to-pump wait p95 was
1.5–1.6 ms, so the old polling interval is not the dominant cost in this run.
ACK p95 was ~93 ms, including viewer and scheduling delay; it is not a raw
network RTT. Stages overlap and must not be added as quantiles. SCK timestamp
deltas included negative values and are not used to infer capture latency.
The current 60 Hz capture limit and these measurements leave limited FPS
headroom for this workload; a codec/transport rewrite is not justified by these
runs alone. Further encode-time reductions need their own quality/age A/B.
