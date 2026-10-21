#include "desktop.h"
#include <stdio.h>
#include <string.h>

int desktop_auto_scale(int w, int h)
{
    int m = h < w ? h : w;
    if (m >= 2000) return 200;
    if (m >= 1300) return 150;
    return 100;
}

static void files_cb(void *a, ui_ctx *u, int w, int h) { fm_frame(a, u, w, h); }
static void settings_cb(void *a, ui_ctx *u, int w, int h) { settings_frame(a, u, w, h); }
static void tasks_cb(void *a, ui_ctx *u, int w, int h) { tm_frame(a, u, w, h); }

static int clampi(int v, int lo, int hi) { return v < lo ? lo : v > hi ? hi : v; }

/* initial window rectangles, designed in logical px and converted to device px */
static shell_rect arrange(const shell_desktop *d, int which)
{
    int sc = (int)d->comp.scale_pct;
    int lw = comp_lw(&d->comp), lh = comp_lh(&d->comp) - TASKBAR_H;
    shell_rect r;
    switch (which) {
    case 0:  r = (shell_rect){ lw * 3 / 100, lh * 4 / 100, clampi(lw * 40 / 100, 520, 760), clampi(lh * 52 / 100, 360, 560) }; break;
    case 1:  r = (shell_rect){ lw * 36 / 100, lh * 12 / 100, clampi(lw * 42 / 100, 560, 780), clampi(lh * 66 / 100, 440, 640) }; break;
    default: r = (shell_rect){ lw * 6 / 100, lh * 48 / 100, clampi(lw * 46 / 100, 620, 840), clampi(lh * 44 / 100, 320, 460) }; break;
    }
    if (r.x + r.w > lw - 8) r.x = lw - 8 - r.w;
    if (r.y + r.h > lh - 8) r.y = lh - 8 - r.h;
    if (r.x < 0) r.x = 0;
    if (r.y < 0) r.y = 0;
    return (shell_rect){ r.x * sc / 100, r.y * sc / 100, r.w * sc / 100, r.h * sc / 100 };
}

static void focus_existing(shell_desktop *d, int win)
{
    comp_restore(&d->comp, win);
    comp_raise(&d->comp, win);
    comp_focus(&d->comp, win);
}

static void open_files(shell_desktop *d)
{
    if (d->aw_files.comp && !d->aw_files.closed) { focus_existing(d, d->aw_files.win); return; }
    memset(&d->aw_files, 0, sizeof d->aw_files);
    appwin_open(&d->aw_files, &d->comp, "Files", arrange(d, 0), 0, 100, d->surf_files, d->fb.w, d->fb.h, files_cb, &d->files);
}
static void open_settings(shell_desktop *d)
{
    if (d->aw_settings.comp && !d->aw_settings.closed) { focus_existing(d, d->aw_settings.win); return; }
    memset(&d->aw_settings, 0, sizeof d->aw_settings);
    appwin_open(&d->aw_settings, &d->comp, "Settings", arrange(d, 1), 0, 101, d->surf_settings, d->fb.w, d->fb.h, settings_cb, &d->settings);
}
static void open_tasks(shell_desktop *d)
{
    if (d->aw_tasks.comp && !d->aw_tasks.closed) { focus_existing(d, d->aw_tasks.win); return; }
    memset(&d->aw_tasks, 0, sizeof d->aw_tasks);
    appwin_open(&d->aw_tasks, &d->comp, "Task Manager", arrange(d, 2), 0, 102, d->surf_tasks, d->fb.w, d->fb.h, tasks_cb, &d->tasks);
}

static void launch(const shell_item *it, void *user)
{
    shell_desktop *d = user;
    settings_page pg;
    if (it->kind == ITEM_SETTING && settings_page_by_id(it->target, &pg)) { d->settings.page = pg; open_settings(d); return; }
    if (!strcmp(it->target, "loos-taskmgr.exe")) { open_tasks(d); return; }
    if (d->log) {
        char line[200];
        snprintf(line, sizeof line, "launch: %s (no process loader yet)", it->target);
        d->log(line, d->aux);
    }
}

/* The shell's in-memory reference backend filled with demo volumes, files and processes until the real backends exist. */
static void seed_backend(shell_desktop *d)
{
    backend_mem *m = &d->mem;
    backend_mem_init(m, &d->be);
    backend_mem_add_volume(m, "/", "LoOS", "btrfs", 500ull << 30, 320ull << 30, false);
    backend_mem_add_volume(m, "/boot/efi", "ESP", "vfat", 512ull << 20, 400ull << 20, false);
    static const struct { const char *p; int dir; uint64_t sz; int64_t t; } nodes[] = {
        { "/home", 1, 0, 1700000000 }, { "/home/user", 1, 0, 1700000100 }, { "/home/user/Documents", 1, 0, 1700000200 },
        { "/home/user/Documents/report.txt", 0, 2048, 1700001000 }, { "/home/user/Documents/notes.md", 0, 512, 1700002000 },
        { "/home/user/Music", 1, 0, 1700000300 }, { "/home/user/Music/song.ogg", 0, 4ull << 20, 1700003000 },
        { "/home/user/Pictures", 1, 0, 1700000350 }, { "/home/user/README.txt", 0, 1337, 1700009999 },
        { "/etc", 1, 0, 1700000000 }, { "/etc/loos.conf", 0, 220, 1700000000 },
    };
    for (unsigned i = 0; i < sizeof nodes / sizeof *nodes; i++) backend_mem_add_node(m, nodes[i].p, nodes[i].dir, nodes[i].sz, nodes[i].t, 0);
    backend_mem_add_proc(m, 1, 0, "system", 12, 18ull << 20, 5);
    backend_mem_add_proc(m, 4, 1, "loos-shell", 4, 42ull << 20, 12);
    backend_mem_add_proc(m, 9, 4, "loos-notepad.exe", 2, 16ull << 20, 0);
    backend_mem_add_proc(m, 12, 1, "loos-netd", 3, 9ull << 20, 1);
    backend_mem_add_app(m, "loos-notepad.exe");
    backend_mem_add_app(m, "calc.exe");
}

bool desktop_init(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux)
{
    if (w < 640 || h < 400 || w > 8192 || h > 8192) return false;
    void (*log)(const char *, void *) = d->log; void *laux = d->aux;
    memset(d, 0, sizeof *d);
    d->log = log; d->aux = laux;
    d->fb = (shell_fb){ backbuf, w, h, w, 100 };
    size_t surf = (size_t)w * (size_t)h * sizeof(uint32_t);
    d->surf_files = alloc(surf, aux); d->surf_settings = alloc(surf, aux); d->surf_tasks = alloc(surf, aux);
    if (!d->surf_files || !d->surf_settings || !d->surf_tasks) return false;

    shell_comp *c = &d->comp;
    comp_init(c, w, h);
    comp_set_scale(c, (uint32_t)(scale_pct > 0 ? scale_pct : desktop_auto_scale(w, h)));
    wm_init(&d->wm, c);
    wm_install(&d->wm);
    seed_backend(d);
    be_cfg_set_int(&d->be, "ui.scale", (int)c->scale_pct);           /* Settings reads the scale back: keep the two in agreement */
    inventory_defaults(&d->inv);
    launcher_init(&d->launcher, &d->inv, launch, d);
    taskbar_init(&d->taskbar, c, &d->launcher);
    settings_init(&d->settings, &d->be, c, &d->taskbar);
    fm_init(&d->files, &d->be, "/home/user");
    tm_init(&d->tasks, &d->be);
    open_files(d); open_tasks(d); open_settings(d);
    d->mx = w / 2; d->my = h / 2;
    return true;
}

void desktop_pointer_to(shell_desktop *d, int x, int y, uint32_t buttons)
{
    desktop_pointer(d, x - d->mx, y - d->my, buttons);
}

void desktop_pointer(shell_desktop *d, int dx, int dy, uint32_t buttons)
{
    shell_comp *c = &d->comp;
    d->mx = clampi(d->mx + dx, 0, c->screen_w - 1);
    d->my = clampi(d->my + dy, 0, c->screen_h - 1);
    bool shell_ui = d->mode == DESKTOP_NORMAL;                           /* taskbar + window manager exist only on the desktop */
    if (dx || dy) {
        if (shell_ui) taskbar_mouse(&d->taskbar, c, d->mx, d->my, 0, false);          /* hover */
        comp_mouse_move(c, d->mx, d->my);
        if (shell_ui) wm_after_mouse(&d->wm);
    }
    uint32_t changed = buttons ^ d->buttons;
    for (uint32_t b = 1; b <= 4; b <<= 1) {
        if (!(changed & b)) continue;
        bool down = (buttons & b) != 0;
        if (!shell_ui || !taskbar_mouse(&d->taskbar, c, d->mx, d->my, (int)b, down)) { comp_mouse_button(c, (int)b, down); if (shell_ui) wm_after_mouse(&d->wm); }
    }
    d->buttons = buttons;
}

void desktop_key(shell_desktop *d, uint32_t key, uint32_t ch, uint32_t mods, bool down)
{
    shell_comp *c = &d->comp;
    if (down && d->mode == DESKTOP_NORMAL) {
        int was_open = d->launcher.open;
        if (taskbar_key(&d->taskbar, c, key, true, mods)) {       /* modal while the launcher is open: it also gets the typed character */
            if (was_open && d->launcher.open && ch && !(mods & (MOD_CTRL | MOD_ALT | MOD_SUPER))) { launcher_char(&d->launcher, ch); comp_damage_all(c); }
            return;
        }
        if (wm_key(&d->wm, key, true, mods)) return;
    }
    comp_key(c, key, down, mods);
    if (down && ch && !(mods & (MOD_CTRL | MOD_ALT | MOD_SUPER))) comp_char(c, ch);
}

void desktop_tick(shell_desktop *d, uint32_t now_ms, int hour, int minute)
{
    d->comp.now_ms = now_ms;
    if (d->mode == DESKTOP_INSTALLER) {
        if (d->installer.page == IP_INSTALL) d->aw_installer.dirty = true;       /* the progress page advances the install every frame */
        appwin_pump(&d->aw_installer);
        return;
    }
    if (d->mode == DESKTOP_LOGIN) {
        if (d->login.now_ms / 1000 != now_ms / 1000) d->aw_login.dirty = true;     /* clock and lockout countdown tick once a second */
        d->login.now_ms = now_ms; d->login.hour = hour; d->login.minute = minute;
        appwin_pump(&d->aw_login);
        return;
    }
    shell_app_window *all[3] = { &d->aw_files, &d->aw_settings, &d->aw_tasks };
    for (int i = 0; i < 3; i++) if (all[i]->comp && !all[i]->closed) appwin_pump(all[i]);
    taskbar_set_time(&d->taskbar, hour, minute);
}

shell_rect desktop_render(shell_desktop *d)
{
    if (!d->comp.damage.n) return (shell_rect){ 0, 0, 0, 0 };
    shell_rect r = damage_bounds(&d->comp.damage);
    comp_render(&d->comp, &d->fb);
    return rect_intersect(r, (shell_rect){ 0, 0, d->fb.w, d->fb.h });
}
void desktop_render_all(shell_desktop *d) { comp_render_full(&d->comp, &d->fb); }

/* ---------------------------------------------------------------------------------------------- single-app modes */
static void installer_cb(void *a, ui_ctx *u, int w, int h) { installer_frame(a, u, w, h); }
static void login_cb(void *a, ui_ctx *u, int w, int h) { login_frame(a, u, w, h); }

static bool init_fullscreen(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux)
{
    if (w < 640 || h < 400 || w > 8192 || h > 8192) return false;
    void (*log)(const char *, void *) = d->log; void *laux = d->aux;
    memset(d, 0, sizeof *d);
    d->log = log; d->aux = laux;
    d->fb = (shell_fb){ backbuf, w, h, w, 100 };
    d->surf_files = alloc((size_t)w * (size_t)h * sizeof(uint32_t), aux);          /* one surface for the single app */
    if (!d->surf_files) return false;
    comp_init(&d->comp, w, h);
    comp_set_scale(&d->comp, (uint32_t)(scale_pct > 0 ? scale_pct : desktop_auto_scale(w, h)));
    d->mx = w / 2; d->my = h / 2;
    return true;
}

bool desktop_init_installer(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux,
                            const shell_inst_backend *be)
{
    if (!init_fullscreen(d, backbuf, w, h, scale_pct, alloc, aux)) return false;
    d->mode = DESKTOP_INSTALLER;
    installer_init(&d->installer, be);
    return appwin_open(&d->aw_installer, &d->comp, "LoOS Setup", (shell_rect){ 0, 0, w, h }, WF_NOCHROME | WF_NOTASKBAR | WF_NORESIZE, 200,
                       d->surf_files, w, h, installer_cb, &d->installer);
}

bool desktop_installer_wants_live(const shell_desktop *d) { return d->mode == DESKTOP_INSTALLER && d->installer.live_requested; }

bool desktop_init_login(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux,
                        const char *accounts_text, uint32_t iters)
{
    if (!init_fullscreen(d, backbuf, w, h, scale_pct, alloc, aux)) return false;
    d->mode = DESKTOP_LOGIN;
    accounts_init(&d->accounts, iters);
    if (accounts_load(&d->accounts, accounts_text) < 1) return false;
    session_init(&d->session, &d->accounts);
    login_init(&d->login, &d->session);
    return appwin_open(&d->aw_login, &d->comp, "Sign in", (shell_rect){ 0, 0, w, h }, WF_NOCHROME | WF_NOTASKBAR | WF_NORESIZE, 201,
                       d->surf_files, w, h, login_cb, &d->login);
}

bool desktop_login_done(const shell_desktop *d, char *user_out, size_t cap)
{
    if (d->mode != DESKTOP_LOGIN || d->session.state != SESS_ACTIVE) return false;
    if (user_out && cap) { strncpy(user_out, d->session.user, cap - 1); user_out[cap - 1] = 0; }
    return true;
}

void desktop_apply_config(shell_desktop *d, const char *text)
{
    if (d->mode != DESKTOP_NORMAL) return;
    for (const char *p = text; *p;) {
        const char *e = p; while (*e && *e != '\n') e++;
        if (*p == '[') break;                                              /* section headers start the account list */
        const char *eq = p; while (eq < e && *eq != '=') eq++;
        if (eq < e && eq > p && e - eq - 1 < 64 && eq - p < 48) {
            char k[48], v[64];
            memcpy(k, p, (size_t)(eq - p)); k[eq - p] = 0;
            memcpy(v, eq + 1, (size_t)(e - eq - 1)); v[e - eq - 1] = 0;
            if (!strcmp(k, "net.hard_off") || !strcmp(k, "updates.enabled") || !strcmp(k, "hostname") || !strcmp(k, "display"))
                d->be.cfg_set(d->be.user, k, v);
        }
        p = *e ? e + 1 : e;
    }
    settings_apply(&d->settings);
}
