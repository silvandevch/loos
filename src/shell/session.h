/* Local accounts, sign-in policy and the session state machine (signed out / active / locked). Local accounts
 * only: no Microsoft/cloud account, no PIN sync. Passwords are PBKDF2-HMAC-SHA256 with a per-account salt;
 * repeated failures back off exponentially. The login screen is an immediate-mode app like the others. */
#ifndef LOOS_SHELL_SESSION_H
#define LOOS_SHELL_SESSION_H
#include "ui.h"

#ifdef __cplusplus
extern "C" {
#endif

#define ACC_MAX 16
#define ACC_NAME_MAX 32
#define ACC_PW_MAX 64
#define ACC_FREE_TRIES 5                    /* failures before the first lockout */
#define ACC_LOCK_BASE_MS 30000u             /* first lockout, doubling per further failure */
#define ACC_LOCK_MAX_MS 900000u

typedef struct {
    char name[ACC_NAME_MAX];                /* login name: [a-z0-9_-], 1..31 chars */
    char display[48];
    uint8_t salt[16], hash[32];
    uint32_t iters;
    bool has_password, admin;
    uint32_t failures;
    uint64_t locked_until_ms;
} shell_account;

typedef struct {
    shell_account acc[ACC_MAX]; int n;
    char autologin[ACC_NAME_MAX];           /* "" = off */
    uint32_t default_iters;
    void (*random)(uint8_t *out, size_t n, void *user);   /* OS CSPRNG; required to create accounts without a caller salt */
    void *random_user;
} shell_accounts;

typedef enum { AUTH_OK, AUTH_BAD_PASSWORD, AUTH_LOCKED_OUT, AUTH_NO_SUCH_USER } auth_result;

void accounts_init(shell_accounts *db, uint32_t iters);
shell_account *account_find(shell_accounts *db, const char *name);
bool account_valid_name(const char *name);
bool account_create(shell_accounts *db, const char *name, const char *display, const char *password, const uint8_t *salt16, bool admin);
bool account_set_password(shell_accounts *db, const char *name, const char *old_pw, const char *new_pw, const uint8_t *salt16);
bool account_delete(shell_accounts *db, const char *name);                 /* refuses to remove the last administrator */
auth_result account_verify(shell_accounts *db, const char *name, const char *password, uint64_t now_ms, uint32_t *retry_ms);
bool accounts_set_autologin(shell_accounts *db, const char *name_or_null);
size_t accounts_save(const shell_accounts *db, char *out, size_t cap);     /* text, one account per line, hashes in hex */
int  accounts_load(shell_accounts *db, const char *text);                  /* returns accounts read */

typedef enum { SESS_SIGNED_OUT, SESS_ACTIVE, SESS_LOCKED } sess_state;
typedef enum { SESS_EV_LOGIN, SESS_EV_LOCK, SESS_EV_UNLOCK, SESS_EV_LOGOUT, SESS_EV_AUTOLOCK } sess_event;
typedef struct {
    shell_accounts *db;
    sess_state state;
    char user[ACC_NAME_MAX];
    uint32_t login_count;
    uint64_t last_activity_ms;
    uint32_t idle_lock_ms;                  /* 0 = never auto-lock */
    void (*on_event)(sess_event ev, const char *user, void *ctx);
    void *ctx;
} shell_session;

void session_init(shell_session *s, shell_accounts *db);
bool session_boot(shell_session *s, uint64_t now_ms);                      /* applies auto sign-in; true if a user is now active */
auth_result session_login(shell_session *s, const char *user, const char *password, uint64_t now_ms, uint32_t *retry_ms);
void session_lock(shell_session *s);
auth_result session_unlock(shell_session *s, const char *password, uint64_t now_ms, uint32_t *retry_ms);
void session_logout(shell_session *s);
void session_activity(shell_session *s, uint64_t now_ms);
void session_tick(shell_session *s, uint64_t now_ms);                     /* idle auto-lock */

typedef struct {
    shell_session *session;
    int sel;                                /* selected account tile */
    char pw[ACC_PW_MAX]; int pw_caret;
    char msg[96];
    uint64_t now_ms;                        /* host feeds the clock */
    int hour, minute;
    /* first-run setup form */
    char su_name[ACC_NAME_MAX], su_disp[48], su_pw[ACC_PW_MAX], su_pw2[ACC_PW_MAX]; int su_c1, su_c2, su_c3, su_c4;
} shell_login;

void login_init(shell_login *l, shell_session *s);
void login_frame(shell_login *l, ui_ctx *ui, int w, int h);

#ifdef __cplusplus
}
#endif
#endif
