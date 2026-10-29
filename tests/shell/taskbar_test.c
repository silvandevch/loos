#include "shell.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>

#define W 1024
#define H 640
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };
static char launched[128]; static int nlaunch, toggled = -1;
static void on_launch(const shell_item *it, void *u) { (void)u; strcpy(launched, it->target); nlaunch++; }
static void on_toggle(bool off, void *u) { (void)u; toggled = off; }

static void type(shell_taskbar *t, shell_comp *c, const char *s) { for (; *s; s++) { launcher_char(t->launcher, (uint32_t)*s); } (void)c; }

int main(void)
{
    /* inventory parsing */
    shell_inventory inv; inventory_init(&inv);
    CHECK(inventory_parse(&inv, "# comment\n\napp|Alpha Tool|a.exe|first greek\nfile|Report 2026.txt|/home/u/report.txt|\nsetting| Dark Mode |settings:dark|theme\nbogus|x|y\napp|NoTarget\n") == 3);
    CHECK(inv.n == 3 && !strcmp(inv.item[2].name, "Dark Mode") && inv.item[1].kind == ITEM_FILE);
    inventory_init(&inv); inventory_defaults(&inv);
    CHECK(inv.n >= 12 && inventory_add(&inv, "Extra", "e.exe", "", ITEM_APP));

    /* scoring order */
    CHECK(fuzzy_score("calc", "LoOS Calculator", "math") > 0);
    CHECK(fuzzy_score("loos", "LoOS Notepad", "") > fuzzy_score("note", "LoOS Notepad", ""));   /* prefix beats word-prefix */
    CHECK(fuzzy_score("note", "LoOS Notepad", "") > fuzzy_score("tep", "LoOS Notepad", ""));    /* word-prefix beats substring */
    CHECK(fuzzy_score("tpd", "LoOS Notepad", "") > 0 && fuzzy_score("zzz", "LoOS Notepad", "editor") == 0);
    CHECK(fuzzy_score("editor", "LoOS Notepad", "text editor notes") > 0);
    CHECK(fuzzy_score("calculatorx", "Calc", "") == 0);
    CHECK(fuzzy_score("CALC", "calc", "") == 1000);

    shell_comp *c = calloc(1, sizeof *c);
    comp_init(c, W, H);
    shell_launcher L; launcher_init(&L, &inv, on_launch, NULL);
    shell_taskbar tb; taskbar_init(&tb, c, &L);
    tb.on_net_toggle = on_toggle;
    CHECK(c->work_area.h == H - TASKBAR_H && taskbar_rect(c).y == H - TASKBAR_H);

    /* launcher search + keys */
    CHECK(!L.open && !launcher_char(&L, 'a'));
    launcher_open(&L);
    CHECK(L.open && L.nhit == LAUNCHER_RESULTS);                                  /* empty query lists the first entries */
    type(&tb, c, "note");
    CHECK(L.nhit >= 1 && !strcmp(inv.item[L.hit[0]].name, "LoOS Notepad"));
    launcher_key(&L, KEY_BACKSPACE, 0); launcher_key(&L, KEY_BACKSPACE, 0); launcher_key(&L, KEY_BACKSPACE, 0); launcher_key(&L, KEY_BACKSPACE, 0);
    CHECK(L.nq == 0);
    type(&tb, c, "fire");
    CHECK(L.nhit >= 1 && !strcmp(inv.item[L.hit[0]].target, "settings:firewall"));
    type(&tb, c, "zzzz"); CHECK(L.nhit == 0);
    launcher_key(&L, KEY_ENTER, 0); CHECK(nlaunch == 0 && !L.open);                 /* Enter with no result just closes */
    launcher_open(&L); type(&tb, c, "term");
    CHECK(!strcmp(inv.item[L.hit[0]].target, "loos-cmd.exe"));
    launcher_key(&L, KEY_DOWN, 0); CHECK(L.sel == (L.nhit > 1 ? 1 : 0));
    launcher_key(&L, KEY_UP, 0); CHECK(L.sel == 0);
    launcher_key(&L, KEY_ENTER, 0);
    CHECK(nlaunch == 1 && !strcmp(launched, "loos-cmd.exe") && !L.open);
    launcher_open(&L); launcher_key(&L, KEY_ESC, 0); CHECK(!L.open);
    for (int i = 0; i < 200; i++) launcher_char(&L, 'x'), launcher_open(&L), launcher_char(&L, 'y');
    CHECK(L.nq == 1);

    /* Super opens the launcher, any further key is consumed by it, Esc closes */
    CHECK(taskbar_key(&tb, c, KEY_SUPER, true, 0) && L.open);
    CHECK(taskbar_key(&tb, c, 'q', true, 0)); CHECK(taskbar_key(&tb, c, KEY_ESC, true, 0) && !L.open);
    CHECK(taskbar_key(&tb, c, KEY_SPACE, true, MOD_CTRL) && L.open); taskbar_key(&tb, c, KEY_ESC, true, 0);
    CHECK(!taskbar_key(&tb, c, 'a', true, 0));

    /* window buttons */
    int a = comp_create(c, "Notepad - report.txt", (shell_rect){ 40, 40, 400, 300 }, 0, 1);
    int b = comp_create(c, "Task Manager", (shell_rect){ 200, 100, 400, 300 }, 0, 2);
    int hid = comp_create(c, "hidden tool", (shell_rect){ 0, 0, 200, 100 }, WF_NOTASKBAR, 3);
    CHECK(taskbar_button_count(&tb, c) == 2 && taskbar_button_window(&tb, c, 0) == a && taskbar_button_window(&tb, c, 1) == b);
    shell_rect r0 = taskbar_button_rect(&tb, c, 0), r1 = taskbar_button_rect(&tb, c, 1);
    CHECK(r1.x > r0.x + r0.w - 1 && r0.w <= 180 && r0.y >= taskbar_rect(c).y);
    CHECK(c->focus == hid || c->focus == b);
    comp_focus(c, b);

    /* click: unfocused -> focus, focused -> minimize, minimized -> restore */
    comp_mouse_move(c, r0.x + 10, r0.y + 10);
    CHECK(taskbar_mouse(&tb, c, r0.x + 10, r0.y + 10, 0, false) && tb.hover_btn == 1);
    CHECK(taskbar_mouse(&tb, c, r0.x + 10, r0.y + 10, BTN_LEFT, true));
    CHECK(c->focus == a && c->order[c->nwin - 1] == a);
    taskbar_mouse(&tb, c, r0.x + 10, r0.y + 10, BTN_LEFT, true);
    CHECK(comp_get(c, a)->flags & WF_MINIMIZED);
    taskbar_mouse(&tb, c, r0.x + 10, r0.y + 10, BTN_LEFT, true);
    CHECK(!(comp_get(c, a)->flags & WF_MINIMIZED) && c->focus == a);
    CHECK(!taskbar_mouse(&tb, c, 300, 200, BTN_LEFT, true));                        /* not in the bar: not consumed */
    CHECK(tb.hover_btn == -1);

    /* Alt+Tab cycles by creation order, Alt+Shift+Tab backwards */
    comp_focus(c, a);
    CHECK(taskbar_key(&tb, c, KEY_TAB, true, MOD_ALT) && c->focus == b);
    CHECK(taskbar_key(&tb, c, KEY_TAB, true, MOD_ALT) && c->focus == a);
    CHECK(taskbar_key(&tb, c, KEY_TAB, true, MOD_ALT | MOD_SHIFT) && c->focus == b);
    comp_minimize(c, a); taskbar_key(&tb, c, KEY_TAB, true, MOD_ALT);
    CHECK(c->focus == a && !(comp_get(c, a)->flags & WF_MINIMIZED));
    shell_event e; while (comp_pop_event(c, a, &e)) { }
    CHECK(taskbar_key(&tb, c, KEY_F4, true, MOD_ALT));
    bool close_req = false; while (comp_pop_event(c, a, &e)) if (e.type == EV_CLOSE_REQUEST) close_req = true;
    CHECK(close_req);

    /* desktops: windows of other desktops get no button, sticky ones do */
    comp_get(c, b)->desktop = 1;
    CHECK(taskbar_button_count(&tb, c) == 1);
    comp_get(c, b)->flags |= WF_STICKY;
    CHECK(taskbar_button_count(&tb, c) == 2);
    comp_get(c, b)->flags &= ~(uint32_t)WF_STICKY; comp_get(c, b)->desktop = 0;

    /* network LED + hard switch */
    CHECK(taskbar_net_state(&tb) == NET_IDLE);
    taskbar_set_net(&tb, 3); CHECK(taskbar_net_state(&tb) == NET_ACTIVE);
    shell_rect tgl = { W - 170 + 24 + 5, H - TASKBAR_H + 14, 10, 10 };
    CHECK(taskbar_mouse(&tb, c, tgl.x, tgl.y, BTN_LEFT, true) && tb.net_hard_off && toggled == 1 && taskbar_net_state(&tb) == NET_BLOCKED);
    taskbar_mouse(&tb, c, tgl.x, tgl.y, BTN_LEFT, true);
    CHECK(!tb.net_hard_off && toggled == 0 && taskbar_net_state(&tb) == NET_ACTIVE);
    taskbar_set_net(&tb, 0); CHECK(taskbar_net_state(&tb) == NET_IDLE);

    /* start button toggles the launcher; clicking outside closes it; clicking a result launches */
    nlaunch = 0;
    shell_rect st = taskbar_start_rect(c);
    int sx = st.x + st.w / 2, sy = st.y + st.h / 2;
    CHECK(taskbar_mouse(&tb, c, sx, sy, BTN_LEFT, true) && L.open);
    CHECK(taskbar_mouse(&tb, c, sx, sy, BTN_LEFT, true) && !L.open);
    taskbar_mouse(&tb, c, sx, sy, BTN_LEFT, true);
    shell_rect lr = launcher_rect(c);
    CHECK(lr.y + lr.h < H - TASKBAR_H && lr.x >= 0);
    CHECK(taskbar_mouse(&tb, c, lr.x + 26 + 98 * 1 + 40, lr.y + 64 + 40 + 40, BTN_LEFT, true) && nlaunch == 1 && !L.open);   /* pinned tile 1 */
    taskbar_mouse(&tb, c, sx, sy, BTN_LEFT, true);
    taskbar_mouse(&tb, c, 900, 100, BTN_LEFT, true);
    CHECK(!L.open);

    /* rendering */
    taskbar_set_time(&tb, 9, 5);
    comp_render_full(c, &fb);
    shell_rect bar = taskbar_rect(c);
    CHECK(pixels[(bar.y + 20) * W + 300] != pixels[(bar.y - 20) * W + 300]);          /* translucent bar differs from the wallpaper */
    int logo = 0; for (int y = st.y; y < st.y + st.h; y++) for (int x = st.x; x < st.x + st.w; x++) if (pixels[y * W + x] == c->theme.accent) logo++;
    CHECK(logo > 20);                                                                   /* start mark in the accent color */
    int led = 0; for (int y = bar.y; y < bar.y + TASKBAR_H; y++) for (int x = W - 170; x < W - 150; x++) if (pixels[y * W + x] == 0xFF4B5563u) led++;
    CHECK(led > 20);                                                                 /* idle LED grey */
    taskbar_set_net(&tb, 2); comp_render(c, &fb);
    int amber = 0; for (int y = bar.y; y < bar.y + TASKBAR_H; y++) for (int x = W - 170; x < W - 150; x++) if (pixels[y * W + x] == 0xFFF59E0Bu) amber++;
    CHECK(amber > 20);                                                               /* active LED amber */
    int clk = 0; for (int y = bar.y; y < bar.y + TASKBAR_H; y++) for (int x = W - 60; x < W - 8; x++) if (pixels[y * W + x] != pixels[(bar.y + 3) * W + W - 100]) clk++;
    CHECK(clk > 30);
    launcher_open(&L); type(&tb, c, "set"); comp_damage_all(c); comp_render(c, &fb);
    shell_rect lrr = launcher_rect(c);
    int y0 = lrr.y + 64 + 8 + 2, y2 = y0 + 2 * 40;                                     /* result rows 0 (selected) and 2 */
    CHECK(L.nhit >= 5 && pixels[y0 * W + lrr.x + 30] != pixels[y2 * W + lrr.x + 30]);  /* "set" lists settings pages; row 0 highlighted */
    if (getenv("LOOS_SHELL_PPM")) {
        FILE *f = fopen(getenv("LOOS_SHELL_PPM"), "wb");
        if (f) { fprintf(f, "P6\n%d %d\n255\n", W, H); for (int k = 0; k < W * H; k++) { fputc(pixels[k] >> 16, f); fputc(pixels[k] >> 8, f); fputc(pixels[k], f); } fclose(f); }
    }
    free(c);
    DONE("taskbar_test");
}
