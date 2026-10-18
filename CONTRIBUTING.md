# Contributing

Thanks for helping! Itda is a small Windows app written in Rust with a WebView2 UI.

## Build

You need:

- Windows 10/11
- Rust (stable, MSVC toolchain)
- WebView2 Runtime (built into Windows 11)

```powershell
cargo build --release      # target\release\itda.exe
cargo test --release
```

For a release, build with `scripts/windows-release.ps1` (and `scripts/macos-bundle.sh universal` on a Mac) so the binaries don't carry your home folder's path.

The home UI lives in `web/` and is embedded in the exe. To edit it without rebuilding, set `ITDA_WEB_DIR=<path to web>` to serve it from disk.

## Guidelines

- **Keep it small and fast.** New dependencies need a good reason; check how much they add to the exe size.
- **Match the surrounding style.** Short doc comments, Korean UI text, and no warnings (`cargo build` must be clean).
- **UI text is Korean.** Write for people who aren't technical, and explain what to do, not just what went wrong.
- **Never test on your own desktop by accident.** GUI and input tests drive real windows and `SendInput`.
- **Keep secrets in Credential Manager.** Never write a password to a file, a log or the web viewer.

## Pull requests

Open an issue first for anything big. Describe:

- what changed
- how you tested it
- screenshots, for UI changes
