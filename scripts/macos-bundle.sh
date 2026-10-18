#!/bin/sh
# Build Itda.app for macOS and zip it: dist/Itda-macos-<arch>.zip
#   scripts/macos-bundle.sh            # this Mac's architecture
#   scripts/macos-bundle.sh universal  # arm64 + x86_64 (needs both rustup targets)
set -eu
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
MODE=${1:-native}
OUT=dist
APP="$OUT/Itda.app"
rm -rf "$APP" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# Keep the build machine's paths (home folder, user name) out of the binary: panic messages
# and dependency file names would otherwise carry them. The last matching prefix wins.
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$HOME=~$(printf '\037')--remap-path-prefix=$PWD=itda"

if [ "$MODE" = universal ]; then
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  lipo -create -output "$APP/Contents/MacOS/itda" \
    target/aarch64-apple-darwin/release/itda target/x86_64-apple-darwin/release/itda
  ARCH=universal
else
  cargo build --release
  cp target/release/itda "$APP/Contents/MacOS/itda"
  ARCH=$(uname -m)
fi

# Icon: render the SVG master (Quick Look), falling back to the 512 px PNG.
ICONSET=$(mktemp -d)/itda.iconset
mkdir -p "$ICONSET"
MASTER="$ICONSET/../master.png"
if qlmanage -t -s 1024 -o "$ICONSET/.." assets/itda.svg >/dev/null 2>&1 && [ -f "$ICONSET/../itda.svg.png" ]; then
  mv "$ICONSET/../itda.svg.png" "$MASTER"
else
  cp assets/itda-512.png "$MASTER"
fi
for s in 16 32 128 256 512; do
  sips -z $s $s "$MASTER" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d "$MASTER" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/itda.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Itda</string>
  <key>CFBundleDisplayName</key><string>Itda</string>
  <key>CFBundleIdentifier</key><string>dev.jiun.itda</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>itda</string>
  <key>CFBundleIconFile</key><string>itda</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleDevelopmentRegion</key><string>ko</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSLocalNetworkUsageDescription</key><string>같은 네트워크의 Mac과 PC를 찾고 연결하려면 로컬 네트워크 접근이 필요합니다.</string>
  <key>NSBonjourServices</key><array><string>_rfb._tcp</string><string>_itda._tcp</string></array>
  <key>NSAppTransportSecurity</key><dict><key>NSAllowsLocalNetworking</key><true/></dict>
  <key>NSHumanReadableCopyright</key><string>MIT License · https://jiun.dev/itda/</string>
</dict>
</plist>
PLIST

# Ad-hoc signature so Gatekeeper treats the bundle as one unit (not notarized).
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

ZIP="$OUT/Itda-macos-$ARCH.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "$ZIP ($(du -h "$ZIP" | cut -f1))"
