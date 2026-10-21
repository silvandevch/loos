/* The LoOS installer: a guided, local-only setup wizard (welcome, disk, account, privacy, review, progress). The wizard is
 * pure UI + validation; everything that touches disks goes through shell_inst_backend, so the same code runs against the
 * kernel's ATA installer and against the in-memory backend in the host tests. */
#ifndef LOOS_SHELL_INSTALLER_H
#define LOOS_SHELL_INSTALLER_H
#include "ui.h"

#ifdef __cplusplus
extern "C" {
#endif

#define INST_MAX_DISKS 8

typedef struct {
    char name[32];             /* "Disk 1" */
    char model[48];
    uint64_t bytes;
    bool boot_media;           /* the media the installer itself runs from: never a target */
    bool too_small;
} inst_disk;

typedef struct {
    int disk;                  /* index into the list the backend returned */
    char display[48], username[32], hostname[32], password[64];
    bool net_blocked;          /* start with the hard network switch on */
    bool updates;              /* opt-in update checks */
} inst_config;

typedef struct shell_inst_backend {
    int  (*list_disks)(void *u, inst_disk *out, int max);
    bool (*begin)(void *u, const inst_config *cfg, char *err, size_t err_cap);
    /* One bounded slice of work. 0 = more to do, 1 = finished, -1 = failed (err set). */
    int  (*step)(void *u, int *percent, char *msg, size_t msg_cap, char *err, size_t err_cap);
    void (*reboot)(void *u);
    void *user;
} shell_inst_backend;

typedef enum { IP_WELCOME, IP_DISK, IP_ACCOUNT, IP_PRIVACY, IP_REVIEW, IP_INSTALL, IP_DONE, IP_ERROR } inst_page;

typedef struct {
    const shell_inst_backend *be;
    inst_page page;
    inst_disk disks[INST_MAX_DISKS]; int ndisks, sel_disk, disk_scroll;
    char display[48], user[32], host[32], pw[64], pw2[64];
    int c_display, c_user, c_host, c_pw, c_pw2;
    bool user_edited, host_edited;
    bool net_blocked, updates;
    int percent; char msg[96], err[160];
    bool live_requested;                 /* "Try LoOS without installing" */
    uint32_t frames;
    uint32_t bg_key;                     /* wallpaper already painted for this size/scale */
} shell_installer;

void installer_init(shell_installer *in, const shell_inst_backend *be);
void installer_frame(shell_installer *in, ui_ctx *ui, int w, int h);

bool installer_valid_username(const char *s);                  /* [a-z][a-z0-9_-]{0,30}, not "root" */
bool installer_valid_hostname(const char *s);                  /* RFC 1123 label, 1..31 chars */
const char *installer_password_problem(const char *pw, const char *pw2);   /* NULL when acceptable */
void installer_suggest_username(const char *display, char *out, size_t cap);

#ifdef __cplusplus
}
#endif
#endif
