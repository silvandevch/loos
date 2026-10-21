/* Renders the installer wizard pages to PPM files for visual review: installer_snap OUT_PREFIX [WxH] [scale] */
#include "installer.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int be_list(void *x, inst_disk *out, int max)
{
    (void)x; (void)max;
    memset(out, 0, 3 * sizeof *out);
    strcpy(out[0].name, "Disk 1"); strcpy(out[0].model, "LoOS installation media"); out[0].bytes = 4ull << 30; out[0].boot_media = true;
    strcpy(out[1].name, "Disk 2"); strcpy(out[1].model, "QEMU HARDDISK"); out[1].bytes = 64ull << 30;
    strcpy(out[2].name, "Disk 3"); strcpy(out[2].model, "Tiny USB"); out[2].bytes = 512ull << 20; out[2].too_small = true;
    return 3;
}
static bool be_begin(void *x, const inst_config *c, char *e, size_t n) { (void)x; (void)c; (void)e; (void)n; return true; }
static int be_step(void *x, int *p, char *m, size_t mc, char *e, size_t ec) { (void)x; (void)e; (void)ec; *p = 62; snprintf(m, mc, "Copying system files"); return 0; }
static void be_reboot(void *x) { (void)x; }

int main(int argc, char **argv)
{
    int w = 1920, h = 1080, sc = 100;
    if (argc > 2) sscanf(argv[2], "%dx%d", &w, &h);
    if (argc > 3) sc = atoi(argv[3]);
    uint32_t *px = calloc((size_t)w * h, 4);
    shell_fb fb = { px, w, h, w, sc };
    shell_theme th; shell_theme_dark(&th, 0xFF3B82F6u);
    static const shell_inst_backend be = { be_list, be_begin, be_step, be_reboot, 0 };
    static shell_installer in; static ui_ctx u;
    for (int page = IP_WELCOME; page <= IP_ERROR; page++) {
        installer_init(&in, &be);
        in.page = (inst_page)page; in.ndisks = be_list(0, in.disks, 8); in.sel_disk = 1;
        strcpy(in.display, "Ada Lovelace"); strcpy(in.user, "ada"); strcpy(in.pw, "correct horse9"); strcpy(in.pw2, "correct horse9");
        strcpy(in.err, "write error at sector 42"); in.percent = 62; strcpy(in.msg, "Copying system files");
        in.c_display = 12; in.c_user = 3; in.c_pw = in.c_pw2 = 14;
        for (int k = 0; k < 2; k++) { ui_begin(&u, &fb, &th, 100); installer_frame(&in, &u, w * 100 / sc, h * 100 / sc); ui_end(&u); }
        char name[256]; snprintf(name, sizeof name, "%s%d.ppm", argv[1], page);
        FILE *f = fopen(name, "wb"); fprintf(f, "P6\n%d %d\n255\n", w, h);
        for (int i = 0; i < w * h; i++) { unsigned char c[3] = { (unsigned char)(px[i] >> 16), (unsigned char)(px[i] >> 8), (unsigned char)px[i] }; fwrite(c, 1, 3, f); }
        fclose(f);
    }
    return 0;
}
