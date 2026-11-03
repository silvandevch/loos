/* Startup menu: F8 during boot, "bootmenu" on the command line, or automatically after a crash. */
#include "kernel.h"
#include "shellgfx.h"
#include <string.h>

static const char *items[] = { "Start LoOS normally", "Safe mode (one CPU, basic display)", "Restart" };
#define N_ITEMS 3

static void draw(struct video *v, int sel, uint32_t crash, int secs)
{
    shell_fb fb = { v->back, v->w, v->h, v->w, v->w >= 2560 ? 150 : 100 };
    shell_rect s = { 0, 0, fb_lw(&fb), fb_lh(&fb) };
    shell_font title = { &loos_font_semibold, 30 }, body = { &loos_font_regular, 18 }, small = { &loos_font_regular, 14 };
    fb_gradient_v(&fb, s, s, 0xFF0B101Au, 0xFF05080Fu);
    int x = s.w / 2 - 260, y = s.h / 2 - 170;
    fb_text_ex(&fb, s, x, y, "Choose how to start LoOS", 0xFFF3F4F6u, title);
    if (crash) {
        char name[64], line[120] = "The last session stopped: ";
        size_t n = strlen(line);
        loos_kbugcheck_name(crash, name, sizeof name);
        strncpy(line + n, name, sizeof line - n - 1);
        fb_text_ex(&fb, s, x, y + 44, line, 0xFFFCA5A5u, small);
    }
    for (int i = 0; i < N_ITEMS; i++) {
        shell_rect r = { x - 12, y + 80 + i * 48, 544, 40 };
        if (i == sel) fb_fill_round(&fb, s, r, 8, 0xFF2563EBu);
        else fb_fill_round(&fb, s, r, 8, 0x22FFFFFFu);
        fb_text_ex(&fb, s, x + 6, r.y + 10, items[i], 0xFFFFFFFFu, body);
    }
    char hint[96] = "Up/Down to choose, Enter to start";
    if (secs >= 0) { size_t n = strlen(hint); memcpy(hint + n, " - automatic in  s", 19); hint[n + 16] = (char)('0' + secs % 10); }
    fb_text_ex(&fb, s, x, y + 80 + N_ITEMS * 48 + 16, hint, 0xFF9CA3AFu, small);
    video_present(v, 0, 0, v->w, v->h);
}

static int f8_held(uint32_t ms)
{
    struct kin k;
    for (uint32_t t = time_ms(); time_ms() - t < ms; )
        while (ps2_poll(&k)) if (k.type == KIN_KEY && k.down && (k.key == 0x127 || k.key == 27)) return 1;
    return 0;
}

/* Returns 0 normal, 1 safe mode. `wait_ms` = F8 window; `force` shows it regardless; `auto_pick` >= 0 answers it (tests). */
int bootmenu_run(struct video *v, int force, uint32_t wait_ms, uint32_t crash, int auto_pick)
{
    if (!v) return 0;
    if (!force && !crash && !f8_held(wait_ms)) return 0;
    int sel = crash ? 1 : 0, secs = 9;
    if (auto_pick >= 0) { draw(v, auto_pick, crash, -1); serial_puts("bootmenu: picked "); serial_puts(items[auto_pick]); serial_puts("\n"); return auto_pick == 1; }
    uint32_t t0 = time_ms();
    draw(v, sel, crash, secs);
    for (;;) {
        struct kin k;
        while (ps2_poll(&k)) {
            if (k.type != KIN_KEY || !k.down) continue;
            secs = -1;
            if (k.key == 0x100) sel = (sel + N_ITEMS - 1) % N_ITEMS;
            else if (k.key == 0x101) sel = (sel + 1) % N_ITEMS;
            else if (k.key >= '1' && k.key < '1' + N_ITEMS) sel = (int)(k.key - '1');
            else if (k.key == 13) goto chosen;
            draw(v, sel, crash, secs);
        }
        if (secs >= 0) {
            int left = 9 - (int)((time_ms() - t0) / 1000);
            if (left < 0) goto chosen;
            if (left != secs) { secs = left; draw(v, sel, crash, secs); }
        }
        __asm__ volatile("pause");
    }
chosen:
    serial_puts("bootmenu: "); serial_puts(items[sel]); serial_puts("\n");
    if (sel == 2) { outb(0x64, 0xFE); for (;;) __asm__ volatile("hlt"); }
    return sel == 1;
}
