#!/bin/bash
# Release gate: does a Mac's high-performance sharing keep its memory while someone watches,
# and give it back once they leave?
#   scripts/hp-soak-gate.sh <ssh-host> [seconds=120] [max-growth-MB=150] [max-left-MB=40]
# Run it from this checkout against a Mac that runs the build about to be released (jiun-mini).
# It connects a view-only viewer over SSH (a request every 16 ms; nothing is typed or clicked)
# twice: with H.264 as a browser asks, then with ZRLE as Itda's own viewers ask, for a quarter
# as long. It reads that Itda's physical footprint before, during and after each. It fails when
# the footprint grows by more than the limit while watched, or when 3 s after the viewer left
# more than max-left-MB is still counted on top of where the gate started.
# For reference, 1.0.25 grew 918 MB in 60 s, and 1.0.26 grew 38 MB and then held steady;
# 1.0.29 still had 130 MB left after a 4K viewer, for minutes or hours.
set -euo pipefail
HOST=${1:?usage: hp-soak-gate.sh <ssh-host> [seconds] [max-growth-MB]}
SECS=${2:-120}
MAX=${3:-150}
LEFT=${4:-40}
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
FAIL=0
START=$(footprint)
watch() { # <encodings> <seconds>
  local encs=$1 secs=$2 before after left
  before=$(footprint)
  echo "$HOST: Itda $VER (pid $PID), ${before} MB before; watching with $encs for ${secs} s"
  ( sleep $((secs / 2)); echo "  halfway: $(footprint) MB" ) &
  ITDA_SOAK_ENCODINGS=$encs ITDA_SOAK_ADDR=127.0.0.1:$PORT ITDA_SOAK_PASS_FILE=$TMP/pass ITDA_SOAK_SECS=$secs \
    cargo test --release --quiet hp_soak_viewer -- --ignored --nocapture 2>&1 | grep -E '^soak:' || { echo "the viewer couldn't watch"; exit 2; }
  wait
  after=$(footprint)
  echo "  as it left: ${after} MB (grew $((after - before)) MB; limit ${MAX} MB)"
  if [ $((after - before)) -gt "$MAX" ]; then echo "  FAIL: grows while watched"; FAIL=1; fi
  sleep 3
  left=$(footprint)
  echo "  3 s later: ${left} MB ($((left - START)) MB more than at the start; limit ${LEFT} MB)"
  if [ $((left - START)) -gt "$LEFT" ]; then echo "  FAIL: keeps the memory after the viewer left"; FAIL=1; fi
}
watch h264 "$SECS"
watch zrle $(( SECS / 4 > 20 ? SECS / 4 : 20 ))
if [ "$FAIL" = 1 ]; then
  echo "FAIL: Itda $VER on $HOST"
  exit 1
fi
echo "PASS"
