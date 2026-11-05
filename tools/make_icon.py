"""Draws res/app.ico (a small battery glyph) with no third-party dependencies.

    py tools/make_icon.py
"""
import struct, zlib, pathlib

SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
BODY = (0x26, 0x26, 0x26)
EDGE = (0xD8, 0xD8, 0xD8)
FILL = (0x4C, 0xC7, 0x64)

def rr_inside(x, y, x0, y0, x1, y1, r):
    if not (x0 <= x <= x1 and y0 <= y <= y1):
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r

def sample(u, v):
    # body outline, inner body, nub, charge fill (in unit coordinates)
    if rr_inside(u, v, 0.08 + 0.12, 0.24 + 0.12, 0.08 + 0.12 + 0.44, 0.76 - 0.12, 0.03):
        return FILL
    if rr_inside(u, v, 0.08 + 0.07, 0.24 + 0.07, 0.84 - 0.07, 0.76 - 0.07, 0.05):
        return BODY
    if rr_inside(u, v, 0.08, 0.24, 0.84, 0.76, 0.11):
        return EDGE
    if rr_inside(u, v, 0.82, 0.38, 0.95, 0.62, 0.04):
        return EDGE
    return None

def render(size, ss=6):
    rows = []
    for y in range(size):
        row = bytearray([0])
        for x in range(size):
            acc = [0, 0, 0]; hits = 0
            for sy in range(ss):
                for sx in range(ss):
                    c = sample((x + (sx + 0.5) / ss) / size, (y + (sy + 0.5) / ss) / size)
                    if c:
                        hits += 1
                        for i in range(3): acc[i] += c[i]
            if hits:
                row += bytes([acc[0] // hits, acc[1] // hits, acc[2] // hits, 255 * hits // (ss * ss)])
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    raw = b"".join(rows)
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))

def main():
    pngs = [render(s, 6 if s < 128 else 2) for s in SIZES]
    out = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    for s, p in zip(SIZES, pngs):
        out += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(p), offset)
        offset += len(p)
    out += b"".join(pngs)
    dest = pathlib.Path(__file__).resolve().parent.parent / "res" / "app.ico"
    dest.write_bytes(out)
    print(f"wrote {dest} ({len(out)} bytes)")

if __name__ == "__main__":
    main()
