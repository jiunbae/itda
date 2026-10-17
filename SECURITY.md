# Security

Itda controls computers remotely, so security reports are very welcome.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private
[security advisory form](https://github.com/jiunbae/itda/security/advisories/new) instead. Include:

- what you found
- steps to reproduce
- the version (Settings → 정보)

You will get a reply within a week.

## Design and known limits

| Area | How it works | Limit |
|---|---|---|
| Passwords | Kept only in Windows Credential Manager (`Itda/…` entries). Never written to files or logs, never sent to the web viewer. | Anyone who can sign in to this Windows account can read them. |
| Mac connections | Apple ARD auth (Diffie-Hellman + AES-128) for the login. | The screen stream itself is not encrypted by Screen Sharing. Turn on **SSH** for an encrypted tunnel (system OpenSSH, host keys are pinned in `%APPDATA%\Itda\known_hosts`). |
| Web viewer | 6-digit PIN → HttpOnly, SameSite=Strict session cookie (12 h). 5 wrong PINs lock sign-in for 1 minute. WebSocket and edits require a same-origin request. | The browser ↔ PC leg is **plain HTTP** unless you use the Tailscale HTTPS address. Use it on your own Wi-Fi or over Tailscale, and **never forward the port to the internet**. Anyone with the PIN can open and edit saved connections. |
| Share this PC | VNC password (8 characters, random) plus an on-screen "allow?" prompt (on by default). | VNC password auth (DES challenge) is weak by today's standards. Only use it on trusted networks or over Tailscale. Viewers coming through the web viewer skip the prompt, because they already passed the PIN. |
| RDP (web viewer) | TLS + NLA (CredSSP/NTLM) via IronRDP. | The server certificate is not pinned. |
| Test hooks | `ITDA_TEST_HTTP`, `ITDA_TEST_SNAP` and `ITDA_DEBUG_PORT` exist for automated tests. | They do nothing unless those environment variables are set. |
