"""Writes the Stream Deck key's digit outlines from Optician Sans, so the plugin can
draw the battery number in it on any PC, with nothing to install.

    py tools/optician-digits.py [path\\to\\Optician-Sans.ttf]

Stream Deck only draws text in installed fonts, so the glyphs the number line needs
(0-9, % and the dash) go in as SVG path data instead, with the font's licence (SIL
OFL 1.1) next to them. No dependencies: reads the TrueType tables itself.
"""
import os
import pathlib
import struct
import sys

CHARS = "0123456789%–"
ROOT = pathlib.Path(__file__).resolve().parent.parent
PLUGIN = ROOT / "streamdeck" / "com.fayewave.hyperlight.sdPlugin"
DEFAULT = pathlib.Path(os.environ.get("LOCALAPPDATA", "")) / "Microsoft" / "Windows" / "Fonts" / "Optician-Sans.ttf"


class Font:
    def __init__(self, data):
        self.d = data
        n = struct.unpack(">H", data[4:6])[0]
        self.t = {}
        for i in range(n):
            tag, _, off, ln = struct.unpack(">4sIII", data[12 + 16 * i : 28 + 16 * i])
            self.t[tag.decode("latin-1")] = (off, ln)
        head = self.t["head"][0]
        self.upm = self.u16(head + 18)
        self.long_loca = self.i16(head + 50) == 1
        self.n_hmetrics = self.u16(self.t["hhea"][0] + 34)

    def u16(self, o):
        return struct.unpack(">H", self.d[o : o + 2])[0]

    def i16(self, o):
        return struct.unpack(">h", self.d[o : o + 2])[0]

    def name(self, nid):
        o = self.t["name"][0]
        _, cnt, so = struct.unpack(">HHH", self.d[o : o + 6])
        for i in range(cnt):
            pid, _, _, n, ln, off = struct.unpack(">6H", self.d[o + 6 + 12 * i : o + 18 + 12 * i])
            if pid == 3 and n == nid:
                return self.d[o + so + off : o + so + off + ln].decode("utf-16-be")
        return None

    def gid(self, ch):
        co = self.t["cmap"][0]
        for i in range(self.u16(co + 2)):
            o = co + struct.unpack(">I", self.d[co + 8 + 8 * i : co + 12 + 8 * i])[0]
            if self.u16(o) != 4:
                continue
            segs = self.u16(o + 6) // 2
            ends, starts = o + 14, o + 16 + 2 * segs
            deltas, ranges = starts + 2 * segs, starts + 4 * segs
            c = ord(ch)
            for s in range(segs):
                if self.u16(starts + 2 * s) <= c <= self.u16(ends + 2 * s):
                    r = self.u16(ranges + 2 * s)
                    if r == 0:
                        return (c + self.i16(deltas + 2 * s)) & 0xFFFF
                    g = self.u16(ranges + 2 * s + r + 2 * (c - self.u16(starts + 2 * s)))
                    return (g + self.i16(deltas + 2 * s)) & 0xFFFF if g else 0
        return 0

    def advance(self, g):
        return self.u16(self.t["hmtx"][0] + 4 * min(g, self.n_hmetrics - 1))

    def glyph_offset(self, g):
        lo = self.t["loca"][0]
        if self.long_loca:
            a, b = struct.unpack(">II", self.d[lo + 4 * g : lo + 4 * g + 8])
        else:
            a, b = (2 * v for v in struct.unpack(">HH", self.d[lo + 2 * g : lo + 2 * g + 4]))
        return (self.t["glyf"][0] + a, b - a)

    def contours(self, g, dx=0, dy=0):
        """Contours as lists of (x, y, on_curve), in font units, y up."""
        o, ln = self.glyph_offset(g)
        if ln == 0:
            return []
        n = self.i16(o)
        if n < 0:  # composite: its parts, offset
            out, p = [], o + 10
            while True:
                flags, sub = self.u16(p), self.u16(p + 2)
                p += 4
                if flags & 1:
                    ax, ay = struct.unpack(">hh", self.d[p : p + 4])
                    p += 4
                else:
                    ax, ay = struct.unpack(">bb", self.d[p : p + 2])
                    p += 2
                if flags & 0x08:
                    p += 2
                elif flags & 0x40:
                    p += 4
                elif flags & 0x80:
                    p += 8
                if flags & 0x0E8:
                    raise SystemExit(f"glyph {g}: scaled composite parts aren't handled")
                out += self.contours(sub, dx + ax, dy + ay)
                if not flags & 0x20:
                    return out
        ends = [self.u16(o + 10 + 2 * i) for i in range(n)]
        pts = ends[-1] + 1
        p = o + 10 + 2 * n
        p += 2 + self.u16(p)  # instructions
        flags = []
        while len(flags) < pts:
            f = self.d[p]
            p += 1
            flags.append(f)
            if f & 8:
                flags += [f] * self.d[p]
                p += 1
        coords = []
        for short, same in ((2, 16), (4, 32)):
            v, vals = 0, []
            for f in flags[:pts]:
                if f & short:
                    b = self.d[p]
                    p += 1
                    v += b if f & same else -b
                elif not f & same:
                    v += self.i16(p)
                    p += 2
                vals.append(v)
            coords.append(vals)
        xs, ys = coords
        out, start = [], 0
        for e in ends:
            out.append([(xs[i] + dx, ys[i] + dy, bool(flags[i] & 1)) for i in range(start, e + 1)])
            start = e + 1
        return out


def path(contours):
    """SVG path data, y flipped so the baseline is y=0 and up is negative."""
    fmt = lambda v: str(round(v, 1)).removesuffix(".0")
    pt = lambda x, y: f"{fmt(x)} {fmt(-y)}"
    out = []
    for c in contours:
        if not c:
            continue
        # Start on an on-curve point (or the midpoint of two off-curve ones).
        k = next((i for i, p in enumerate(c) if p[2]), None)
        if k is None:
            a, b = c[0], c[1]
            c = [((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, True)] + c[1:] + [c[0]]
        else:
            c = c[k:] + c[:k]
        s = f"M{pt(c[0][0], c[0][1])}"
        pts = c[1:] + [c[0]]
        i, prev_off = 0, None
        for x, y, on in pts:
            if on:
                s += f"Q{pt(*prev_off)} {pt(x, y)}" if prev_off else f"L{pt(x, y)}"
                prev_off = None
            else:
                if prev_off:
                    mx, my = (prev_off[0] + x) / 2, (prev_off[1] + y) / 2
                    s += f"Q{pt(*prev_off)} {pt(mx, my)}"
                prev_off = (x, y)
        if prev_off:
            s += f"Q{pt(*prev_off)} {pt(c[0][0], c[0][1])}"
        out.append(s + "Z")
    return "".join(out)


def main():
    src = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT
    font = Font(src.read_bytes())
    if font.upm != 1000:
        raise SystemExit(f"expected 1000 units per em, got {font.upm}")
    rows = []
    for ch in CHARS:
        g = font.gid(ch)
        if not g:
            raise SystemExit(f"{ch!r} is missing from {src}")
        key = '"\\u2013"' if ch == "–" else f'"{ch}"'
        rows.append(f'  {key}: {{ w: {font.advance(g) / font.upm}, d: "{path(font.contours(g))}" }},')
    version = font.name(5) or "?"
    designers = font.name(9) or "?"
    js = (
        "// Generated by tools/optician-digits.py: don't edit by hand.\n"
        f"// Optician Sans {version.removeprefix('Version ')}, by {designers} (www.scannerlicker.net),\n"
        "// licensed under the SIL Open Font License 1.1: see OFL.txt in this plugin.\n"
        "//\n"
        "// The glyphs the key's number line needs, as SVG paths in a 1000-unit em: baseline\n"
        "// at y=0, up is negative. w = advance width in ems.\n"
        "export const DIGITS = {\n" + "\n".join(rows) + "\n};\n"
    )
    (PLUGIN / "bin" / "digits.js").write_text(js, encoding="utf-8", newline="\n")
    licence = font.name(13) or ""
    ofl = (
        f"The battery digits in bin/digits.js are outlines from Optician Sans {version.removeprefix('Version ')},\n"
        f"by {designers} (www.scannerlicker.net).\n\n" + licence.replace("\r\n", "\n") + "\n"
    )
    (PLUGIN / "OFL.txt").write_text(ofl, encoding="utf-8", newline="\n")
    print(f"wrote bin/digits.js ({len(CHARS)} glyphs) and OFL.txt from {src}")


main()
