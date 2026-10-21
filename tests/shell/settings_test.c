#include "settings.h"
#include "apphost.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>

#define SW 1280
#define SH 800
static uint32_t screen[SW * SH];
static shell_fb fb = { screen, SW, SH, SW, 100 };
#define CW 900
#define CH 640
static uint32_t surface[CW * CH];

static shell_comp *comp; static shell_app_window aw; static shell_settings st;
static void frame_cb(void *app, ui_ctx *u, int w, int h) { settings_frame(app, u, w, h); }
static void pump(void) { appwin_pump(&aw); }
static void click(int x, int y)                                   /* client coordinates */
{
    shell_rect cr = comp_client_rect(comp, comp_get(comp, aw.win));
    int sc = (int)comp->scale_pct;                                      /* the app lays out in logical px; events arrive in device px */
    x = (x * sc + sc / 2) / 100; y = (y * sc + sc / 2) / 100;
    comp_mouse_move(comp, cr.x + x, cr.y + y); pump();
    comp_mouse_button(comp, BTN_LEFT, true); pump();
    comp_mouse_button(comp, BTN_LEFT, false); pump();
}
static void type(const char *s) { for (; *s; s++) comp_char(comp, (uint32_t)*s); pump(); }
static void key(uint32_t k) { comp_key(comp, k, true, 0); pump(); }

int main(void)
{
    comp = calloc(1, sizeof *comp);
    comp_init(comp, SW, SH);
    backend_mem mem; shell_backend be; backend_mem_init(&mem, &be);
    backend_mem_add_conn(&mem, "browser.exe", 412, "93.184.216.34", 443, "TCP", "ESTABLISHED");
    backend_mem_add_conn(&mem, "updater.exe", 508, "10.0.0.7", 8080, "TCP", "SYN_SENT");
    backend_mem_add_conn(&mem, "chat.exe", 77, "198.51.100.9", 5222, "TCP", "ESTABLISHED");
    backend_mem_add_app(&mem, "browser.exe"); backend_mem_add_app(&mem, "loos-notepad.exe"); backend_mem_add_app(&mem, "chat.exe");
    shell_launcher L; shell_inventory inv; inventory_init(&inv); launcher_init(&L, &inv, NULL, NULL);
    shell_taskbar tb; taskbar_init(&tb, comp, &L);
    settings_init(&st, &be, comp, &tb);
    CHECK(appwin_open(&aw, comp, "Settings", (shell_rect){ 100, 40, CW + 2, CH + 30 }, 0, 9, surface, CW, CH, frame_cb, &st));

    int cx = 190 + 28; const int cy = 24;                            /* content origin follows the UI scale */
    CHECK(st.page == PG_PRIVACY && st.theme == 0 && st.scale == 100);
    CHECK(!be_cfg_bool(&be, "updates.enabled", false));              /* updates default to off: opt-in */

    /* sidebar navigation */
    click(60, 12 + PG_NETWORK * 38 + 16); CHECK(st.page == PG_NETWORK && st.nconns == 3);

    /* firewall: select a connection, block its process; the row reflects it */
    click(cx + 40, cy + 100 + 24 + 12);                              /* second row */
    CHECK(st.conn_sel == 1);
    int y = cy + 100 + 220 + 12 + 14;
    CHECK(be.fw_check(be.user, "updater.exe") == FW_ASK);
    click(cx + 336 + 40, y);
    CHECK(be.fw_check(be.user, "updater.exe") == FW_DENY && mem.nrule == 1 && st.changes >= 1);
    click(cx + 240 + 40, y); CHECK(be.fw_check(be.user, "updater.exe") == FW_ALLOW);
    click(cx + 432 + 40, y); CHECK(be.fw_check(be.user, "updater.exe") == FW_ASK && mem.nrule == 0);
    click(cx + 40, cy + 100 + 12);                                   /* first row */
    click(cx + 336 + 40, y); CHECK(be.fw_check(be.user, "browser.exe") == FW_DENY);

    /* hard switch: backend, taskbar and LED agree */
    CHECK(settings_net_state(&st) == NET_ACTIVE);
    click(cx + 150, cy + 44 + 13);
    CHECK(mem.hard_off && tb.net_hard_off && taskbar_net_state(&tb) == NET_BLOCKED && settings_net_state(&st) == NET_BLOCKED);
    CHECK(be.fw_check(be.user, "anything.exe") == FW_DENY);
    click(cx + 150, cy + 44 + 13);
    CHECK(!mem.hard_off && !tb.net_hard_off);

    /* permissions: default deny; toggling writes through */
    click(60, 12 + PG_PERMISSIONS * 38 + 16);
    CHECK(st.page == PG_PERMISSIONS && st.napps == 3);
    click(cx + 60, cy + 50 + 13 + 26);                               /* second app */
    CHECK(st.app_sel == 1);
    CHECK(!be.perm_get(be.user, "loos-notepad.exe", RES_CAMERA));
    click(cx + 270 + 100, cy + 56 + 32 + 14);                        /* Camera toggle */
    CHECK(be.perm_get(be.user, "loos-notepad.exe", RES_CAMERA) && !be.perm_get(be.user, "browser.exe", RES_CAMERA));
    click(cx + 270 + 100, cy + 56 + 32 + 36 * 2 + 14);               /* Files */
    CHECK(be.perm_get(be.user, "loos-notepad.exe", RES_FILES));
    click(cx + 270 + 100, cy + 56 + 32 + 14);
    CHECK(!be.perm_get(be.user, "loos-notepad.exe", RES_CAMERA));

    /* updates: opt-in toggle and mirror field */
    click(60, 12 + PG_UPDATES * 38 + 16);
    click(cx + 200, cy + 50 + 14); CHECK(be_cfg_bool(&be, "updates.enabled", false));
    click(cx + 200, cy + 90 + 14); CHECK(be_cfg_bool(&be, "updates.offline", false));
    click(cx + 100, cy + 160 + 15); type("https://mirror.home.lan/loos"); key(KEY_ENTER);
    char v[96]; CHECK(be.cfg_get(be.user, "updates.mirror", v, sizeof v) && !strcmp(v, "https://mirror.home.lan/loos"));

    /* appearance: theme, accent, scale apply live to the compositor */
    click(60, 12 + PG_APPEARANCE * 38 + 16);
    uint32_t dark_surface = comp->theme.surface;
    click(cx + 150 + 10, cy + 72 + 12);
    CHECK(st.theme == 1 && comp->theme.surface != dark_surface && comp->theme.surface == 0xFFF7F8FAu);
    click(cx + 2 * 44 + 16, cy + 140 + 16);
    CHECK(st.accent == 2 && comp->theme.accent == settings_accents[2]);
    click(cx + 2 * 90 + 10, cy + 216 + 12);
    CHECK(st.scale == 150 && comp->scale_pct == 150 && comp->m.title_h == 42);
    click(cx + 10, cy + 72 + 12); CHECK(st.theme == 0);
    click(cx + 3 * 90 + 10, cy + 216 + 12);
    CHECK(comp->scale_pct == 200);
    click(cx + 0 * 90 + 10, cy + 216 + 12);
    CHECK(comp->scale_pct == 100);
    click(cx + 300 + 10, cy + 72 + 12);                                 /* High contrast */
    CHECK(st.theme == 2 && comp->theme.surface == 0xFF000000u && be_cfg_bool(&be, "ui.high_contrast", false));
    click(cx + 10, cy + 72 + 12);
    CHECK(st.theme == 0 && !be_cfg_bool(&be, "ui.high_contrast", true));

    /* accessibility mirrors the same switch */
    click(60, 12 + PG_ACCESS * 38 + 16);
    click(cx + 200, cy + 50 + 14);
    CHECK(be_cfg_bool(&be, "ui.high_contrast", false) && comp->theme.surface == 0xFF000000u);
    click(cx + 200, cy + 50 + 14);
    CHECK(!be_cfg_bool(&be, "ui.high_contrast", true));
    click(cx + 200, cy + 130 + 14);
    CHECK(comp->scale_pct == 150);
    click(cx + 200, cy + 130 + 14);
    CHECK(comp->scale_pct == 100);

    /* accounts page renders the default local account */
    click(60, 12 + PG_ACCOUNTS * 38 + 16);
    CHECK(st.page == PG_ACCOUNTS);
    click(cx + 200, cy + 224 + 14); CHECK(be_cfg_bool(&be, "login.auto", false));

    /* persistence round trip: a fresh backend loaded from text reproduces rules, grants and config */
    static char saved[8192];
    size_t n = backend_mem_save(&mem, saved, sizeof saved);
    CHECK(n > 50 && strstr(saved, "updates.mirror=https://mirror.home.lan/loos") && strstr(saved, "browser.exe=deny"));
    backend_mem m2; shell_backend be2; backend_mem_init(&m2, &be2);
    CHECK(backend_mem_load(&m2, saved) >= 8);
    CHECK(be2.fw_check(be2.user, "browser.exe") == FW_DENY && be_cfg_bool(&be2, "login.auto", false) && be_cfg_bool(&be2, "updates.enabled", false));
    shell_settings s2; settings_init(&s2, &be2, NULL, NULL);
    CHECK(!strcmp(s2.mirror, "https://mirror.home.lan/loos") && s2.theme == 0);

    /* launcher deep links */
    settings_page pg;
    CHECK(settings_page_by_id("settings:firewall", &pg) && pg == PG_NETWORK && settings_page_by_id("settings:appearance", &pg) && pg == PG_APPEARANCE);
    CHECK(!settings_page_by_id("settings:nope", &pg) && !settings_page_by_id("calc.exe", &pg));

    /* it paints real pixels into the window and the compositor shows them */
    comp_render_full(comp, &fb);
    shell_rect cr = comp_client_rect(comp, comp_get(comp, aw.win));
    int lit = 0; for (int i = 0; i < 4000; i += 7) if (screen[(cr.y + 100) * SW + cr.x + i % CW] != comp->theme.surface) lit++;
    CHECK(lit > 10);
    if (getenv("LOOS_SHELL_PPM")) {
        click(60, 12 + PG_NETWORK * 38 + 16); click(cx + 40, cy + 100 + 12); comp_render_full(comp, &fb);
        FILE *f = fopen(getenv("LOOS_SHELL_PPM"), "wb");
        if (f) { fprintf(f, "P6\n%d %d\n255\n", SW, SH); for (int k = 0; k < SW * SH; k++) { fputc(screen[k] >> 16, f); fputc(screen[k] >> 8, f); fputc(screen[k], f); } fclose(f); }
    }
    appwin_close(&aw); CHECK(comp_get(comp, aw.win) == NULL);
    free(comp);
    DONE("settings_test");
}
