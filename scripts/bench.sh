#!/bin/bash
# Frame-time benchmark for the wireframe viewer: a few fixed views, three runs each.
#
#   scripts/bench.sh LABEL
#
# Each run stands still for 330 frames with vsync off and prints the average render time
# and whole-frame time after a 30-frame warm-up. Needs the game data (see the README) and
# the MinGW target on Windows; set TARGET for another one.
cd "$(dirname "$0")/.." || exit 1
TARGET="${TARGET:-x86_64-pc-windows-gnu}"
cargo build -q --release --target "$TARGET" --example wireframe 2>&1 | grep -E "^error" -A6
EXE="target/$TARGET/release/examples/wireframe"
[ -f "$EXE.exe" ] && EXE="$EXE.exe"
COMMON="--first-person --textured --max-fps 10000 --walk 330 --hold X"
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
