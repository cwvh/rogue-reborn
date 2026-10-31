#!/bin/sh
# compare.sh [--status] C_DUMP RUST_DUMP
#
# Checks that the C reader reads every map as the Rust one does: their full
# dumps of every map in data/map, then their errors on maps cut short. With
# --status only whether each read fails is compared, not its message. The
# dumpers' paths are relative to the repository root. Fails if anything differs.
set -u
status=false
if [ "${1-}" = --status ]; then
    status=true
    shift
fi
cd "$(dirname "$0")/../.." || exit 2
c_dump=$1
rust_dump=$2
work=$(mktemp -d) || exit 2
trap 'rm -rf "$work"' EXIT

same=0
diff=0
for m in $(find data/map -iname "*.map" | sort); do
    "$c_dump" --full "$m" > "$work/c" 2> "$work/c.err"
    c_failed=$?
    "$rust_dump" "$m" > "$work/rust" 2> "$work/rust.err"
    rust_failed=$?
    if $status; then
        [ $c_failed -eq 0 ] && : > "$work/c.err" || echo failed > "$work/c.err"
        [ $rust_failed -eq 0 ] && : > "$work/rust.err" || echo failed > "$work/rust.err"
    fi
    if cmp -s "$work/c" "$work/rust" && cmp -s "$work/c.err" "$work/rust.err"; then
        same=$((same + 1))
    else
        diff=$((diff + 1))
        echo "different: $m"
    fi
done
echo "full dumps: $same same, $diff different"
failed=$diff

# 133 cuts of each, spread over the file and a little off any regular spacing
same=0
diff=0
for m in data/map/rm12/rm12.map data/map/rm05/rm05.map data/map/m00/citystreet_large.map; do
    size=$(wc -c < "$m")
    for i in $(seq 0 132); do
        n=$((size * i / 133 + (i * 7919) % 97))
        head -c "$n" "$m" > "$work/cut.map"
        c=$("$c_dump" --full "$work/cut.map" 2>&1 > /dev/null)
        c_failed=$?
        rust=$("$rust_dump" "$work/cut.map" 2>&1 > /dev/null)
        rust_failed=$?
        if $status; then
            c=$c_failed
            rust=$rust_failed
        fi
        if [ "$c" = "$rust" ]; then
            same=$((same + 1))
        else
            diff=$((diff + 1))
            [ "$diff" -le 5 ] && printf '%s cut to %s bytes\n C:    %s\n Rust: %s\n' "$m" "$n" "$c" "$rust"
        fi
    done
done
echo "cut short: $same same, $diff different"

[ $((failed + diff)) -eq 0 ]
