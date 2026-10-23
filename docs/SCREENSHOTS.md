# Public screenshots and documentation

README.md, README.ko.md, the landing page (`docs/index.html`) and getting-started guide (`docs/start.html`) share images in `docs/img/`. Update both languages when changing a feature or prerequisite. The app's help center lives in `web/help.js`, with images in `web/help/`.

## Recreate the images

The captures use the current product frontend, fictional device names, reserved example addresses and in-memory file responses. They do not connect to devices, use saved accounts, grant permissions or invoke native file transfers. A Mac/Windows user agent selects the product's actual platform layout; this is not a photograph of a running Windows native window. On a non-Windows capture host, the Windows icon font is replaced by the app's SVG fallback.

Install the Playwright dependencies described in [CONTRIBUTING.md](../CONTRIBUTING.md), then run from the repository root:

```sh
node scripts/site-capture.mjs
# Also refresh the shared app-help images (requires cwebp):
node scripts/site-capture.mjs --help-images
```

`PLAYWRIGHT` can point to an existing Playwright `index.mjs`, `CHROMIUM` to a browser executable, and `CAPTURE_DIR` to an alternative output directory. The script checks that nearby devices are present, columns retain ancestors, gallery preview loads and the Windows toolbar icon is sized correctly. It generates light/dark images, whole-window overviews at 2× density, readable file details, mobile variants and the social preview. Keep overview images uncropped; use the existing responsive detail crops where the landing page needs readable controls on a small screen.

The current capture set was refreshed on 2026-10-24 from the product frontend at `03d832b7` on `main`, including the accessibility contrast and file-navigation updates. The seven help-center images use the same capture set. This records source provenance, not an installed or released build. Screenshots omit the app version so a docs-only or release version bump does not make otherwise current UI images obsolete. Recreate them after changes to tabs, commands, navigation, device cards, account editing or sharing. Do not label fictional data as a live connection or use captures as proof of native transfer/permission behavior.

## Check the result

Serve the repository root (the site is under `/docs/`):

```sh
python3 -m http.server 4186 --bind 127.0.0.1
# In another terminal:
node scripts/site-check.mjs
```

This checks 40 landing/guide combinations across five widths, light/dark themes and Korean/English, including image loading, overflow, platform tabs, enlargement and keyboard focus. QA images go to the temporary directory `itda-landing-qa`; use `SCREENSHOT_DIR` to override it. `SITE_URL` can target the published site for the same checks.

Visually review desktop/mobile layouts and both file-browser platforms. Confirm that the product controls remain legible, whole-window views preserve context, device frames do not distort images and enlargement opens the matching platform. Bump an image URL's existing cache revision when replacing its content.

## Maintain descriptions

Check current source and release behavior before describing an option as supported. In particular, distinguish built-in Screen Sharing from Itda's optional Mac high-performance server; remote permissions from local permissions; saved Retina preference from active shared capture; and SFTP copies from optional rsync deltas. Keep dated measurement reports as historical evidence, with their limits, rather than rewriting them as current universal performance claims.
