#!/bin/sh
# Play multiplayer alone: a server, computer players that fight (rrbot --fight) and your own
# window, all on this machine. Closing your window stops the rest.
#
#   scripts/mp.sh [LEVEL] [--bots N] [--skill easy|normal|hard] [--net clean|good|typical|bad|awful]
#                 [--npcs N] [--no-pvp] [--port PORT] [-- options for rr...]
#
# LEVEL is a level's name (rm11, citystreet_large), or --demo for the box room (the default).
# There are 3 bots unless --bots says otherwise (at most 7). --net makes your own connection
# worse, both ways:
#
#   good      15 ms each way,  5 ms jitter,  0% lost
#   typical   40 ms each way, 10 ms jitter,  1% lost
#   bad       90 ms each way, 30 ms jitter,  4% lost
#   awful    180 ms each way, 60 ms jitter, 10% lost
#
# The bots' connections stay clean. Who killed whom is printed as it happens; the server's log is
# in $TMP/mp/server.txt. Needs the game data (see the README) and the MinGW target on Windows; set
# TARGET for another one. Plain sh (w64devkit's BusyBox runs it), so no arrays.
cd "$(dirname "$0")/.." || exit 1
TARGET="${TARGET:-x86_64-pc-windows-gnu}"

level=--demo; bots=3; skill=normal; net=clean; port=27960; server_extra=""
while [ $# -gt 0 ]; do
    case "$1" in
        --bots) bots=$2; shift 2 ;;
        --skill) skill=$2; shift 2 ;;
        --net) net=$2; shift 2 ;;
        --port) port=$2; shift 2 ;;
        --npcs) server_extra="$server_extra --npcs $2"; shift 2 ;;
        --no-pvp) server_extra="$server_extra --no-pvp"; shift ;;
        --demo) level=--demo; shift ;;
        --) shift; break ;;
        -*) echo "mp.sh: unknown option $1 (see the top of scripts/mp.sh)"; exit 2 ;;
        *) level="--level $1"; shift ;;
    esac
done
# What is left in "$@" goes to rr
case "$net" in
    clean) lag="" ;;
    good) lag="--sim-latency 15 --sim-jitter 5 --sim-loss 0" ;;
    typical) lag="--sim-latency 40 --sim-jitter 10 --sim-loss 1" ;;
    bad) lag="--sim-latency 90 --sim-jitter 30 --sim-loss 4" ;;
    awful) lag="--sim-latency 180 --sim-jitter 60 --sim-loss 10" ;;
    *) echo "mp.sh: --net is clean, good, typical, bad or awful"; exit 2 ;;
esac
case "$skill" in easy|normal|hard) ;; *) echo "mp.sh: --skill is easy, normal or hard"; exit 2 ;; esac
if [ "$bots" -lt 0 ] || [ "$bots" -gt 7 ]; then echo "mp.sh: --bots is 0 to 7"; exit 2; fi

cargo build -q --release --target "$TARGET" --bins 2>&1 | grep -E "^error" -A6
BIN="target/$TARGET/release"
EXE=""; [ -f "$BIN/rr.exe" ] && EXE=.exe
OUT="${TMPDIR:-${TEMP:-/tmp}}/mp"
mkdir -p "$OUT"
rm -f "$OUT"/bot*.pid

pids=""
stop() {
    for f in "$OUT"/bot*.pid; do [ -f "$f" ] && pids="$pids $(cat "$f")"; done
    [ -n "$pids" ] && kill $pids 2>/dev/null
    wait 2>/dev/null
}
trap stop EXIT
trap 'exit 130' INT TERM

# shellcheck disable=SC2086 # level and server_extra are words on purpose
"$BIN/rrsrv$EXE" $level --port "$port" --max-players $((bots + 1)) $server_extra > "$OUT/server.txt" 2>&1 &
server=$!
pids="$server"
# Wait until it listens (loading a level takes a moment)
for _ in $(seq 100); do
    grep -q "listening" "$OUT/server.txt" 2>/dev/null && break
    if ! kill -0 "$server" 2>/dev/null; then cat "$OUT/server.txt"; exit 1; fi
    sleep 0.1
done
grep "listening" "$OUT/server.txt"

# Each bot's lines are prefixed with its number; the bot's own pid goes to a file so that it
# can be stopped (the pipeline's pid would be the shell's)
for i in $(seq "$bots"); do
    {
        "$BIN/rrbot$EXE" --connect "127.0.0.1:$port" --fight --skill "$skill" 2>&1 &
        echo $! > "$OUT/bot$i.pid"
        wait
    } | while IFS= read -r line; do
        case "$line" in loaded\ *) ;; *) echo "[bot $i] $line" ;; esac
    done &
    pids="$pids $!"
done

echo "joining${lag:+ with $lag}; close the window to stop"
# shellcheck disable=SC2086
"$BIN/rr$EXE" --connect "127.0.0.1:$port" --first-person --textured $lag "$@"
