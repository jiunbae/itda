#!/bin/bash
# Release gate: does a Mac's high-performance sharing keep its memory while someone watches,
# and give it back once they leave?
#   scripts/hp-soak-gate.sh <ssh-host> [seconds=120] [max-held-MB=250] [max-left-MB=40]
# Run it from this checkout against a Mac that runs the build about to be released (jiun-mini).
# It connects a view-only viewer over SSH (a request every 16 ms; nothing is typed or clicked)
# three times: with scaled H.264 as Itda's browser and Windows viewers ask (a 4K screen goes out
# at 2/3, through the encoder's scaler), then for a quarter as long each with H.264 at the
# screen's size (viewers that can't scale) and with ZRLE (no H.264). It reads that Itda's
# physical footprint before each, halfway, 3 s before the viewer leaves and 3 s after. It fails
# when, while watched, the footprint is more than max-held-MB over where that watch began or
# grew more than 40 MB over the second half, or when 3 s after the viewer left more than
# max-left-MB is still counted on top of where the gate started.
# For reference, 1.0.25 grew 918 MB in 60 s, and 1.0.26 grew 38 MB and then held steady;
# 1.0.29 still had 130 MB left after a 4K viewer, for minutes or hours.
set -euo pipefail
HOST=${1:?usage: hp-soak-gate.sh <ssh-host> [seconds] [max-held-MB] [max-left-MB]}
SECS=${2:-120}
MAX=${3:-250}
LEFT=${4:-40}
# A leak shows as growth between halfway and the end; a 4K screen's working set (captured
# frames, the encoder's buffers) is there from the first seconds: about 160-200 MB.
CREEP=40
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
  local encs=$1 secs=$2 before half late left
  before=$(footprint)
  echo "$HOST: Itda $VER (pid $PID), ${before} MB before; watching with $encs for ${secs} s"
  # Read while the viewer still watches: halfway, and 3 s before it leaves (once it has left,
  # the memory goes back within moments, so a reading then says nothing about the watching).
  ( sleep $((secs / 2)); footprint > "$TMP/half" ) &
  ( sleep $((secs - 3)); footprint > "$TMP/late" ) &
  ITDA_SOAK_ENCODINGS=$encs ITDA_SOAK_ADDR=127.0.0.1:$PORT ITDA_SOAK_PASS_FILE=$TMP/pass ITDA_SOAK_SECS=$secs \
    cargo test --release --quiet hp_soak_viewer -- --ignored --nocapture 2>&1 | grep -E '^soak:' || { echo "the viewer couldn't watch"; exit 2; }
  wait
  half=$(cat "$TMP/half"); late=$(cat "$TMP/late")
  echo "  watching: ${half} MB halfway, ${late} MB at the end (+$((late - before)) MB; limit ${MAX} MB, and ${CREEP} MB over the second half)"
  if [ $((late - before)) -gt "$MAX" ]; then echo "  FAIL: holds too much while watched"; FAIL=1; fi
  if [ $((late - half)) -gt "$CREEP" ]; then echo "  FAIL: keeps growing while watched"; FAIL=1; fi
  sleep 3
  left=$(footprint)
  echo "  3 s after it left: ${left} MB ($((left - START)) MB more than at the start; limit ${LEFT} MB)"
  if [ $((left - START)) -gt "$LEFT" ]; then echo "  FAIL: keeps the memory after the viewer left"; FAIL=1; fi
}
watch scaled "$SECS"
watch h264 $(( SECS / 4 > 20 ? SECS / 4 : 20 ))
watch zrle $(( SECS / 4 > 20 ? SECS / 4 : 20 ))
if [ "$FAIL" = 1 ]; then
  echo "FAIL: Itda $VER on $HOST"
  exit 1
fi
echo "PASS"
