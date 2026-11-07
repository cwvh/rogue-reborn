#!/bin/sh
# A 4v4 Team Survival test on this machine: a server, seven bots that fight (three on your side,
# four on the other) and your own window. Stop the script with Ctrl+C to stop the server and bots.
#
#   scripts/team-test.sh [LEVEL] [--side blue|gold] [--skill easy|normal|hard]
#                        [--net clean|good|typical|bad|awful] [--round-time SECONDS]
#                        [--base-pair N] [--seed N] [--port PORT] [--no-window | --host]
#                        [-- options for rr...]
#
# LEVEL is a level's name, citystreet_large unless given. The rounds start by themselves once
# both sides have someone. Closing your window leaves the game running: press Enter to join
# again (as a late joiner, if a round is under way), or Ctrl+C to stop. With --no-window only
# the server and bots start, and you join yourself.
#
# With --host your window starts the server (`rr --host`) and you are its host: you start each
# round from the lobby, and closing the window ends the game.
#
# --net makes your own connection worse, both ways, as in scripts/mp.sh:
#
#   good      15 ms each way,  5 ms jitter,  0% lost
#   typical   40 ms each way, 10 ms jitter,  1% lost
#   bad       90 ms each way, 30 ms jitter,  4% lost
#   awful    180 ms each way, 60 ms jitter, 10% lost
#
# Everything is kept in $TMP/team-test, cleared at each start:
#
#   server.txt, botN.txt   the logs (with --host, the server's is in window.txt)
#   match.rrs              the match as the server ran it, written as it goes
#   seenN.rrc              what your Nth window saw, written when it closes. Press M in the
#                          window to mark a moment worth looking at.
#
# `rrsrv --check FILE` plays a recording back. Round starts and results are printed as they
# happen. Needs the game data (see the README) and the MinGW target on Windows; set TARGET for
# another one. Plain sh (w64devkit's BusyBox runs it), so no arrays.
cd "$(dirname "$0")/.." || exit 1
TARGET="${TARGET:-x86_64-pc-windows-gnu}"

level=citystreet_large; side=blue; skill=normal; net=clean; port=27960; window=yes; host=no
server_extra=""
while [ $# -gt 0 ]; do
    case "$1" in
        --side) side=$2; shift 2 ;;
        --skill) skill=$2; shift 2 ;;
        --net) net=$2; shift 2 ;;
        --round-time) server_extra="$server_extra --round-time $2"; shift 2 ;;
        --base-pair) server_extra="$server_extra --base-pair $2"; shift 2 ;;
        --seed) server_extra="$server_extra --seed $2"; shift 2 ;;
        --port) port=$2; shift 2 ;;
        --no-window) window=no; shift ;;
        --host) host=yes; shift ;;
        --) shift; break ;;
        -*) echo "team-test.sh: unknown option $1 (see the top of scripts/team-test.sh)"; exit 2 ;;
        *) level=$1; shift ;;
    esac
done
# What is left in "$@" goes to rr
case "$side" in
    blue) other=gold ;;
    gold) other=blue ;;
    *) echo "team-test.sh: --side is blue or gold"; exit 2 ;;
esac
case "$net" in
    clean) lag="" ;;
    good) lag="--sim-latency 15 --sim-jitter 5 --sim-loss 0" ;;
    typical) lag="--sim-latency 40 --sim-jitter 10 --sim-loss 1" ;;
    bad) lag="--sim-latency 90 --sim-jitter 30 --sim-loss 4" ;;
    awful) lag="--sim-latency 180 --sim-jitter 60 --sim-loss 10" ;;
    *) echo "team-test.sh: --net is clean, good, typical, bad or awful"; exit 2 ;;
esac
case "$skill" in
    easy|normal|hard) ;;
    *) echo "team-test.sh: --skill is easy, normal or hard"; exit 2 ;;
esac
if [ "$host" = yes ] && [ "$window" = no ]; then
    echo "team-test.sh: --host needs your window, so not --no-window"; exit 2
fi

cargo build -q --release --target "$TARGET" --bins 2>&1 | grep -E "^error" -A6
BIN="target/$TARGET/release"
EXE=""; [ -f "$BIN/rr.exe" ] && EXE=.exe
OUT="${TMPDIR:-${TEMP:-/tmp}}/team-test"
mkdir -p "$OUT"
rm -f "$OUT"/*.txt "$OUT"/*.rrs "$OUT"/*.rrc "$OUT"/tail.pid

pids=""
stop() {
    trap - EXIT INT TERM
    [ -f "$OUT/tail.pid" ] && pids="$pids $(cat "$OUT/tail.pid")"
    # shellcheck disable=SC2086 # pids is words on purpose
    [ -n "$pids" ] && kill $pids 2>/dev/null
    wait 2>/dev/null
    echo "stopped; logs and recordings are in $OUT"
}
trap stop EXIT
trap 'exit 130' INT TERM

# Three bots on your side and four on the other, each for as long as the server runs
bots() {
    n=0
    for s in $side $side $side $other $other $other $other; do
        n=$((n + 1))
        "$BIN/rrbot$EXE" --connect "127.0.0.1:$port" --side "$s" --fight --skill "$skill" \
            --seconds 0 > "$OUT/bot$n.txt" 2>&1 &
        pids="$pids $!"
    done
}

# The rounds as they go, from a log. tail's own pid goes to a file so that it can be stopped
# (the pipeline's pid would be the loop's).
follow() {
    {
        tail -n +1 -f "$1" 2>/dev/null &
        echo $! > "$OUT/tail.pid"
        wait
    } | while IFS= read -r line; do
        case "$line" in *"round "[0-9]*|*" connected"|*"the game is over"*) echo "$line" ;; esac
    done &
    pids="$pids $!"
}

# Hosting: the window starts the server, and the bots join once it listens
if [ "$host" = yes ]; then
    # shellcheck disable=SC2086 # server_extra and lag are words on purpose
    "$BIN/rr$EXE" --host --level "$level" --port "$port" $server_extra --side "$side" \
        --first-person --textured $lag --record "$OUT/seen1.rrc" "$@" > "$OUT/window.txt" 2>&1 &
    you=$!
    pids="$you"
    follow "$OUT/window.txt"
    for _ in $(seq 300); do
        grep -q "listening" "$OUT/window.txt" && break
        if ! kill -0 "$you" 2>/dev/null; then cat "$OUT/window.txt"; exit 1; fi
        sleep 0.1
    done
    echo "hosting as $side${lag:+ with $lag}: start each round from the lobby"
    bots
    wait "$you"
    exit
fi

# shellcheck disable=SC2086 # server_extra is words on purpose
"$BIN/rrsrv$EXE" --level "$level" --port "$port" --max-players 8 --teams --rounds \
    --record "$OUT/match.rrs" $server_extra > "$OUT/server.txt" 2>&1 &
server=$!
pids="$server"
# Wait until it listens (loading a level takes a moment)
for _ in $(seq 100); do
    grep -q "listening" "$OUT/server.txt" 2>/dev/null && break
    if ! kill -0 "$server" 2>/dev/null; then cat "$OUT/server.txt"; exit 1; fi
    sleep 0.1
done
grep -E "listening|teams start" "$OUT/server.txt"
follow "$OUT/server.txt"

join="$BIN/rr$EXE --connect 127.0.0.1:$port --side $side --first-person --textured $lag"
if [ "$window" = no ]; then
    bots
    echo "server and bots running. Round 1 has started without you: joining now, you watch"
    echo "until the next round. Join with"
    echo "  $join $*"
    echo "Ctrl+C to stop"
    wait "$server"
    exit
fi
# A round starts once both sides have someone, so your window joins first and the bots follow,
# all within the countdown
k=0
while kill -0 "$server" 2>/dev/null; do
    k=$((k + 1))
    echo "joining as $side${lag:+ with $lag}"
    # shellcheck disable=SC2086 # join is words on purpose
    $join --record "$OUT/seen$k.rrc" "$@" &
    you=$!
    pids="$pids $you"
    if [ "$k" = 1 ]; then
        for _ in $(seq 300); do
            grep -q "1 player connected" "$OUT/server.txt" && break
            kill -0 "$you" 2>/dev/null || break
            sleep 0.1
        done
        bots
    fi
    wait "$you"
    echo "window closed: Enter to join again, Ctrl+C to stop"
    read -r _ || break
done
