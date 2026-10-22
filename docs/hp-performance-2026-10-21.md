# macOS high-performance sharing qualification, 2026-10-21

This is a dated pipeline measurement report, not a current release performance guarantee. For the later points/Retina comparison and its quality/frame-rate trade-offs, see the [2026-10-22 quality report](https://github.com/jiunbae/itda/blob/main/scripts/hp-quality.md).

The bounded change retains ScreenCaptureKit buffers for VideoToolbox and releases
the shared capture mutex before comparison, encoding and lossless repaint. CPU
pixels remain available for exact change detection and still-frame repaint. This
is synchronous encoding, not a full asynchronous or completely zero-copy pipeline.

## Source and checks

- Phase 1: `0e2fcc204a4bc998ce254ac6f5df887de4a7d964` (pacing, per-viewer freshness,
  acknowledgement age gate, macOS-only timing).
- Phase 2 production: `bdc5c3a83f12fe3c707b69a1c76b78a405e770d1`.
- Signed Phase 2 CI candidate: run `39538660200`, PR head `4d49529`, synthetic
  merge `51567bfdcf2d74c26abedf44ed8fc3a6dd5005e4` against main. Universal binary
  SHA-256 `bc3c20e2ff9a799c28f4dc9698ae9ce624ee18db0352db30d93fb4f86409c238`;
  Developer ID team `728FW73BS8`, hardened runtime, strict signature verified on
  both build and test Macs. This candidate was not notarized.
- Independent reviews approved focused code correctness after the initial global
  freshness bug was replaced with per-viewer generations.
- 89 Rust tests passed, 9 ignored; two new tests cover retained snapshot lifetime,
  CPU immutability, independent viewer freshness and native size fallback.
- Windows build/tests, universal macOS build/tests, Windows file UI CI passed.
- Four synthetic receive-only measurement-probe modes and four Raw decoder cases
  passed. `scripts/h264-e2e.mjs` exercises actual browser decoding and still repaint;
  its expected-image validation now renders once per report instead of 851 times.
  The corrected two-session run passed in 25.9 seconds: both ZRLE and H.264 had
  zero worst-channel error across 851 samples after still repaint; H.264 performed
  one lossless refine. Its debug-build frame rate is not a production benchmark.

## Measurements

Host: jiun-mbp, Apple M1 Pro, macOS 27.0.1. Capture 1512×982 points, not the full
3024×1964 Retina pixel count. Only a temporary nonactivating synthetic panel and
receive-only test servers on loopback port 15910 were used. No keyboard/pointer
events, permission grants/resets or production app replacement were performed.

Single-client runs used the same scrolling barcode workload, 10 seconds remote
loopback followed by 10 seconds through SSH. Each had one initial request, one IDR,
and nonempty H.264 updates; no rectangle-count inflation.

| Candidate | Loopback updates/s | SSH updates/s | SSH gap p95 |
| --- | ---: | ---: | ---: |
| Installed 0.9.15 repeat | 51.1 | 48.3 | 41.4 ms |
| Phase 1 | 54.3 | 50.3 | 37.2 ms |
| Phase 2 | 51.4 | 51.0 | 46.0 ms |

These do not show a meaningful single-client Phase 2 improvement. Historical
34.9 updates/s measurements used different conditions and must not be presented
as the baseline for a 34.9→51 improvement caused by this code.

Four simultaneous remote-loopback protocol receivers, 10 seconds each:

| Candidate | Per-client updates/s | Per-client gap p95 |
| --- | --- | --- |
| Phase 1 | 24.2, 19.7, 19.1, 20.0 | 57.1, 123.2, 154.0, 162.6 ms |
| Phase 2 | 34.3, 34.1, 34.5, 34.2 | 47.6, 48.6, 50.9, 46.7 ms |

Mean received update rate increased 65% in this pair. A separate 30-second Phase 2
four-receiver run sustained 27.6–27.7 updates/s per client, p95 gaps 48.7–49.7 ms,
without a permanent capture stall. This longer run has no matched Phase 1 control;
do not compare its rate directly with the 10-second pair. These are protocol
receivers, not four simultaneously decoded/composited native viewers.

Actual bundled noVNC in a temporary native WK receiver, using low-cost asynchronous
barcode readback every fifth frame and the same public receiver keepalive:

| Candidate | Decoded/drawn frames | Content age p50 / p95 | Clock uncertainty |
| --- | ---: | ---: | ---: |
| 0.9.15 | 634 | 67 / 126 ms | ±5.35 ms |
| Phase 1 | 626 | 69.5 / 101.5 ms | ±4.86 ms |
| Phase 2 | 648 | 70.5 / 123.5 ms | ±5.23 ms |

No decoder errors or malformed barcode samples were observed in these corrected
runs. Phase 2 decode p95 was 3 ms. Content age ends at the noVNC backbuffer draw
call; it is not physical presentation or remote input latency. A single pair does
not establish a stable latency improvement. Earlier synchronous canvas readbacks
perturbed the receiver and are excluded, as are earlier background-suspended runs.

Six unobstructed colour samples from native-buffer H.264 matched the corresponding
CPU Raw source samples in a separate static-panel check. White/black regions were
occluded by macOS capture permission reminder dialogs and were not qualified by
that check. The reminders also prevent claiming a clean full-screen live still
repaint test. They appeared with the existing ScreenCaptureKit path despite a true
preflight result; no automatic permission action was taken. Further clean-state
live colour, still and input-latency testing needs the device owner's permission
workflow. The code does not change that workflow.

## Interpretation and remaining work

The supported result is reduced contention and more even delivery with concurrent
receivers. Single-viewer scroll latency is not proven fixed. Native SCK buffer
retention and copy-on-write CPU snapshots must remain bounded; higher viewer counts,
slow encoders, other hardware and sustained thermal load need further qualification.
The native buffer and CPU pixels describe the same immutable generation, and
dimension mismatch retains the CPU encoding fallback. Windows capture timing and
borrowing remain unchanged; the acknowledgement-window policy is shared.

Use `scripts/hp-performance.md` to reproduce protocol measurements. Preserve server
binary identity separately from receiver HTML/bundle/native-host hashes. Do not
drop encoded H.264 reference frames when experimenting with an asynchronous path;
any later pipeline change requires independent ordering, reset, lifetime, colour,
still-repaint and decoded content-age validation.
