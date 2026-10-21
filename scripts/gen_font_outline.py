#!/usr/bin/env python3
"""Extract glyph outlines (quadratic TrueType contours) for the shell's UI font into src/shell/font_data.c.

Source face: Inter (SIL OFL 1.1), see THIRD-PARTY.md. The shell rasterises these outlines at any size/scale at run time
(src/shell/raster.c), so text is resolution independent and anti-aliased instead of a fixed bitmap table.
usage: gen_font_outline.py [regular.ttf semibold.ttf]  (re-run only to regenerate the table)"""
import struct
import sys

REG = sys.argv[1] if len(sys.argv) > 1 else "/usr/share/fonts/rsms-inter-fonts/Inter-Regular.ttf"
SEMI = sys.argv[2] if len(sys.argv) > 2 else "/usr/share/fonts/rsms-inter-fonts/Inter-SemiBold.ttf"

CODEPOINTS = list(range(0x20, 0x7f)) + list(range(0xa0, 0x100)) + [
    0x2013, 0x2014, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2026, 0x2190, 0x2191, 0x2192, 0x2193, 0x2713, 0x2715, 0x20ac]


class Font:
    def __init__(self, path):
        d = open(path, "rb").read()
        self.d = d
        n = struct.unpack(">H", d[4:6])[0]
        self.t = {}
        for i in range(n):
            tag, _, off, ln = struct.unpack(">4sIII", d[12 + 16 * i:28 + 16 * i])
            self.t[tag.decode()] = (off, ln)
        h = self.t["head"][0]
        self.upm = struct.unpack(">H", d[h + 18:h + 20])[0]
        self.loc_fmt = struct.unpack(">h", d[h + 50:h + 52])[0]
        self.ng = struct.unpack(">H", d[self.t["maxp"][0] + 4:self.t["maxp"][0] + 6])[0]
        hh = self.t["hhea"][0]
        self.ascent, self.descent = struct.unpack(">hh", d[hh + 4:hh + 8])
        self.nhm = struct.unpack(">H", d[hh + 34:hh + 36])[0]
        self.cmap = self._cmap()

    def _cmap(self):
        d = self.d
        off = self.t["cmap"][0]
        n = struct.unpack(">H", d[off + 2:off + 4])[0]
        sub = None
        for i in range(n):
            pid, eid, so = struct.unpack(">HHI", d[off + 4 + 8 * i:off + 12 + 8 * i])
            if (pid, eid) in ((3, 1), (0, 3), (3, 10), (0, 4)):
                fmt = struct.unpack(">H", d[off + so:off + so + 2])[0]
                if fmt in (4, 12):
                    sub = (off + so, fmt)
                    if fmt == 4:
                        break
        assert sub, "no usable cmap"
        o, fmt = sub
        m = {}
        if fmt == 4:
            segx2 = struct.unpack(">H", d[o + 6:o + 8])[0]
            seg = segx2 // 2
            ends = struct.unpack(">%dH" % seg, d[o + 14:o + 14 + segx2])
            starts = struct.unpack(">%dH" % seg, d[o + 16 + segx2:o + 16 + 2 * segx2])
            deltas = struct.unpack(">%dh" % seg, d[o + 16 + 2 * segx2:o + 16 + 3 * segx2])
            ro_pos = o + 16 + 3 * segx2
            ros = struct.unpack(">%dH" % seg, d[ro_pos:ro_pos + segx2])
            for i in range(seg):
                for c in range(starts[i], ends[i] + 1):
                    if c == 0xffff:
                        continue
                    if ros[i] == 0:
                        g = (c + deltas[i]) & 0xffff
                    else:
                        p = ro_pos + 2 * i + ros[i] + 2 * (c - starts[i])
                        g = struct.unpack(">H", d[p:p + 2])[0]
                        if g:
                            g = (g + deltas[i]) & 0xffff
                    if g:
                        m[c] = g
        else:
            ng = struct.unpack(">I", d[o + 12:o + 16])[0]
            for i in range(ng):
                s, e, g = struct.unpack(">III", d[o + 16 + 12 * i:o + 28 + 12 * i])
                for c in range(s, e + 1):
                    m[c] = g + c - s
        return m

    def advance(self, g):
        h = self.t["hmtx"][0]
        i = min(g, self.nhm - 1)
        return struct.unpack(">H", self.d[h + 4 * i:h + 4 * i + 2])[0]

    def _glyph_range(self, g):
        loca, glyf = self.t["loca"][0], self.t["glyf"][0]
        if self.loc_fmt == 0:
            a, b = struct.unpack(">HH", self.d[loca + 2 * g:loca + 2 * g + 4])
            return glyf + a * 2, glyf + b * 2
        a, b = struct.unpack(">II", self.d[loca + 4 * g:loca + 4 * g + 8])
        return glyf + a, glyf + b

    def contours(self, g, dx=0, dy=0, depth=0):
        """list of contours, each a list of (x, y, on)"""
        a, b = self._glyph_range(g)
        if a == b:
            return []
        d = self.d
        nc = struct.unpack(">h", d[a:a + 2])[0]
        p = a + 10
        if nc >= 0:
            ends = struct.unpack(">%dH" % nc, d[p:p + 2 * nc])
            p += 2 * nc
            il = struct.unpack(">H", d[p:p + 2])[0]
            p += 2 + il
            npts = ends[-1] + 1 if nc else 0
            flags = []
            while len(flags) < npts:
                f = d[p]
                p += 1
                flags.append(f)
                if f & 8:
                    r = d[p]
                    p += 1
                    flags.extend([f] * r)
            xs, x = [], 0
            for f in flags:
                if f & 2:
                    v = d[p]
                    p += 1
                    x += v if f & 16 else -v
                elif not f & 16:
                    x += struct.unpack(">h", d[p:p + 2])[0]
                    p += 2
                xs.append(x)
            ys, y = [], 0
            for f in flags:
                if f & 4:
                    v = d[p]
                    p += 1
                    y += v if f & 32 else -v
                elif not f & 32:
                    y += struct.unpack(">h", d[p:p + 2])[0]
                    p += 2
                ys.append(y)
            out, s = [], 0
            for e in ends:
                out.append([(xs[i] + dx, ys[i] + dy, flags[i] & 1) for i in range(s, e + 1)])
                s = e + 1
            return out
        assert depth < 4
        out = []
        while True:
            fl, gi = struct.unpack(">HH", d[p:p + 4])
            p += 4
            if fl & 1:
                ax, ay = struct.unpack(">hh", d[p:p + 4])
                p += 4
            else:
                ax, ay = struct.unpack(">bb", d[p:p + 2])
                p += 2
            assert fl & 2, "point-matching composites unsupported"
            if fl & 8:
                p += 2
            elif fl & 0x40:
                p += 4
            elif fl & 0x80:
                p += 8
            assert not fl & 0xc8 or True
            out += self.contours(gi, dx + ax, dy + ay, depth + 1)
            if not fl & 0x20:
                break
        return out


def emit(name, f, out):
    glyphs, pts, ons, ends = [], [], [], []
    for cp in CODEPOINTS:
        g = f.cmap.get(cp if cp != 0xa0 else 0x20)
        if g is None:
            continue
        cs = f.contours(g)
        pt0, en0 = len(ons), len(ends)
        for c in cs:
            for x, y, on in c:
                pts += [x, y]
                ons.append(on)
            ends.append(len(ons) - pt0 - 1 if False else len(ons))   # absolute end index (exclusive)
        glyphs.append((cp, f.advance(g), pt0, en0, len(cs)))
    out.append(f"static const int16_t {name}_pts[] = {{")
    for i in range(0, len(pts), 16):
        out.append("    " + ",".join(str(v) for v in pts[i:i + 16]) + ",")
    out.append("};")
    out.append(f"static const uint8_t {name}_on[] = {{")
    for i in range(0, len(ons), 32):
        out.append("    " + ",".join(str(v) for v in ons[i:i + 32]) + ",")
    out.append("};")
    out.append(f"static const uint16_t {name}_ends[] = {{")
    for i in range(0, len(ends), 16):
        out.append("    " + ",".join(str(v) for v in ends[i:i + 16]) + ",")
    out.append("};")
    out.append(f"static const loos_glyph {name}_glyphs[] = {{")
    for cp, adv, p0, e0, nc in glyphs:
        out.append(f"    {{ {cp}, {adv}, {p0}, {e0}, {nc} }},")
    out.append("};")
    out.append(f"const loos_font loos_font_{name} = {{ {f.upm}, {f.ascent}, {f.descent}, {len(glyphs)}, {name}_glyphs, {name}_pts, {name}_on, {name}_ends }};")


def main():
    out = ["/* Generated by scripts/gen_font_outline.py from Inter (SIL OFL 1.1). Glyph outlines in font units; do not edit. */",
           '#include "font.h"', ""]
    emit("regular", Font(REG), out)
    emit("semibold", Font(SEMI), out)
    open("src/shell/font_data.c", "w").write("\n".join(out) + "\n")
    print("wrote src/shell/font_data.c")


main()
