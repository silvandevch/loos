/* Backend contract between the shell/apps and the OS services (UI-SPEC section 3). The shell only ever talks to
 * these entry points; the real implementations sit on the kernel's NetMonitor/Firewall/Perms services and the
 * registry hive, backend_mem.c is the in-process reference used by tests and by the shell before those land. */
#ifndef LOOS_SHELL_BACKEND_H
#define LOOS_SHELL_BACKEND_H
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct { char proc[32]; uint32_t pid; char remote[40]; uint16_t port; char proto[4]; char state[12]; } net_conn;
typedef enum { FW_ASK, FW_ALLOW, FW_DENY } fw_action;
typedef struct { char proc[32]; fw_action action; } fw_rule;
typedef enum { RES_CAMERA, RES_MIC, RES_FILES, RES_NOTIFY, RES_LOCATION, RES_COUNT } perm_res;
typedef enum { PERM_ASK, PERM_ALLOW, PERM_DENY } perm_state;      /* ASK = the user has not decided yet */
extern const char *const perm_res_name[RES_COUNT];

typedef struct { char name[64]; bool is_dir; uint64_t size; int64_t mtime; uint32_t attrs; } fs_entry;
enum { FSA_HIDDEN = 1, FSA_READONLY = 2, FSA_SYMLINK = 4 };
typedef struct { char mount[32]; char label[24]; char fstype[12]; uint64_t total, avail; bool readonly; } fs_volume;
typedef struct {
    uint32_t pid, ppid; char name[32];
    uint32_t threads, handles, sockets;
    uint64_t mem;                       /* working set, bytes */
    uint32_t cpu_x10;                   /* CPU percent * 10 */
    uint64_t io_read, io_write;         /* bytes since start */
} ps_proc;

typedef struct shell_backend {
    void *user;
    /* NetMonitor / Firewall */
    int  (*net_query)(void *u, net_conn *out, int max);                    /* live sockets, one row each */
    int  (*fw_list)(void *u, fw_rule *out, int max);
    bool (*fw_set)(void *u, const char *proc, fw_action a);                /* FW_ASK removes the rule */
    fw_action (*fw_check)(void *u, const char *proc);
    bool (*net_hard_off_get)(void *u);
    void (*net_hard_off_set)(void *u, bool off);
    /* Perms */
    int  (*perm_apps)(void *u, char names[][32], int max);
    bool (*perm_get)(void *u, const char *app, perm_res r);
    bool (*perm_set)(void *u, const char *app, perm_res r, bool allowed);
    perm_state (*perm_state_get)(void *u, const char *app, perm_res r);
    bool (*perm_reset)(void *u, const char *app, perm_res r);              /* back to ASK */
    /* Fs */
    int  (*fs_list)(void *u, const char *path, fs_entry *out, int max);   /* -1 if path is not a directory */
    bool (*fs_stat)(void *u, const char *path, fs_entry *out);
    bool (*fs_mkdir)(void *u, const char *path);
    bool (*fs_remove)(void *u, const char *path);                          /* recursive for directories */
    bool (*fs_rename)(void *u, const char *from, const char *to);
    bool (*fs_copy)(void *u, const char *from, const char *to);            /* recursive */
    int  (*fs_volumes)(void *u, fs_volume *out, int max);
    /* Ps */
    int  (*ps_snapshot)(void *u, ps_proc *out, int max);
    bool (*ps_kill)(void *u, uint32_t pid);
    /* local config store (registry hive / config file): never synced, never networked */
    bool (*cfg_get)(void *u, const char *key, char *out, size_t cap);
    bool (*cfg_set)(void *u, const char *key, const char *value);
} shell_backend;

/* convenience wrappers */
int  be_cfg_int(const shell_backend *b, const char *key, int def);
bool be_cfg_bool(const shell_backend *b, const char *key, bool def);
void be_cfg_set_int(const shell_backend *b, const char *key, int v);
void be_cfg_set_bool(const shell_backend *b, const char *key, bool v);

/* ---- in-memory reference backend ---- */
#define MEM_MAX_CONN 64
#define MEM_MAX_RULES 64
#define MEM_MAX_APPS 32
#define MEM_MAX_CFG 128
#define MEM_MAX_NODES 512
#define MEM_MAX_PROCS 64
typedef struct {
    net_conn conn[MEM_MAX_CONN]; int nconn;
    fw_rule rule[MEM_MAX_RULES]; int nrule;
    bool hard_off;
    char app[MEM_MAX_APPS][32]; uint8_t perm[MEM_MAX_APPS], decided[MEM_MAX_APPS]; int napps;   /* perm bit r = allowed, decided bit r = user answered */
    struct { char key[40]; char val[96]; } cfg[MEM_MAX_CFG]; int ncfg;
    struct { char path[160]; bool dir; uint64_t size; int64_t mtime; uint32_t attrs; } node[MEM_MAX_NODES]; int nnode;
    fs_volume vol[4]; int nvol;
    ps_proc proc[MEM_MAX_PROCS]; int nproc;
} backend_mem;
void backend_mem_init(backend_mem *m, shell_backend *out);
void backend_mem_add_conn(backend_mem *m, const char *proc, uint32_t pid, const char *remote, uint16_t port, const char *proto, const char *state);
bool backend_mem_add_app(backend_mem *m, const char *name);
bool backend_mem_add_node(backend_mem *m, const char *path, bool dir, uint64_t size, int64_t mtime, uint32_t attrs);
void backend_mem_add_volume(backend_mem *m, const char *mount, const char *label, const char *fstype, uint64_t total, uint64_t avail, bool ro);
void backend_mem_add_proc(backend_mem *m, uint32_t pid, uint32_t ppid, const char *name, uint32_t threads, uint64_t mem, uint32_t cpu_x10);
size_t backend_mem_save(const backend_mem *m, char *out, size_t cap);        /* INI-style "key=value" lines + [firewall]/[perms] sections */
int  backend_mem_load(backend_mem *m, const char *text);                    /* returns entries read */

#ifdef __cplusplus
}
#endif
#endif
