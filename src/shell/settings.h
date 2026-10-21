/* The LoOS Settings app (privacy control center). One window, sidebar of pages, every control writes through
 * the local backend and applies live (theme, scale, hard network switch), so the taskbar LED/switch and this app
 * can never disagree. */
#ifndef LOOS_SHELL_SETTINGS_H
#define LOOS_SHELL_SETTINGS_H
#include "ui.h"
#include "backend.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum { PG_PRIVACY, PG_NETWORK, PG_PERMISSIONS, PG_UPDATES, PG_APPEARANCE, PG_ACCESS, PG_ACCOUNTS, PG_COUNT } settings_page;
extern const char *const settings_page_name[PG_COUNT];
extern const uint32_t settings_accents[6];

typedef struct {
    const shell_backend *be;
    shell_comp *comp;                  /* theme/scale are applied here; may be NULL in tests */
    shell_taskbar *taskbar;            /* hard-off switch is mirrored here; may be NULL */
    settings_page page;
    /* per-page widget state */
    int conn_sel, conn_scroll, app_sel, app_scroll, account_sel, account_scroll;
    char mirror[96]; int mirror_caret;
    int theme;                         /* 0 dark, 1 light, 2 high contrast */
    int scale;                         /* percent */
    int accent;                        /* index into settings_accents */
    net_conn conns[32]; int nconns;
    char apps[MEM_MAX_APPS][32]; int napps;
    int changes;                       /* count of applied changes, observable by the shell/tests */
} shell_settings;

void settings_init(shell_settings *s, const shell_backend *be, shell_comp *comp, shell_taskbar *tb);
void settings_apply(shell_settings *s);                         /* push persisted values into compositor/taskbar */
void settings_frame(shell_settings *s, ui_ctx *ui, int w, int h);   /* draw + handle one frame inside a w x h client area */
shell_net_state settings_net_state(const shell_settings *s);
bool settings_page_by_id(const char *id, settings_page *out);       /* "settings:firewall" etc. from the launcher */

#ifdef __cplusplus
}
#endif
#endif
