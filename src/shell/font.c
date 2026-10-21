#include "font.h"
#include "raster.h"
#include <string.h>

/* ---------------------------------------------------------------- UTF-8 and glyph lookup */
uint32_t font_next_cp(const char **sp)
{
    const unsigned char *s = (const unsigned char *)*sp;
    uint32_t c = s[0];
    if (c == 0) return 0;
    int n = 0; uint32_t cp = 0xfffd;
    if (c < 0x80) { cp = c; n = 1; }
    else if (c >= 0xc2 && c < 0xe0 && (s[1] & 0xc0) == 0x80) { cp = (c & 0x1f) << 6 | (s[1] & 0x3f); n = 2; }
    else if (c >= 0xe0 && c < 0xf0 && (s[1] & 0xc0) == 0x80 && (s[2] & 0xc0) == 0x80) {
        cp = (c & 0x0f) << 12 | (s[1] & 0x3f) << 6 | (s[2] & 0x3f); n = 3;
        if (cp < 0x800 || (cp >= 0xd800 && cp < 0xe000)) { cp = 0xfffd; }
    } else n = 1;
    *sp += n;
    return cp;
}

const loos_glyph *font_find(const loos_font *f, uint32_t cp)
{
    int lo = 0, hi = f->nglyphs - 1;
    while (lo <= hi) {
        int mid = (lo + hi) / 2;
        if (f->glyphs[mid].cp == cp) return &f->glyphs[mid];
        if (f->glyphs[mid].cp < cp) lo = mid + 1; else hi = mid - 1;
    }
    return 0;
}

static const loos_glyph *glyph_or_fallback(const loos_font *f, uint32_t cp)
{
    const loos_glyph *g = font_find(f, cp);
    if (!g) g = font_find(f, 0xfffd);
    if (!g) g = font_find(f, '?');
    return g;
}

int font_advance_q8(shell_font f, uint32_t cp)
{
    const loos_glyph *g = glyph_or_fallback(f.face, cp);
    return g ? (int)(((int64_t)g->adv * f.size * 256 + f.face->upm / 2) / f.face->upm) : 0;
}

int font_text_width(shell_font f, const char *s)
{
    int64_t q8 = 0;
    for (uint32_t cp; (cp = font_next_cp(&s)) != 0;) q8 += font_advance_q8(f, cp);
    return (int)((q8 + 128) >> 8);
}

int font_fit(shell_font f, const char *s, int max_px)
{
    int64_t q8 = 0; const char *p = s;
    for (;;) {
        const char *q = p;
        uint32_t cp = font_next_cp(&q);
        if (!cp) break;
        q8 += font_advance_q8(f, cp);
        if (((q8 + 128) >> 8) > max_px) break;
        p = q;
    }
    return (int)(p - s);
}

int font_index_at(shell_font f, const char *s, int x_px)
{
    int64_t q8 = 0; const char *p = s;
    for (;;) {
        const char *q = p;
        uint32_t cp = font_next_cp(&q);
        if (!cp) break;
        int a = font_advance_q8(f, cp);
        if (q8 + a / 2 > (int64_t)x_px * 256) break;               /* caret goes before this glyph when x is in its left half */
        q8 += a;
        p = q;
    }
    return (int)(p - s);
}

/* ---------------------------------------------------------------- glyph bitmap cache */
#define CACHE_BYTES (384 * 1024)
#define CACHE_SLOTS 2048
typedef struct { const loos_font *face; uint16_t gi; uint16_t size_q4; uint8_t subx; uint8_t used; font_bitmap bm; } slot;
static uint8_t pool[CACHE_BYTES];
static size_t pool_used;
static slot slots[CACHE_SLOTS];

void font_cache_flush(void) { pool_used = 0; memset(slots, 0, sizeof slots); }

/* text contrast: light-on-dark and dark-on-light glyphs both read better with a mild coverage boost */
static uint8_t boost[256];
static int boost_ready;
static void init_boost(void)
{
    /* cov' = 255 * (cov/255)^0.8, approximated by a smooth integer curve */
    for (int i = 0; i < 256; i++) {
        int x = i * 256 / 255;                                              /* 0..256 */
        int y = x + ((x * (256 - x)) >> 9) * 3 / 4;                         /* lift mid-tones by up to ~19% */
        if (y > 256) y = 256;
        boost[i] = (uint8_t)(y * 255 / 256);
    }
    boost_ready = 1;
}

const font_bitmap *font_bitmap_get(const loos_font *f, const loos_glyph *g, int size_q4, int subx)
{
    if (!boost_ready) init_boost();
    uint32_t gi = (uint32_t)(g - f->glyphs);
    uint32_t h = (gi * 2654435761u + (uint32_t)size_q4 * 40503u + (uint32_t)subx * 977u + (f == &loos_font_semibold ? 7919u : 0)) % CACHE_SLOTS;
    for (uint32_t probe = 0; probe < 8; probe++) {
        slot *s = &slots[(h + probe) % CACHE_SLOTS];
        if (s->used && s->face == f && s->gi == gi && s->size_q4 == size_q4 && s->subx == subx) return &s->bm;
        if (s->used) continue;
        /* render */
        int64_t k_q24 = ((int64_t)size_q4 << 24) / ((int64_t)f->upm * 16);          /* px per unit * 2^24 */
        raster_path path = { f->pts + 2 * g->pt0, f->on + g->pt0, 0, (int)g->ncont };
        static uint16_t rel_ends[256];
        if (g->ncont > 256) return 0;
        for (int c = 0; c < g->ncont; c++) rel_ends[c] = (uint16_t)(f->ends[g->end0 + c] - g->pt0);
        path.ends = rel_ends;
        /* bounding box in device px (with a 1 px margin for the filter) */
        int xmin = 32767, ymin = 32767, xmax = -32768, ymax = -32768;
        int npts = g->ncont ? rel_ends[g->ncont - 1] : 0;
        for (int i = 0; i < npts; i++) {
            int x = path.xy[2 * i], y = path.xy[2 * i + 1];
            if (x < xmin) xmin = x; if (x > xmax) xmax = x; if (y < ymin) ymin = y; if (y > ymax) ymax = y;
        }
        font_bitmap bm = { 0, 0, 0, 0, 0 };
        if (npts) {
            int64_t k = k_q24 >> 8;                                                 /* px per unit * 2^16 */
            int left = (int)(((int64_t)xmin * k) >> 16) - 1, right = (int)(((int64_t)xmax * k) >> 16) + 2;
            int top = -(int)(((int64_t)ymax * k) >> 16) - 1, bottom = -(int)(((int64_t)ymin * k) >> 16) + 2;
            bm.left = left; bm.top = top; bm.w = right - left + 1; bm.h = bottom - top + 1;
            if (bm.w > RASTER_MAX_SIZE || bm.h > RASTER_MAX_SIZE || bm.w <= 0 || bm.h <= 0) return 0;
            if (pool_used + (size_t)bm.w * bm.h > CACHE_BYTES) { font_cache_flush(); return font_bitmap_get(f, g, size_q4, subx); }
            uint8_t *cov = pool + pool_used;
            int ox = -left * 256 + subx * 64, oy = -top * 256;
            if (raster_fill(&path, (int32_t)k_q24, ox, oy, bm.w, bm.h, cov) != 0) return 0;
            for (int i = 0; i < bm.w * bm.h; i++) cov[i] = boost[cov[i]];
            bm.cov = cov;
            pool_used += (size_t)bm.w * bm.h;
        }
        s->used = 1; s->face = f; s->gi = (uint16_t)gi; s->size_q4 = (uint16_t)size_q4; s->subx = (uint8_t)subx; s->bm = bm;
        return &s->bm;
    }
    font_cache_flush();
    return font_bitmap_get(f, g, size_q4, subx);
}
