/* Kernel host for the portable shell desktop: PS/2 input in, back buffer to the framebuffer out. */
#include "kernel.h"
#include "desktop.h"

static shell_desktop D;                               /* ~1 MB of state: lives in .bss */

static void *host_alloc(size_t n, void *aux) { (void)aux; return kalloc(n, 4096); }
static void host_log(const char *line, void *aux) { (void)aux; serial_puts(line); serial_puts("\n"); }

static void on_input(const struct kin *e, uint32_t *buttons)
{
    (void)buttons;
    if (e->type == KIN_MOUSE) { *buttons = e->buttons; desktop_pointer(&D, e->dx, e->dy, e->buttons); return; }
    desktop_key(&D, e->key, e->ch, e->mods, e->down);
}

static const char *g_accounts_text;

static void feed_inputs(void)
{
    struct kin e; uint32_t buttons = 0;
    for (int n = 0; n < 48 && ps2_poll(&e); n++) on_input(&e, &buttons);
}

/* one frame of the main loop: input, apps, repaint of what changed */
static void host_frame(struct video *v, uint32_t *last_clock, int *h, int *m)
{
    feed_inputs();
    uint32_t now = time_ms();
    if (now - *last_clock > 1000) {
        *last_clock = now; int s; rtc_time(h, m, &s);
        int64_t z = (int64_t)(rtc_unix() / 86400) + 719468, era = z / 146097;      /* civil date from days (Howard Hinnant) */
        unsigned doe = (unsigned)(z - era * 146097), yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        unsigned doy = doe - (365 * yoe + yoe / 4 - yoe / 100), mp = (5 * doy + 2) / 153;
        int mon = (int)(mp < 10 ? mp + 3 : mp - 9);
        desktop_set_date(&D, (int)(doy - (153 * mp + 2) / 5 + 1), mon, (int)(yoe + era * 400 + (mon <= 2)));
    }
    desktop_tick(&D, now, *h, *m);
    shell_rect r = desktop_render(&D);
    if (r.w > 0) video_present(v, r.x, r.y, r.w, r.h);
}

static void report(const struct video *v, const char *what)
{
    uint64_t drawn = 0;
    for (int i = 0; i < v->w * v->h; i++) drawn += v->back[i] != D.comp.theme.desktop;
    serial_puts(what); serial_puts(": "); serial_dec((uint64_t)D.comp.nwin); serial_puts(" windows at "); serial_dec((uint64_t)v->w);
    serial_puts("x"); serial_dec((uint64_t)v->h); serial_puts(", UI scale "); serial_dec(D.comp.scale_pct); serial_puts("%, ");
    serial_dec(drawn * 100 / ((uint64_t)v->w * (uint64_t)v->h)); serial_puts("% of the screen drawn\n");
}

static void settle(struct video *v)
{
    int h = 12, m = 0;
    desktop_tick(&D, 0, h, m); desktop_render_all(&D);
    desktop_tick(&D, 0, h, m); desktop_render_all(&D);
    (void)v;
}

int shellhost_run(const struct loos_bootinfo *bi, struct video *v, int test_mode, int scale_pct)
{
    (void)bi;
    D.log = host_log;
    if (!desktop_init(&D, v->back, v->w, v->h, scale_pct, host_alloc, 0)) return -1;
    time_init();
    if (g_accounts_text) desktop_apply_config(&D, g_accounts_text);
    settle(v);
    report(v, "desktop");
    return test_mode ? 0 : 1;                          /* 1 = caller should fade in, then call shellhost_loop() */
}

void shellhost_set_config(const char *text) { g_accounts_text = text; }

void shellhost_loop(struct video *v)
{
    ps2_init();
    serial_puts("desktop: running ("); serial_puts(ps2_has_mouse() ? "mouse+keyboard" : "keyboard only"); serial_puts(")\n");
    uint32_t last_clock = 0;
    int h = 0, m = 0, s = 0;
    rtc_time(&h, &m, &s);
    for (;;) { host_frame(v, &last_clock, &h, &m); __asm__ volatile("pause"); }
}

/* installer: returns 1 if the user asked to try LoOS live, 0 never (the installer ends by rebooting) */
int shellhost_installer(struct video *v, int scale_pct, int test_mode)
{
    D.log = host_log;
    if (!desktop_init_installer(&D, v->back, v->w, v->h, scale_pct, host_alloc, 0, install_backend())) return -1;
    settle(v);
    report(v, "installer");
    video_present(v, 0, 0, v->w, v->h);
    if (test_mode) return 0;
    ps2_init();
    uint32_t last_clock = 0; int h = 0, m = 0, s = 0;
    rtc_time(&h, &m, &s);
    while (!desktop_installer_wants_live(&D)) { host_frame(v, &last_clock, &h, &m); __asm__ volatile("pause"); }
    return 1;
}

/* login screen for an installed system; returns 0 once someone signed in (user name copied out) */
int shellhost_login(struct video *v, int scale_pct, const char *accounts_text, char *user, size_t cap, int test_mode)
{
    D.log = host_log;
    if (!desktop_init_login(&D, v->back, v->w, v->h, scale_pct, host_alloc, 0, accounts_text, 10000)) return -1;
    settle(v);
    report(v, "login");
    video_present(v, 0, 0, v->w, v->h);
    if (test_mode) return 0;
    ps2_init();
    uint32_t last_clock = 0; int h = 0, m = 0, s = 0;
    rtc_time(&h, &m, &s);
    while (!desktop_login_done(&D, user, cap)) { host_frame(v, &last_clock, &h, &m); __asm__ volatile("pause"); }
    return 0;
}

int shellhost_login_selftest(const char *user, const char *password)
{
    uint32_t retry = 0;
    auth_result bad = session_login(&D.session, user, "definitely-not-the-password", 1000, &retry);
    serial_puts(bad == AUTH_BAD_PASSWORD ? "login: wrong password rejected\n" : "login: WRONG PASSWORD ACCEPTED\n");
    auth_result good = session_login(&D.session, user, password, 2000, &retry);
    if (good == AUTH_OK) { serial_puts("login: ok "); serial_puts(user); serial_puts("\n"); return 0; }
    serial_puts("login: correct password refused\n");
    return -1;
}
