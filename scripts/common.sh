# Shared by shots.sh, golden.sh, bench.sh and gate.sh (sourced, not run).
#
# PROFILE picks release (the default) or fast. TARGET picks the Rust target. NOBUILD=1 skips
# the build (the gate builds once). JOBS is how many games run at a time.
START_DIR="$PWD"
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
TARGET="${TARGET:-x86_64-pc-windows-gnu}"
PROFILE="${PROFILE:-release}"
JOBS="${JOBS:-4}"
WIDTH="${WIDTH:-1280}"
HEIGHT="${HEIGHT:-1024}"
case "$PROFILE" in release|fast) ;; *) echo "PROFILE must be release or fast"; exit 1 ;; esac
EXE="target/$TARGET/$PROFILE/rr"
[ -f "$EXE.exe" ] && EXE="$EXE.exe"

# A path as given from where the script was started, made absolute
absdir() { case "$1" in /*|?:*) echo "$1" ;; *) echo "$START_DIR/$1" ;; esac; }

# Any build error stops the run: a stale exe must never be used
build() {
  [ -n "$NOBUILD" ] || cargo build -q --profile "$PROFILE" --target "$TARGET" --bin rr || { echo "BUILD FAILED"; exit 1; }
  EXE="target/$TARGET/$PROFILE/rr"
  [ -f "$EXE.exe" ] && EXE="$EXE.exe"
  [ -f "$EXE" ] || { echo "no exe at $EXE"; exit 1; }
}

# Refuse a directory that holds anything, so stale files can't pass for fresh ones
fresh_dir() {
  if [ -n "$(ls -A "$1" 2>/dev/null)" ]; then
    echo "$1 is not empty"; exit 1
  fi
  mkdir -p "$1/.exit"
}

# Runs FUNC NAME [--cinematic] ARGS for each case of a list, JOBS at a time. A line is
# "name args". "@hud" in the args leaves --cinematic out, and "@DIR@" is replaced by $D (spaces are fine).
each_case() {
  local list="$1" fn="$2" name args cine a i
  local -a arr kept
  while read -r name args; do
    case "$name" in ''|'#'*) continue ;; esac
    cine=(--cinematic)
    read -ra arr <<< "$args"
    kept=()
    for a in "${arr[@]}"; do
      if [ "$a" = "@hud" ]; then cine=(); else kept+=("${a//@DIR@/$D}"); fi
    done
    while [ "$(jobs -rp | wc -l)" -ge "$JOBS" ]; do wait -n; done
    "$fn" "$name" "${cine[@]}" "${kept[@]}" &
  done <<< "$list"
  wait
}

case_names() { grep -vE '^(#|[[:space:]]*$)' <<< "$1" | awk '{print $1}'; }

# Checks that every expected file (one per line on stdin) is in DIR, and that every game that
# ran there exited 0. Prints what is wrong and returns 1.
verify() {
  local dir="$1" bad=0 f
  while read -r f; do
    [ -e "$dir/$f" ] || { echo "MISSING $dir/$f"; bad=1; continue; }
    case "$f" in *.png|*.rrd) [ -s "$dir/$f" ] || { echo "EMPTY $dir/$f"; bad=1; } ;; esac
  done
  for f in "$dir"/.exit/*; do
    [ "$(cat "$f")" = 0 ] || { echo "FAILED ${f##*/} in $dir (exit $(cat "$f"))"; bad=1; }
  done
  return $bad
}
