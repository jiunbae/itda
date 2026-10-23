# Contributing

Itda is a Rust app for Windows and macOS, with a shared HTML/CSS/JavaScript interface. Remote screens, file browsing and the mobile web viewer use platform-specific native bridges where needed. UI text is Korean; public guides are available in Korean and English.

## Build

Use stable Rust and Git. On Windows, install the MSVC build tools and WebView2 Runtime (included in Windows 11). On macOS, install Xcode Command Line Tools; a universal bundle also needs both Rust targets:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

On either platform:

```sh
cargo build --release
cargo test --release
```

The executable is `target/release/itda.exe` on Windows or `target/release/itda` on Mac. Package with `scripts/windows-release.ps1` or `scripts/macos-bundle.sh universal`; these scripts also remove build-machine paths. The Mac script writes `dist/Itda.app` and `dist/Itda-macos-universal.zip`. Signing and notarization require the release credentials; a local development bundle is not an official signed release.

The interface is embedded from `web/`. Set `ITDA_WEB_DIR` to an absolute `web/` directory for local iteration without embedding each edit. Restart when changing native Rust code. Static public documentation is in `docs/`, published by GitHub Pages.

## Check a change

Run the checks that exercise the affected behavior. CI builds/tests Windows and universal macOS, and runs browser regressions on Windows. Browser fixtures use Playwright:

```sh
npm install --no-save --package-lock=false playwright
npx playwright install chromium
node scripts/secure-context-check.mjs
node scripts/files-windows-e2e.mjs
```

Other focused suites include `files-tabs-e2e.mjs`, `files-network-local-e2e.mjs`, `editor-e2e.mjs`, `permissions-e2e.mjs`, `remote-setup-e2e.mjs`, `update-e2e.mjs`, and `monitors-e2e.mjs`. See `.github/workflows/ci.yml` for current CI coverage. Browser IPC fixtures verify the frontend, not real SSH, native OS drag acceptance, macOS permission grants or physical monitor behavior. Record those runtime checks separately when relevant.

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
