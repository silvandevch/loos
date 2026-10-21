/* Per-app permission broker. Apps ask for camera/microphone/files/notifications/location; the broker answers
 * from the persisted decision, or queues a prompt the user must answer ("Allow once" is session-only, never
 * persisted). Nothing is granted by default. While a sensitive resource is in use the tray shows an indicator. */
#ifndef LOOS_SHELL_PERMS_H
#define LOOS_SHELL_PERMS_H
#include "ui.h"
#include "backend.h"

#ifdef __cplusplus
extern "C" {
#endif

#define PERMS_MAX_REQ 16
#define PERMS_MAX_USE 16
typedef enum { PERM_ANSWER_ALLOW_ONCE, PERM_ANSWER_ALLOW_ALWAYS, PERM_ANSWER_DENY } perm_answer;

typedef struct {
    char app[32]; perm_res res;
    void (*done)(const char *app, perm_res res, bool allowed, void *ctx);
    void *ctx;
} perm_request_t;

typedef struct {
    const shell_backend *be;
    perm_request_t q[PERMS_MAX_REQ]; int nq;            /* q[0] is the prompt on screen */
    struct { char app[32]; perm_res res; } once[32]; int nonce;       /* "allow once" grants for this session */
    struct { char app[32]; perm_res res; } use[PERMS_MAX_USE]; int nuse;
    int prompts_shown, denied_total;
    shell_comp *comp;
} shell_perms;

void perms_init(shell_perms *p, const shell_backend *be, shell_comp *comp);
perm_state perms_check(shell_perms *p, const char *app, perm_res res);       /* ALLOW (incl. allow-once), DENY or ASK */
/* Ask: resolves immediately when decided, otherwise queues a prompt. Returns the immediate verdict, or PERM_ASK if queued.
 * Duplicate requests for the same app+resource share one prompt (callbacks are all invoked). */
perm_state perms_request(shell_perms *p, const char *app, perm_res res,
                         void (*done)(const char *app, perm_res res, bool allowed, void *ctx), void *ctx);
void perms_answer(shell_perms *p, perm_answer a);                              /* answer the current prompt */
bool perms_prompting(const shell_perms *p);
void perms_revoke_all(shell_perms *p, const char *app);                        /* forget every decision for an app */
void perms_end_session(shell_perms *p);                                        /* drops allow-once grants */

/* in-use indicators (camera/mic/location active) */
void perms_set_in_use(shell_perms *p, const char *app, perm_res res, bool active);
bool perms_in_use(const shell_perms *p, perm_res res, char *app_out, size_t cap);

shell_rect perms_prompt_rect(const shell_comp *c);
bool perms_mouse(shell_perms *p, int x, int y, bool down);                     /* true when the prompt consumed the event */
bool perms_key(shell_perms *p, uint32_t key);
void perms_install(shell_perms *p);                                            /* overlay: prompt + tray indicators */

#ifdef __cplusplus
}
#endif
#endif
