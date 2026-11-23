# Contributing

Itda is a Rust app for Windows and macOS, with a shared HTML/CSS/JavaScript interface. Remote screens, file browsing and the mobile web viewer use platform-specific native bridges where needed. UI text is Korean; public guides are available in Korean and English.

## Build

Use stable Rust and Git. On Windows, install the MSVC build tools and WebView2 Runtime (included in Windows 11). On macOS, install Xcode Command Line Tools; a universal bundle also needs both Rust targets:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

On either platform:

```sh
cargo build --locked --release
cargo test --locked --release
```

The executable is `target/release/itda.exe` on Windows or `target/release/itda` on Mac. Package with `scripts/windows-release.ps1` or `scripts/macos-bundle.sh universal`; these scripts also remove build-machine paths. The Mac script writes `dist/Itda.app` and `dist/Itda-macos-universal.zip`. Signing and notarization require the release credentials; a local development bundle is not an official signed release.

The interface is embedded from `web/`. Set `ITDA_WEB_DIR` to an absolute `web/` directory for local iteration without embedding each edit. Restart when changing native Rust code. Static public documentation is in `docs/`, published by GitHub Pages.

Interface text is translated (Korean and English). Web UI strings live in `web/locales/ko.js` and `en.js`, keyed by stable message IDs and read with `T('area.thing', { name })` (see `web/i18n.js`); Korean is the source language. Text Rust shows itself (menus, dialogs, the Windows viewer, errors) uses `tr!("area.thing", name = value)` with `src/locales/ko.rs` and `en.rs`; `cargo test` checks those catalogs and runs in Korean. Add every new string to both catalogs with the same `{placeholders}`; `node scripts/i18n-check.mjs` fails on a missing key, a placeholder mismatch or Korean left in UI code (web or Rust). To see the UI in one language, add `?lang=en` (or `ko`) to the page URL, give Playwright `locale: 'en-US'`, or run the app with `ITDA_LANG=en`.

## Check a change

Run the checks that exercise the affected behavior. CI builds/tests Windows and universal macOS, and runs browser regressions on Windows. Browser fixtures use Playwright:

```sh
npm install --no-save --package-lock=false playwright@1.65.0
npx playwright install chromium
node scripts/secure-context-check.mjs
node scripts/files-windows-e2e.mjs
```

Other focused suites include `files-tabs-e2e.mjs`, `files-network-local-e2e.mjs`, `editor-e2e.mjs`, `permissions-e2e.mjs`, `remote-setup-e2e.mjs`, `update-e2e.mjs`, and `monitors-e2e.mjs`. See `.github/workflows/ci.yml` for current CI coverage. Browser IPC fixtures verify the frontend, not real SSH, native OS drag acceptance, macOS permission grants or physical monitor behavior. Record those runtime checks separately when relevant.

CI also runs `scripts/site-check.mjs` against the local checkout. To run the same check locally, serve the repository root with `python3 -m http.server 4186 --bind 127.0.0.1` and run `node scripts/site-check.mjs` from another terminal.

### Dependency audit

```sh
cargo install cargo-audit --locked --version 0.22.2
cargo audit --file Cargo.lock --ignore RUSTSEC-2023-0071 --deny yanked
```

CI fails on vulnerabilities and yanked crates, with one exception for `RUSTSEC-2023-0071`. Picky, SSPI and the vendored `winscard` each require `rsa =0.10.0-rc.18`. Remove the exception when the IronRDP dependency stack can resolve `rsa` 0.10.0 or later. Run `cargo audit --file Cargo.lock` to see the outstanding advisory alongside the other findings. [RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)

The lockfile audit reports packages across all targets. Unmaintained and unsound advisories remain visible as warnings while the upstream dependency chains are in use. These include `glib` and `proc-macro-error` in the GTK stack for targets outside Windows and macOS, `heapless` and `atomic-polyfill` via `iso7816` in `winscard`, and `pin-utils` via `async-dnssd` in SSPI.

### Before a release: memory and runtime

CI can't watch a real screen, so these runs on real machines gate every release:

- **Mac sharing memory:** install the CI build on a Mac that shares at high performance (jiun-mini), then run `scripts/hp-soak-gate.sh <ssh-host> 120 250 40`. A view-only viewer watches for two minutes with scaled H.264 (as Itda's browser and Windows viewers ask: a 4K screen goes out at 2/3 through the encoder's scaler), then for 30 s each with H.264 at the screen's size (viewers that can't scale) and with ZRLE (viewers without H.264). Footprints are read while the viewer still watches (halfway and 3 s before it leaves) and 3 s after it left. The gate fails if Itda holds more than 250 MB over where a watch began, grows more than 40 MB over a watch's second half, or 3 s after the viewer left is still more than 40 MB above where it started. A 4K screen's working set is about 160 MB scaled and 200 MB at full size; an earlier version of the gate read it only after the viewer had left, by which time it was often already returned. 1.0.25 grew 918 MB a minute; 1.0.26 settled at about +40 MB; 1.0.29 kept about 130 MB of a departed 4K viewer's frames for minutes, sometimes hours.
- **Windows:** use the Windows 11 test VM only (Proxmox VM 220), never a personal PC. The release exe must start and open its window in the signed-in session. CI's `windows-deps-check.mjs` already refuses an exe that needs the Visual C++ runtime. For a change to sharing, also watch the VM's share with `hp_soak_viewer` and check that memory and handle counts return to where they started after each viewer.
- In use, Itda logs its own memory to `itda.log`: a `memory:` line at each new 256 MB step and every hour, and a warning past 1 GiB with the number of viewers.

For public screenshots and site checks, follow [docs/SCREENSHOTS.md](docs/SCREENSHOTS.md). Performance methodology and reproducible quality probes are in [scripts/hp-quality.md](scripts/hp-quality.md) and [scripts/hp-performance.md](scripts/hp-performance.md).


For a disposable configuration, set `ITDA_DATA_DIR` to a temporary parent directory. Both platforms append `Itda` to it; without the override, the normal platform directory is used. This separates profiles/settings/thumbnails, logs, and Windows WebView2 storage and instances. Isolated/test runs cannot refresh or change the real login-start entry. It does not create a separate OS credential vault or reset system permissions: use fictional accounts with unique ids and disabled sharing, and never copy real hosts/accounts into a disposable directory. An empty override is treated as unset. Specialized read-only saved-device probes such as `hp-viewer-probe.mjs` intentionally reuse real vault identities; they are not disposable-vault fixtures and must not edit or delete those copied profile/account ids.

The real Mac-to-Windows file test uses an installed Mac bundle and an already trusted Windows SSH alias:

```sh
ITDA_WINDOWS_SSH=my-windows-test-host node scripts/files-native-windows-e2e.mjs
```

It creates and removes its own temporary directories, checks Korean filename composition and byte preservation, and reports cancellation timing limits. It does not verify Explorer drag/drop or native Windows layout.

## Guidelines

- **Keep it small and fast.** New dependencies need a clear reason.
- **Match the surrounding style.** Use short comments and keep builds free of warnings.
- **Write actionable Korean UI text.** Explain the next step in words a nontechnical person can follow. Update both public languages when behavior changes.
- **Use isolated fixtures.** GUI/input tests can affect real windows, accounts and files. Do not inject input, change permissions or replace a running installation as a side effect of an ordinary unit/UI test.
- **Keep secrets in the OS vault.** Use Windows Credential Manager or the macOS Keychain. Never put passwords, key material or live QR/PIN credentials into logs, fixtures, screenshots or browser responses.
- **Keep documentation honest.** Separate a saved preference from active behavior, implementation from physical verification, and supported features from remaining work.

## Pull requests

Describe the concrete behavior changed, how it was checked, and any remaining limitations. Include screenshots for visual changes. For major work, discuss scope before starting; security reports belong in the [private advisory form](SECURITY.md), not a public issue.
