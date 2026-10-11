#!/bin/bash
# Render a fixed set of views (a.png to h.png) with the current build, to check that a
# change leaves the picture alone.
#
#   scripts/shots.sh DIR
#
# To compare before and after a change, render once with the old code (git stash) and once
# with the new, then run scripts/pngdiff.py on the two directories:
#
#   git stash && scripts/shots.sh /tmp/old; git stash pop && scripts/shots.sh /tmp/new
#   python scripts/pngdiff.py /tmp/old /tmp/new
#
# Needs the game data (see the README) and the MinGW target on Windows; set TARGET for
# another one.
cd "$(dirname "$0")/.." || exit 1
[ -n "$1" ] || { echo "usage: scripts/shots.sh DIR"; exit 1; }
TARGET="${TARGET:-x86_64-pc-windows-gnu}"
mkdir -p "$1"
cargo build -q --release --target "$TARGET" --bin rr 2>&1 | grep -E "^error" -A6
EXE="target/$TARGET/release/rr"
[ -f "$EXE.exe" ] && EXE="$EXE.exe"
# Standing still for 40 frames at a fixed time step, so the characters pose the same each time
C="--first-person --textured --walk 40 --hold X --doors-open 1"

shot() {
  name="$1"; shift
  "$EXE" $C "$@" --screenshot "$OUT/$name.png" > /dev/null 2>&1
}
OUT="$1"
shot a --map data/map/rm11/rm11.map --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0
shot b --map data/map/rm11/rm11.map --pos 29737.0,32932.0,-40042.5 --yaw 271.9 --pitch 19.3
shot c --map data/map/rm11/rm11.map --pos 30047.0,32932.0,-39977.3 --yaw -233.2 --pitch -2.4
shot d --map data/map/rm11/rm11.map --pos 29093.9,32932.0,-41069.6 --yaw 0 --pitch 0
shot e --map data/map/rm05/rm05.map --pos 36500,32877,-36541 --yaw -90 --pitch 0
shot f --map data/map/rm05/rm05.map --pos 37792.9,33272.0,-35970.0 --yaw -52.8 --pitch -0.4
shot g --map data/map/rm05/rm05.map --pos 36348.9,32232.0,-36341.3 --yaw -106.3 --pitch 22.5
shot h --map data/map/rm11/rm11.map --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0 --rooms
ls "$OUT"/*.png
