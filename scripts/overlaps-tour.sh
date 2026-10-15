#!/bin/bash
# Visit every overlap the audit found (debug/overlaps.txt, from examples/overlaps.rs), one
# game window at a time: each opens standing where the crossing is seen, looking at it, with
# culling off. Escape closes the window and opens the next.
#
#   scripts/overlaps-tour.sh [FIRST] [LEVEL] [--top N]
#
# FIRST is the number of the view to start at (each is numbered as it opens), to pick up
# where a tour left off. LEVEL keeps only the levels whose name contains it (rm11, BT0).
# --top N keeps only the N longest overlaps of each level. In the game, V turns culling on and
# off, G colors the rooms and P outlines the portals. Ctrl+C here ends the tour.
cd "$(dirname "$0")/.." || exit 1
FIRST=1; LEVEL=""; TOP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --top) TOP="$2"; shift ;;
    [0-9]*) FIRST="$1" ;;
    *) LEVEL="$1" ;;
  esac
  shift
done
LIST="${LIST:-debug/overlaps.txt}"
[ -f "$LIST" ] || { echo "no $LIST: run the overlaps example first (see examples/overlaps.rs)"; exit 1; }
TARGET="${TARGET:-x86_64-pc-windows-gnu}"
cargo build -q --release --target "$TARGET" --bin rr 2>&1 | grep -E "^error" -A6
EXE="target/$TARGET/release/rr"
[ -f "$EXE.exe" ] && EXE="$EXE.exe"

# One line per view: level, rooms, what the audit says about it, and the game's arguments
VIEWS=$(awk -v level="$LEVEL" -v top="$TOP" '
  /^[^ ].*pairs of rooms cross where seen/ { name = $1; sub(/:$/, "", name); n = 0; next }
  / crossings / && /\[/ {
    pair = $0; sub(/.*\[/, "", pair); sub(/\].*/, "", pair)
    seen = $0; sub(/.*seen */, "", seen); sub(/ *\(.*/, "", seen)
    unlike = $0; sub(/.*\( */, "", unlike); sub(/ unlike.*/, "", unlike)
    long = $0; sub(/.*unlike\), */, "", long); sub(/ long.*/, "", long)
    portal = ($0 ~ / portal /) ? ", joined by a portal" : ""
    info = "rooms " pair ": " seen " crossings seen (" unlike " between different materials), " long " units long" portal
    next
  }
  /cargo rr/ {
    n++
    if (index(name, level) == 0 || (top > 0 && n > top)) next
    args = $0; sub(/.*cargo rr /, "", args)
    print name "\t" info "\t" args
  }' "$LIST")
TOTAL=$(printf '%s\n' "$VIEWS" | grep -c .)
[ "$TOTAL" -gt 0 ] || { echo "no views match"; exit 1; }
echo "$TOTAL views in $(printf '%s\n' "$VIEWS" | cut -f1 | sort -u | wc -l) levels"

i=0
while IFS=$'\t' read -r name info args; do
  i=$((i + 1))
  [ "$i" -lt "$FIRST" ] && continue
  echo
  echo "[$i/$TOTAL] $name, $info"
  echo "  cargo rr $args"
  # The game reads the keyboard through its window; keep it off the list being read here
  "$EXE" $args < /dev/null > /dev/null 2>&1
done <<< "$VIEWS"
echo
echo "done"
