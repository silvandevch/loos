#include "shell.h"
#include <string.h>

shell_rect rect_intersect(shell_rect a, shell_rect b)
{
    int x0 = a.x > b.x ? a.x : b.x, y0 = a.y > b.y ? a.y : b.y;
    int x1 = a.x + a.w < b.x + b.w ? a.x + a.w : b.x + b.w, y1 = a.y + a.h < b.y + b.h ? a.y + a.h : b.y + b.h;
    shell_rect r = { x0, y0, x1 - x0, y1 - y0 };
    if (r.w < 0 || r.h < 0) { r.w = r.h = 0; }
    return r;
}
shell_rect rect_union(shell_rect a, shell_rect b)
{
    if (rect_empty(a)) return b;
    if (rect_empty(b)) return a;
    int x0 = a.x < b.x ? a.x : b.x, y0 = a.y < b.y ? a.y : b.y;
    int x1 = a.x + a.w > b.x + b.w ? a.x + a.w : b.x + b.w, y1 = a.y + a.h > b.y + b.h ? a.y + a.h : b.y + b.h;
    shell_rect r = { x0, y0, x1 - x0, y1 - y0 };
    return r;
}
bool rect_overlaps(shell_rect a, shell_rect b) { return !rect_empty(rect_intersect(a, b)); }

/* ---- colors ---- */
static inline uint32_t div255(uint32_t v) { v += 128; return (v + (v >> 8)) >> 8; }      /* exact for 0..65025 */
uint32_t color_blend(uint32_t d, uint32_t s)
{
    uint32_t a = s >> 24;
    if (a == 255) return s | 0xFF000000u;
    if (a == 0) return d;
    uint32_t ia = 255 - a;
    uint32_t r = div255(((s >> 16) & 255) * a + ((d >> 16) & 255) * ia);
    uint32_t g = div255(((s >> 8) & 255) * a + ((d >> 8) & 255) * ia);
    uint32_t b = div255((s & 255) * a + (d & 255) * ia);
    return 0xFF000000u | r << 16 | g << 8 | b;
}
uint32_t color_mix(uint32_t a, uint32_t b, int t)
{
    if (t < 0) t = 0;
    if (t > 256) t = 256;
    uint32_t r = (((a >> 16) & 255) * (256 - t) + ((b >> 16) & 255) * t) >> 8;
    uint32_t g = (((a >> 8) & 255) * (256 - t) + ((b >> 8) & 255) * t) >> 8;
    uint32_t bl = ((a & 255) * (256 - t) + (b & 255) * t) >> 8;
    return 0xFF000000u | r << 16 | g << 8 | bl;
}

#define RGB(r, g, b) (0xFF000000u | (uint32_t)(r) << 16 | (uint32_t)(g) << 8 | (uint32_t)(b))
void shell_theme_dark(shell_theme *t, uint32_t accent)
{
    t->desktop = RGB(0x0B, 0x10, 0x1A); t->surface = RGB(0x12, 0x18, 0x24); t->surface_hi = RGB(0x1C, 0x25, 0x36);
    t->border = RGB(0x2A, 0x35, 0x4A); t->text = RGB(0xF3, 0xF4, 0xF6); t->text_dim = RGB(0x9C, 0xA3, 0xAF);
    t->accent = accent | 0xFF000000u; t->accent_text = RGB(0xFF, 0xFF, 0xFF); t->danger = RGB(0xEF, 0x44, 0x44);
    t->title_inactive = RGB(0x16, 0x1E, 0x2C);
}
void shell_theme_light(shell_theme *t, uint32_t accent)
{
    t->desktop = RGB(0xE5, 0xE9, 0xF0); t->surface = RGB(0xF7, 0xF8, 0xFA); t->surface_hi = RGB(0xFF, 0xFF, 0xFF);
    t->border = RGB(0xC9, 0xCF, 0xDA); t->text = RGB(0x11, 0x18, 0x27); t->text_dim = RGB(0x6B, 0x72, 0x80);
    t->accent = accent | 0xFF000000u; t->accent_text = RGB(0xFF, 0xFF, 0xFF); t->danger = RGB(0xDC, 0x26, 0x26);
    t->title_inactive = RGB(0xEC, 0xEF, 0xF4);
}
void shell_theme_high_contrast(shell_theme *t)
{
    t->desktop = RGB(0, 0, 0); t->surface = RGB(0, 0, 0); t->surface_hi = RGB(0x20, 0x20, 0x20);
    t->border = RGB(0xFF, 0xFF, 0xFF); t->text = RGB(0xFF, 0xFF, 0xFF); t->text_dim = RGB(0xFF, 0xFF, 0x00);
    t->accent = RGB(0x00, 0xFF, 0xFF); t->accent_text = RGB(0, 0, 0); t->danger = RGB(0xFF, 0x40, 0x40);
    t->title_inactive = RGB(0x30, 0x30, 0x30);
}

/* ---- primitives ----
 * Every drawing call takes *logical* coordinates; fb->scale (percent, 0 = 100) maps them to device pixels, so one
 * layout renders crisply at any UI scale. Rect edges are rounded individually (adjacent rects never gap or overlap),
 * strokes keep a constant device thickness, curves and glyphs are anti-aliased at device resolution. The compositor
 * draws into a scale-100 view (device space) and fb_blit is always device space. */
static int fb_sc(const shell_fb *fb) { return fb->scale > 0 ? fb->scale : 100; }
int fb_lw(const shell_fb *fb) { return fb->w * 100 / fb_sc(fb); }
int fb_lh(const shell_fb *fb) { return fb->h * 100 / fb_sc(fb); }

static int fdiv(int64_t a, int64_t b) { int64_t q = a / b; if ((a % b != 0) && ((a < 0) != (b < 0))) q--; return (int)q; }
static int dv(const shell_fb *fb, int v) { int sc = fb_sc(fb); return sc == 100 ? v : fdiv((int64_t)v * sc * 2 + 100, 200); }
static shell_rect drect(const shell_fb *fb, shell_rect r)
{
    if (fb_sc(fb) == 100) return r;
    int x0 = dv(fb, r.x), y0 = dv(fb, r.y), x1 = dv(fb, r.x + r.w), y1 = dv(fb, r.y + r.h);
    return (shell_rect){ x0, y0, x1 - x0, y1 - y0 };
}
static int stroke_px(const shell_fb *fb) { int t = (fb_sc(fb) + 50) / 100; return t < 1 ? 1 : t; }

static shell_rect fb_clip_dev(const shell_fb *fb, shell_rect clip)
{
    shell_rect s = { 0, 0, fb->w, fb->h };
    return rect_intersect(s, drect(fb, clip));
}

static void fill_dev(shell_fb *fb, shell_rect cd, shell_rect r, uint32_t c)
{
    r = rect_intersect(r, cd);
    if (rect_empty(r)) return;
    bool opaque = (c >> 24) == 255;
    for (int y = r.y; y < r.y + r.h; y++) {
        uint32_t *row = fb->px + (size_t)y * (size_t)fb->stride + r.x;
        if (opaque) for (int x = 0; x < r.w; x++) row[x] = c;
        else for (int x = 0; x < r.w; x++) row[x] = color_blend(row[x], c);
    }
}
void fb_fill(shell_fb *fb, shell_rect clip, shell_rect r, uint32_t c) { fill_dev(fb, fb_clip_dev(fb, clip), drect(fb, r), c); }
void fb_hline(shell_fb *fb, shell_rect clip, int x, int y, int len, uint32_t c)
{
    shell_rect r = drect(fb, (shell_rect){ x, y, len, 1 });
    r.h = stroke_px(fb);
    fill_dev(fb, fb_clip_dev(fb, clip), r, c);
}
void fb_vline(shell_fb *fb, shell_rect clip, int x, int y, int len, uint32_t c)
{
    shell_rect r = drect(fb, (shell_rect){ x, y, 1, len });
    r.w = stroke_px(fb);
    fill_dev(fb, fb_clip_dev(fb, clip), r, c);
}
void fb_frame(shell_fb *fb, shell_rect clip, shell_rect r, uint32_t c)
{
    if (rect_empty(r)) return;
    shell_rect d = drect(fb, r), cd = fb_clip_dev(fb, clip);
    int t = stroke_px(fb);
    if (t * 2 > d.w || t * 2 > d.h) { fill_dev(fb, cd, d, c); return; }
    fill_dev(fb, cd, (shell_rect){ d.x, d.y, d.w, t }, c);
    fill_dev(fb, cd, (shell_rect){ d.x, d.y + d.h - t, d.w, t }, c);
    fill_dev(fb, cd, (shell_rect){ d.x, d.y + t, t, d.h - 2 * t }, c);
    fill_dev(fb, cd, (shell_rect){ d.x + d.w - t, d.y + t, t, d.h - 2 * t }, c);
}

/* ---- anti-aliased shapes (device space; distances in 1/16 px) ---- */
static uint32_t isqrt(uint32_t v) { uint32_t r = 0, b = 1u << 30; while (b > v) b >>= 2; while (b) { if (v >= r + b) { v -= r + b; r = (r >> 1) + b; } else r >>= 1; b >>= 2; } return r; }

/* 0..255 coverage of the rounded rect r (device px, radius rad) at pixel (px,py) */
static int rr_cov(shell_rect r, int rad, int px, int py)
{
    if (px < r.x || py < r.y || px >= r.x + r.w || py >= r.y + r.h) {
        /* outside the box: only the 1-px rim can be partially covered, which we ignore (box edges are pixel aligned) */
        return 0;
    }
    int cx, cy;
    if (px < r.x + rad) cx = r.x + rad; else if (px >= r.x + r.w - rad) cx = r.x + r.w - rad; else return 255;
    if (py < r.y + rad) cy = r.y + rad; else if (py >= r.y + r.h - rad) cy = r.y + r.h - rad; else return 255;
    int dx = px * 16 + 8 - cx * 16, dy = py * 16 + 8 - cy * 16;
    int d = (int)isqrt((uint32_t)(dx * dx + dy * dy));                  /* 1/16 px from the corner circle's centre */
    int c = rad * 16 - d + 8;                                          /* +-0.5 px ramp */
    return c <= 0 ? 0 : c >= 16 ? 255 : c * 255 / 16;
}

static void fill_round_dev(shell_fb *fb, shell_rect cd, shell_rect r, int rad, uint32_t c)
{
    if (rect_empty(r)) return;
    if (rad * 2 > r.w) rad = r.w / 2;
    if (rad * 2 > r.h) rad = r.h / 2;
    if (rad <= 0) { fill_dev(fb, cd, r, c); return; }
    shell_rect v = rect_intersect(r, cd);
    uint32_t a0 = c >> 24;
    for (int y = v.y; y < v.y + v.h; y++) {
        uint32_t *row = fb->px + (size_t)y * (size_t)fb->stride;
        bool corner_row = y < r.y + rad || y >= r.y + r.h - rad;
        for (int x = v.x; x < v.x + v.w; x++) {
            int cov = 255;
            if (corner_row && (x < r.x + rad || x >= r.x + r.w - rad)) cov = rr_cov(r, rad, x, y);
            if (cov == 0) continue;
            uint32_t a = cov == 255 ? a0 : a0 * (uint32_t)cov / 255;
            row[x] = a == 255 ? c | 0xFF000000u : color_blend(row[x], (c & 0x00FFFFFFu) | a << 24);
        }
    }
}
void fb_fill_round(shell_fb *fb, shell_rect clip, shell_rect r, int rad, uint32_t c)
{
    fill_round_dev(fb, fb_clip_dev(fb, clip), drect(fb, r), dv(fb, rad), c);
}

/* border ring of a rounded rect: outer coverage minus inner coverage */
void fb_stroke_round(shell_fb *fb, shell_rect clip, shell_rect r, int rad, int thick, uint32_t c)
{
    shell_rect d = drect(fb, r), cd = fb_clip_dev(fb, clip);
    int t = thick * fb_sc(fb) / 100; if (t < 1) t = 1;
    int ro = dv(fb, rad); if (ro * 2 > d.w) ro = d.w / 2; if (ro * 2 > d.h) ro = d.h / 2;
    shell_rect in = { d.x + t, d.y + t, d.w - 2 * t, d.h - 2 * t };
    int ri = ro > t ? ro - t : 0;
    shell_rect v = rect_intersect(d, cd);
    uint32_t a0 = c >> 24;
    for (int y = v.y; y < v.y + v.h; y++) {
        uint32_t *row = fb->px + (size_t)y * (size_t)fb->stride;
        bool band_y = y < d.y + t + ro || y >= d.y + d.h - t - ro;
        for (int x = v.x; x < v.x + v.w; x++) {
            if (!band_y && x >= d.x + t + ro && x < d.x + d.w - t - ro) { x = d.x + d.w - t - ro - 1; continue; }
            int co = rr_cov(d, ro, x, y), ci = (in.w > 0 && in.h > 0) ? rr_cov(in, ri, x, y) : 0;
            int cov = co - ci; if (cov <= 0) continue;
            uint32_t a = a0 * (uint32_t)cov / 255;
            row[x] = color_blend(row[x], (c & 0x00FFFFFFu) | a << 24);
        }
    }
}

void fb_fill_circle(shell_fb *fb, shell_rect clip, int cx, int cy, int rad, uint32_t c)
{
    fb_fill_round(fb, clip, (shell_rect){ cx - rad, cy - rad, rad * 2, rad * 2 }, rad, c);
}

/* soft drop shadow around a rounded rect: quadratic falloff of the signed distance, never painted under the rect itself */
void fb_shadow(shell_fb *fb, shell_rect clip, shell_rect r, int rad, int blur, int dy, uint32_t c)
{
    shell_rect d = drect(fb, r), cd = fb_clip_dev(fb, clip);
    int b = dv(fb, blur); if (b < 1) b = 1;
    int off = dv(fb, dy), rd = dv(fb, rad);
    shell_rect box = { d.x, d.y + off, d.w, d.h };
    shell_rect area = rect_intersect((shell_rect){ box.x - b, box.y - b, box.w + 2 * b, box.h + 2 * b }, cd);
    int hx = box.w * 8, hy = box.h * 8;                                 /* half extents in 1/16 px */
    int ccx = box.x * 16 + hx, ccy = box.y * 16 + hy;
    uint32_t a0 = c >> 24;
    for (int y = area.y; y < area.y + area.h; y++) {
        uint32_t *row = fb->px + (size_t)y * (size_t)fb->stride;
        for (int x = area.x; x < area.x + area.w; x++) {
            int qx = (x * 16 + 8 > ccx ? x * 16 + 8 - ccx : ccx - x * 16 - 8) - (hx - rd * 16);
            int qy = (y * 16 + 8 > ccy ? y * 16 + 8 - ccy : ccy - y * 16 - 8) - (hy - rd * 16);
            int mx = qx > 0 ? qx : 0, my = qy > 0 ? qy : 0;
            int dist;
            if (!mx && !my) dist = 0;                                            /* inside the (offset) box: full strength */
            else dist = (!mx || !my ? mx + my : (int)isqrt((uint32_t)(mx * mx + my * my))) - rd * 16;   /* sqrt only at the corners */
            if (dist >= b * 16) continue;
            if (rr_cov(d, rd, x, y) > 200) continue;                    /* under the shadow-casting shape itself: nothing to paint */
            if (dist < 0) dist = 0;                                     /* inside the offset box but outside the shape: full strength */
            int64_t t = (int64_t)(b * 16 - dist);                       /* 0..b*16 */
            uint32_t a = (uint32_t)((int64_t)a0 * t * t / ((int64_t)b * 16 * b * 16));
            if (!a) continue;
            row[x] = color_blend(row[x], (c & 0x00FFFFFFu) | a << 24);
        }
    }
}

void fb_gradient_v(shell_fb *fb, shell_rect clip, shell_rect r, uint32_t top, uint32_t bottom)
{
    shell_rect d = drect(fb, r), cd = fb_clip_dev(fb, clip);
    shell_rect v = rect_intersect(d, cd);
    for (int y = v.y; y < v.y + v.h; y++) {
        uint32_t c = color_mix(top, bottom, d.h > 1 ? (y - d.y) * 256 / (d.h - 1) : 0);
        fill_dev(fb, cd, (shell_rect){ v.x, y, v.w, 1 }, c);
    }
}

void fb_blit(shell_fb *fb, shell_rect clip, int dx, int dy, const uint32_t *src, int sw, int sh, int sstride, shell_rect sr)
{
    shell_rect full = { 0, 0, sw, sh };
    sr = rect_intersect(sr, full);
    shell_rect dst = { dx, dy, sr.w, sr.h };
    shell_rect vis = rect_intersect(dst, rect_intersect((shell_rect){ 0, 0, fb->w, fb->h }, clip));   /* device space */
    if (rect_empty(vis)) return;
    for (int y = vis.y; y < vis.y + vis.h; y++) {
        const uint32_t *s = src + (size_t)(sr.y + (y - dy)) * (size_t)sstride + sr.x + (vis.x - dx);
        uint32_t *d = fb->px + (size_t)y * (size_t)fb->stride + vis.x;
        for (int x = 0; x < vis.w; x++) d[x] = (s[x] >> 24) == 255 ? s[x] : color_blend(d[x], s[x]);
    }
}

/* fb_blit whose pixels are additionally masked by the rounded rect `round` (device px, radius rad): used so a client
 * surface does not poke out of its window's rounded bottom corners */
void fb_blit_rounded(shell_fb *fb, shell_rect clip, int dx, int dy, const uint32_t *src, int sw, int sh, int sstride, shell_rect sr,
                     shell_rect round, int rad)
{
    shell_rect full = { 0, 0, sw, sh };
    sr = rect_intersect(sr, full);
    shell_rect vis = rect_intersect((shell_rect){ dx, dy, sr.w, sr.h }, rect_intersect((shell_rect){ 0, 0, fb->w, fb->h }, clip));
    if (rect_empty(vis)) return;
    for (int y = vis.y; y < vis.y + vis.h; y++) {
        const uint32_t *s = src + (size_t)(sr.y + (y - dy)) * (size_t)sstride + sr.x + (vis.x - dx);
        uint32_t *d = fb->px + (size_t)y * (size_t)fb->stride + vis.x;
        bool corner_row = y >= round.y + round.h - rad;
        for (int x = 0; x < vis.w; x++) {
            uint32_t p = s[x];
            if (corner_row && rad > 0 && (vis.x + x < round.x + rad || vis.x + x >= round.x + round.w - rad)) {
                int cov = rr_cov(round, rad, vis.x + x, y);
                if (cov < 255) p = (p & 0x00FFFFFFu) | ((p >> 24) * (uint32_t)cov / 255) << 24;
            }
            d[x] = (p >> 24) == 255 ? p : color_blend(d[x], p);
        }
    }
}

/* soft radial glow: colour c at `strength` (0..255) fading quadratically to nothing at `radius` (logical px); ordered-dithered
 * so large faint gradients do not band. Blends over what is already there. */
void fb_radial_glow(shell_fb *fb, shell_rect clip, int cx, int cy, int radius, uint32_t c, int strength)
{
    static const uint8_t bayer[16] = { 0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5 };
    shell_rect cd = fb_clip_dev(fb, clip);
    int dx0 = dv(fb, cx), dy0 = dv(fb, cy), dr = dv(fb, radius);
    if (dr < 1) return;
    shell_rect box = rect_intersect((shell_rect){ dx0 - dr, dy0 - dr, 2 * dr, 2 * dr }, cd);
    int64_t r2 = (int64_t)dr * dr;
    int64_t inv = (256ll << 24) / r2;                                   /* q = d2 * inv >> 24 (no per-pixel division) */
    for (int y = box.y; y < box.y + box.h; y++) {
        int64_t dy = y - dy0;
        int64_t rem = r2 - dy * dy;
        if (rem <= 0) continue;
        int xr = (int)isqrt((uint32_t)(rem > 0x7fffffff ? 0x7fffffff : rem));     /* only the part of the row inside the circle */
        int x0 = dx0 - xr > box.x ? dx0 - xr : box.x, x1 = dx0 + xr + 1 < box.x + box.w ? dx0 + xr + 1 : box.x + box.w;
        uint32_t *row = fb->px + (size_t)y * (size_t)fb->stride;
        for (int x = x0; x < x1; x++) {
            int64_t dx = x - dx0;
            int64_t q = ((dx * dx + dy * dy) * inv) >> 24;
            if (q >= 256) continue;
            int a = (int)(((256 - q) * (256 - q) * strength) >> 8);             /* 8.8 alpha */
            int a8 = (a + bayer[(y & 3) * 4 + (x & 3)] * 16 + 8) >> 8;
            if (a8 <= 0) continue;
            if (a8 > 255) a8 = 255;
            row[x] = color_blend(row[x], (c & 0x00FFFFFFu) | (uint32_t)a8 << 24);
        }
    }
}

/* ---- text ---- */
int shell_text_width_ex(const char *s, shell_font f) { return font_text_width(f, s); }
int shell_text_width(const char *s) { return font_text_width(SHELL_FONT_BODY, s); }
int shell_text_height(void) { return 16; }
void shell_ellipsize(char *out, size_t cap, const char *s, int max_px)
{
    if (!cap) return;
    if (font_text_width(SHELL_FONT_BODY, s) <= max_px) { strncpy(out, s, cap - 1); out[cap - 1] = 0; return; }
    static const char dots[] = "\xe2\x80\xa6";
    int room = max_px - font_text_width(SHELL_FONT_BODY, dots);
    int n = room > 0 ? font_fit(SHELL_FONT_BODY, s, room) : 0;
    if ((size_t)n + sizeof dots > cap) n = cap > sizeof dots ? (int)(cap - sizeof dots) : 0;
    memcpy(out, s, (size_t)n);
    memcpy(out + n, dots, sizeof dots);
}
int shell_text_fit(const char *s, int max_px) { return font_fit(SHELL_FONT_BODY, s, max_px); }
int shell_text_index_at(const char *s, int x) { return font_index_at(SHELL_FONT_BODY, s, x); }

int fb_text_ex(shell_fb *fb, shell_rect clip, int x, int y, const char *s, uint32_t c, shell_font f)
{
    int sc = fb_sc(fb);
    shell_rect cd = fb_clip_dev(fb, clip);
    int size_q4 = (int)(((int64_t)f.size * 16 * sc + 50) / 100);
    int64_t pen = (int64_t)x * 256 * sc / 100;                         /* device pen position, 1/256 px */
    int64_t logical_q8 = 0;
    int baseline = dv(fb, y) + (int)(((int64_t)f.size * sc * 86 + 5000) / 10000);
    uint32_t a0 = c >> 24;
    for (uint32_t cp; (cp = font_next_cp(&s)) != 0;) {
        const loos_glyph *g = font_find(f.face, cp);
        if (!g) g = font_find(f.face, 0xfffd);
        if (!g) g = font_find(f.face, '?');
        if (!g) continue;
        int adv = font_advance_q8(f, cp);
        if (g->ncont) {
            const font_bitmap *b = font_bitmap_get(f.face, g, size_q4, (int)((pen >> 6) & 3));
            if (b && b->cov) {
                int gx = (int)(pen >> 8) + b->left, gy = baseline + b->top;
                shell_rect gr = rect_intersect((shell_rect){ gx, gy, b->w, b->h }, cd);
                for (int yy = gr.y; yy < gr.y + gr.h; yy++) {
                    const uint8_t *src = b->cov + (size_t)(yy - gy) * (size_t)b->w + (gr.x - gx);
                    uint32_t *dst = fb->px + (size_t)yy * (size_t)fb->stride + gr.x;
                    for (int xx = 0; xx < gr.w; xx++) {
                        uint32_t cov = src[xx];
                        if (!cov) continue;
                        uint32_t a = a0 == 255 ? cov : cov * a0 / 255;
                        dst[xx] = a == 255 ? (c | 0xFF000000u) : color_blend(dst[xx], (c & 0x00FFFFFFu) | a << 24);
                    }
                }
            }
        }
        pen += (int64_t)adv * sc / 100;
        logical_q8 += adv;
    }
    return (int)((logical_q8 + 128) >> 8);
}
int fb_text(shell_fb *fb, shell_rect clip, int x, int y, const char *s, uint32_t c) { return fb_text_ex(fb, clip, x, y, s, c, SHELL_FONT_BODY); }

/* ---- damage ---- */
void damage_clear(shell_damage *d) { d->n = 0; }
shell_rect damage_bounds(const shell_damage *d)
{
    shell_rect b = { 0, 0, 0, 0 };
    for (int i = 0; i < d->n; i++) b = rect_union(b, d->r[i]);
    return b;
}
void damage_add(shell_damage *d, shell_rect r)
{
    if (rect_empty(r)) return;
    for (int i = 0; i < d->n; i++) {
        if (rect_overlaps(d->r[i], r) || (d->r[i].x == r.x + r.w && d->r[i].y == r.y && d->r[i].h == r.h)) {
            r = rect_union(d->r[i], r);
            d->r[i] = d->r[--d->n];
            i = -1;                                               /* restart: the merged rect may touch others */
        }
    }
    if (d->n == SHELL_DAMAGE_MAX) { shell_rect b = rect_union(damage_bounds(d), r); d->n = 0; d->r[d->n++] = b; return; }
    d->r[d->n++] = r;
}
