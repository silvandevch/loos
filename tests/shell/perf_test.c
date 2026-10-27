/* Occlusion culling must be invisible: pixel-identical to painting everything, and cheaper when a window hides the rest. */
#define _POSIX_C_SOURCE 200809L
#include "shell.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define W 1280
#define H 720
static uint32_t pa[W * H], pb[W * H];

static double now_s(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return (double)t.tv_sec + (double)t.tv_nsec * 1e-9; }

static void render(shell_comp *c, uint32_t *px, bool cull, shell_rect dmg)
{
    shell_fb fb = { px, W, H, W, 100 };
    c->no_cull = !cull;
    damage_clear(&c->damage);
    damage_add(&c->damage, dmg);
    comp_render(c, &fb);
}

static bool same_after(shell_comp *c, shell_rect dmg)
{
    memset(pa, 0, sizeof pa); memset(pb, 0, sizeof pb);
    render(c, pa, true, (shell_rect){ 0, 0, W, H });          /* identical starting content */
    render(c, pb, false, (shell_rect){ 0, 0, W, H });
    render(c, pa, true, dmg);
    render(c, pb, false, dmg);
    return memcmp(pa, pb, sizeof pa) == 0;
}

int main(void)
{
    shell_comp *c = calloc(1, sizeof *c);
    comp_init(c, W, H);
    comp_set_work_area(c, (shell_rect){ 0, 0, W, H - 40 });
    int w1 = comp_create(c, "Back", (shell_rect){ 100, 100, 500, 400 }, 0, 1);
    int w2 = comp_create(c, "Mid", (shell_rect){ 300, 200, 500, 400 }, 0, 2);
    CHECK(w1 && w2);

    /* no window hides the damage: nothing is culled, output identical */
    uint32_t before = c->stat_culled;
    CHECK(same_after(c, (shell_rect){ 50, 50, 300, 250 }));
    CHECK(c->stat_culled == before);

    /* damage inside the top window's body: wallpaper and the window below are skipped */
    CHECK(same_after(c, (shell_rect){ 400, 300, 120, 90 }));
    CHECK(c->stat_culled > before);

    /* maximize: every repaint inside it is a single window draw */
    comp_toggle_maximize(c, w2);
    uint32_t b2 = c->stat_culled;
    CHECK(same_after(c, (shell_rect){ 50, 50, 600, 500 }));
    CHECK(c->stat_culled > b2);

    /* damage that includes the taskbar strip (not covered) must still paint the wallpaper there */
    CHECK(same_after(c, (shell_rect){ 0, H - 60, W, 60 }));

    /* a rounded corner of the maximized-then-restored window must not be culled away */
    comp_toggle_maximize(c, w2);
    CHECK(same_after(c, (shell_rect){ 296, 196, 24, 24 }));

    /* a hidden window above never culls what is below */
    comp_minimize(c, w2);
    CHECK(same_after(c, (shell_rect){ 120, 120, 300, 200 }));

    /* timing (informational): repaint inside a maximized window */
    comp_restore(c, w2);
    comp_toggle_maximize(c, w2);
    shell_rect d = { 0, 0, W, H - 40 };
    double t0 = now_s(); for (int i = 0; i < 20; i++) render(c, pa, false, d);
    double t1 = now_s(); for (int i = 0; i < 20; i++) render(c, pa, true, d);
    double t2 = now_s();
    printf("full repaint under a maximized window: %.2f ms uncached, %.2f ms culled\n", (t1 - t0) * 50.0, (t2 - t1) * 50.0);
    CHECK((t2 - t1) < (t1 - t0));

    free(c);
    DONE("perf_test");
}
