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
# rm11's stairs climb through the wall and ceiling of the room below (119): half way up, the
# stairs must be drawn, and walking all the way up and all the way down must not stop at an
# invisible wall (the logs hold where the feet are)
run i "$D" --level rm11 --pos 29863,33152,-40320 --yaw 180 --pitch -10 --walk 26 --hold W
run j "$D" --level rm11 --pos 29863,33332,-39860 --yaw 0 --pitch -30 --walk 14 --hold W
run k "$D" --level rm11 --pos 29863,33152,-40320 --yaw 180 --walk 100 --hold W
run l "$D" --level rm11 --pos 29863,33332,-39860 --yaw 0 --walk 110 --hold W
# Standing still a little over 40 units before rm05's doorways, the room beyond must show
# whole: through the doorway into 317, and from outside the plane's door into the cabin. In
# the plane's door, the galley must hide the far doorway into 201
run m "$D" --level rm05 --pos 37789.8,32932,-35897.7 --yaw 370.5 --pitch -5.1 --walk 3 --hold ""
run n "$D" --level rm05 --pos 37358.7,32932,-36550 --yaw 262.9 --pitch -10.1 --walk 3 --hold ""
run o "$D" --level rm05 --pos 37400.8,32932,-36566.2 --yaw 189.7 --pitch -13.3 --walk 3 --hold ""
# In rm11's hall doorway into 119, looking through 119 into 120: the hall's wall, which stands
# inside 120, covers part of 120's doorway, as in the original engine (BUGS.txt); drawn once
# per path of portals (--portal-paths), it does not
run p "$D" --level rm11 --doors-open 1 --pos 29467.9,32932,-39542.9 --yaw 312.8 --pitch -5 --walk 3 --hold ""
run q "$D" --level rm11 --doors-open 1 --pos 29467.9,32932,-39542.9 --yaw 312.8 --pitch -5 --walk 3 --hold "" --portal-paths
# rm08 by the canal: through the canal side's portals the alley's far facade is seen through room
# 521 and back, and 521 must not cover it; and from the stairwell's landing
# the railing across it, on the edge of 521's volume, must show
run s "$D" --level rm08 --pos 34368,32567,-29610.3 --yaw 329.4 --pitch -0.8 --walk 3 --hold ""
run t "$D" --level rm08 --pos 34448,32567,-29726.5 --yaw 356.1 --pitch -10.9 --walk 3 --hold ""
# On rm05's boarding stairs, looking up at the top of the fuselage: a character on the upper
# deck, whose room is not drawn, must not poke out of it
run u "$D" --level rm05 --spawn 13 --pos 37229.5,32932,-36546.5 --yaw -100.6 --pitch 52 --walk 3 --hold ""
# On rm08 beside the shed (room 201, no portal into the sky over it), looking up at the
# crane: its tower must be whole
run v "$D" --level rm08 --pos 33136,32567,-30433.2 --yaw 352.5 --pitch 39 --walk 3 --hold ""
# In rm11's hall, 77 units from the doorway into 119 (outside the doorway cut), looking
# through it: the hall's stairs inside 119 must not show, and the hall's wall in 120's
# doorway does, as in the original
run w "$D" --level rm11 --doors-open 1 --pos 29400.5,32932,-39432.1 --yaw -35.8 --pitch 2.7 --walk 3 --hold ""
# On rm02's deck, which is in two rooms (100, the deck, and 605, the ship's outside) with a
# floor each at the same height and no portal between them: the deck's railings, crates and
# chimneys (100) must show, as in the original, wherever the ship's outside's floor comes first
run y "$D" --level rm02 --pos 39365.4,32932,-30775 --yaw -58 --pitch 2.5 --walk 3 --hold ""
# On the top landing of rm02's stairwell (room 5), over the doorway into 404 a floor below:
# the stairwell's walls around the open door into the generators must show
run z "$D" --level rm02 --doors-open 1 --pos 28527.1,34372,-31874 --yaw -691.6 --pitch -10.3 --walk 3 --hold ""
# One step past rm11's hall door into 119, looking through 119 into 120: the hall's wall in
# 120's doorway, which shows from the hall (p), must not show (the original hides it too)
run aa "$D" --level rm11 --doors-open 1 --pos 29495.1,32932,-39551.7 --yaw -51.2 --pitch -2.9 --walk 3 --hold ""
# rm08's room 105, looking up at the door in the nook between the crates (209): 105's brick
# wall above the nook must show, not the street beyond it (206)
run ab "$D" --level rm08 --pos 35561.5,32567,-31217.5 --yaw -17.4 --pitch 16.4 --walk 3 --hold ""
# The same door from the street (206), standing in the doorway into 208 and looking up: the
# street's roof over the door must show whole
run ac "$D" --level rm08 --pos 35619.9,32567,-31889.9 --yaw 540.9 --pitch 76.7 --walk 3 --hold ""
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
