"""Writes the Stream Deck key's glyph outlines, so the plugin draws all its text the
same on any PC, with no fonts to install.

    py tools/key-glyphs.py path\\to\\Optician-Sans.ttf path\\to\\BaiJamjuree-Bold.ttf

Stream Deck only draws text in installed fonts, so the key's glyphs go in as SVG path
data instead: the battery number from Optician Sans (0-9, % and the dash), everything
else from Bai Jamjuree Bold (printable ASCII plus … ’ °). Both fonts are SIL OFL 1.1;
their licence goes next to the outlines in OFL.txt. No dependencies: reads the
TrueType tables itself.
"""
import pathlib
import struct
import sys

DIGIT_CHARS = "0123456789%–"
TEXT_CHARS = "".join(chr(c) for c in range(32, 127)) + "…’°"
ROOT = pathlib.Path(__file__).resolve().parent.parent
PLUGIN = ROOT / "streamdeck" / "com.fayewave.hyperlight.sdPlugin"


class Font:
    def __init__(self, path):
        self.path = path
        self.d = data = path.read_bytes()
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

    def f2dot14(self, o):
        return self.i16(o) / 16384

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

    def contours(self, g, m=(1, 0, 0, 1, 0, 0)):
        """Contours as lists of (x, y, on_curve), in font units, y up. m = (a, b, c, d, dx, dy)."""
        o, ln = self.glyph_offset(g)
        if ln == 0:
            return []
        n = self.i16(o)
        if n < 0:  # composite: its parts, each with its own offset and scale
            out, p = [], o + 10
            while True:
                flags, sub = self.u16(p), self.u16(p + 2)
                p += 4
                if not flags & 2:
                    raise SystemExit(f"{self.path.name} glyph {g}: point-matched composite parts aren't handled")
                if flags & 1:
                    ex, ey = struct.unpack(">hh", self.d[p : p + 4])
                    p += 4
                else:
                    ex, ey = struct.unpack(">bb", self.d[p : p + 2])
                    p += 2
                a, b, c, d = 1, 0, 0, 1
                if flags & 0x08:
                    a = d = self.f2dot14(p)
                    p += 2
                elif flags & 0x40:
                    a, d = self.f2dot14(p), self.f2dot14(p + 2)
                    p += 4
                elif flags & 0x80:
                    a, b, c, d = (self.f2dot14(p + 2 * i) for i in range(4))
                    p += 8
                ma, mb, mc, md, mx, my = m
                # The part's transform, then the parent's.
                sub_m = (ma * a + mc * b, mb * a + md * b, ma * c + mc * d, mb * c + md * d, ma * ex + mc * ey + mx, mb * ex + md * ey + my)
                out += self.contours(sub, sub_m)
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
        a, b, c, d, dx, dy = m
        out, start = [], 0
        for e in ends:
            out.append([(a * xs[i] + c * ys[i] + dx, b * xs[i] + d * ys[i] + dy, bool(flags[i] & 1)) for i in range(start, e + 1)])
            start = e + 1
        return out


def path(contours):
    """SVG path data, y flipped so the baseline is y=0 and up is negative."""
    def fmt(v):
        t = str(round(v, 1)).removesuffix(".0")
        return "0" if t == "-0" else t

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
        prev_off = None
        for x, y, on in c[1:] + [c[0]]:
            if on:
                s += f"Q{pt(*prev_off)} {pt(x, y)}" if prev_off else f"L{pt(x, y)}"
                prev_off = None
            else:
                if prev_off:
                    s += f"Q{pt(*prev_off)} {pt((prev_off[0] + x) / 2, (prev_off[1] + y) / 2)}"
                prev_off = (x, y)
        if prev_off:
            s += f"Q{pt(*prev_off)} {pt(c[0][0], c[0][1])}"
        out.append(s + "Z")
    return "".join(out)


def js_key(ch):
    if ch in "\"\\" or not ch.isascii():
        return '"\\u%04x"' % ord(ch)
    return f'"{ch}"'


def table(font, chars):
    if font.upm != 1000:
        raise SystemExit(f"{font.path.name}: expected 1000 units per em, got {font.upm}")
    rows = []
    for ch in chars:
        g = font.gid(ch)
        if not g:
            raise SystemExit(f"{ch!r} is missing from {font.path.name}")
        d = path(font.contours(g))
        rows.append(f"  {js_key(ch)}: {{ w: {font.advance(g) / font.upm}" + (f', d: "{d}"' if d else "") + " },")
    return "\n".join(rows)


def credit(font):
    version = (font.name(5) or "?").removeprefix("Version ").split(";")[0]
    who = font.name(0) or f"by {font.name(9) or '?'}"
    return f"{font.name(1)} {font.name(2)} {version}: {who}"


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    digits, text = Font(pathlib.Path(sys.argv[1])), Font(pathlib.Path(sys.argv[2]))
    js = (
        "// Generated by tools/key-glyphs.py: don't edit by hand.\n"
        "// Outlines from two SIL Open Font License 1.1 fonts (see OFL.txt in this plugin):\n"
        f"//   DIGITS: {credit(digits)}\n"
        f"//   TEXT: {credit(text)}\n"
        "//\n"
        "// SVG paths in a 1000-unit em: baseline at y=0, up is negative. w = advance width\n"
        "// in ems; no d = nothing to draw (a space).\n"
        f"export const DIGITS = {{\n{table(digits, DIGIT_CHARS)}\n}};\n\n"
        f"export const TEXT = {{\n{table(text, TEXT_CHARS)}\n}};\n"
    )
    (PLUGIN / "bin" / "glyphs.js").write_text(js, encoding="utf-8", newline="\n")
    ofl_path = text.path.with_name("OFL.txt")
    body = ofl_path.read_text(encoding="utf-8") if ofl_path.exists() else ""
    body = body[body.index("SIL OPEN FONT LICENSE") :] if "SIL OPEN FONT LICENSE" in body else digits.name(13) or ""
    ofl = (
        "The key's glyphs in bin/glyphs.js are outlines from these fonts, both licensed under\n"
        "the SIL Open Font License, Version 1.1 (below; FAQ at http://scripts.sil.org/OFL):\n\n"
        f"- {credit(digits)} (www.scannerlicker.net)\n"
        f"- {credit(text)}\n\n" + body.replace("\r\n", "\n").strip() + "\n"
    )
    (PLUGIN / "OFL.txt").write_text(ofl, encoding="utf-8", newline="\n")
    print(f"wrote bin/glyphs.js ({len(DIGIT_CHARS)} + {len(TEXT_CHARS)} glyphs) and OFL.txt")


main()
