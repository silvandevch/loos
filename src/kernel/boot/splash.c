/* Boot screen: wordmark, tagline and a smooth progress bar over a soft gradient. Drawn with the shell's own renderer so it
 * is anti-aliased and resolution independent. */
#include "kernel.h"
#include "shell.h"
#include <string.h>

static struct video *V;
static shell_fb fb;
static int cur_q8;                                   /* displayed progress, 0..100*256 */
static char label[48];

#define BAR_W 320
#define BAR_H 5

static shell_rect lscreen(void) { return (shell_rect){ 0, 0, fb_lw(&fb), fb_lh(&fb) }; }
static shell_rect bar_rect(void) { shell_rect s = lscreen(); return (shell_rect){ s.w / 2 - BAR_W / 2, s.h / 2 + 70, BAR_W, BAR_H }; }
static shell_rect progress_area(void) { shell_rect b = bar_rect(); return (shell_rect){ b.x - 60, b.y - 20, b.w + 120, 70 }; }

static void background(shell_rect clip)
{
    shell_rect s = lscreen();
    fb_gradient_v(&fb, clip, s, 0xFF0B101Au, 0xFF05080Fu);
    fb_radial_glow(&fb, clip, s.w * 78 / 100, s.h * 20 / 100, s.w * 55 / 100, 0xFF3B82F6u, 70);
    fb_radial_glow(&fb, clip, s.w * 12 / 100, s.h * 95 / 100, s.w * 45 / 100, 0xFF7C3AEDu, 46);
}

static void wordmark(shell_rect clip)
{
    shell_rect s = lscreen();
    shell_font big = { &loos_font_semibold, 72 }, small = { &loos_font_regular, 16 };
    int w = shell_text_width_ex("LoOS", big);
    fb_text_ex(&fb, clip, s.w / 2 - w / 2, s.h / 2 - 96, "LoOS", 0xFFF3F4F6u, big);
    const char *tag = "Local only.";
    fb_text_ex(&fb, clip, s.w / 2 - shell_text_width_ex(tag, small) / 2, s.h / 2 - 8, tag, 0xFF9CA3AFu, small);
    fb_fill_circle(&fb, clip, s.w / 2 + w / 2 + 7, s.h / 2 - 96 + 56, 5, 0xFF3B82F6u);       /* the accent dot after the wordmark */
}

static void progress(shell_rect clip)
{
    shell_rect b = bar_rect();
    shell_font small = { &loos_font_regular, 13 };
    fb_fill_round(&fb, clip, b, BAR_H / 2 + 1, 0x30FFFFFFu);
    int fill = b.w * cur_q8 / (100 * 256);
    if (fill > 0) {
        shell_rect f = { b.x, b.y, fill < BAR_H ? BAR_H : fill, b.h };
        fb_radial_glow(&fb, clip, f.x + f.w, b.y + b.h / 2, 26, 0xFF3B82F6u, 90);
        fb_fill_round(&fb, clip, f, BAR_H / 2 + 1, 0xFF60A5FAu);
    }
    if (label[0]) fb_text_ex(&fb, clip, b.x + b.w / 2 - shell_text_width_ex(label, small) / 2, b.y + 20, label, 0xFF6B7280u, small);
}

static void present_logical(shell_rect r)
{
    int sc = fb.scale;
    video_present(V, r.x * sc / 100 - 2, r.y * sc / 100 - 2, (r.w * sc + 99) / 100 + 4, (r.h * sc + 99) / 100 + 4);
}

void splash_start(struct video *v, int scale_pct)
{
    V = v;
    fb = (shell_fb){ v->back, v->w, v->h, v->w, scale_pct };
    cur_q8 = 0; label[0] = 0;
    shell_rect all = lscreen();
    background(all); wordmark(all); progress(all);
    video_present(v, 0, 0, v->w, v->h);
}

/* ease the bar toward `percent` over ~150 ms */
void splash_progress(int percent, const char *step)
{
    if (!V) return;
    int target = percent * 256;
    strncpy(label, step ? step : "", sizeof label - 1); label[sizeof label - 1] = 0;
    uint32_t t0 = time_ms();
    int start = cur_q8;
    shell_rect area = progress_area();
    for (;;) {
        uint32_t el = time_ms() - t0;
        int t = el >= 150 ? 256 : (int)(el * 256 / 150);
        int e = 256 - ((256 - t) * (256 - t) >> 8);                      /* ease-out */
        cur_q8 = start + (target - start) * e / 256;
        background(area); wordmark(area); progress(area);
        present_logical(area);
        if (t >= 256) break;
    }
}
