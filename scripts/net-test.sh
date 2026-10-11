#!/bin/bash
# Try the server with real sockets: a server and two scripted clients on this machine, once over
# a clean connection and once over a bad one, and check that the game stayed in agreement.
#
#   scripts/net-test.sh
#
# The clients (rrbot) have no window. One runs and fires, the other throws a grenade. For each
# run this checks that both joined, that the grenade went off once for each of them, that the
# kills went to whoever threw it, and that the server never had to move a player far from where
# their client had predicted them. Needs the game data (see the README) and the MinGW target on
# Windows; set TARGET for another one.
cd "$(dirname "$0")/.." || exit 1
TARGET="${TARGET:-x86_64-pc-windows-gnu}"
cargo build -q --release --target "$TARGET" --bins 2>&1 | grep -E "^error" -A6
BIN="target/$TARGET/release"
SRV="$BIN/rrsrv"; BOT="$BIN/rrbot"
[ -f "$SRV.exe" ] && SRV="$SRV.exe" && BOT="$BOT.exe"
OUT="${TMPDIR:-${TEMP:-/tmp}}/net-test"
mkdir -p "$OUT"
status=0

check() { # name, then a test command
    local name=$1; shift
    if "$@"; then echo "  ok    $name"; else echo "  FAIL  $name"; status=1; fi
}

run() { # label port bad-network-options...
    local label=$1 port=$2; shift 2
    echo "$label"
    "$SRV" --demo --port "$port" --duration 10 --max-players 4 ${SRVOPT-} "$@" > "$OUT/srv.txt" 2>&1 &
    local srv=$!
    sleep 1.5
    "$BOT" --connect "127.0.0.1:$port" --keys WF --seconds 7 "$@" > "$OUT/bot1.txt" 2>&1 &
    local b1=$!
    "$BOT" --connect "127.0.0.1:$port" --keys T --seconds 7 "$@" > "$OUT/bot2.txt" 2>&1 &
    local b2=$!
    wait $b1 $b2 $srv
    local f1 f2
    f1=$(grep '^final' "$OUT/bot1.txt"); f2=$(grep '^final' "$OUT/bot2.txt")
    field() { echo "$1" | grep -o "$2=[^ ]*" | cut -d= -f2; }
    check "both clients joined and ended connected" test "$(field "$f1" status)" = Connected -a "$(field "$f2" status)" = Connected
    check "the server saw two players" grep -q "2 players connected" "$OUT/srv.txt"
    check "the grenade went off once, and both clients saw it" test "$(grep -c 'grenade went off' "$OUT/srv.txt")" = 1 -a "$(field "$f1" blasts)" = 1 -a "$(field "$f2" blasts)" = 1
    check "the kills went to whoever threw it" test "$(grep -c 'Killed' "$OUT/srv.txt")" = "$(field "$f2" kills)" -a "$(field "$f1" kills)" = 0
    local worst1 worst2
    worst1=$(field "$f1" worst_correction); worst2=$(field "$f2" worst_correction)
    check "prediction never strayed far ($worst1 and $worst2 units, at worst, against ${LIMIT:-1})" \
        awk -v a="$worst1" -v b="$worst2" -v l="${LIMIT:-1}" 'BEGIN { exit !(a < l && b < l) }'
}

# Players can hurt each other here: one throws a grenade that also catches the other, who dies and
# comes back, which moves them a long way. So this checks that everyone agrees on who died, not
# that nobody was moved. The server stops first, so that it can say how things stood while they
# were still there.
fight() {
    local port=27973
    echo "players hurting each other"
    "$SRV" --demo --port "$port" --duration 7 --max-players 4 > "$OUT/srv.txt" 2>&1 &
    local srv=$!
    sleep 1.5
    "$BOT" --connect "127.0.0.1:$port" --keys WF --seconds 8 > "$OUT/bot1.txt" 2>&1 &
    local b1=$!
    "$BOT" --connect "127.0.0.1:$port" --keys T --seconds 8 > "$OUT/bot2.txt" 2>&1 &
    local b2=$!
    wait $b1 $b2 $srv
    field() { echo "$1" | grep -o "$2=[^ ]*" | cut -d= -f2; }
    local f1 f2
    f1=$(grep '^final' "$OUT/bot1.txt"); f2=$(grep '^final' "$OUT/bot2.txt")
    check "both clients ended connected" test "$(field "$f1" status)" = Connected -a "$(field "$f2" status)" = Connected
    check "someone died" grep -q " died" "$OUT/srv.txt"
    for f in "$f1" "$f2"; do
        local slot server
        slot=$(field "$f" slot)
        server=$(grep "^player $slot " "$OUT/srv.txt")
        check "the server and player $slot agree on kills, deaths and health" \
            test "$(field "$f" kills) $(field "$f" deaths) $(field "$f" hp)" = "$(field "$server" kills) $(field "$server" deaths) $(field "$server" hp)"
    done
}

SRVOPT=--no-pvp LIMIT=1 run "a clean connection" 27971
SRVOPT=--no-pvp LIMIT=60 run "a bad connection (60 ms each way, 20 ms jitter, 5% lost)" 27972 --sim-latency 60 --sim-jitter 20 --sim-loss 5
fight
exit $status
