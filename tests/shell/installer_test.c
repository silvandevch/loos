#include "installer.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>

#define W 1280
#define H 800
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };
static shell_theme th;
static ui_ctx u;
static shell_installer in;

/* ---- in-memory backend ---- */
static struct { inst_config cfg; int begun, steps, fail_begin, fail_step, rebooted; } mock;
static int be_list(void *x, inst_disk *out, int max)
{
    (void)x; (void)max;
    memset(out, 0, 3 * sizeof *out);
    strcpy(out[0].name, "Disk 1"); strcpy(out[0].model, "USB stick (install media)"); out[0].bytes = 4ull << 30; out[0].boot_media = true;
    strcpy(out[1].name, "Disk 2"); strcpy(out[1].model, "Tiny"); out[1].bytes = 512ull << 20; out[1].too_small = true;
    strcpy(out[2].name, "Disk 3"); strcpy(out[2].model, "QEMU HARDDISK"); out[2].bytes = 64ull << 30;
    return 3;
}
static bool be_begin(void *x, const inst_config *cfg, char *err, size_t cap)
{
    (void)x;
    if (mock.fail_begin) { snprintf(err, cap, "disk is write protected"); return false; }
    mock.cfg = *cfg; mock.begun++; mock.steps = 0;
    return true;
}
static int be_step(void *x, int *pct, char *msg, size_t mc, char *err, size_t ec)
{
    (void)x;
    mock.steps++;
    *pct = mock.steps * 20;
    snprintf(msg, mc, "Step %d", mock.steps);
    if (mock.fail_step && mock.steps == 3) { snprintf(err, ec, "write error at sector 42"); return -1; }
    return mock.steps >= 5 ? 1 : 0;
}
static void be_reboot(void *x) { (void)x; mock.rebooted++; }
static const shell_inst_backend be = { be_list, be_begin, be_step, be_reboot, 0 };

/* ---- helpers ---- */
static void frame(void) { ui_begin(&u, &fb, &th, 100); installer_frame(&in, &u, W, H); ui_end(&u); }
static void ev(shell_event_type t, int x, int y, int button) { shell_event e = { t, x, y, button, 0, 0 }; ui_feed(&u, &e); }
static const ui_a11y_item *find(const char *name)
{
    for (int i = 0; i < ui_a11y_count(&u); i++) { const ui_a11y_item *it = ui_a11y_get(&u, i); if (!strcmp(it->name, name)) return it; }
    return NULL;
}
static bool click(const char *name)
{
    frame();
    const ui_a11y_item *it = find(name);
    if (!it) return false;
    int x = it->rect.x + it->rect.w / 2, y = it->rect.y + it->rect.h / 2;
    ev(EV_MOUSE_MOVE, x, y, 0); frame();
    ev(EV_MOUSE_DOWN, x, y, BTN_LEFT); frame();
    ev(EV_MOUSE_UP, x, y, BTN_LEFT); frame();
    frame();                                                       /* settle: the next page's widgets are now registered */
    return true;
}
static void type(const char *field, const char *s)
{
    click(field);
    for (; *s; s++) { shell_event e = { EV_CHAR, 0, 0, 0, (uint32_t)(unsigned char)*s, 0 }; ui_feed(&u, &e); }
    frame();
}

int main(void)
{
    /* validation rules */
    CHECK(installer_valid_username("ada") && installer_valid_username("a1_b-c") && !installer_valid_username("Ada") && !installer_valid_username("1ada"));
    CHECK(!installer_valid_username("") && !installer_valid_username("root") && !installer_valid_username("admin") && !installer_valid_username("a b"));
    CHECK(installer_valid_hostname("loos-pc") && installer_valid_hostname("A1") && !installer_valid_hostname("-x") && !installer_valid_hostname("x-") && !installer_valid_hostname("a_b") && !installer_valid_hostname(""));
    CHECK(installer_password_problem("short", "short") != NULL);                  /* too short */
    CHECK(installer_password_problem("longenough1", "different1") != NULL);        /* mismatch */
    CHECK(installer_password_problem("onlyletters", "onlyletters") != NULL);       /* one character class */
    CHECK(installer_password_problem("correct horse9", "correct horse9") == NULL);
    char sug[32]; installer_suggest_username("Ada Lovelace", sug, sizeof sug); CHECK(!strcmp(sug, "ada"));
    installer_suggest_username("  Émile", sug, sizeof sug); CHECK(sug[0] == 0 || installer_valid_username(sug));
    installer_suggest_username("J. R. Hacker-9", sug, sizeof sug); CHECK(installer_valid_username(sug) || !sug[0]);

    shell_theme_dark(&th, 0xFF3B82F6u);
    installer_init(&in, &be);
    frame();
    CHECK(in.page == IP_WELCOME && find("Install LoOS") && find("Try LoOS without installing") && !find("Continue"));
    CHECK(in.net_blocked && !in.updates);                                         /* private defaults */

    /* disk page: the boot media and the too-small disk are never selectable */
    CHECK(click("Install LoOS") && in.page == IP_DISK && in.ndisks == 3);
    CHECK(in.sel_disk == 2 && find("Continue") != NULL);
    { ui_a11y_item *dummy = NULL; (void)dummy; }
    in.sel_disk = -1; frame(); CHECK(!find("Continue"));                          /* nothing valid selected: Continue is disabled */
    in.sel_disk = 2; frame();
    CHECK(click("Back") && in.page == IP_WELCOME);
    CHECK(click("Install LoOS") && click("Continue") && in.page == IP_ACCOUNT);

    /* account page: Continue stays disabled until everything validates; the username follows the name */
    CHECK(!find("Continue"));
    type("Ada Lovelace", "Ada Lovelace");
    CHECK(!strcmp(in.display, "Ada Lovelace") && !strcmp(in.user, "ada"));
    type("At least 8 characters", "weak");
    type("Repeat the password", "weak");
    CHECK(!find("Continue"));
    for (int i = 0; i < 4; i++) { shell_event e = { EV_KEY_DOWN, 0, 0, 0, KEY_BACKSPACE, 0 }; ui_feed(&u, &e); frame(); }      /* clear confirm */
    click("At least 8 characters");
    for (int i = 0; i < 4; i++) { shell_event e = { EV_KEY_DOWN, 0, 0, 0, KEY_BACKSPACE, 0 }; ui_feed(&u, &e); frame(); }      /* clear password */
    CHECK(in.pw[0] == 0 && in.pw2[0] == 0);
    type("At least 8 characters", "correct horse9");
    type("Repeat the password", "correct horse8");
    CHECK(!find("Continue"));                                                     /* mismatch */
    shell_event bs = { EV_KEY_DOWN, 0, 0, 0, KEY_BACKSPACE, 0 }; ui_feed(&u, &bs); frame();
    shell_event ch = { EV_CHAR, 0, 0, 0, '9', 0 }; ui_feed(&u, &ch); frame();
    CHECK(!strcmp(in.pw, in.pw2) && find("Continue"));
    type("ada", "x");                                                             /* editing the username stops auto-suggest */
    CHECK(!strcmp(in.user, "adax") && in.user_edited);
    type("loos-pc", "!");                                                         /* invalid hostname blocks Continue */
    CHECK(!find("Continue"));
    shell_event bs2 = { EV_KEY_DOWN, 0, 0, 0, KEY_BACKSPACE, 0 }; ui_feed(&u, &bs2); frame();
    CHECK(find("Continue"));
    CHECK(click("Continue") && in.page == IP_PRIVACY);

    /* privacy defaults, then the review page */
    CHECK(click("Continue") && in.page == IP_REVIEW && find("Erase disk and install"));
    CHECK(click("Back") && in.page == IP_PRIVACY && click("Continue") && in.page == IP_REVIEW);

    /* failing start reports the backend's error and can retry */
    mock.fail_begin = 1;
    CHECK(click("Erase disk and install") && in.page == IP_ERROR && strstr(in.err, "write protected"));
    mock.fail_begin = 0;
    CHECK(click("Back to review") && in.page == IP_REVIEW);

    /* success: config handed to the backend, progress shown, then the done page */
    CHECK(click("Erase disk and install") && mock.begun == 1);
    CHECK(mock.cfg.disk == 2 && !strcmp(mock.cfg.display, "Ada Lovelace") && !strcmp(mock.cfg.username, "adax") &&
          !strcmp(mock.cfg.hostname, "loos-pc") && !strcmp(mock.cfg.password, "correct horse9") && mock.cfg.net_blocked && !mock.cfg.updates);
    CHECK(in.page == IP_INSTALL || in.page == IP_DONE);
    for (int i = 0; i < 10 && in.page == IP_INSTALL; i++) frame();
    CHECK(in.page == IP_DONE && in.percent == 100);
    frame();
    CHECK(find("Restart now") && click("Restart now") && mock.rebooted == 1);

    /* failure mid-install lands on the error page */
    installer_init(&in, &be); in.page = IP_REVIEW; in.sel_disk = 2; strcpy(in.display, "Ada"); strcpy(in.user, "ada"); strcpy(in.host, "h"); strcpy(in.pw, "abcdefg1"); strcpy(in.pw2, "abcdefg1");
    mock.fail_step = 1;
    CHECK(click("Erase disk and install"));
    for (int i = 0; i < 10 && in.page == IP_INSTALL; i++) frame();
    CHECK(in.page == IP_ERROR && strstr(in.err, "sector 42"));

    /* "try without installing" is a request the host acts on */
    installer_init(&in, &be); frame();
    CHECK(click("Try LoOS without installing") && in.live_requested);

    /* the password field never exposes the text to the accessibility tree */
    installer_init(&in, &be); in.page = IP_ACCOUNT; frame();
    type("At least 8 characters", "secret-pass1");
    { const ui_a11y_item *it = find("At least 8 characters"); CHECK(it && it->value[0] == 0); }
    { char dump[4096]; ui_a11y_dump(&u, dump, sizeof dump); CHECK(!strstr(dump, "secret-pass1")); }

    /* renders at other sizes and scales without touching memory outside the surface */
    for (int sc = 100; sc <= 200; sc += 50) {
        shell_fb f2 = { pixels, W, H, W, sc };
        installer_init(&in, &be); in.page = IP_DISK; in.ndisks = 0;
        ui_begin(&u, &f2, &th, 100); installer_frame(&in, &u, W * 100 / sc, H * 100 / sc); ui_end(&u);
    }
    DONE("installer_test");
}
