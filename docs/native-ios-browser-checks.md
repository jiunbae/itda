# Native iOS browser QA: build-only scaffold

Status: **runtime blocked; no iOS test has run**. This is a reviewable XCTest scaffold,
not evidence that MobileSafari, a physical phone, or the mobile persona passed.

`scripts/mobile-native/build-only.sh` compiles a standalone, unsigned Simulator UI-test
bundle with the installed Xcode and XcodeGen. It uses a fresh owned temporary output
directory, `generic/platform=iOS Simulator`, and `build-for-testing`. It does not
create, boot, enumerate, install on, erase, or operate a device. There is no test/serve
wrapper, CI workflow, or runtime-enable environment flag. `setUpWithError` unconditionally
skips execution before any Safari or network action. The fixture module has no server
CLI and does not listen when imported.

```sh
bash scripts/mobile-native/build-only.sh
```

The command prints its `/tmp/itda-mobile-native-build.*` artifact directory. The
generated Xcode project, DerivedData and build result bundle stay there. It rejects
arguments and records SHA-256 hashes of the byte-compared source copies in
`copied-source.sha256`. Do not change it to `test`/`test-without-building` or choose an existing
device. A successful compile proves only that the scaffold builds.

## Runtime prerequisites that are not satisfied

Before a future independently reviewed change may unblock runtime, establish all of:

1. A **specific new QA UUID** and an owned device set/output directory, with no personal
   device or existing Simulator reuse. No `booted`, `all`, automatic selection,
   shutdown-other-devices, pairing, Apple Account sign-in, or real credentials.
2. Concrete, current Xcode/DeviceHub evidence that booting that device and running its
   XCTest runner cannot automatically exchange pasteboard contents with the Mac. A
   boolean asserting “isolation verified” is not evidence. Not tapping a clipboard
   button, using a new UUID, or booting without the GUI does **not** prove this.
3. An owned loopback fictional fixture, its ephemeral control token, an exact source
   revision and artifact manifest. Neither personal Itda settings nor the deployed
   gateway may be the test server. The proposed Swift configuration accepts only
   `http://127.0.0.1:<port>/`, no URL credentials/query, and a private QA control token.
   The Node factory can bind only explicit `127.0.0.1`. Neither is started here.
4. Software keyboard visible in the guest, native accessibility selectors inspected,
   and screenshots/coordinate geometry verified before canvas taps. If absent or
   ambiguous, fail the test; do not substitute `typeText`, hardware events, guessed
   coordinates, or host-global keyboard preferences.
5. Host Developer Tools/TCC, signing, ATS or Simulator startup prerequisites already
   usable within the authorized scope. If a prompt or restriction blocks a step,
   record the exact blocker; do not grant it automatically, reuse personal provisioning,
   weaken product ATS, install a CA, bypass certificate trust, or modify host Safari.

Local investigation found Xcode 27.1, iOS 27 Simulator profiles, XcodeGen and XCTest.
Appium, WebDriverAgent, idb and Maestro were not found in PATH or the standard locations
examined. Their absence is not proof that no copy exists anywhere on the machine.

Safari WebDriver is a separate route: the installed `safaridriver(1)` documents that
`--enable` changes Safari Remote Automation settings, and that a matching unbooted
Simulator may be booted during session creation. Neither the setting change nor a
WebDriver session belongs in build-only validation. XCTest uses public
`XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")` and `open(URL)` instead;
the MobileSafari runner still needs a future runtime verification.

## Proposed cases and required evidence

| Case | Native action | Receiving proof | Does not establish |
| --- | --- | --- | --- |
| PIN error and recovery | Tap PIN, assert visible software keyboard, tap unique observed digit keys, tap submit, correct the PIN | Error/list AX state, screenshots, fixture HTTP status sequence | Rust gateway authentication or actual device credentials |
| PIN lockout | Five wrong PINs arm the fixture lock; next request gets 429; wait for the ordinary UI countdown, then correct PIN | Exact `401×5 → 429 → 200` receiver sequence, disabled submit/countdown and no requests during countdown | Real gateway rate limiting or network-source isolation |
| Session expiry | Expire the fictional cookie through token-protected test control, tap refresh, sign in again | Explanation visible, repeat PIN entry and list proof | Production cookie lifetime or browser behavior across OS process death |
| Touch and keyboard | Identify the actual magenta fixture framebuffer in a native screenshot, tap a chosen non-central point; open keyboard and tap observed ASCII `a` key | Original peer ID retained; ordered adjacent down→up at identical coordinates within 4 framebuffer pixels of the chosen point, plus ordered `a` down→up on that peer; JSON attachments paired with native captures | Physical finger UX, Korean IME, real remote OS input or real VNC account login |
| Context observation | Open separate diagnostic page | Native screenshot/AX text containing origin, `isSecureContext` and Clipboard API availability | Clipboard permission/operation, real LAN HTTP or trusted HTTPS |

Tests attach screenshots, native accessibility trees and bounded receiver JSON to
`.xcresult`. The fixture has no performance stream, CPU throttling, burst benchmark,
real remote sockets, external host selection or user files. It emits one fictional
640×480 framebuffer. The RFB/WebSocket parser is deliberately limited and fails closed
on unsupported messages; it is a fixture, not a production protocol implementation.
The URL/selector/screenshot assumptions are unverified until authorized runtime.

`fixture.mjs` adapts the real-frontend/static asset and fictional PIN/session API from
`scripts/viewer-phone-e2e.mjs`; it is a model of the gateway, not `src/gateway.rs`.
`rfb-fixture.mjs` adapts only the functional WebSocket/RFB handshake and pointer/key
receiver from `scripts/monitors-e2e.mjs`. It excludes the benchmark/burst/fragment
campaign and does not exercise multi-monitor hardware. The product's `web/` assets
are served unchanged from an explicit allowlist.

The fixture PIN sequence was source-reviewed against `src/gateway.rs`: failures 1–5
return 401; failure 5 arms `MAX_FAILS=5`/`LOCKOUT=60s`; an attempt while locked returns
429 before checking its PIN; a correct PIN after expiry returns 200. Each native
submit asserts exactly one appended status, the recovery case asserts `[401,200]`
then `[401,200,200]` after session expiry, and the lockout case asserts the full
sequence above. These are assertions in an unrun scaffold, not a gateway test pass.

The static fixture allowlist includes the service-worker shell's `icon-512.png`,
mapped to existing `assets/itda-512.png`. Both that asset and the production gateway's
`GET /icon-512.png` handler exist; only the scaffold's earlier mapping was missing.

## Clipboard and pasteboard audit

Clipboard operations and QR camera tests are intentionally unimplemented. Do not
execute `pbcopy`, `pbpaste`, `UIPasteboard`, Web Clipboard methods, native Copy/Paste
menus or the viewer's clipboard tool with this scaffold.

Source inspection at the scaffold base found:

- The viewer's only `navigator.clipboard.writeText` call is inside the explicit
  “받은 내용 복사” sheet-click branch. No `navigator.clipboard.readText` call was found.
- The normal page load/connection path registers a remote clipboard event handler
  but does not invoke a browser Clipboard API. The native app's `itdaPaste` callback
  is not called by this fixture. The hypothetical host has `opts.clipboard=false`,
  no `window.ipc` injection, no real credentials, and no pending paste text.
- The bundled noVNC search found no `navigator.clipboard`, `clipboard.readText` or
  `clipboard.writeText` calls. This is source evidence, not a native runtime audit.
- The fixture never emits ServerCutText or Extended Clipboard capabilities. It rejects
  ClientCutText without parsing/retaining the payload. Receiver JSON records a counter
  and failure only. This prevents fixture clipboard traffic; it does not isolate the
  OS pasteboard bridge or make Simulator boot safe.
- The context page only reads property availability; it does not call Clipboard methods.

Apple documents synchronization between Simulator and Mac pasteboards. Appium's
current capabilities describe a pasteboard synchronization launch option, but this is
not proof of isolation for an arbitrary existing DeviceHub process. Its current
Xcode-27 simulator class uses DeviceHub, and its headless path says UI-related options
are ignored before calling `boot()`. Therefore a headless boot is **not certified
isolated** by these sources. No reliable per-new-device/per-process isolation proof
was obtained. Runtime remains blocked even if clipboard-specific test bodies are absent.

Do not change global Simulator/DeviceHub preferences or copy a legacy launch argument
into a command and claim that the prerequisite is solved.

## Limits retained after any future Simulator success

- Real QR camera scan and opening the scanned URL require a physical phone.
- Loopback is potentially trustworthy and is not representative plain LAN HTTP.
  Trusted HTTPS and general LAN origins need separate authorized environments without
  certificate bypass. Clipboard capability availability alone is not a read/write pass.
- A visible keyboard and native digit/ASCII taps do not prove Korean composition,
  dictation, hardware keyboards or a physical keyboard's behavior.
- Real iOS/Android clipboard/privacy prompts, physical touch feel, backgrounding,
  connectivity changes and phone performance remain separate gaps.
- Simulator, mock gateway and synthetic RFB evidence cannot certify the whole mobile
  persona at 9/10 or replace real-device verification.

## Primary references

- [Apple: application proxy by bundle identifier](https://developer.apple.com/documentation/xcuiautomation/xcuiapplication/init%28bundleidentifier%3A%29?language=objc)
- [Apple: launch an app by URL](https://developer.apple.com/documentation/xcuiautomation/xcuiapplication/open%28_%3A%29?language=objc)
- [Apple Simulator help: pasteboard synchronization](https://developer.apple.com/library/archive/documentation/IDEs/Conceptual/simulator_help_topics/Chapter/Chapter.html)
- [Appium: Simulator capabilities](https://github.com/appium/appium-xcuitest-driver/blob/master/docs/reference/capabilities.md)
- [Appium: DeviceHub support change](https://github.com/appium/appium-xcuitest-driver/commit/3cda6b203eaa9e497bc998748506ba75bc9e51f3)
- [Appium: Xcode-27 UI client](https://github.com/appium/appium-ios-simulator/blob/master/lib/simulator-xcode-27.ts), [headless lifecycle](https://github.com/appium/appium-ios-simulator/blob/master/lib/extensions/ui-client.ts)
- [WebKit: Clipboard secure context and gesture requirements](https://webkit.org/blog/10855/async-clipboard-api/)

The archived Apple Simulator help explains synchronization, not all current Xcode-27
behavior. Appium `master` links can change; record the exact revision if relying on
them for a future unblock decision. Local Apple API headers and `safaridriver(1)`
were also read; no driver, Simulator, server or native browser session was started.
