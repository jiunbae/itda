#!/bin/bash
# Release gate: does a Mac's high-performance sharing keep its memory while someone watches?
#   scripts/hp-soak-gate.sh <ssh-host> [seconds=120] [max-growth-MB=150]
# Run it from this checkout against a Mac that runs the build about to be released (jiun-mini).
# It connects a view-only viewer over SSH (H.264, the pointer's shape and motion, a request
# every 16 ms; nothing is typed or clicked) and reads that Itda's physical footprint before,
# during and after. It fails when the footprint grows by more than the limit while watched.
# For reference, 1.0.25 grew 918 MB in 60 s, and 1.0.26 grew 38 MB and then held steady.
set -euo pipefail
HOST=${1:?usage: hp-soak-gate.sh <ssh-host> [seconds] [max-growth-MB]}
SECS=${2:-120}
MAX=${3:-150}
cd "$(dirname "$0")/.."
TMP=$(mktemp -d); trap 'rm -rf "$TMP"; [ -n "${TUN:-}" ] && kill "$TUN" 2>/dev/null || true' EXIT
umask 077

PID=$(ssh "$HOST" "pgrep -f '^/Applications/Itda.app/Contents/MacOS/itda' | head -1")
[ -n "$PID" ] || { echo "Itda isn't running on $HOST"; exit 2; }
VER=$(ssh "$HOST" 'defaults read /Applications/Itda.app/Contents/Info.plist CFBundleShortVersionString')
ssh "$HOST" 'cat ~/Library/Application\ Support/Itda/hp-password' > "$TMP/pass"
[ -s "$TMP/pass" ] || { echo "high-performance sharing isn't set up on $HOST"; exit 2; }
footprint() { ssh "$HOST" "footprint -p $PID 2>/dev/null | awk '/phys_footprint:/ {print \$2\$3}'" | python3 -c '
import sys,re
s=sys.stdin.read().strip(); m=re.match(r"([\d.]+)(KB|MB|GB)",s)
print(int(float(m[1])*{"KB":1/1024,"MB":1,"GB":1024}[m[2]]) if m else -1)'; }

PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
ssh -N -o ExitOnForwardFailure=yes -L "$PORT:127.0.0.1:5910" "$HOST" & TUN=$!
sleep 2
cargo test --release --quiet --no-run 2>/dev/null
BEFORE=$(footprint)
echo "$HOST: Itda $VER (pid $PID), ${BEFORE} MB before; watching for ${SECS} s"
( sleep $((SECS / 2)); echo "  halfway: $(footprint) MB" ) &
ITDA_SOAK_ADDR=127.0.0.1:$PORT ITDA_SOAK_PASS_FILE=$TMP/pass ITDA_SOAK_SECS=$SECS \
  cargo test --release --quiet hp_soak_viewer -- --ignored --nocapture 2>&1 | grep -E '^soak:' || { echo "the viewer couldn't watch"; exit 2; }
wait
AFTER=$(footprint)
GROWTH=$((AFTER - BEFORE))
echo "  after: ${AFTER} MB (grew ${GROWTH} MB; limit ${MAX} MB)"
if [ "$GROWTH" -gt "$MAX" ]; then
  echo "FAIL: Itda $VER on $HOST grows while watched"
  exit 1
fi
echo "PASS"
