#include "shell.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>

#define W 1600
#define H 800
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };

static int drain(shell_comp *c, int id, shell_event_type t, int *last_x)
{
    shell_event e; int n = 0;
    while (comp_pop_event(c, id, &e)) if (e.type == t) { n++; if (last_x) *last_x = e.x; }
    return n;
}
static void mouse(shell_wm *wm, int x, int y) { comp_mouse_move(wm->comp, x, y); wm_after_mouse(wm); }
static void button(shell_wm *wm, bool down) { comp_mouse_button(wm->comp, BTN_LEFT, down); wm_after_mouse(wm); }

int main(void)
{
    shell_comp *c = calloc(1, sizeof *c);
    comp_init(c, W, H);
    shell_launcher L; shell_inventory inv; inventory_init(&inv); launcher_init(&L, &inv, NULL, NULL);
    shell_taskbar tb; taskbar_init(&tb, c, &L);
    shell_wm wm; wm_init(&wm, c); wm_install(&wm);
    CHECK(wm.nmon == 1 && wm_monitor_work_area(&wm, 0).h == H - TASKBAR_H);

    /* snap geometry */
    shell_rect a = wm_monitor_work_area(&wm, 0);
    shell_rect l = wm_snap_rect(&wm, 0, SNAP_LEFT), r = wm_snap_rect(&wm, 0, SNAP_RIGHT);
    CHECK(l.x == 0 && l.y == 0 && l.h == a.h && r.x + r.w == a.w && r.h == a.h);
    CHECK(l.x + l.w + wm.gap / 2 <= r.x + 1 && r.x - (l.x + l.w) == wm.gap);          /* a gap between neighbours, none at screen edges */
    shell_rect m = wm_snap_rect(&wm, 0, SNAP_MAXIMIZE); CHECK(m.w == a.w && m.h == a.h && m.x == 0);
    shell_rect tl = wm_snap_rect(&wm, 0, SNAP_TL), br = wm_snap_rect(&wm, 0, SNAP_BR);
    CHECK(tl.w < a.w / 2 + 1 && tl.h < a.h / 2 + 1 && br.x > a.w / 2 - 1 && br.y > a.h / 2 - 1 && br.x + br.w == a.w && br.y + br.h == a.h);
    shell_rect t1 = wm_snap_rect(&wm, 0, SNAP_THIRD_L), t3 = wm_snap_rect(&wm, 0, SNAP_THIRD_R), t23 = wm_snap_rect(&wm, 0, SNAP_TWO_THIRDS_L);
    CHECK(t1.w < a.w / 3 + 1 && t3.x + t3.w == a.w && t23.w > t1.w * 2 - wm.gap * 2);

    /* detection */
    int mon;
    CHECK(wm_detect_snap(&wm, 2, 300, &mon) == SNAP_LEFT && wm_detect_snap(&wm, W - 2, 300, &mon) == SNAP_RIGHT);
    CHECK(wm_detect_snap(&wm, 800, 1, &mon) == SNAP_MAXIMIZE && wm_detect_snap(&wm, 2, 2, &mon) == SNAP_TL && wm_detect_snap(&wm, W - 2, 3, &mon) == SNAP_TR);
    CHECK(wm_detect_snap(&wm, 800, 400, &mon) == SNAP_NONE);
    CHECK(wm_detect_snap(&wm, 2, H - TASKBAR_H - 3, &mon) == SNAP_BL && wm_detect_snap(&wm, W - 2, H - TASKBAR_H - 3, &mon) == SNAP_BR);

    /* drag to the left edge: preview while held, snapped on release, restore size on drag away */
    int w1 = comp_create(&*c, "One", (shell_rect){ 300, 200, 400, 300 }, 0, 1);
    shell_window *W1 = comp_get(c, w1);
    mouse(&wm, 400, 210); button(&wm, true);
    mouse(&wm, 200, 250); CHECK(wm.preview == SNAP_NONE);
    mouse(&wm, 2, 300);
    CHECK(wm.preview == SNAP_LEFT && !(W1->flags & WF_SNAPPED));
    comp_render_full(c, &fb);
    shell_rect pl = wm_snap_rect(&wm, 0, SNAP_LEFT);
    CHECK(pixels[(pl.y + pl.h / 2) * W + pl.x] == c->theme.accent);                  /* preview outline drawn */
    mouse(&wm, 2, 300); button(&wm, false);
    CHECK((W1->flags & WF_SNAPPED) && W1->rect.x == pl.x && W1->rect.w == pl.w && W1->rect.h == pl.h && wm.preview == SNAP_NONE);
    CHECK(W1->restore.w == 400 && W1->restore.h == 300);
    drain(c, w1, EV_RESIZE, NULL);
    c->now_ms += 1000;
    mouse(&wm, 300, 10); button(&wm, true); mouse(&wm, 700, 400); button(&wm, false);   /* drag away from the snapped state */
    CHECK(!(W1->flags & WF_SNAPPED) && W1->rect.w == 400 && W1->rect.h == 300);

    /* drag to the top edge maximizes; keyboard Super+Down restores */
    c->now_ms += 1000;
    mouse(&wm, W1->rect.x + 100, W1->rect.y + 10); button(&wm, true); mouse(&wm, 800, 1);
    CHECK(wm.preview == SNAP_MAXIMIZE); button(&wm, false);
    CHECK(W1->flags & WF_MAXIMIZED && W1->rect.w == a.w);
    CHECK(wm_key(&wm, KEY_DOWN, true, MOD_SUPER) && !(W1->flags & WF_MAXIMIZED) && W1->rect.w == 400);

    /* keyboard snapping */
    CHECK(wm_key(&wm, KEY_LEFT, true, MOD_SUPER) && W1->rect.x == 0 && (W1->flags & WF_SNAPPED));
    CHECK(wm_key(&wm, KEY_RIGHT, true, MOD_SUPER) && !(W1->flags & WF_SNAPPED) && W1->rect.w == 400);   /* opposite arrow undoes the snap */
    CHECK(wm_key(&wm, KEY_RIGHT, true, MOD_SUPER) && W1->rect.x + W1->rect.w == a.w);
    CHECK(wm_key(&wm, KEY_UP, true, MOD_SUPER) && (W1->flags & WF_MAXIMIZED));
    wm_unsnap(&wm, w1);
    CHECK(wm_key(&wm, KEY_DOWN, true, MOD_SUPER) && (W1->flags & WF_MINIMIZED));
    comp_restore(c, w1);
    CHECK(!wm_key(&wm, KEY_LEFT, true, 0) && !wm_key(&wm, 'x', true, MOD_CTRL));

    /* NORESIZE windows are never snapped */
    int fixed = comp_create(c, "Dialog", (shell_rect){ 500, 300, 300, 200 }, WF_NORESIZE, 2);
    CHECK(!wm_snap_window(&wm, fixed, SNAP_LEFT, 0));
    comp_destroy(c, fixed);

    /* multi-monitor + DPI */
    CHECK(wm_add_monitor(&wm, (shell_rect){ W, 0, 1200, 700 }, 150, "right"));
    comp_init(c, W + 1200, H);                                                          /* grow the virtual screen */
    taskbar_init(&tb, c, &L); wm_install(&wm);
    CHECK(wm_monitor_at(&wm, 10, 10) == 0 && wm_monitor_at(&wm, W + 10, 10) == 1 && wm_monitor_at(&wm, W + 5000, 10) == 1 && wm_monitor_at(&wm, -50, 10) == 0);
    int w2 = comp_create(c, "Two", (shell_rect){ 100, 100, 500, 400 }, 0, 1);
    shell_window *W2 = comp_get(c, w2);
    comp_focus(c, w2);
    drain(c, w2, EV_DPI_CHANGED, NULL);
    wm_move_to_monitor(&wm, w2, 1);
    CHECK(W2->rect.x >= W && wm_monitor_at(&wm, W2->rect.x + 10, W2->rect.y + 10) == 1);
    int dpi = 0; CHECK(drain(c, w2, EV_DPI_CHANGED, &dpi) == 1 && dpi == 144 && W2->dpi == 144);
    wm_move_to_monitor(&wm, w2, 0);
    CHECK(W2->dpi == 96 && wm_monitor_at(&wm, W2->rect.x + 10, W2->rect.y + 10) == 0);
    wm_snap_window(&wm, w2, SNAP_RIGHT, 0);
    wm_move_to_monitor(&wm, w2, 1);
    CHECK((W2->flags & WF_SNAPPED) && W2->rect.x >= W + 1200 / 2 - 2);                  /* stays a right-half snap on the new monitor */
    wm_unsnap(&wm, w2);

    /* dragging across monitors updates DPI on release */
    drain(c, w2, EV_DPI_CHANGED, NULL);
    shell_rect cur = W2->rect;
    c->now_ms += 1000;
    mouse(&wm, cur.x + 50, cur.y + 10); button(&wm, true); mouse(&wm, 300, 300); button(&wm, false);
    CHECK(wm_monitor_at(&wm, W2->rect.x + W2->rect.w / 2, W2->rect.y + W2->rect.h / 2) == 0 && W2->dpi == 96);

    /* virtual desktops */
    int w3 = comp_create(c, "Three", (shell_rect){ 50, 50, 300, 200 }, 0, 3);
    comp_get(c, w3)->desktop = 0;
    CHECK(c->cur_desktop == 0 && comp_is_shown(c, comp_get(c, w3)));
    wm_switch_desktop(&wm, 1);
    CHECK(c->cur_desktop == 1 && !comp_is_shown(c, comp_get(c, w3)) && c->focus == 0 && taskbar_button_count(&tb, c) == 0);
    int w4 = comp_create(c, "Four", (shell_rect){ 60, 60, 300, 200 }, 0, 4);
    CHECK(comp_get(c, w4)->desktop == 1 && c->focus == w4);
    int hw; comp_hit_test(c, 70, 70, &hw); CHECK(hw == w4);
    CHECK(wm_move_to_desktop(&wm, w4, 0) && c->focus != w4 && !comp_is_shown(c, comp_get(c, w4)));
    wm_switch_desktop(&wm, 0);
    CHECK(comp_is_shown(c, comp_get(c, w4)) && comp_is_shown(c, comp_get(c, w3)) && c->focus != 0);
    CHECK(wm_key(&wm, '2', true, MOD_SUPER) && c->cur_desktop == 1);
    CHECK(wm_key(&wm, '1', true, MOD_SUPER) && c->cur_desktop == 0);
    comp_focus(c, w3);
    CHECK(wm_key(&wm, '3', true, MOD_SUPER | MOD_SHIFT) && comp_get(c, w3)->desktop == 2 && !comp_is_shown(c, comp_get(c, w3)));
    wm_switch_desktop(&wm, 99); CHECK(c->cur_desktop == 0);
    comp_get(c, w3)->flags |= WF_STICKY; comp_get(c, w3)->desktop = 0;
    wm_switch_desktop(&wm, 4); CHECK(comp_is_shown(c, comp_get(c, w3)) && !comp_is_shown(c, comp_get(c, w4)));
    wm_switch_desktop(&wm, 0);
    comp_get(c, w3)->flags &= ~(uint32_t)WF_STICKY;

    /* tiling */
    comp_destroy(c, w3); comp_destroy(c, w4); comp_destroy(c, w1); comp_destroy(c, w2);
    int t1w = comp_create(c, "T1", (shell_rect){ 10, 10, 300, 200 }, 0, 1);
    int t2w = comp_create(c, "T2", (shell_rect){ 20, 20, 300, 200 }, 0, 1);
    int t3w = comp_create(c, "T3", (shell_rect){ 30, 30, 300, 200 }, 0, 1);
    wm_set_tiling(&wm, true);
    shell_rect wa = wm_monitor_work_area(&wm, 0);
    shell_rect rt[3] = { comp_get(c, t3w)->rect, comp_get(c, t2w)->rect, comp_get(c, t1w)->rect };   /* top of the z-order is the master */
    CHECK(rt[0].x < rt[1].x && rt[1].x == rt[2].x && rt[0].h > rt[1].h + rt[2].h - wm.gap * 2);
    CHECK(rt[1].y < rt[2].y && rt[1].y + rt[1].h + wm.gap <= rt[2].y + 1);
    for (int i = 0; i < 3; i++) CHECK(rt[i].x >= wa.x && rt[i].y >= wa.y && rt[i].x + rt[i].w <= wa.x + wa.w && rt[i].y + rt[i].h <= wa.y + wa.h);
    CHECK(!rect_overlaps(rt[0], rt[1]) && !rect_overlaps(rt[1], rt[2]) && !rect_overlaps(rt[0], rt[2]));
    comp_destroy(c, t3w); wm_retile(&wm);
    CHECK(comp_get(c, t2w)->rect.x < comp_get(c, t1w)->rect.x);
    comp_destroy(c, t2w); wm_retile(&wm);
    CHECK(comp_get(c, t1w)->rect.w > wa.w - 4 * wm.gap);                                  /* a lone window fills the area */
    wm_set_tiling(&wm, false);
    CHECK(wm_key(&wm, 't', true, MOD_SUPER) && wm.tiling && wm_key(&wm, 'T', true, MOD_SUPER) && !wm.tiling);
    comp_destroy(c, t1w);

    /* focus follows mouse */
    int f1 = comp_create(c, "F1", (shell_rect){ 100, 100, 300, 200 }, 0, 1);
    int f2 = comp_create(c, "F2", (shell_rect){ 700, 100, 300, 200 }, 0, 1);
    CHECK(c->focus == f2);
    mouse(&wm, 150, 150); CHECK(c->focus == f2);                                       /* click-to-focus: hover does nothing */
    wm.focus_mode = FOCUS_FOLLOWS_MOUSE;
    mouse(&wm, 160, 160); CHECK(c->focus == f1);
    mouse(&wm, 800, 160); CHECK(c->focus == f2);
    mouse(&wm, 1300, 700); CHECK(c->focus == f2);                                      /* desktop hover keeps focus */
    CHECK(comp_get(c, f1) && 1);
    free(c);
    DONE("wm_test");
}
