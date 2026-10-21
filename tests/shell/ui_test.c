#include "ui.h"
#include "test.h"
#include <string.h>
#include <stdlib.h>

#define W 400
#define H 300
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };
static shell_theme th;
static ui_ctx u;

static void ev(shell_event_type t, int x, int y, int button, uint32_t key) { shell_event e = { t, x, y, button, key, 0 }; ui_feed(&u, &e); }
static void frame_begin(void) { ui_begin(&u, &fb, &th, 100); }
static void click(shell_rect r, bool (*fn)(shell_rect, void *), void *arg, bool *result)
{
    /* press and release over r across three frames, like a real event stream */
    ev(EV_MOUSE_MOVE, r.x + r.w / 2, r.y + r.h / 2, 0, 0);
    frame_begin(); fn(r, arg); ui_end(&u);
    ev(EV_MOUSE_DOWN, r.x + r.w / 2, r.y + r.h / 2, BTN_LEFT, 0);
    frame_begin(); fn(r, arg); ui_end(&u);
    ev(EV_MOUSE_UP, r.x + r.w / 2, r.y + r.h / 2, BTN_LEFT, 0);
    frame_begin(); *result = fn(r, arg); ui_end(&u);
}
static bool btn_fn(shell_rect r, void *a) { return ui_button(&u, r, a); }
static bool tog_fn(shell_rect r, void *a) { return ui_toggle(&u, r, "Toggle me", a); }

static const char *rows[] = { "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet" };
static void draw_row(ui_ctx *c, shell_rect row, int i, bool sel, void *user) { (void)user; ui_label(c, row.x + 6, row.y + 2, rows[i], sel ? c->theme->accent_text : c->theme->text); }

int main(void)
{
    shell_theme_dark(&th, 0xFF3B82F6u);
    memset(&u, 0, sizeof u);
    CHECK(ui_id("a") != ui_id("b") && ui_id("") != 0);

    /* button: click completes only when released over the button */
    bool r;
    shell_rect b = { 20, 20, 100, 32 };
    click(b, btn_fn, "OK", &r); CHECK(r);
    ev(EV_MOUSE_MOVE, 60, 30, 0, 0); frame_begin(); ui_button(&u, b, "OK"); ui_end(&u);
    ev(EV_MOUSE_DOWN, 60, 30, BTN_LEFT, 0); frame_begin(); ui_button(&u, b, "OK"); ui_end(&u);
    CHECK(u.active != 0);
    ev(EV_MOUSE_MOVE, 300, 250, 0, 0); ev(EV_MOUSE_UP, 300, 250, BTN_LEFT, 0);
    frame_begin(); bool c2 = ui_button(&u, b, "OK"); ui_end(&u);
    CHECK(!c2 && u.active == 0);                                                      /* dragged off: cancelled */
    int bg = 0; for (int y = 20; y < 52; y++) for (int x = 20; x < 120; x++) if (pixels[y * W + x] != 0) bg++;
    CHECK(bg > 2000);

    /* toggle */
    bool val = false; shell_rect t = { 20, 70, 200, 28 };
    click(t, tog_fn, &val, &r); CHECK(r && val);
    click(t, tog_fn, &val, &r); CHECK(r && !val);
    CHECK(u.focus != 0);

    /* radio */
    int grp = 0; shell_rect rr = { 20, 110, 150, 24 };
    ev(EV_MOUSE_MOVE, 40, 120, 0, 0); frame_begin(); ui_radio(&u, rr, "Light", &grp, 1); ui_end(&u);
    ev(EV_MOUSE_DOWN, 40, 120, BTN_LEFT, 0); frame_begin(); ui_radio(&u, rr, "Light", &grp, 1); ui_end(&u);
    ev(EV_MOUSE_UP, 40, 120, BTN_LEFT, 0); frame_begin(); bool rc = ui_radio(&u, rr, "Light", &grp, 1); ui_end(&u);
    CHECK(rc && grp == 1);

    /* text field: focus by click, typing, caret movement, backspace/delete, Enter, capacity */
    char buf[16] = ""; int caret = 0; shell_rect tf = { 20, 150, 200, 28 };
    ev(EV_MOUSE_MOVE, 30, 160, 0, 0); ev(EV_MOUSE_DOWN, 30, 160, BTN_LEFT, 0);
    frame_begin(); ui_textfield(&u, tf, buf, sizeof buf, &caret, "name"); ui_end(&u);
    ev(EV_MOUSE_UP, 30, 160, BTN_LEFT, 0);
    for (const char *s = "hello"; *s; s++) ev(EV_CHAR, 0, 0, 0, (uint32_t)*s);
    frame_begin(); ui_textfield(&u, tf, buf, sizeof buf, &caret, "name"); ui_end(&u);
    CHECK(!strcmp(buf, "hello") && caret == 5);
    ev(EV_KEY_DOWN, 0, 0, 0, KEY_LEFT); ev(EV_KEY_DOWN, 0, 0, 0, KEY_LEFT); ev(EV_CHAR, 0, 0, 0, 'X');
    frame_begin(); ui_textfield(&u, tf, buf, sizeof buf, &caret, "name"); ui_end(&u);
    CHECK(!strcmp(buf, "helXlo") && caret == 4);
    ev(EV_KEY_DOWN, 0, 0, 0, KEY_BACKSPACE); ev(EV_KEY_DOWN, 0, 0, 0, KEY_DELETE);
    frame_begin(); ui_textfield(&u, tf, buf, sizeof buf, &caret, "name"); ui_end(&u);
    CHECK(!strcmp(buf, "helo") && caret == 3);
    ev(EV_KEY_DOWN, 0, 0, 0, KEY_HOME); ev(EV_KEY_DOWN, 0, 0, 0, KEY_BACKSPACE);
    frame_begin(); ui_textfield(&u, tf, buf, sizeof buf, &caret, "name"); ui_end(&u);
    CHECK(!strcmp(buf, "helo") && caret == 0);
    ev(EV_KEY_DOWN, 0, 0, 0, KEY_END);
    for (int i = 0; i < 40; i++) ev(EV_CHAR, 0, 0, 0, 'z');
    frame_begin(); ui_textfield(&u, tf, buf, sizeof buf, &caret, "name"); ui_end(&u);
    CHECK(strlen(buf) == sizeof buf - 1);                                             /* never overflows */
    ev(EV_KEY_DOWN, 0, 0, 0, KEY_ENTER);
    frame_begin(); bool entered = ui_textfield(&u, tf, buf, sizeof buf, &caret, "name"); ui_end(&u);
    CHECK(entered);
    ev(EV_MOUSE_DOWN, 380, 290, BTN_LEFT, 0); ev(EV_MOUSE_UP, 380, 290, BTN_LEFT, 0);        /* click elsewhere: no text focus change by itself */
    ev(EV_CHAR, 0, 0, 0, 'q');
    frame_begin(); ui_textfield(&u, tf, buf, sizeof buf, &caret, "name"); ui_end(&u);
    CHECK(strlen(buf) == sizeof buf - 1);

    /* tabs */
    const char *labels[] = { "One", "Two", "Three" }; int cur = 0; shell_rect tr = { 0, 200, 400, 28 };
    ev(EV_MOUSE_MOVE, 100, 212, 0, 0); frame_begin(); ui_tabs(&u, tr, labels, 3, &cur); ui_end(&u);
    ev(EV_MOUSE_DOWN, 100, 212, BTN_LEFT, 0); frame_begin(); ui_tabs(&u, tr, labels, 3, &cur); ui_end(&u);
    ev(EV_MOUSE_UP, 100, 212, BTN_LEFT, 0); frame_begin(); int picked = ui_tabs(&u, tr, labels, 3, &cur); ui_end(&u);
    CHECK(picked == 1 && cur == 1);

    /* list: selection, wheel scrolling, keyboard, double click activation, scroll clamp */
    int sel = -1, scroll = 0; shell_rect lr = { 220, 20, 170, 5 * 20 };
    u.now_ms = 1000;
    ev(EV_MOUSE_MOVE, 240, 30, 0, 0); ev(EV_MOUSE_DOWN, 240, 30, BTN_LEFT, 0);
    frame_begin(); int act = ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    ev(EV_MOUSE_UP, 240, 30, BTN_LEFT, 0); frame_begin(); ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    CHECK(sel == 0 && act == -1);
    u.now_ms = 1200;
    ev(EV_MOUSE_DOWN, 240, 30, BTN_LEFT, 0);
    frame_begin(); act = ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    CHECK(act == 0);                                                                  /* double-click */
    ev(EV_MOUSE_UP, 240, 30, BTN_LEFT, 0); frame_begin(); ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    u.now_ms = 3000;
    ev(EV_MOUSE_DOWN, 240, 70, BTN_LEFT, 0); frame_begin(); act = ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    ev(EV_MOUSE_UP, 240, 70, BTN_LEFT, 0);
    CHECK(sel == 2 && act == -1);
    for (int i = 0; i < 12; i++) ev(EV_KEY_DOWN, 0, 0, 0, KEY_DOWN);
    frame_begin(); ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    CHECK(sel == 9 && scroll == 5);                                                   /* selection scrolled into view, clamped */
    ev(EV_KEY_DOWN, 0, 0, 0, KEY_HOME); ev(EV_KEY_DOWN, 0, 0, 0, KEY_ENTER);
    frame_begin(); act = ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    CHECK(sel == 0 && scroll == 0 && act == 0);
    ev(EV_MOUSE_MOVE, 240, 50, 0, 0); ev(EV_MOUSE_WHEEL, 240, 50, -3, 0);
    frame_begin(); ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    CHECK(scroll == 3);
    ev(EV_MOUSE_WHEEL, 240, 50, -100, 0);
    frame_begin(); ui_list(&u, lr, "list", 10, 20, draw_row, NULL, &sel, &scroll); ui_end(&u);
    CHECK(scroll == 5);
    int sbpix = 0; for (int y = lr.y; y < lr.y + lr.h; y++) for (int x = lr.x + lr.w - 6; x < lr.x + lr.w - 2; x++) if (pixels[y * W + x] == th.text_dim) sbpix++;
    CHECK(sbpix > 10);                                                                /* scrollbar thumb */

    /* slider */
    int sv = 10; shell_rect sr = { 20, 250, 200, 20 };
    ev(EV_MOUSE_MOVE, 120, 260, 0, 0); ev(EV_MOUSE_DOWN, 120, 260, BTN_LEFT, 0);
    frame_begin(); bool ch = ui_slider(&u, sr, "vol", &sv, 0, 100); ui_end(&u);
    CHECK(ch && sv > 45 && sv < 55);
    ev(EV_MOUSE_MOVE, 5000, 260, 0, 0); frame_begin(); ui_slider(&u, sr, "vol", &sv, 0, 100); ui_end(&u);
    CHECK(sv == 100);                                                                 /* dragging beyond the end clamps */
    ev(EV_MOUSE_UP, 5000, 260, BTN_LEFT, 0); frame_begin(); ui_slider(&u, sr, "vol", &sv, 0, 100); ui_end(&u);

    /* heading/progress draw something; clip protects outside pixels */
    memset(pixels, 0, sizeof pixels);
    frame_begin(); ui_heading(&u, 10, 10, "LoOS"); ui_progress(&u, (shell_rect){ 10, 60, 100, 8 }, 50); ui_end(&u);
    int hp = 0; for (int y = 10; y < 42; y++) for (int x = 10; x < 74; x++) if (pixels[y * W + x] == th.text) hp++;
    CHECK(hp > 80);
    CHECK(pixels[63 * W + 20] == th.accent && pixels[63 * W + 100] == th.border);
    shell_rect saved; memset(pixels, 0, sizeof pixels);
    frame_begin(); ui_clip_push(&u, (shell_rect){ 0, 0, 50, 50 }, &saved); ui_panel(&u, (shell_rect){ 0, 0, 200, 200 }, 0xFFFFFFFFu); ui_clip_pop(&u, saved); ui_end(&u);
    CHECK(pixels[10 * W + 10] == 0xFFFFFFFFu && pixels[100 * W + 100] == 0 && pixels[10 * W + 60] == 0);
    DONE("ui_test");
}
