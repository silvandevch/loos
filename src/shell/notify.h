/* Notification service + center. Local sources only: the API takes an app name and text, never a URL or a push
 * channel. Every notification is gated by the per-app Notifications permission (ask on first use), rate limited
 * per app, and kept in a history the user can clear. */
#ifndef LOOS_SHELL_NOTIFY_H
#define LOOS_SHELL_NOTIFY_H
#include "ui.h"
#include "perms.h"

#ifdef __cplusplus
extern "C" {
#endif

#define NOTIFY_MAX 64
#define NOTIFY_PENDING 8
#define NOTIFY_RATE_COUNT 5
#define NOTIFY_RATE_WINDOW_MS 10000
#define NOTIFY_TOAST_MS 5000
#define NOTIFY_MAX_TOASTS 3
typedef enum { NOTE_LOW, NOTE_NORMAL, NOTE_URGENT } note_urgency;

typedef struct {
    uint32_t id;                      /* 0 = free */
    char app[32], title[48], body[120], action[24];
    uint32_t action_id;
    note_urgency urgency;
    uint64_t time_ms;
    bool read;
    uint64_t toast_until;             /* 0 = no toast; URGENT toasts use UINT64_MAX until dismissed */
} shell_note;

typedef struct shell_notify {
    shell_perms *perms;
    shell_comp *comp;
    shell_note note[NOTIFY_MAX];
    struct { char app[32], title[48], body[120], action[24]; uint32_t action_id; note_urgency urgency; } pending[NOTIFY_PENDING]; int npending;
    struct { char app[32]; uint64_t stamp[NOTIFY_RATE_COUNT]; int n; } rate[16]; int nrate;
    uint32_t next_id;
    bool dnd, center_open;
    int scroll, sel;
    uint64_t now_ms;
    uint32_t published, dropped_denied, dropped_rate, dropped_full;
    void (*on_action)(uint32_t note_id, uint32_t action_id, void *ctx);
    void *ctx;
} shell_notify;

void notify_init(shell_notify *n, shell_perms *p, shell_comp *c);
void notify_install(shell_notify *n);
uint32_t notify_publish(shell_notify *n, const char *app, const char *title, const char *body, note_urgency u, const char *action, uint32_t action_id);
void notify_tick(shell_notify *n, uint64_t now_ms);
void notify_dismiss(shell_notify *n, uint32_t id);
void notify_clear_all(shell_notify *n);
void notify_clear_app(shell_notify *n, const char *app);
int  notify_count(const shell_notify *n);
int  notify_unread(const shell_notify *n);
int  notify_visible_toasts(const shell_notify *n);
const shell_note *notify_get(const shell_notify *n, uint32_t id);
void notify_set_center(shell_notify *n, bool open);
void notify_set_dnd(shell_notify *n, bool on);
bool notify_mouse(shell_notify *n, int x, int y, bool down);
bool notify_key(shell_notify *n, uint32_t key, bool down, uint32_t mods);
shell_rect notify_center_rect(const shell_comp *c);
shell_rect notify_bell_rect(const shell_comp *c);

#ifdef __cplusplus
}
#endif
#endif
