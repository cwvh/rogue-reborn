#!/bin/bash
# Pack the files a level needs into one archive that unpacks to a data/ directory.
#
#   scripts/pack-level.sh MANIFEST [OUT]
#
# MANIFEST lists the files, one path per line (make one with `cargo run --example
# level-manifest`; see docs/PACK.md). OUT defaults to the manifest's name with .zip. The zip is
# made with Windows' own tar (bsdtar); elsewhere, without it, a .tar.gz. Players unpack it
# where they run rr, so data/ sits beside it.
cd "$(dirname "$0")/.." || exit 1
[ -n "$1" ] || { echo "usage: scripts/pack-level.sh MANIFEST [OUT]"; exit 1; }
MANIFEST="$1"
OUT="${2:-$(basename "$MANIFEST" .txt).zip}"
[ -f "$MANIFEST" ] || { echo "no manifest $MANIFEST"; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
missing=0
while IFS= read -r path; do
    path="${path%$'\r'}"
    [ -n "$path" ] || continue
    if [ ! -f "$path" ]; then
        echo "missing: $path"
        missing=1
        continue
    fi
    mkdir -p "$STAGE/$(dirname "$path")"
    cp "$path" "$STAGE/$path"
done < "$MANIFEST"
[ "$missing" = 0 ] || { echo "not packed: files are missing"; exit 1; }

rm -f "$OUT"
OUT_ABS="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"
if [ -x /c/Windows/System32/tar.exe ]; then
    /c/Windows/System32/tar.exe -a -cf "$OUT_ABS" -C "$STAGE" data || exit 1
else
    OUT_ABS="${OUT_ABS%.zip}.tar.gz"
    tar -czf "$OUT_ABS" -C "$STAGE" data || exit 1
fi
echo "packed $(grep -c . "$MANIFEST") files into $OUT_ABS ($(du -k "$OUT_ABS" | cut -f1) KB)"
