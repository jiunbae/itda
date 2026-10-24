# Native macOS local-browser checks

This component checks local navigation, views, previews and tabs before any remote
device is configured. It does not verify transfers, native Finder exports, permission
grants, screen-reader speech or full persona journeys.

## Prepare a reproducible private candidate

Use the private bundle/profile procedure and the two isolated fixture switches in
[native-macos-quit-checks.md](native-macos-quit-checks.md), without its SFTP server or
generated account. Keep hosts/accounts empty, discovery and announcement disabled,
web/share off, and updates, notifications and login startup off. Do not set
`ITDA_TEST_HTTP`; this check needs actual native windows. Do not use protected
Desktop/Documents/Downloads locations or change TCC.

Build from the exact recorded source with clean status before and after:

```sh
cargo build --release --target aarch64-apple-darwin
```

If a shared `CARGO_TARGET_DIR` is used, record the actual source checkout, command,
source hashes and output hash. The checkout containing the cache is not the binary's
source. Verify that the complete current `web/files.js` bytes are embedded in the
binary. Copy into a unique private app, verify the copied pre-sign hash against the
build receipt, then ad hoc sign and record the signed executable's hash separately.
Omit URL handlers; record the launched PID, bundle ID and executable identity.

Create a unique generated folder inside the isolated worktree, accessible through
ordinary folder navigation from the local home. Its contents in this run were:

```text
<fixture>/
  Projects/2026/plan.txt
  readme.txt
  sample.png
```

`readme.txt` contains generated English and Korean text. `sample.png` is a generated
valid 64×64 RGB PNG. Record each relative filename and SHA256 before the UI test.
Neither the workspace nor the app profile should contain user documents or secrets.

## Native receiving checks

Use signed Tacto observations and semantic selectors in the unique candidate.
Observe before acting and after uncertain results. If input needs preparation,
explicitly prepare the selected native window, observe again, then focus the named
field before setting its value. Do not substitute global keys, raw coordinates or
unsigned input after a refusal. Stop on a locked screen or actual permission error.

1. Open File Browser with zero saved devices. Verify a separate Network window with
   an enabled **이 Mac** entry and the main window retained.
2. Enter **이 Mac** and navigate to the generated folder using ordinary columns.
   A search may narrow each folder's entries; focus its field first and clear the
   query on arrival. Pin the path field to the expected folder after each step.
3. In **열 보기**, open `Projects`, then `2026`. Verify both path changes,
   `plan.txt` and the still-selected columns control. Keep the ancestor columns.
4. Return through the fixture breadcrumb and select **아이콘 보기**. Verify its
   selected state and the three generated entries.
5. Select `readme.txt` and open the preview panel. Read its actual English/Korean
   body from the native accessibility tree; a selected filename alone is insufficient.
6. Select `sample.png` and **갤러리 보기**. Verify the selected control, the image
   description and the visible fit control. This establishes the native preview
   component, not pixel fidelity or visual Finder parity.
7. Add a tab with the button. Change the new tab to columns and open `Projects`.
   Return to the first tab: require the fixture path and gallery state. Return to
   the second: require its Projects path and columns state. Finally verify list
   view and the file table on the first tab.
8. Close the file window and independently observe the main window alone. Read
   the idle Quit menu state before dispatching it. If the target disappears during
   post-observation, do not repeat Quit: inspect the supervised child's exit code.

Do not invoke the native Finder-reveal action as part of this bounded run. Its
frontend dispatch and folder validation have separate tests. Go-to-folder was also
not accepted in this run: the tool refused its menu contract before dispatch. Do
not change an effect declaration to obtain an approval or credit that path anyway.

## Cleanup and scope

Require normal candidate exit, unchanged generated-file hashes and unchanged
general pasteboard `changeCount`; never inspect or restore clipboard contents.
Unregister only the unique bundle, remove its exact UUID caches/preferences/saved
state and both generated workspaces. Preserve failure receipts if cleanup or any
controller command fails; normal cleanup must not rewrite them into a PASS.

The current [receipt](../assessments/evidence/native-macos-local-browser.json) records
the component observations separately from a rejected unsupported supervisor
command. Its supervisor status remains `failed-cleaned`. It is a private ad-hoc
candidate with package version 1.0.3 at the later PR #121 source, not the published
1.0.3 binary or a signed 1.0.4 acceptance run. All verified persona scores remain null.
