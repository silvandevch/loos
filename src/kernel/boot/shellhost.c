/* Kernel host for the portable shell desktop: PS/2 input in, back buffer to the framebuffer out. */
#include "kernel.h"
#include "shellabi.h"
#include <string.h>

#define DESK (desktop_instance())                    /* ~1 MB of state, owned by the Rust shell (.bss) */

static void *host_alloc(size_t n, void *aux) { (void)aux; return kalloc(n, 4096); }
static void host_log(const char *line, void *aux) { (void)aux; serial_puts(line); serial_puts("\n"); }

static void on_input(const struct kin *e, uint32_t *buttons)
{
    (void)buttons;
    if (e->type == KIN_MOUSE) { *buttons = e->buttons; desktop_pointer(DESK, e->dx, e->dy, e->buttons); return; }
    desktop_key(DESK, e->key, e->ch, e->mods, e->down);
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
        desktop_set_date(DESK, (int)(doy - (153 * mp + 2) / 5 + 1), mon, (int)(yoe + era * 400 + (mon <= 2)));
    }
    desktop_tick(DESK, now, *h, *m);
    shell_rect r = desktop_render(DESK);
    if (r.w > 0) video_present(v, r.x, r.y, r.w, r.h);
}

static void report(const struct video *v, const char *what)
{
    uint64_t drawn = 0;
    for (int i = 0; i < v->w * v->h; i++) drawn += v->back[i] != desktop_theme_desktop(DESK);
    serial_puts(what); serial_puts(": "); serial_dec((uint64_t)desktop_window_count(DESK)); serial_puts(" windows at "); serial_dec((uint64_t)v->w);
    serial_puts("x"); serial_dec((uint64_t)v->h); serial_puts(", UI scale "); serial_dec(desktop_scale_pct(DESK)); serial_puts("%, ");
    serial_dec(drawn * 100 / ((uint64_t)v->w * (uint64_t)v->h)); serial_puts("% of the screen drawn\n");
}

static void settle(struct video *v)
{
    int h = 12, m = 0;
    desktop_tick(DESK, 0, h, m); desktop_render_all(DESK);
    desktop_tick(DESK, 0, h, m); desktop_render_all(DESK);
    (void)v;
}

int shellhost_run(const struct loos_bootinfo *bi, struct video *v, int test_mode, int scale_pct)
{
    desktop_set_log(DESK, host_log, 0);
    if (!desktop_init(DESK, v->back, v->w, v->h, scale_pct, host_alloc, 0)) return -1;
    time_init();
    if (g_accounts_text) desktop_apply_config(DESK, g_accounts_text);
    for (const char *p = bi->cmdline; *p; p++) {                /* loos.look=<id> and loos.night=<0-100> (./loos qemu --look tiler) */
        if (p != bi->cmdline && p[-1] != ' ') continue;
        char id[32]; int n = 0;
        if (!strncmp(p, "loos.look=", 10)) { for (p += 10; *p && *p != ' ' && n < 31; p++) id[n++] = *p; id[n] = 0; desktop_apply_look(DESK, id); p--; }
        else if (!strncmp(p, "loos.night=", 11)) { char cfg[32] = "ui.night="; int k = 9; for (p += 11; *p >= '0' && *p <= '9' && k < 30; p++) cfg[k++] = *p; cfg[k] = 0; desktop_apply_config(DESK, cfg); p--; }
    }
    settle(v);
    report(v, "desktop");
    return test_mode ? 0 : 1;                          /* 1 = caller should fade in, then call shellhost_loop() */
}

void shellhost_set_config(const char *text) { g_accounts_text = text; }

extern void loos_net_poll(uint64_t now_ms);

void shellhost_loop(struct video *v)
{
    ps2_init();
    serial_puts("desktop: running ("); serial_puts(ps2_has_mouse() ? "mouse+keyboard" : "keyboard only"); serial_puts(")\n");
    uint32_t last_clock = 0;
    int h = 0, m = 0, s = 0;
    rtc_time(&h, &m, &s);
    for (;;) { host_frame(v, &last_clock, &h, &m); loos_net_poll(time_ms()); __asm__ volatile("pause"); }
}

/* installer: returns 1 if the user asked to try LoOS live, 0 never (the installer ends by rebooting) */
int shellhost_installer(struct video *v, int scale_pct, int test_mode)
{
    desktop_set_log(DESK, host_log, 0);
    if (!desktop_init_installer(DESK, v->back, v->w, v->h, scale_pct, host_alloc, 0, install_backend())) return -1;
    settle(v);
    report(v, "installer");
    video_present(v, 0, 0, v->w, v->h);
    if (test_mode) return 0;
    ps2_init();
    uint32_t last_clock = 0; int h = 0, m = 0, s = 0;
    rtc_time(&h, &m, &s);
    while (!desktop_installer_wants_live(DESK)) { host_frame(v, &last_clock, &h, &m); loos_net_poll(time_ms()); __asm__ volatile("pause"); }
    return 1;
}

/* login screen for an installed system; returns 0 once someone signed in (user name copied out) */
int shellhost_login(struct video *v, int scale_pct, const char *accounts_text, char *user, size_t cap, int test_mode)
{
    desktop_set_log(DESK, host_log, 0);
    if (!desktop_init_login(DESK, v->back, v->w, v->h, scale_pct, host_alloc, 0, accounts_text, 10000)) return -1;
    settle(v);
    report(v, "login");
    video_present(v, 0, 0, v->w, v->h);
    if (test_mode) return 0;
    ps2_init();
    uint32_t last_clock = 0; int h = 0, m = 0, s = 0;
    rtc_time(&h, &m, &s);
    while (!desktop_login_done(DESK, user, cap)) { host_frame(v, &last_clock, &h, &m); __asm__ volatile("pause"); }
    return 0;
}

int shellhost_login_selftest(const char *user, const char *password)
{
    auth_result bad = desktop_login_try(DESK, user, "definitely-not-the-password", 1000);
    serial_puts(bad == AUTH_BAD_PASSWORD ? "login: wrong password rejected\n" : "login: WRONG PASSWORD ACCEPTED\n");
    auth_result good = desktop_login_try(DESK, user, password, 2000);
    if (good == AUTH_OK) { serial_puts("login: ok "); serial_puts(user); serial_puts("\n"); return 0; }
    serial_puts("login: correct password refused\n");
    return -1;
}
