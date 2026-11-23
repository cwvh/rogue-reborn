#!/bin/bash
# Record what a few scripted play-throughs do, to compare before and after a change. The gate
# (scripts/gate.sh) runs this and compares.
#
#   scripts/golden.sh DIR      # DIR must be empty or new
#   scripts/golden.sh --expect # the files a complete run leaves, one per line
#
# Each run uses the fixed 1/60 s step of --walk, so it is repeatable: DIR gets a log
# (a.txt...) and a screenshot (a.png...) per scenario. The logs hold what the game prints:
# shots, hits, kills, grenades and the player's position. Needs the game data (see the
# README) and the MinGW target on Windows. See common.sh for PROFILE, TARGET and JOBS.
source "$(dirname "$0")/common.sh"

# One case a line: name and game arguments
CASES=$(cat <<'EOF'
# Fire the gun at the characters in the demo room
a   --demo --walk 90 --hold F
# Run forward while firing, then reload
b   --demo --walk 150 --hold WFL
# Take out a grenade, throw it, and watch it go off
c   --demo --walk 330 --hold T
# Fire the whole magazine at a wall of the demo room, turned away from the characters
d   --demo --walk 120 --hold AF --yaw 90
# A real level: walk, press the use key for a door, and fire
e   --level rm11 --spawn 1 --walk 200 --hold WUF --doors-open 0
# Stand before a door of rm11, open it, and push forward while firing
g   --level rm11 --pos 33562,33330,-34647 --yaw 0 --walk 240 --hold WUF
# rm11's stairs climb through the wall and ceiling of the room below (119): half way up, the
# stairs must be drawn, and walking all the way up and all the way down must not stop at an
# invisible wall (the logs hold where the feet are)
i   --level rm11 --pos 29863,33152,-40320 --yaw 180 --pitch -10 --walk 26 --hold W
j   --level rm11 --pos 29863,33332,-39860 --yaw 0 --pitch -30 --walk 14 --hold W
k   --level rm11 --pos 29863,33152,-40320 --yaw 180 --walk 100 --hold W
l   --level rm11 --pos 29863,33332,-39860 --yaw 0 --walk 110 --hold W
# Standing still a little over 40 units before rm05's doorways, the room beyond must show
# whole: through the doorway into 317, and from outside the plane's door into the cabin. In
# the plane's door, the galley must hide the far doorway into 201
m   --level rm05 --pos 37789.8,32932,-35897.7 --yaw 370.5 --pitch -5.1 --walk 3 --hold -
n   --level rm05 --pos 37358.7,32932,-36550 --yaw 262.9 --pitch -10.1 --walk 3 --hold -
o   --level rm05 --pos 37400.8,32932,-36566.2 --yaw 189.7 --pitch -13.3 --walk 3 --hold -
# In rm11's hall doorway into 119, looking through 119 into 120: the hall's wall, which stands
# inside 120, covers part of 120's doorway, as in the original engine (BUGS.txt); drawn once
# per path of portals (--portal-paths), it does not
p   --level rm11 --doors-open 1 --pos 29467.9,32932,-39542.9 --yaw 312.8 --pitch -5 --walk 3 --hold -
q   --level rm11 --doors-open 1 --pos 29467.9,32932,-39542.9 --yaw 312.8 --pitch -5 --walk 3 --hold - --portal-paths
# rm08 by the canal: through the canal side's portals the alley's far facade is seen through room
# 521 and back, and 521 must not cover it; and from the stairwell's landing
# the railing across it, on the edge of 521's volume, must show
s   --level rm08 --pos 34368,32567,-29610.3 --yaw 329.4 --pitch -0.8 --walk 3 --hold -
t   --level rm08 --pos 34448,32567,-29726.5 --yaw 356.1 --pitch -10.9 --walk 3 --hold -
# On rm05's boarding stairs, looking up at the top of the fuselage: a character on the upper
# deck, whose room is not drawn, must not poke out of it
u   --level rm05 --spawn 13 --pos 37229.5,32932,-36546.5 --yaw -100.6 --pitch 52 --walk 3 --hold -
# On rm08 beside the shed (room 201, no portal into the sky over it), looking up at the
# crane: its tower must be whole
v   --level rm08 --pos 33136,32567,-30433.2 --yaw 352.5 --pitch 39 --walk 3 --hold -
# In rm11's hall, 77 units from the doorway into 119 (outside the doorway cut), looking
# through it: the hall's stairs inside 119 must not show, and the hall's wall in 120's
# doorway does, as in the original
w   --level rm11 --doors-open 1 --pos 29400.5,32932,-39432.1 --yaw -35.8 --pitch 2.7 --walk 3 --hold -
# On rm02's deck, which is in two rooms (100, the deck, and 605, the ship's outside) with a
# floor each at the same height and no portal between them: the deck's railings, crates and
# chimneys (100) must show, as in the original, wherever the ship's outside's floor comes first
y   --level rm02 --pos 39365.4,32932,-30775 --yaw -58 --pitch 2.5 --walk 3 --hold -
# On the top landing of rm02's stairwell (room 5), over the doorway into 404 a floor below:
# the stairwell's walls around the open door into the generators must show
z   --level rm02 --doors-open 1 --pos 28527.1,34372,-31874 --yaw -691.6 --pitch -10.3 --walk 3 --hold -
# One step past rm11's hall door into 119, looking through 119 into 120: the hall's wall in
# 120's doorway, which shows from the hall (p), must not show (the original hides it too)
aa  --level rm11 --doors-open 1 --pos 29495.1,32932,-39551.7 --yaw -51.2 --pitch -2.9 --walk 3 --hold -
# rm08's room 105, looking up at the door in the nook between the crates (209): 105's brick
# wall above the nook must show, not the street beyond it (206)
ab  --level rm08 --pos 35561.5,32567,-31217.5 --yaw -17.4 --pitch 16.4 --walk 3 --hold -
# The same door from the street (206), standing in the doorway into 208 and looking up: the
# street's roof over the door must show whole
ac  --level rm08 --pos 35619.9,32567,-31889.9 --yaw 540.9 --pitch 76.7 --walk 3 --hold -
# Strafe and walk slowly in the demo room
f   --demo --walk 120 --hold DRF
# Record a run and play it back (below): it must do the same, and every checksum must match.
# One on a level, one in the demo room (which desynced at tick 60 when the replay placed one
# character fewer than the run)
h   --level rm11 --spawn 1 --walk 330 --hold T --record @DIR@/h.rrd
dr  --demo --walk 150 --hold WF --record @DIR@/dr.rrd
EOF
)
REPLAYS="h dr"

expect() {
  for n in $(case_names "$CASES"); do echo ".exit/$n"; done
  for n in $REPLAYS; do echo ".exit/$n-play"; echo ".exit/$n-check"; done
  for n in $(case_names "$CASES"); do echo "$n.png"; echo "$n.txt"; done
  for n in $REPLAYS; do echo "$n.rrd"; echo "$n-play.png"; echo "$n-play.txt"; done
}

# The frame-rate lines and the file names differ from run to run, and which animation the
# body plays is the viewer's business, so the position is kept and the rest is cut
log_filter() {
  grep -vE "^(timing|wrote|loaded|spawning at|recorded|lit the level in )" | sed -E 's/ movement=.*//'
}

golden() {
  local name="$1"; shift
  "$EXE" --first-person --textured --no-sound --seed 1 --max-fps 10000 "$@"     --screenshot "$D/$name.png" 2>&1 < /dev/null | log_filter > "$D/$name.txt"
  echo "${PIPESTATUS[0]}" > "$D/.exit/$name"
}

# Play case $1's recording back: the game must exit 0, match it, play every tick, and print
# what the run printed
replay() {
  local n="$1"
  "$EXE" --first-person --textured --no-sound --seed 1 --max-fps 10000 --cinematic --play "$D/$n.rrd"     --screenshot "$D/$n-play.png" 2>&1 < /dev/null | grep -vE "^(timing|wrote|loaded|spawning at|lit the level in )" > "$D/$n-play.txt"
  echo "${PIPESTATUS[0]}" > "$D/.exit/$n-play"
  local ok=1
  grep -qE "^played ([0-9]+) of \1 ticks: the game matched the recording" "$D/$n-play.txt" || ok=0
  diff <(grep -v "^frame" "$D/$n.txt") <(grep -vE "^(played|DESYNC)" "$D/$n-play.txt") > /dev/null || ok=0
  if [ $ok = 1 ]; then
    echo "playing $n's recording back matched the run"
    echo 0 > "$D/.exit/$n-check"
  else
    echo "PLAYING $n's RECORDING BACK DID NOT MATCH THE RUN (see $D/$n.txt and $D/$n-play.txt)"
    echo 1 > "$D/.exit/$n-check"
  fi
}

if [ "$1" = "--expect" ]; then expect; exit 0; fi
[ -n "$1" ] || { echo "usage: scripts/golden.sh DIR"; exit 1; }
D="$(absdir "$1")"
fresh_dir "$D"
build
each_case "$CASES" golden
for n in $REPLAYS; do replay $n; done
expect | verify "$D"
