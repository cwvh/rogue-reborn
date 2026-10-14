#!/bin/bash
# Frame-time benchmark for the viewer: a few fixed views, three runs each.
#
#   scripts/bench.sh LABEL
#
# Each run stands still for 330 frames with vsync off and prints the average render time
# and whole-frame time after a 30-frame warm-up. Needs the game data (see the README) and
# the MinGW target on Windows; set TARGET for another one.
cd "$(dirname "$0")/.." || exit 1
TARGET="${TARGET:-x86_64-pc-windows-gnu}"
cargo build -q --release --target "$TARGET" --bin rr 2>&1 | grep -E "^error" -A6
EXE="target/$TARGET/release/rr"
[ -f "$EXE.exe" ] && EXE="$EXE.exe"
COMMON="--first-person --textured --max-fps 10000 --walk 330 --hold X --doors-open 1"
OUT="${TMPDIR:-${TEMP:-/tmp}}/bench.png"

run() {
  name="$1"; shift
  for i in 1 2 3; do
    out=$("$EXE" $COMMON "$@" --screenshot "$OUT" 2>&1 | grep "timing over")
    echo "$name: $out"
  done
}

echo "== $1"
run "rm11 doorway   " --map data/map/rm11/rm11.map --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0
run "rm11 hall      " --map data/map/rm11/rm11.map --pos 29093.9,32932.0,-41069.6 --yaw 0 --pitch 0
run "rm05 cabin     " --map data/map/rm05/rm05.map --pos 37792.9,33272.0,-35970.0 --yaw -52.8 --pitch -0.4
run "rm11 no-cull   " --map data/map/rm11/rm11.map --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0 --no-cull
# Views that were slow in play (7 to 32 ms to render), doors closed as they were found
run "rm08 room 203  " --map data/map/rm08/rm08.map --pos 34332.5,32567.0,-29667.5 --yaw 345.3 --pitch -2.1 --doors-open 0
run "rm12 room 184  " --map data/map/rm12/rm12.map --pos 38878.0,33082.0,-34830.6 --yaw 87.3 --pitch 1.1 --doors-open 0
run "rm12 room 902  " --map data/map/rm12/rm12.map --pos 38860.1,34222.0,-32371.0 --yaw 97.0 --pitch -4.0 --doors-open 0
run "rm12 room 850  " --map data/map/rm12/rm12.map --pos 38692.2,34222.0,-32381.3 --yaw -3.5 --pitch -4.7 --doors-open 0
run "rm17 room 24   " --map data/map/rm17/rm17.map --pos 27010.6,32932.0,-28963.6 --yaw -59.0 --pitch -3.7 --doors-open 0
