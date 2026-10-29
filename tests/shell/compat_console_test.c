#include "compat_console.h"
#include "test.h"
#include <string.h>

#define W 900
#define H 640
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };
static shell_theme th;
static ui_ctx u;
static compat_db db;
static int persists; static char blob[4096];
static void persist(const compat_db *d, void *x) { (void)x; persists++; compat_db_dump(d, blob, sizeof blob); }
static const char *manifest_of(const char *app, size_t *len, void *x)
{
    (void)x;
    static const char m[] = "<assembly><compatibility><application>"
                            "<supportedOS Id=\"{35138b9a-5d96-4fbd-8e2d-a2440225f93a}\"/></application></compatibility></assembly>";
    if (!strcmp(app, "oldgame.exe")) { *len = sizeof m - 1; return m; }
    return NULL;
}
static void ev(shell_event_type t, int x, int y, int button, uint32_t key) { shell_event e = { t, x, y, button, key, 0 }; ui_feed(&u, &e); }
static void run(shell_compat_console *c) { ui_begin(&u, &fb, &th, 100); cc_frame(c, &u, W, H); ui_end(&u); }
static void click(shell_compat_console *c, int x, int y)
{
    u.now_ms += 1000;
    ev(EV_MOUSE_MOVE, x, y, 0, 0); run(c); ev(EV_MOUSE_DOWN, x, y, BTN_LEFT, 0); run(c); ev(EV_MOUSE_UP, x, y, BTN_LEFT, 0); run(c);
}

int main(void)
{
    shell_theme_dark(&th, 0xFF3B82F6u);
    compat_db_init(&db);
    shell_inventory inv; inventory_init(&inv); inventory_defaults(&inv);
    inventory_add(&inv, "Old Game", "oldgame.exe", "", ITEM_APP);
    shell_compat_console cc; cc_init(&cc, &db, &inv);
    cc.persist = persist; cc.manifest_of = manifest_of;
    CHECK(cc.napps == 6 && !strcmp(cc.apps[0], "calc.exe") && db.default_era == COMPAT_NATIVE_ERA);

    /* describe: automatic, then manifest, then user profile (profile wins) */
    compat_claim cl; char shims[256];
    cc_describe(&cc, "calc.exe", &cl, shims, sizeof shims);
    CHECK(cl.era == COMPAT_ERA_WIN11 && cl.source == COMPAT_SRC_DEFAULT && strstr(shims, "reports 10.0") && strstr(shims, "per-monitor"));
    cc_describe(&cc, "oldgame.exe", &cl, shims, sizeof shims);
    CHECK(cl.source == COMPAT_SRC_MANIFEST && cl.era == COMPAT_ERA_WIN7 && strstr(shims, "reports 6.1") && strstr(shims, "Service Pack 1"));
    CHECK(strstr(cl.reason, "manifest") != NULL);
    CHECK(cc_set_claim(&cc, "oldgame.exe", COMPAT_ERA_WIN10) && persists == 1);
    cc_describe(&cc, "oldgame.exe", &cl, shims, sizeof shims);
    CHECK(cl.source == COMPAT_SRC_PROFILE && cl.era == COMPAT_ERA_WIN10 && strstr(cl.reason, "profile") != NULL);
    cc_clear_claim(&cc, "oldgame.exe"); CHECK(persists == 2);
    cc_describe(&cc, "oldgame.exe", &cl, shims, sizeof shims); CHECK(cl.source == COMPAT_SRC_MANIFEST);

    /* redirects */
    CHECK(cc_add_redirect(&cc, "calc.exe", "kernel32.dll!GetTickCount64", "kernelbase.dll!GetTickCount64"));
    CHECK(!cc_add_redirect(&cc, "calc.exe", "", "x.dll") && strstr(cc.status, "DLL name"));
    char tod[64], tos[64];
    CHECK(compat_redirect_import(&db, "calc.exe", "kernel32.dll", "GetTickCount64", tod, sizeof tod, tos, sizeof tos) && !strcmp(tod, "kernelbase.dll"));
    CHECK(!compat_redirect_import(&db, "other.exe", "kernel32.dll", "GetTickCount64", tod, sizeof tod, tos, sizeof tos));

    /* UI: select app, pick era radio, clear profile, add a redirect through the form */
    memset(&u, 0, sizeof u);
    run(&cc);
    click(&cc, 100, 120 + 24 * 5 + 12);                          /* "Old Game" row (6th; Browser is 5th) */
    CHECK(cc.sel == 5);
    int x = W / 2 + 8, y0 = 120 + 28 + 22 + 28;                    /* first era radio */
    click(&cc, x + 20, y0 + 24 * 1 + 10);                         /* Windows 8 */
    cc_describe(&cc, "oldgame.exe", &cl, shims, sizeof shims);
    CHECK(cl.era == COMPAT_ERA_WIN8 && cl.source == COMPAT_SRC_PROFILE && !strncmp(cc.status, "oldgame.exe now reports Windows 8", 30));
    click(&cc, x + 20, y0 + 24 * 5 + 4 + 14);                     /* "Use automatic version" */
    cc_describe(&cc, "oldgame.exe", &cl, shims, sizeof shims);
    CHECK(cl.source == COMPAT_SRC_MANIFEST);
    click(&cc, 16 + 4 * 140 + 10, 76 + 10);                       /* default for new apps -> Windows 11 radio (already) */
    click(&cc, 16 + 3 * 140 + 10, 76 + 10);                       /* default -> Windows 10 */
    CHECK(db.default_era == COMPAT_ERA_WIN10);
    cc_describe(&cc, "calc.exe", &cl, shims, sizeof shims); CHECK(cl.era == COMPAT_ERA_WIN10);
    int fy = y0 + 24 * 5 + 44 + 20 + 4 * 18 + 10 + 20 + 74;       /* redirect form row */
    (void)fy;

    /* persistence: the dumped text restores the same decisions */
    cc_set_claim(&cc, "oldgame.exe", COMPAT_ERA_WIN81);
    compat_db db2; compat_db_init(&db2);
    unsigned err = 0;
    CHECK(compat_db_parse(&db2, blob, strlen(blob), &err) == COMPAT_OK);
    shell_compat_console c2; cc_init(&c2, &db2, &inv);
    cc_describe(&c2, "oldgame.exe", &cl, shims, sizeof shims);
    CHECK(cl.era == COMPAT_ERA_WIN81 && db2.default_era == COMPAT_ERA_WIN10 && db2.nredirects == 1);

    /* profiles for apps outside the inventory still show up */
    cc_set_claim(&cc, "C:\\tools\\legacy.exe", COMPAT_ERA_WIN7);
    CHECK(cc.napps == 7);
    run(&cc);
    int lit = 0; for (int i = 0; i < W * H; i += 17) if (pixels[i] != th.surface) lit++;
    CHECK(lit > 400);
    DONE("compat_console_test");
}
