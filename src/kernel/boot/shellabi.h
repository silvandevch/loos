/* C view of the Rust desktop shell (crates/loos-shell): the only header the kernel needs from it.
 * Everything behind these calls is Rust; the layouts here are the ones the installer backend and the host
 * share by value (shell_rect, shell_accounts, shell_inst_backend). Keep in step with crates/loos-shell. */
#ifndef LOOS_SHELLABI_H
#define LOOS_SHELLABI_H
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#include "shellgfx.h"

/* ---- local accounts (crates/loos-shell/src/session.rs) ---- */
#define ACC_MAX 16
#define ACC_NAME_MAX 32
typedef struct {
    char name[ACC_NAME_MAX];
    char display[48];
    uint8_t salt[16], hash[32];
    uint32_t iters;
    bool has_password, admin;
    uint32_t failures;
    uint64_t locked_until_ms;
} shell_account;
typedef struct {
    shell_account acc[ACC_MAX]; int n;
    char autologin[ACC_NAME_MAX];
    uint32_t default_iters;
    void (*random)(uint8_t *out, size_t n, void *user);
    void *random_user;
} shell_accounts;
typedef enum { AUTH_OK, AUTH_BAD_PASSWORD, AUTH_LOCKED_OUT, AUTH_NO_SUCH_USER } auth_result;
void accounts_init(shell_accounts *db, uint32_t iters);
bool account_create(shell_accounts *db, const char *name, const char *display, const char *password, const uint8_t *salt16, bool admin);
size_t accounts_save(const shell_accounts *db, char *out, size_t cap);

/* ---- installer backend (crates/loos-shell/src/installer.rs) ---- */
#define INST_MAX_DISKS 8
typedef struct { char name[32]; char model[48]; uint64_t bytes; bool boot_media; bool too_small; } inst_disk;
typedef struct {
    int disk;
    char display[48], username[32], hostname[32], password[64];
    bool net_blocked;
    bool updates;
} inst_config;
typedef struct shell_inst_backend {
    int  (*list_disks)(void *u, inst_disk *out, int max);
    bool (*begin)(void *u, const inst_config *cfg, char *err, size_t err_cap);
    int  (*step)(void *u, int *percent, char *msg, size_t msg_cap, char *err, size_t err_cap);   /* 0 more, 1 done, -1 failed */
    void (*reboot)(void *u);
    void *user;
} shell_inst_backend;

/* ---- the desktop (crates/loos-shell/src/desktop.rs) ---- */
typedef struct shell_desktop shell_desktop;
shell_desktop *desktop_instance(void);                      /* the one desktop, kept in the Rust image's .bss */
int  desktop_auto_scale(int w, int h);
void desktop_set_log(shell_desktop *d, void (*log)(const char *line, void *aux), void *aux);
bool desktop_init(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux);
bool desktop_init_installer(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux,
                            const shell_inst_backend *be);
bool desktop_installer_wants_live(const shell_desktop *d);
bool desktop_init_login(shell_desktop *d, uint32_t *backbuf, int w, int h, int scale_pct, void *(*alloc)(size_t, void *), void *aux,
                        const char *accounts_text, uint32_t iters);
bool desktop_login_done(const shell_desktop *d, char *user_out, size_t cap);
int  desktop_login_try(shell_desktop *d, const char *user, const char *password, uint64_t now_ms);   /* auth_result */
void desktop_apply_config(shell_desktop *d, const char *text);
void desktop_set_system_volume(shell_desktop *d, uint64_t total, uint64_t used);   /* real size of the installed system partition */
bool desktop_apply_look(shell_desktop *d, const char *id);   /* Looks: loos, classic, dock, tiler, paper, midnight, lean, large */
void desktop_open_url(shell_desktop *d, const char *url);
void desktop_pointer(shell_desktop *d, int dx, int dy, uint32_t buttons);
void desktop_pointer_to(shell_desktop *d, int x, int y, uint32_t buttons);
void desktop_key(shell_desktop *d, uint32_t key, uint32_t ch, uint32_t mods, bool down);
void desktop_tick(shell_desktop *d, uint32_t now_ms, int hour, int minute);
void desktop_set_date(shell_desktop *d, int day, int month, int year);
shell_rect desktop_render(shell_desktop *d);
void desktop_render_all(shell_desktop *d);
uint32_t desktop_theme_desktop(const shell_desktop *d);      /* wallpaper base colour: tests count pixels that differ from it */
int  desktop_window_count(const shell_desktop *d);
uint32_t desktop_scale_pct(const shell_desktop *d);
#endif
