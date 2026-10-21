/* Compatibility console: pick the claimed Windows version per app (7/8/8.1/10/11), see why that version is
 * used and which shims it implies, and manage import redirects. Front end for src/win32/compat (compat_db). */
#ifndef LOOS_SHELL_COMPAT_CONSOLE_H
#define LOOS_SHELL_COMPAT_CONSOLE_H
#include "ui.h"
#include "backend.h"
#include "../win32/compat/compat.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    compat_db *db;
    const shell_inventory *inv;
    const char *(*manifest_of)(const char *app, size_t *len, void *user);   /* optional: embedded manifest text */
    void (*persist)(const compat_db *db, void *user);                       /* called after every change */
    void *user;
    int sel, scroll, redir_scroll, redir_sel;
    char r_from[48], r_to[48]; int r_from_caret, r_to_caret;
    char status[96];
    char apps[SHELL_MAX_APPS][COMPAT_NAME_MAX]; int napps;
    int changes;
} shell_compat_console;

void cc_init(shell_compat_console *c, compat_db *db, const shell_inventory *inv);
void cc_refresh(shell_compat_console *c);                         /* rebuild the app list: inventory apps + apps with profiles */
bool cc_set_claim(shell_compat_console *c, const char *app, compat_era era);
void cc_clear_claim(shell_compat_console *c, const char *app);
bool cc_add_redirect(shell_compat_console *c, const char *app, const char *from, const char *to);   /* "dll!sym" syntax */
void cc_describe(const shell_compat_console *c, const char *app, compat_claim *claim, char *shims, size_t cap);
void cc_frame(shell_compat_console *c, ui_ctx *ui, int w, int h);

#ifdef __cplusplus
}
#endif
#endif
