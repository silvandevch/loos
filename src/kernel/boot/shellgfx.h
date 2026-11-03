/* C view of the Rust shell's drawing primitives (crates/loos-shell/src/ffi.rs, font.rs): what the boot splash, boot menu and
 * bugcheck screen draw with. Colors are 0xAARRGGBB; sizes are logical px scaled by shell_fb.scale. */
#ifndef LOOS_SHELLGFX_H
#define LOOS_SHELLGFX_H
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

typedef struct { int x, y, w, h; } shell_rect;
typedef struct { uint32_t *px; int w, h, stride; int scale; } shell_fb;   /* w/h/stride in device px; scale = percent (0 = 100) */

typedef struct { uint16_t cp, adv; uint16_t pt0, end0, ncont; } loos_glyph;
#include <stddef.h>


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


int  fb_lw(const shell_fb *fb);
int  fb_lh(const shell_fb *fb);
void fb_fill(shell_fb *fb, shell_rect clip, shell_rect r, uint32_t c);
void fb_fill_round(shell_fb *fb, shell_rect clip, shell_rect r, int radius, uint32_t c);
void fb_fill_circle(shell_fb *fb, shell_rect clip, int cx, int cy, int radius, uint32_t c);
void fb_radial_glow(shell_fb *fb, shell_rect clip, int cx, int cy, int radius, uint32_t color, int strength);
void fb_gradient_v(shell_fb *fb, shell_rect clip, shell_rect r, uint32_t top, uint32_t bottom);
int  fb_text_ex(shell_fb *fb, shell_rect clip, int x, int y, const char *s, uint32_t c, shell_font f);   /* y = top of the em box */
int  shell_text_width_ex(const char *s, shell_font f);
#endif
