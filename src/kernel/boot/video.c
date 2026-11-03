#include "kernel.h"
#include <string.h>

int video_init(struct video *v, const struct loos_bootinfo *bi)
{
    if (!bi->fb_phys || bi->fb_bpp != 32 || bi->fb_width < 640 || bi->fb_height < 400 || bi->fb_width > 8192 || bi->fb_height > 8192) return -1;
    v->screen = (volatile uint8_t *)(uintptr_t)bi->fb_phys;
    v->pitch = bi->fb_pitch;
    v->w = (int)bi->fb_width; v->h = (int)bi->fb_height;
    v->rgbx = bi->fb_format == LOOS_FB_RGBX;
    v->back = kalloc((uint64_t)v->w * (uint64_t)v->h * 4, 4096);
    return v->back ? 0 : -1;
}

extern int32_t loos_comp_present(const uint32_t *back, int32_t bw, int32_t bh, int32_t x, int32_t y, int32_t w, int32_t h, volatile uint8_t *screen, uint32_t pitch);

/* The desktop goes through the Rust compositor when the scan-out is BGRX and there is memory for its surface. */
static int comp_state;                                  /* 0 unknown, 1 on, -1 off */

void video_present(const struct video *v, int x, int y, int w, int h)
{
    if (comp_state == 0) comp_state = (!v->rgbx && kalloc_free() > (uint64_t)v->w * v->h * 4 + (4u << 20)) ? 1 : -1;
    if (x < 0) { w += x; x = 0; }
    if (y < 0) { h += y; y = 0; }
    if (x + w > v->w) w = v->w - x;
    if (y + h > v->h) h = v->h - y;
    if (w <= 0 || h <= 0) return;
    if (comp_state > 0 && loos_comp_present(v->back, v->w, v->h, x, y, w, h, v->screen, v->pitch) == 0) return;
    for (int yy = y; yy < y + h; yy++) {
        const uint32_t *src = v->back + (uint64_t)yy * (uint64_t)v->w + x;
        volatile uint32_t *dst = (volatile uint32_t *)(v->screen + (uint64_t)yy * v->pitch) + x;
        if (!v->rgbx) {
            uint64_t n = (uint64_t)w; const uint32_t *s = src; volatile uint32_t *d = dst;
            while (n--) *d++ = *s++ & 0x00ffffffu;
        } else {
            for (int i = 0; i < w; i++) { uint32_t p = src[i]; dst[i] = (p & 0x0000ff00u) | (p >> 16 & 0xffu) | (p & 0xffu) << 16; }
        }
    }
}

/* cross-fade: frame = from * (1-t) + back * t, written straight to the screen */
void video_fade(const struct video *v, const uint32_t *from, uint32_t ms)
{
    uint32_t t0 = time_ms();
    for (;;) {
        uint32_t el = time_ms() - t0;
        uint32_t t = el >= ms ? 256 : el * 256 / ms;
        for (int y = 0; y < v->h; y++) {
            const uint32_t *a = from + (uint64_t)y * (uint64_t)v->w, *b = v->back + (uint64_t)y * (uint64_t)v->w;
            volatile uint32_t *dst = (volatile uint32_t *)(v->screen + (uint64_t)y * v->pitch);
            for (int x = 0; x < v->w; x++) {
                uint32_t p = a[x], q = b[x];
                uint32_t r = (((p >> 16 & 255) * (256 - t) + (q >> 16 & 255) * t) >> 8);
                uint32_t g = (((p >> 8 & 255) * (256 - t) + (q >> 8 & 255) * t) >> 8);
                uint32_t bl = (((p & 255) * (256 - t) + (q & 255) * t) >> 8);
                dst[x] = v->rgbx ? (bl << 16 | g << 8 | r) : (r << 16 | g << 8 | bl);
            }
        }
        if (t >= 256) break;
    }
}
