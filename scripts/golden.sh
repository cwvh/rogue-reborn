#!/bin/bash
# Record what a few scripted play-throughs do, to compare before and after a change.
#
#   scripts/golden.sh DIR
#   python scripts/pngdiff.py OLD_DIR NEW_DIR   # the screenshots
#   diff -r OLD_DIR NEW_DIR --exclude='*.png'   # the logs
#
# Each run uses the fixed 1/60 s step of --walk, so it is repeatable: DIR gets a log
# (a.txt...) and a screenshot (a.png...) per scenario. The logs hold what the game prints:
# shots, hits, kills, grenades and the player's position. Needs the game data (see the
# README) and the MinGW target on Windows; set TARGET for another one.
cd "$(dirname "$0")/.." || exit 1
[ -n "$1" ] || { echo "usage: scripts/golden.sh DIR"; exit 1; }
TARGET="${TARGET:-x86_64-pc-windows-gnu}"
mkdir -p "$1"
cargo build -q --release --target "$TARGET" --bin rr 2>&1 | grep -E "^error" -A6
EXE="target/$TARGET/release/rr"
[ -f "$EXE.exe" ] && EXE="$EXE.exe"

run() {
    name=$1; shift
    # The frame-rate lines and the file names differ from run to run, and which animation the
    # body plays is the viewer's business, so the position is kept and the rest is cut
    "$EXE" --first-person --textured --no-sound --max-fps 10000 --screenshot "$1/$name.png" "${@:2}" 2>&1 \
        | grep -vE "^(timing|wrote|loaded|spawning at|recorded)"         | sed -E 's/ movement=.*//' > "$1/$name.txt"
}

D="$1"
# Fire the gun at the characters in the demo room
run a "$D" --demo --walk 90 --hold F
# Run forward while firing, then reload
run b "$D" --demo --walk 150 --hold WFL
# Take out a grenade, throw it, and watch it go off
run c "$D" --demo --walk 330 --hold T
# Fire the whole magazine at a wall of the demo room, turned away from the characters
run d "$D" --demo --walk 120 --hold AF --yaw 90
# A real level: walk, press the use key for a door, and fire
run e "$D" --level rm11 --spawn 1 --walk 200 --hold WUF --doors-open 0
# Stand before a door of rm11, open it, and push forward while firing
run g "$D" --level rm11 --pos 33562,33330,-34647 --yaw 0 --walk 240 --hold WUF
# Strafe and walk slowly in the demo room
run f "$D" --demo --walk 120 --hold DRF
# Record a run and play it back: it must do the same, and every checksum must match
run h "$D" --demo --walk 330 --hold T --record "$D/h.rrd"
"$EXE" --first-person --textured --no-sound --max-fps 10000 --play "$D/h.rrd" --screenshot "$D/h-play.png" 2>&1     | grep -vE "^(timing|wrote|loaded|spawning at)" > "$D/h-play.txt"
if grep -q "matched the recording" "$D/h-play.txt"     && diff <(grep -v "^frame" "$D/h.txt") <(grep -vE "^(played|DESYNC)" "$D/h-play.txt") > /dev/null; then
    echo "playing the recording back matched the run"
else
    echo "PLAYING THE RECORDING BACK DID NOT MATCH THE RUN (see $D/h.txt and $D/h-play.txt)"
fi
echo "recorded in $D"
