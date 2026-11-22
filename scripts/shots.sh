#!/bin/bash
# Render a fixed set of views (a.png and so on) with the current build, to check that a
# change leaves the picture alone. The gate (scripts/gate.sh) runs this and compares.
#
#   scripts/shots.sh DIR      # DIR must be empty or new
#   scripts/shots.sh --expect # the files a complete run leaves, one per line
#
# Needs the game data (see the README) and the MinGW target on Windows. See common.sh for
# PROFILE, TARGET and JOBS.
source "$(dirname "$0")/common.sh"

# One case a line: name and game arguments. Every case stands still for --walk frames at a
# fixed time step, so the characters pose the same each time. "@hud" keeps the debug text.
CASES=$(cat <<'EOF'
a   --map data/map/rm11/rm11.map --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0
b   --map data/map/rm11/rm11.map --pos 29737.0,32932.0,-40042.5 --yaw 271.9 --pitch 19.3
c   --map data/map/rm11/rm11.map --pos 30047.0,32932.0,-39977.3 --yaw -233.2 --pitch -2.4
d   --map data/map/rm11/rm11.map --pos 29093.9,32932.0,-41069.6 --yaw 0 --pitch 0
e   --map data/map/rm05/rm05.map --pos 36500,32877,-36541 --yaw -90 --pitch 0
f   --map data/map/rm05/rm05.map --pos 37792.9,33272.0,-35970.0 --yaw -52.8 --pitch -0.4
g   --map data/map/rm05/rm05.map --pos 36348.9,32232.0,-36341.3 --yaw -106.3 --pitch 22.5
h   --map data/map/rm11/rm11.map --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0 --rooms
# One round, the frame after it: the gun in view and its flash (its casing is not drawn in
# first person)
i   --demo --viewmodel --walk 62 --hold B
# One round from the third-person view, five frames after it: a casing in the air
j   --demo --third-person --walk 66 --hold B
# The portal outlines and the collision shapes on one view
k   --map data/map/rm11/rm11.map --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0 --portals --collisions
# Debug text and HUD (no --cinematic), with the frame rate and render time fixed
l   --map data/map/rm11/rm11.map --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0 --rooms @hud
EOF
)

expect() { for n in $(case_names "$CASES"); do echo "$n.png"; echo ".exit/$n"; done; }

shot() {
  local name="$1"; shift
  "$EXE" --first-person --textured --no-sound --seed 1 --walk 40 --hold X --doors-open 1 "$@" \
    --screenshot "$D/$name.png" > /dev/null 2>&1 < /dev/null
  echo $? > "$D/.exit/$name"
}

if [ "$1" = "--expect" ]; then expect; exit 0; fi
[ -n "$1" ] || { echo "usage: scripts/shots.sh DIR"; exit 1; }
D="$(absdir "$1")"
fresh_dir "$D"
build
each_case "$CASES" shot
expect | verify "$D"
