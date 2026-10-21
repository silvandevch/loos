/* The LoOS desktop as one portable object: compositor, window manager, taskbar, launcher and the built-in apps drawing
 * into a caller-owned back buffer. No hardware access: the kernel feeds it PS/2 input and presents the back buffer, the
 * host snapshot tool feeds scripted input and writes PNM files, so both exercise identical code. */
#ifndef LOOS_SHELL_DESKTOP_H
#define LOOS_SHELL_DESKTOP_H
#include "filemgr.h"
#include "settings.h"
#include "taskmon.h"
#include "apphost.h"
#include "installer.h"
#include "session.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum { DESKTOP_NORMAL, DESKTOP_INSTALLER, DESKTOP_LOGIN } desktop_mode;

typedef struct shell_desktop {
    desktop_mode mode;
    shell_installer installer;                       /* installer / login modes: one full-screen app, no taskbar */
    shell_app_window aw_installer;
    shell_accounts accounts;
    shell_session session;
    shell_login login;
    shell_app_window aw_login;
    shell_comp comp;
    shell_wm wm;
    shell_inventory inv;
    shell_launcher launcher;
    shell_taskbar taskbar;
    backend_mem mem;
    shell_backend be;
    shell_settings settings;
    shell_filemgr files;
    shell_taskmon tasks;
    shell_app_window aw_files, aw_settings, aw_tasks;
    shell_fb fb;
    uint32_t *surf_files, *surf_settings, *surf_tasks;
    int mx, my;
    uint32_t buttons;
    void (*log)(const char *line, void *aux);       /* optional diagnostics sink */
    void *aux;
} shell_desktop;

/* UI scale (percent) that gives a comfortable size at this resolution: 100 up to ~1440p, 150 around 1440p-1800p, 200 at 4K. */
int desktop_auto_scale(int w, int h);

/* Initialise on a w x h back buffer. scale_pct 0 = automatic. alloc supplies the (large, uninitialised-ok) app surfaces:
 * one device-resolution surface per app. Returns false if allocation fails or the size is unsupported. */
bool desktop_init(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t bytes, void *aux), void *aux);

/* Installer: a full-screen setup wizard on the given backend (no taskbar, no other apps). */
bool desktop_init_installer(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux,
                            const shell_inst_backend *be);
bool desktop_installer_wants_live(const shell_desktop *d);          /* the user chose "Try LoOS without installing" */

/* Login screen for an installed system: `accounts_text` is the saved account list (shell_accounts text format). */
bool desktop_init_login(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux,
                        const char *accounts_text, uint32_t iters);
bool desktop_login_done(const shell_desktop *d, char *user_out, size_t cap);   /* signed in: user name copied out */

/* Applies machine configuration lines (key=value, up to an optional [accounts] section) to the backend and Settings. */
void desktop_apply_config(shell_desktop *d, const char *text);

/* Relative pointer motion plus the current button mask (BTN_*), routed to the taskbar/launcher first, then the compositor. */
void desktop_pointer(shell_desktop *d, int dx, int dy, uint32_t buttons);
/* Absolute pointer position (tablet / tests). */
void desktop_pointer_to(shell_desktop *d, int x, int y, uint32_t buttons);
/* Key press/release; `ch` is the typed character (0 for non-printing keys). */
void desktop_key(shell_desktop *d, uint32_t key, uint32_t ch, uint32_t mods, bool down);
/* Advance time: runs the apps, updates the taskbar clock. now_ms is a monotonic clock. */
void desktop_tick(shell_desktop *d, uint32_t now_ms, int hour, int minute);
/* Repaint what changed into the back buffer; returns the dirty rectangle (device px), empty if nothing changed. */
shell_rect desktop_render(shell_desktop *d);
void desktop_render_all(shell_desktop *d);

#ifdef __cplusplus
}
#endif
#endif
