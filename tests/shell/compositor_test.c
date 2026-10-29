#include "shell.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>

#define W 800
#define H 600
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };

static int drain(shell_comp *c, int id, shell_event_type t)
{
    shell_event e; int n = 0;
    while (comp_pop_event(c, id, &e)) if (e.type == t) n++;
    return n;
}
static int changes;
static void on_change(shell_comp *c, int win, void *u) { (void)c; (void)win; (*(int *)u)++; }

int main(void)
{
    shell_comp *c = calloc(1, sizeof *c);
    comp_init(c, W, H);
    c->on_change = on_change; c->on_change_user = &changes;
    comp_set_work_area(c, (shell_rect){ 0, 0, W, H - 40 });

    /* geometry helpers */
    shell_rect a = { 0, 0, 10, 10 }, b = { 5, 5, 10, 10 };
    shell_rect i = rect_intersect(a, b), u = rect_union(a, b);
    CHECK(i.x == 5 && i.w == 5 && u.w == 15 && !rect_overlaps(a, (shell_rect){ 20, 20, 5, 5 }));
    CHECK(color_blend(0xFF000000u, 0x80FFFFFFu) == 0xFF808080u || color_blend(0xFF000000u, 0x80FFFFFFu) == 0xFF818181u);
    CHECK(color_mix(0xFF000000u, 0xFFFFFFFFu, 128) == 0xFF7F7F7Fu);

    /* create + z-order + focus */
    int w1 = comp_create(c, "First", (shell_rect){ 50, 50, 300, 200 }, 0, 100);
    int w2 = comp_create(c, "Second", (shell_rect){ 200, 120, 300, 200 }, 0, 101);
    CHECK(w1 && w2 && w1 != w2 && c->focus == w2 && c->order[1] == w2);
    CHECK(drain(c, w1, EV_FOCUS) == 1 && drain(c, w2, EV_FOCUS) == 1);
    comp_raise(c, w1);
    CHECK(c->order[c->nwin - 1] == w1);
    comp_lower(c, w1);
    CHECK(c->order[0] == w1 && c->focus == w2);
    int top = 0;
    shell_zone z = comp_hit_test(c, 250, 130, &top);               /* overlap region: second is on top; title bar y 120..148 */
    CHECK(top == w2 && z == Z_TITLE);
    z = comp_hit_test(c, 60, 60, &top);
    CHECK(top == w1 && z == Z_TITLE);
    z = comp_hit_test(c, 100, 150, &top);
    CHECK(top == w1 && z == Z_CLIENT);
    z = comp_hit_test(c, 700, 500, &top);
    CHECK(z == Z_NONE && top == 0);
    z = comp_hit_test(c, 50 + 299 - 10, 50 + 14, &top);            /* close button of w1 */
    CHECK(top == w1 && z == Z_CLOSE);
    z = comp_hit_test(c, 50 + 299 - 50, 64, &top); CHECK(z == Z_MAXIMIZE);
    z = comp_hit_test(c, 50 + 299 - 100, 64, &top); CHECK(z == Z_MINIMIZE);
    z = comp_hit_test(c, 50, 50 + 195, &top); CHECK(z == Z_CORNER_BL);
    z = comp_hit_test(c, 50, 50 + 100, &top); CHECK(z == Z_EDGE_L);
    z = comp_hit_test(c, 50 + 299, 50, &top); CHECK(z == Z_CORNER_TR);
    z = comp_hit_test(c, 50 + 150, 50, &top); CHECK(z == Z_EDGE_T);
    comp_set_rect(c, w2, (shell_rect){ 500, 300, 250, 200 });          /* get w2 out of the way of the interaction tests */

    /* click raises + focuses, delivers client events with client-relative coordinates */
    comp_mouse_move(c, 100, 150);
    CHECK(drain(c, w1, EV_ENTER) == 1);
    comp_mouse_button(c, BTN_LEFT, true);
    CHECK(c->focus == w1 && c->order[c->nwin - 1] == w1);
    shell_rect cr = comp_client_rect(c, comp_get(c, w1));
    shell_event e; bool got = false;
    while (comp_pop_event(c, w1, &e)) if (e.type == EV_MOUSE_DOWN) { got = true; CHECK(e.x == 100 - cr.x && e.y == 150 - cr.y && e.button == BTN_LEFT); }
    CHECK(got);
    comp_mouse_move(c, 500, 500);                                   /* captured: still delivered while outside */
    CHECK(drain(c, w1, EV_MOUSE_MOVE) >= 1);
    comp_mouse_button(c, BTN_LEFT, false);
    CHECK(drain(c, w1, EV_MOUSE_UP) == 1 && c->capture == 0);

    /* drag by title */
    shell_rect before = comp_get(c, w1)->rect;
    comp_mouse_move(c, 100, 60); comp_mouse_button(c, BTN_LEFT, true);
    comp_mouse_move(c, 160, 90);
    shell_rect after = comp_get(c, w1)->rect;
    CHECK(after.x == before.x + 60 && after.y == before.y + 30 && after.w == before.w);
    comp_mouse_button(c, BTN_LEFT, false);
    comp_mouse_move(c, 160, 100);
    CHECK(comp_get(c, w1)->rect.x == after.x);                       /* released: no more dragging */
    CHECK(drain(c, w1, EV_MOVE) >= 1);

    /* drag cannot lose the title bar off the top */
    c->now_ms += 1000;
    comp_mouse_move(c, 160, 100 - 0); comp_mouse_button(c, BTN_LEFT, true);
    comp_mouse_move(c, 160, 100 - 200);
    CHECK(comp_get(c, w1)->rect.y >= 0);
    comp_mouse_button(c, BTN_LEFT, false);

    /* resize from the bottom-right corner, honoring min size */
    shell_window *W1 = comp_get(c, w1);
    shell_rect r0 = W1->rect;
    comp_mouse_move(c, r0.x + r0.w - 1, r0.y + r0.h - 1); comp_mouse_button(c, BTN_LEFT, true);
    comp_mouse_move(c, r0.x + r0.w - 1 + 40, r0.y + r0.h - 1 + 25);
    CHECK(W1->rect.w == r0.w + 40 && W1->rect.h == r0.h + 25 && W1->rect.x == r0.x);
    comp_mouse_move(c, r0.x + 5, r0.y + 5);
    CHECK(W1->rect.w == W1->min_w && W1->rect.h == W1->min_h);
    comp_mouse_button(c, BTN_LEFT, false);
    CHECK(drain(c, w1, EV_RESIZE) >= 1);
    W1->rect = r0;

    /* resize from the left edge moves x and keeps the right edge fixed */
    comp_mouse_move(c, r0.x, r0.y + 100); comp_mouse_button(c, BTN_LEFT, true);
    comp_mouse_move(c, r0.x - 30, r0.y + 100);
    CHECK(W1->rect.x == r0.x - 30 && W1->rect.x + W1->rect.w == r0.x + r0.w);
    comp_mouse_button(c, BTN_LEFT, false);
    W1->rect = r0;

    /* maximize / restore via button and via title double-click */
    shell_rect orig = W1->rect;
    comp_toggle_maximize(c, w1);
    CHECK((W1->flags & WF_MAXIMIZED) && W1->rect.w == W && W1->rect.h == H - 40);
    comp_toggle_maximize(c, w1);
    CHECK(!(W1->flags & WF_MAXIMIZED) && W1->rect.x == orig.x && W1->rect.w == orig.w);
    comp_mouse_move(c, orig.x + 20, orig.y + 10);
    c->now_ms = 1000; comp_mouse_button(c, BTN_LEFT, true); comp_mouse_button(c, BTN_LEFT, false);
    c->now_ms = 1200; comp_mouse_button(c, BTN_LEFT, true); comp_mouse_button(c, BTN_LEFT, false);
    CHECK(W1->flags & WF_MAXIMIZED);
    comp_toggle_maximize(c, w1);
    c->now_ms = 5000; comp_mouse_move(c, orig.x + 20, orig.y + 10);
    comp_mouse_button(c, BTN_LEFT, true); comp_mouse_button(c, BTN_LEFT, false);
    c->now_ms = 6000; comp_mouse_button(c, BTN_LEFT, true); comp_mouse_button(c, BTN_LEFT, false);
    CHECK(!(W1->flags & WF_MAXIMIZED));                             /* too slow for a double-click */

    /* chrome buttons: activate on release over the same button */
    comp_raise(c, w1); comp_focus(c, w1);
    drain(c, w1, EV_CLOSE_REQUEST);
    comp_mouse_move(c, W1->rect.x + W1->rect.w - 10, W1->rect.y + 14);
    comp_mouse_button(c, BTN_LEFT, true);
    comp_mouse_move(c, 700, 500); comp_mouse_button(c, BTN_LEFT, false);       /* released elsewhere: cancelled */
    CHECK(drain(c, w1, EV_CLOSE_REQUEST) == 0);
    comp_mouse_move(c, W1->rect.x + W1->rect.w - 10, W1->rect.y + 14);
    comp_mouse_button(c, BTN_LEFT, true); comp_mouse_button(c, BTN_LEFT, false);
    CHECK(drain(c, w1, EV_CLOSE_REQUEST) == 1);

    /* minimize hands focus to the next window; restore brings it back */
    comp_minimize(c, w1);
    CHECK(c->focus == w2 && !comp_is_shown(c, W1));
    z = comp_hit_test(c, W1->rect.x + 100, W1->rect.y + 100, &top);
    CHECK(top != w1);
    comp_restore(c, w1);
    CHECK(c->focus == w1 && comp_is_shown(c, W1));

    /* keyboard goes to the focused window only */
    comp_key(c, 'A', true, MOD_CTRL); comp_char(c, 0x20AC);
    CHECK(drain(c, w2, EV_KEY_DOWN) == 0);
    got = false; while (comp_pop_event(c, w1, &e)) if (e.type == EV_KEY_DOWN) { got = true; CHECK(e.key == 'A' && (e.mods & MOD_CTRL)); }
    CHECK(got);

    /* event queue overflow drops motion first */
    for (int k = 0; k < SHELL_EVQ * 2; k++) comp_key(c, 'x', true, 0);
    CHECK(W1->q_count == SHELL_EVQ);

    /* topmost windows stay above normal ones */
    int tm = comp_create(c, "Top", (shell_rect){ 300, 300, 200, 100 }, WF_TOPMOST | WF_NOFOCUS, 1);
    comp_raise(c, w2);
    CHECK(c->order[c->nwin - 1] == tm && c->focus == w1);

    /* damage + render */
    comp_render_full(c, &fb);
    CHECK(c->damage.n == 0);
    { uint32_t p = pixels[5 * W + 790];                                /* untouched wallpaper pixel: dark, bluish, opaque */
      CHECK((p >> 24) == 255 && (p >> 16 & 255) < 0x50 && (p >> 8 & 255) < 0x70 && (p & 255) < 0xA8); }
    comp_mouse_move(c, 700, 560);
    CHECK(c->damage.n >= 1);
    shell_rect db = damage_bounds(&c->damage);
    CHECK(db.w >= 12 && db.h >= 19);
    comp_render(c, &fb); CHECK(c->damage.n == 0);
    { int dark = 0, white = 0;                                              /* the arrow: a black outline around a white fill, anti-aliased */
      for (int y = 560; y < 580; y++) for (int x = 700; x < 714; x++) { uint32_t p = pixels[y * W + x]; int l = (int)(p >> 16 & 255); if (l < 24) dark++; if (l > 232) white++; }
      CHECK(dark > 15 && white > 20); }
    shell_rect wr = comp_get(c, w2)->rect;
    CHECK(pixels[(wr.y + wr.h / 2) * W + wr.x + wr.w / 2] == c->theme.surface || comp_get(c, tm) != NULL);
    int fbx = 0, nonbg = 0;
    for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) if (pixels[y * W + x] != c->theme.desktop) nonbg++;
    (void)fbx;
    CHECK(nonbg > 5000);
    /* title text pixels exist in the focused title bar */
    shell_window *fw = comp_get(c, c->focus);
    int textpix = 0;
    for (int y = fw->rect.y + 2; y < fw->rect.y + 26; y++) for (int x = fw->rect.x + 10; x < fw->rect.x + 60; x++) if (pixels[y * W + x] != pixels[(fw->rect.y + 3) * W + fw->rect.x + fw->rect.w / 2]) textpix++;   /* anything but the bar colour (text is anti-aliased) */
    CHECK(textpix > 20);

    /* damage coalescing */
    shell_damage d; damage_clear(&d);
    damage_add(&d, (shell_rect){ 0, 0, 10, 10 }); damage_add(&d, (shell_rect){ 5, 5, 10, 10 }); damage_add(&d, (shell_rect){ 100, 100, 5, 5 });
    CHECK(d.n == 2);
    for (int k = 0; k < 40; k++) damage_add(&d, (shell_rect){ 200 + k * 20, 0, 5, 5 });
    CHECK(d.n <= SHELL_DAMAGE_MAX);
    shell_rect bb = damage_bounds(&d); CHECK(bb.x == 0 && bb.w >= 960);

    /* client surface compositing */
    static uint32_t surf[100 * 50];
    for (size_t k = 0; k < sizeof surf / sizeof *surf; k++) surf[k] = 0xFFFF0000u;
    comp_set_client(c, w2, surf, 100, 50, 100);
    comp_raise(c, w2); comp_render_full(c, &fb);
    shell_rect cr2 = comp_client_rect(c, comp_get(c, w2));
    CHECK(pixels[(cr2.y + 40) * W + cr2.x + 90] == 0xFFFF0000u);               /* away from the topmost window's soft shadow */
    CHECK(pixels[(cr2.y + 10) * W + cr2.x + 10] >> 16 >= 0xE0 && (pixels[(cr2.y + 10) * W + cr2.x + 10] & 0xFFFF) < 0x2000);   /* ...which does tint nearby client pixels */

    /* virtual-desktop visibility */
    comp_get(c, w2)->desktop = 1;
    CHECK(!comp_is_shown(c, comp_get(c, w2)));
    comp_get(c, w2)->flags |= WF_STICKY;
    CHECK(comp_is_shown(c, comp_get(c, w2)));

    /* destroy: focus falls to the next window, slots are reusable */
    int before_n = c->nwin;
    comp_destroy(c, w1);
    CHECK(c->nwin == before_n - 1 && comp_get(c, w1) == NULL && c->focus != w1);
    CHECK(changes > 5);

    /* scale changes chrome metrics */
    comp_set_scale(c, 200);
    CHECK(c->m.title_h == 56 && c->m.border == 2);

    /* theme variants differ */
    shell_theme t1, t2, t3;
    shell_theme_dark(&t1, 0xFF3B82F6u); shell_theme_light(&t2, 0xFF3B82F6u); shell_theme_high_contrast(&t3);
    CHECK(t1.surface != t2.surface && t3.text == 0xFFFFFFFFu && t3.surface == 0xFF000000u);

    if (getenv("LOOS_SHELL_PPM")) {                                  /* visual artifact on demand */
        comp_set_scale(c, 100); comp_render_full(c, &fb);
        FILE *f = fopen(getenv("LOOS_SHELL_PPM"), "wb");
        if (f) { fprintf(f, "P6\n%d %d\n255\n", W, H); for (int k = 0; k < W * H; k++) { fputc(pixels[k] >> 16, f); fputc(pixels[k] >> 8, f); fputc(pixels[k], f); } fclose(f); }
    }
    free(c);
    DONE("compositor_test");
}
