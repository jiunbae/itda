# Native macOS transfer and quit checks

Use a private app bundle and an isolated profile. The production app, personal files,
login items, permissions and general clipboard must stay untouched. Do not reset TCC
or grant capture/accessibility permissions to the candidate. The existing signed Tacto
helper drives the UI; stop on `screen_locked` or an actual permission error.

Two explicit fixture switches require a nonempty `ITDA_DATA_DIR`:

- `ITDA_NATIVE_TEST_NO_DISCOVERY=1` skips startup Bonjour/tailnet discovery and
  announcement, avoiding a new bundle's Local Network permission prompt.
- `ITDA_NATIVE_TEST_PRIVATE_PASTEBOARD=1` routes native Copy, its asynchronous
  completion and Paste through one unique NSPasteboard. Normal quit clears and releases
  that board. Its name is logged in the private profile for failed-run cleanup.

Neither switch changes a normal profile when set alone. Discovery is only skipped
at startup; keep `announce`, sharing, web access, notifications and automatic updates
explicitly off throughout the fixture. Keep `ITDA_TEST_HTTP` unset: that bridge makes
macOS headless and cannot verify the native menu or window.

Build the current source and copy it into a uniquely named private `.app` outside
`/Applications`, with a unique bundle identifier and display name. Omit
`CFBundleURLTypes` so the fixture cannot take over `itda://` links. Ad hoc sign and
verify it, then hash the **signed bundle's executable**, separately from the build
output. Record the source commit, both hashes, OS, bundle ID and candidate PID/path/
launch identity. An early `isFinishedLaunching` observation alone does not establish
UI readiness; independently observe the actual candidate window with Tacto.

The read-only loopback fixture `scripts/fixtures/read-only-sftp-barrier.py` supports
`--client-public-key <generated-ed25519.pub>`. This opt-in advertises public-key auth
and rejects passwords. Its default password mode is unchanged. Put the generated
private key in the owned 0700 run directory with mode0600; use a unique fictional
saved account referencing it, and a private `known_hosts`. Never upload the key,
connection manifest or passwords. The server exposes generated files only.

Before launch, read only the general pasteboard's `changeCount`. Verify it again after
cleanup; do not inspect or restore the person's clipboard contents. Native remote
Copy prepares an export first: leave the fixture unheld until that export completes.
Then arm `hold-read` before Paste to a separate generated local destination. Inspect
the copy confirmation's source: it must be the remote fixture, not the local export
cache. This distinction matters when opening another file window or changing tabs.

For each case, independently confirm an actual source read held at2MiB and a nonzero
`.itda-*.partial` destination before requesting quit through the native app menu:

1. Cancel must leave the app, held transfer and staging alive.
2. Wait must leave the app alive while held; only then release the fixture. Require
   normal exit and exactly one4MiB destination with the source's SHA256.
3. Stop must exit normally, preserve the generated source and leave no destination
   or staging file. Transport close by itself does not prove cleanup.

The native menu action verifies the custom quit handler; it does not establish
physical CmdQ, AppleEvent/Dock/logout, Finder promise queues or forced exit. A Tacto
`commit_unverified` result requires independent observation and file/audit evidence;
never replay an uncertain dispatched action. A policy refusal before dispatch gives
no behavior credit. Resolve the authorization or retain that case as unverified.

After normal exit, stop the owned SFTP server, unregister the private app from
LaunchServices, delete its generated workspace/key and only that UUID bundle's caches.
Record forced cleanup separately if a run fails; release a leftover unique pasteboard
by its logged name, never the general board. Recheck clipboard metadata after cleanup.
No candidate installation, release, persona score or platform-wide completion follows
from these component checks.

The retained receipt `assessments/evidence/native-macos-quit-component.json` currently
proves idle menu quit, a real held transfer, its busy warning and Cancel. Wait and Stop
were refused by the automation policy before action and remain unverified. Its build
hash is explicitly pre-signing; the signed candidate hash was not captured. These
limitations must remain until a new correctly recorded run supplies the evidence.
