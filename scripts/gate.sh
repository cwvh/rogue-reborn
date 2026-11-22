#!/bin/bash
# The refactor gate: render the views and scripted runs, then compare two runs strictly.
#
#   scripts/gate.sh run DIR          # render into DIR/shots and DIR/golden (DIR must be empty)
#   scripts/gate.sh compare BASE NEW # pass only if NEW matches BASE byte for byte
#
# Take BASE from the code before a change and NEW after it, with the same PROFILE (release by
# default, or fast). Any failure prints why and exits 1. NEW is also checked by playing BASE's
# recording back with the current build, which must run every tick and match. Needs the game
# data (see the README) and the MinGW target on Windows. See common.sh for PROFILE and JOBS.
source "$(dirname "$0")/common.sh"
PY="${PYTHON:-python}"
fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

# The golden logs without the lines that differ from run to run
log_filter() { grep -vE "^(timing|wrote|loaded|spawning at|recorded|lit the level in )" "$1"; }

# N from the baseline's own "played N of N ticks" line
replay_ticks() { sed -nE 's/^played ([0-9]+) of \1 ticks: the game matched the recording.*/\1/p' "$1" | head -1; }

run() {
  [ -n "$1" ] || { echo "usage: scripts/gate.sh run DIR"; exit 1; }
  local d
  d="$(absdir "$1")"
  fresh_dir "$d"
  rm -rf "$d/.exit"
  build
  export NOBUILD=1
  local start=$SECONDS
  scripts/shots.sh "$d/shots" || fail "shots"
  scripts/golden.sh "$d/golden" || fail "golden"
  [ -f "$d/golden/h.rrd" ] || fail "no recording"
  echo "gate run took $((SECONDS - start)) s (PROFILE=$PROFILE)"
}

# Every expected file is there and every game exited 0
complete() {
  local d="$1"
  scripts/shots.sh --expect | verify "$d/shots" || fail "$d/shots is incomplete"
  scripts/golden.sh --expect | verify "$d/golden" || fail "$d/golden is incomplete"
}

compare() {
  [ -d "$1" ] && [ -d "$2" ] || { echo "usage: scripts/gate.sh compare BASE NEW"; exit 1; }
  local base new
  base="$(absdir "$1")"; new="$(absdir "$2")"
  complete "$base"
  complete "$new"

  for sub in shots golden; do
    "$PY" -I scripts/pngdiff.py --strict "$base/$sub" "$new/$sub" "${WIDTH}x${HEIGHT}" || fail "$sub pictures"
  done

  # The logs
  for f in "$base"/golden/*.txt; do
    n="${f##*/}"
    diff <(log_filter "$f") <(log_filter "$new/golden/$n") > /dev/null || { fail "log $n differs"; diff <(log_filter "$f") <(log_filter "$new/golden/$n") | head -10; }
  done
  [ "$(ls "$base/golden" | grep -c '\.txt$')" = "$(ls "$new/golden" | grep -c '\.txt$')" ] || fail "different numbers of logs"

  # The baseline's recording, played with this build: every tick, no desync, exit 0
  local ticks out="$new/replay"
  ticks="$(replay_ticks "$base/golden/h-play.txt")"
  [ -n "$ticks" ] || fail "the baseline's own replay did not play every tick"
  build
  mkdir -p "$out"
  "$EXE" --first-person --textured --no-sound --seed 1 --max-fps 10000 --cinematic --play "$base/golden/h.rrd" \
    --screenshot "$out/h-play.png" 2>&1 < /dev/null | grep -vE "^(timing|wrote|loaded|spawning at|lit the level in )" > "$out/h-play.txt"
  [ "${PIPESTATUS[0]}" = 0 ] || fail "the replay did not exit 0"
  grep -q "^played $ticks of $ticks ticks: the game matched the recording" "$out/h-play.txt" \
    || fail "the replay did not play $ticks of $ticks ticks and match (see $out/h-play.txt)"
  cmp -s "$out/h-play.png" "$base/golden/h-play.png" || fail "the replay's picture differs from the baseline's"

  if [ $fails = 0 ]; then echo "GATE PASSED"; else echo "GATE FAILED ($fails problems)"; return 1; fi
}

case "$1" in
  run) shift; run "$@"; [ $fails = 0 ] || { echo "GATE RUN FAILED"; exit 1; } ;;
  compare) shift; compare "$@" ;;
  *) echo "usage: scripts/gate.sh run DIR | compare BASE NEW"; exit 1 ;;
esac
