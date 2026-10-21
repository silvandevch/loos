/* Outline font engine: glyph outlines (src/shell/font_data.c) are rasterised on demand at any size into 8-bit
 * coverage bitmaps with exact-area anti-aliasing and cached, so text is smooth at every resolution and UI scale. */
#ifndef LOOS_SHELL_FONT_H
#define LOOS_SHELL_FONT_H
#include <stdint.h>
#include <stddef.h>

typedef struct { uint16_t cp, adv; uint16_t pt0, end0, ncont; } loos_glyph;     /* adv in font units; pt0/end0 index the tables */

typedef struct {
    uint16_t upm; int16_t ascent, descent;
    uint16_t nglyphs;
    const loos_glyph *glyphs;
    const int16_t *pts;          /* x,y pairs, y up */
    const uint8_t *on;           /* on-curve flag per point */
    const uint16_t *ends;        /* exclusive end index (into pts/on) of every contour */
} loos_font;

extern const loos_font loos_font_regular, loos_font_semibold;

typedef struct { const loos_font *face; int size; } shell_font;            /* size = logical px (em height) */
#define SHELL_FONT_BODY   ((shell_font){ &loos_font_regular, 14 })
#define SHELL_FONT_BOLD   ((shell_font){ &loos_font_semibold, 14 })

/* UTF-8 decoding: returns the code point and advances *s; invalid bytes decode as U+FFFD */
uint32_t font_next_cp(const char **s);
const loos_glyph *font_find(const loos_font *f, uint32_t cp);                   /* NULL if absent */
/* Horizontal advance in 1/256 logical px */
int font_advance_q8(shell_font f, uint32_t cp);
int font_text_width(shell_font f, const char *s);                               /* logical px, rounded */
int font_fit(shell_font f, const char *s, int max_px);                          /* bytes of s that fit within max_px */
int font_index_at(shell_font f, const char *s, int x_px);                       /* byte offset of the caret position nearest x_px */

typedef struct {
    int left, top;               /* offset of the bitmap's top-left from the pen origin / baseline, device px */
    int w, h;
    const uint8_t *cov;          /* w*h coverage, valid until the next cache flush */
} font_bitmap;
/* size_q4: em height in 1/16 device px; subx: 0..3 quarter-pixel horizontal phase */
const font_bitmap *font_bitmap_get(const loos_font *f, const loos_glyph *g, int size_q4, int subx);
void font_cache_flush(void);
#endif
