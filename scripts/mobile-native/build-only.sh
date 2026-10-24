#!/bin/bash
# Compile only. Never creates/boots a device or runs a test, server, or Simulator UI.
set -euo pipefail
if [ "$#" -ne 0 ]; then
  echo "This scaffold supports build-only with no arguments. Runtime is blocked." >&2
  exit 64
fi
task_source="$(cd -- "$(dirname -- "$0")" && pwd)"
task_output="$(mktemp -d /tmp/itda-mobile-native-build.XXXXXX)"
printf 'Build-only artifacts: %s\n' "$task_output"
cp "$task_source/project.yml" "$task_output/project.yml"
cp -R "$task_source/Tests" "$task_output/Tests"
cmp "$task_source/project.yml" "$task_output/project.yml"
cmp "$task_source/Tests/ItdaMobileSafariTests.swift" "$task_output/Tests/ItdaMobileSafariTests.swift"
/usr/bin/shasum -a 256 "$task_output/project.yml" "$task_output/Tests/ItdaMobileSafariTests.swift" > "$task_output/copied-source.sha256"
node --check "$task_source/fixture.mjs"
node --check "$task_source/rfb-fixture.mjs"
xcodegen generate --spec "$task_output/project.yml" --project "$task_output"
xcrun xcodebuild build-for-testing \
  -project "$task_output/ItdaMobileSafariQA.xcodeproj" \
  -scheme ItdaMobileSafariQA \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath "$task_output/DerivedData" \
  -resultBundlePath "$task_output/build.xcresult" \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO
printf '\nCompiled only. No runtime verification occurred.\nArtifacts: %s\n' "$task_output"
printf 'Copied source hashes: %s/copied-source.sha256\n' "$task_output"
