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

/* Rust apps: frame functions live in crates/loos-shell/src/apps */
void *loos_notepad_new(const shell_backend *be, const char *path);
void loos_notepad_frame(void *app, ui_ctx *u, int w, int h);
void *loos_term_new(const shell_backend *be);
void loos_term_frame(void *app, ui_ctx *u, int w, int h);
bool loos_term_take_exit(void *app);
void *loos_calc_new(void);
void loos_calc_frame(void *app, ui_ctx *u, int w, int h);
void *loos_browser_new(const shell_backend *be, const char *url);
void loos_browser_frame(void *app, ui_ctx *u, int w, int h);
bool loos_browser_navigate_to(void *app, const char *url);

static shell_rect centered(const shell_desktop *d, int w, int h, int nudge)
{
    int sc = (int)d->comp.scale_pct, lw = comp_lw(&d->comp), lh = comp_lh(&d->comp) - TASKBAR_H;
    w = clampi(w, 200, lw - 16); h = clampi(h, 160, lh - 16);
    int x = clampi((lw - w) / 2 + nudge, 0, lw - w), y = clampi((lh - h) / 2 + nudge / 2, 0, lh - h);
    return (shell_rect){ x * sc / 100, y * sc / 100, w * sc / 100, h * sc / 100 };
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
static void on_quick(int action, uint32_t value, void *user)
{
    shell_desktop *d = user;
    if (action == QS_SETTINGS) open_settings(d);
    else settings_quick(&d->settings, action, value);
}
static void on_net_toggle(bool off, void *user) { shell_desktop *d = user; d->be.net_hard_off_set(d->be.user, off); settings_apply(&d->settings); }
static void open_tasks(shell_desktop *d)
{
    if (d->aw_tasks.comp && !d->aw_tasks.closed) { focus_existing(d, d->aw_tasks.win); return; }
    memset(&d->aw_tasks, 0, sizeof d->aw_tasks);
    appwin_open(&d->aw_tasks, &d->comp, "Task Manager", arrange(d, 2), 0, 102, d->surf_tasks, d->fb.w, d->fb.h, tasks_cb, &d->tasks);
}

static void open_rust_app(shell_desktop *d, shell_app_window *aw, uint32_t **surf, void **app, const char *title, shell_rect r, uint32_t owner,
                          void (*frame)(void *, ui_ctx *, int, int))
{
    if (aw->comp && !aw->closed) { focus_existing(d, aw->win); return; }
    if (!*surf && d->alloc) *surf = d->alloc((size_t)d->fb.w * (size_t)d->fb.h * sizeof(uint32_t), d->alloc_aux);
    if (!*surf || !*app) return;
    memset(aw, 0, sizeof *aw);
    appwin_open(aw, &d->comp, title, r, 0, owner, *surf, d->fb.w, d->fb.h, frame, *app);
}
static void open_notepad(shell_desktop *d)
{
    if (!d->notepad) d->notepad = loos_notepad_new(&d->be, "/home/user/README.txt");
    open_rust_app(d, &d->aw_notepad, &d->surf_notepad, &d->notepad, "Notepad", centered(d, 720, 480, 40), 103, loos_notepad_frame);
}
static void open_term(shell_desktop *d)
{
    if (!d->term) d->term = loos_term_new(&d->be);
    open_rust_app(d, &d->aw_term, &d->surf_term, &d->term, "LoCmd", centered(d, 760, 440, -60), 104, loos_term_frame);
}
static void open_calc(shell_desktop *d)
{
    if (!d->calc) d->calc = loos_calc_new();
    open_rust_app(d, &d->aw_calc, &d->surf_calc, &d->calc, "Calculator", centered(d, 320, 480, 200), 105, loos_calc_frame);
}
static void open_browser(shell_desktop *d)
{
    if (!d->browser) d->browser = loos_browser_new(&d->be, "loos://home");
    open_rust_app(d, &d->aw_browser, &d->surf_browser, &d->browser, "LoOS Browser", centered(d, 1000, 720, 0), 106, loos_browser_frame);
}
/* Other apps can hand a local page to the browser (used by Help and by the file manager later). */
void desktop_open_url(shell_desktop *d, const char *url)
{
    open_browser(d);
    if (d->browser && url && *url) loos_browser_navigate_to(d->browser, url);
    if (d->aw_browser.comp && !d->aw_browser.closed) { comp_raise(&d->comp, d->aw_browser.win); comp_focus(&d->comp, d->aw_browser.win); }
}

static void launch(const shell_item *it, void *user)
{
    shell_desktop *d = user;
    settings_page pg;
    if (it->kind == ITEM_SETTING && settings_page_by_id(it->target, &pg)) { d->settings.page = pg; open_settings(d); return; }
    if (!strcmp(it->target, "loos-taskmgr.exe")) { open_tasks(d); return; }
    if (!strcmp(it->target, "loos-notepad.exe")) { open_notepad(d); return; }
    if (!strcmp(it->target, "loos-cmd.exe")) { open_term(d); return; }
    if (!strcmp(it->target, "calc.exe")) { open_calc(d); return; }
    if (!strcmp(it->target, "loos-browser.exe")) { open_browser(d); return; }
    if (!strcmp(it->target, "files")) { open_files(d); return; }
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
        { "/home/user/.config", 1, 0, 1700000400 }, { "/home/user/Web", 1, 0, 1700000500 },
        { "/etc", 1, 0, 1700000000 }, { "/etc/loos.conf", 0, 220, 1700000000 },
    };
    for (unsigned i = 0; i < sizeof nodes / sizeof *nodes; i++) backend_mem_add_node(m, nodes[i].p, nodes[i].dir, nodes[i].sz, nodes[i].t, 0);
    static const char readme[] =
        "Welcome to LoOS.\n\n"
        "Everything stays on this computer: no telemetry, no account, no cloud.\n\n"
        "Try these:\n"
        "  - Super opens Start. Type to search apps, files and settings.\n"
        "  - LoCmd is the terminal. Type HELP. Windows paths work too: cd C:\\Users\\user\n"
        "  - Notepad edits text files. Ctrl+S saves.\n"
        "  - LoOS Browser opens local pages. Web addresses explain why they are offline.\n"
        "  - Click the clock for a calendar, the tray light for Quick Settings.\n"
        "  - Settings > Personalize changes colors, taskbar, icons and wallpaper.\n";
    d->be.fs_write(d->be.user, "/home/user/README.txt", readme, sizeof readme - 1);
    static const char web_index[] =
        "<html><head><title>LoOS Local Site</title></head><body>\n"
        "<h1>LoOS local site</h1>\n"
        "<p>This sample site lives on the LoOS disk. It exists so the browser can be tested without a network.</p>\n"
        "<h2>Pages</h2><ul>\n"
        "<li><a href=\"guide.html\">Browser guide</a> - tabs, history and bookmarks</li>\n"
        "<li><a href=\"privacy.html\">Privacy</a> - what this browser does and does not do</li>\n"
        "</ul>\n"
        "<h2>Local-first</h2>\n"
        "<p>No accounts, no trackers, no remote start page. Bookmarks and history are files under "
        "<b>/home/user/.config/loos-browser</b>.</p>\n"
        "<h2>An external link</h2>\n"
        "<p><a href=\"https://example.com\">example.com</a> opens an offline explanation page; no request is made.</p>\n"
        "<pre>local site /home/user/Web\n"
        "offline by design</pre>\n"
        "</body></html>\n";
    static const char web_guide[] =
        "<html><head><title>Browser Guide</title></head><body>\n"
        "<h1>Browser guide</h1>\n"
        "<p>The address bar accepts internal pages, local paths and web addresses.</p>\n"
        "<h2>Keys</h2><ul>\n"
        "<li><b>Ctrl+L</b> - focus the address bar</li>\n"
        "<li><b>Ctrl+T</b> / <b>Ctrl+W</b> - new / close tab</li>\n"
        "<li><b>Ctrl+R</b> - reload; <b>Ctrl+D</b> - bookmark this page</li>\n"
        "<li><b>Alt+Left</b> / <b>Alt+Right</b> - back / forward</li>\n"
        "<li><b>Page Up</b> / <b>Page Down</b> - scroll</li>\n"
        "</ul>\n"
        "<h2>Addresses</h2><ul>\n"
        "<li><b>loos://home</b> - start page</li>\n"
        "<li><b>loos://bookmarks</b> and <b>loos://history</b></li>\n"
        "<li><b>/home/user/Web/index.html</b> or <b>file:///home/user/Web/index.html</b></li>\n"
        "<li>A bare word searches bookmarks and /home/user/Web locally.</li>\n"
        "</ul>\n"
        "<p><a href=\"index.html\">Back to the local site</a></p>\n"
        "</body></html>\n";
    static const char web_privacy[] =
        "<html><head><title>Privacy in this browser</title></head><body>\n"
        "<h1>Privacy</h1>\n"
        "<p>LoOS Browser has no socket code. It cannot send a request, a telemetry ping or a crash report.</p>\n"
        "<ul><li>No network: HTTP and HTTPS show a local explanation page.</li>\n"
        "<li>No telemetry and no accounts.</li>\n"
        "<li>History and bookmarks are plain files you can delete.</li>\n"
        "<li>No remote search provider. Search is local.</li></ul>\n"
        "<blockquote>Nothing leaves the machine unless the user started it, and this build cannot start it at all.</blockquote>\n"
        "<p><a href=\"index.html\">Back</a></p>\n"
        "</body></html>\n";
    d->be.fs_write(d->be.user, "/home/user/Web/index.html", web_index, sizeof web_index - 1);
    d->be.fs_write(d->be.user, "/home/user/Web/guide.html", web_guide, sizeof web_guide - 1);
    d->be.fs_write(d->be.user, "/home/user/Web/privacy.html", web_privacy, sizeof web_privacy - 1);
    backend_mem_add_proc(m, 1, 0, "system", 12, 18ull << 20, 5);
    backend_mem_add_proc(m, 4, 1, "loos-shell", 4, 42ull << 20, 12);
    backend_mem_add_proc(m, 9, 4, "loos-notepad.exe", 2, 16ull << 20, 0);
    backend_mem_add_proc(m, 12, 1, "loos-netd", 3, 9ull << 20, 1);
    backend_mem_add_app(m, "loos-notepad.exe");
    backend_mem_add_app(m, "calc.exe");
    backend_mem_add_app(m, "loos-browser.exe");
}

bool desktop_init(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux)
{
    if (w < 640 || h < 400 || w > 8192 || h > 8192) return false;
    void (*log)(const char *, void *) = d->log; void *laux = d->aux;
    memset(d, 0, sizeof *d);
    d->log = log; d->aux = laux;
    d->alloc = alloc; d->alloc_aux = aux;
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
    taskbar_configure(&d->taskbar, be_cfg_bool(&d->be, "ui.taskbar.center", true), be_cfg_bool(&d->be, "ui.taskbar.labels", false),
                      be_cfg_bool(&d->be, "ui.taskbar.top", false), be_cfg_int(&d->be, "ui.taskbar.size", 1));
    d->taskbar.on_quick = on_quick; d->taskbar.on_net_toggle = on_net_toggle; d->taskbar.user = d;
    comp_set_wallpaper_cache(c, alloc(surf, aux));                     /* NULL is fine: wallpaper is then drawn live */
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
    shell_app_window *all[7] = { &d->aw_files, &d->aw_settings, &d->aw_tasks, &d->aw_notepad, &d->aw_term, &d->aw_calc, &d->aw_browser };
    for (int i = 0; i < 7; i++) if (all[i]->comp && !all[i]->closed) appwin_pump(all[i]);
    if (d->term && d->aw_term.comp && !d->aw_term.closed && loos_term_take_exit(d->term)) appwin_close(&d->aw_term);
    taskbar_set_time(&d->taskbar, hour, minute);
}

void desktop_set_date(shell_desktop *d, int day, int month, int year) { taskbar_set_date(&d->taskbar, day, month, year); }

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
