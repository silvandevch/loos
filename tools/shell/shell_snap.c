/* Host snapshot tool: runs the real desktop (src/shell/desktop.c) at any resolution/scale, optionally replays a few input
 * steps, and writes the frame as a PPM. Used for visual review and by the shell.snapshot ctest.
 *   shell_snap OUT.ppm [WxH] [scale%] [steps]
 * steps: comma separated: super, type:TEXT, key:enter|esc|tab, move:X,Y (device px), click:X,Y, win:N (settings page) */
#include "desktop.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void *xalloc(size_t n, void *aux) { (void)aux; return malloc(n); }
static void logline(const char *s, void *aux) { (void)aux; fprintf(stderr, "%s\n", s); }

static uint32_t now;
static void settle(shell_desktop *d) { for (int i = 0; i < 4; i++) { now += 50; desktop_tick(d, now, 14, 7); desktop_render(d); } }

int main(int argc, char **argv)
{
    if (argc < 2) { fprintf(stderr, "usage: shell_snap OUT.ppm [WxH] [scale] [steps]\n"); return 2; }
    int w = 1920, h = 1080, scale = 0;
    if (argc > 2) sscanf(argv[2], "%dx%d", &w, &h);
    if (argc > 3) scale = atoi(argv[3]);
    uint32_t *back = calloc((size_t)w * (size_t)h, 4);
    static shell_desktop d;
    d.log = logline;
    if (!desktop_init(&d, back, w, h, scale, xalloc, 0)) { fprintf(stderr, "desktop_init failed\n"); return 1; }
    desktop_render_all(&d);
    settle(&d);
    if (argc > 4) {
        char *steps = malloc(strlen(argv[4]) + 1); strcpy(steps, argv[4]);
        for (char *tok = strtok(steps, ";"); tok; tok = strtok(NULL, ";")) {
            int x, y;
            if (!strcmp(tok, "super")) { desktop_key(&d, 0x140, 0, 0, true); desktop_key(&d, 0x140, 0, 0, false); }
            else if (!strncmp(tok, "type:", 5)) for (const char *p = tok + 5; *p; p++) { desktop_key(&d, (uint32_t)*p, (uint32_t)*p, 0, true); desktop_key(&d, (uint32_t)*p, 0, 0, false); settle(&d); }
            else if (!strcmp(tok, "key:enter")) { desktop_key(&d, 13, 0, 0, true); desktop_key(&d, 13, 0, 0, false); }
            else if (!strcmp(tok, "key:esc")) { desktop_key(&d, 27, 0, 0, true); desktop_key(&d, 27, 0, 0, false); }
            else if (sscanf(tok, "move:%d,%d", &x, &y) == 2) desktop_pointer_to(&d, x, y, 0);
            else if (sscanf(tok, "click:%d,%d", &x, &y) == 2) { desktop_pointer_to(&d, x, y, 0); settle(&d); desktop_pointer_to(&d, x, y, BTN_LEFT); settle(&d); desktop_pointer_to(&d, x, y, 0); }
            else if (sscanf(tok, "win:%d", &x) == 1) d.settings.page = (settings_page)x;
            settle(&d);
        }
        free(steps);
    }
    FILE *f = fopen(argv[1], "wb");
    if (!f) return 1;
    fprintf(f, "P6\n%d %d\n255\n", w, h);
    for (int i = 0; i < w * h; i++) { unsigned char rgb[3] = { (unsigned char)(back[i] >> 16), (unsigned char)(back[i] >> 8), (unsigned char)back[i] }; fwrite(rgb, 1, 3, f); }
    fclose(f);
    return 0;
}
