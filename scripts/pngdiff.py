"""Compare the screenshots two runs of scripts/shots.sh made.

    python scripts/pngdiff.py OLD_DIR NEW_DIR

Prints how many pixels differ in each scenario's screenshot (a.png, b.png...), leaving out the block of debug
text in the top right corner (it shows the live frame rate, so it always differs).
Needs only the standard library.
"""
import os
import struct
import sys
import zlib


def read_png(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", f"{path} is not a PNG"
    pos, idat, width, height, ctype = 8, b"", 0, 0, 0
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            width, height, _depth, ctype = struct.unpack(">IIBB", body[:10])
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    channels = {2: 3, 6: 4, 0: 1}[ctype]
    raw = zlib.decompress(idat)
    stride = width * channels
    rows, prev, i = [], bytearray(stride), 0
    for _ in range(height):
        f = raw[i]
        line = bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - channels] if x >= channels else 0
            b = prev[x]
            c = prev[x - channels] if x >= channels else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else (b if pb <= pc else c))) & 255
        rows.append(line)
        prev = line
    return width, height, channels, rows


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    old_dir, new_dir = sys.argv[1:]
    # Every scenario's screenshot (a.png, b.png...), not the replay's (h-play.png)
    names = sorted(f[:-4] for f in os.listdir(old_dir) if f.endswith(".png") and "-" not in f)
    for name in names:
        if not os.path.exists(f"{new_dir}/{name}.png"):
            print(f"{name}: missing from {new_dir}")
            continue
        w, h, ch, a = read_png(f"{old_dir}/{name}.png")
        w2, h2, ch2, b = read_png(f"{new_dir}/{name}.png")
        assert (w, h, ch) == (w2, h2, ch2), f"{name}.png differs in size"
        diff = 0
        for y in range(h):
            for x in range(w):
                if y < 90 and x > 600:  # the FPS / render / room text
                    continue
                if a[y][x * ch:(x + 1) * ch] != b[y][x * ch:(x + 1) * ch]:
                    diff += 1
        print(f"{name}: {diff} differing pixels outside the debug text")


if __name__ == "__main__":
    main()
