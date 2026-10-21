# High-performance mode measurements

Use the same display, workload, server version, receiver and network for both
builds. Run one probe at a time; multiple viewers share capture and add encoders.
Record the exact commit, app signature, resolution and clock/network conditions.
Do not change a user's permissions or inject input to make a probe pass.

## Receive-only protocol probe

The server must already be running. For a diagnostic server, the signed Mac app's
`ITDA_HP_TEST=1` mode suppresses permission/consent prompts, listens on loopback,
and rejects remote input. Use a separate port and stop only the process you own.
The probe does not install, launch or stop the app.

```sh
node scripts/hp-performance-probe.mjs jiun-mbp --port 15910 --seconds 10 --samples
```

The SSH alias must already be trusted. The probe uses strict host checking, a
separate SSH connection and `IPQoS=lowdelay`, like the app's screen tunnel. It
reads the sharing password over SSH into memory, never into arguments or output.
It sends no input events or clipboard writes. Desktop names and pixel payloads
are not recorded. A disconnected/authentication/unsupported-CU run fails with a
nonzero exit code and `fps:null`; do not treat it as a successful zero-fps result.

Copy the script to the serving Mac and bypass SSH for a local baseline:

```sh
node /tmp/hp-performance-probe.mjs --local --port 15910 --seconds 10 --samples
```

`--mode classic` measures request/response behavior separately. The default
continuous mode must negotiate ContinuousUpdates. The JSON distinguishes
complete framebuffer updates from H.264/ZRLE rectangles: a single lossless
repaint containing many tiles is one update, not many displayed frames.
Fence RTT includes server pump scheduling and is not a raw network RTT.
Handshake time includes connecting the SSH tunnel; `first_after_init_ms`
separates that from the first framebuffer response. `--samples` includes each
update's arrival time and payload byte count, without image data.

## Repeatable motion

Compile and run the temporary workload on the serving Mac:

```sh
swiftc -O scripts/fixtures/hp-motion.swift -o /tmp/itda-hp-motion
/tmp/itda-hp-motion 30
```

It draws a temporary nonactivating, click-through panel and closes itself after
the specified number of seconds. It does not modify documents or request
permissions. It prints the workload geometry and a timestamp barcode's sample
coordinates. The barcode is 24-bit epoch milliseconds modulo 2^24, little-endian,
following white and black sentinel cells, with 20-point cell spacing.

An instrumented viewer can read the barcode after drawing to estimate content
age. First measure sender/receiver clock offset and its uncertainty using
multiple timestamp exchanges on one persistent connection; exclude SSH startup
from the RTT. Reject invalid sentinel/timestamp samples. Canvas readback adds
measurement overhead, so use it identically in both builds. This estimates
source-content age, not physical glass-to-glass latency or input responsiveness.

Compare FPS **and** content age/arrival gaps. A larger in-flight window can raise
FPS while retaining older pictures; a rate increase alone is not acceptance.
Also test two viewers, slow acknowledgements, motion after a pause and the final
lossless repaint. Verify H.264 decoding and reference continuity in the actual
WKWebView; a protocol-only probe does not validate decoded pixels.

## Probe integration checks

```sh
node scripts/hp-performance-probe-test.mjs
```

The local synthetic server checks authentication, fragmented mixed-encoding
updates, update-versus-rectangle counts, continuous/classic negotiation and
failure reporting. It uses a temporary credential and no real devices.
