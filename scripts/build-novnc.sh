#!/bin/sh
# Rebuild web/viewer/novnc.min.js: noVNC 1.7.0 from npm with Itda's ZRLE decoder
# (web/viewer/novnc-src/zrle.js, see web/viewer/NOVNC.md), bundled and minified by esbuild.
#   scripts/build-novnc.sh        (needs node/npx)
set -eu
cd "$(dirname "$0")/.."
VERSION=1.7.0
ESBUILD=0.24.2
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
(cd "$work" && npm pack "@novnc/novnc@$VERSION" --silent >/dev/null && tar xzf "novnc-novnc-$VERSION.tgz")
cp web/viewer/novnc-src/zrle.js "$work/package/core/decoders/zrle.js"
python3 scripts/patch-novnc.py "$work"
cp web/viewer/novnc-src/itda.js "$work/entry.js"
npx --yes "esbuild@$ESBUILD" "$work/entry.js" --bundle --format=esm --minify --target=es2022 --outfile=web/viewer/novnc.min.js
