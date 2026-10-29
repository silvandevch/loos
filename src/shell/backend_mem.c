#include "backend.h"
#include <string.h>
#include <stdio.h>
#include <stdlib.h>

const char *const perm_res_name[RES_COUNT] = { "Camera", "Microphone", "Files", "Notifications", "Location" };

static backend_mem *M(void *u) { return u; }

static int mem_net_query(void *u, net_conn *out, int max)
{
    backend_mem *m = M(u);
    int n = m->nconn < max ? m->nconn : max;
    memcpy(out, m->conn, (size_t)n * sizeof *out);
    return n;
}
static int mem_fw_list(void *u, fw_rule *out, int max) { backend_mem *m = M(u); int n = m->nrule < max ? m->nrule : max; memcpy(out, m->rule, (size_t)n * sizeof *out); return n; }
static bool mem_fw_set(void *u, const char *proc, fw_action a)
{
    backend_mem *m = M(u);
    for (int i = 0; i < m->nrule; i++)
        if (!strcmp(m->rule[i].proc, proc)) {
            if (a == FW_ASK) m->rule[i] = m->rule[--m->nrule]; else m->rule[i].action = a;
            return true;
        }
    if (a == FW_ASK) return true;
    if (m->nrule >= MEM_MAX_RULES) return false;
    strncpy(m->rule[m->nrule].proc, proc, 31); m->rule[m->nrule].proc[31] = 0; m->rule[m->nrule].action = a; m->nrule++;
    return true;
}
static fw_action mem_fw_check(void *u, const char *proc)
{
    backend_mem *m = M(u);
    if (m->hard_off) return FW_DENY;
    for (int i = 0; i < m->nrule; i++) if (!strcmp(m->rule[i].proc, proc)) return m->rule[i].action;
    return FW_ASK;
}
static bool mem_hard_get(void *u) { return M(u)->hard_off; }
static void mem_hard_set(void *u, bool off) { M(u)->hard_off = off; }
/* ---- in-memory filesystem: absolute '/'-separated paths, directories are explicit nodes ---- */
static int file_find(backend_mem *m, const char *p) { for (int i = 0; i < m->nfile; i++) if (!strcmp(m->file[i].path, p)) return i; return -1; }
static int node_find(backend_mem *m, const char *p) { for (int i = 0; i < m->nnode; i++) if (!strcmp(m->node[i].path, p)) return i; return -1; }
static void norm_path(const char *in, char *out, size_t cap)
{
    size_t n = strlen(in);
    while (n > 1 && in[n - 1] == '/') n--;
    if (n >= cap) n = cap - 1;
    memcpy(out, in, n); out[n] = 0;
}
static const char *base_name(const char *p) { const char *s = strrchr(p, '/'); return s ? s + 1 : p; }
static bool is_child(const char *dir, const char *p)
{
    size_t dl = strlen(dir);
    if (!strcmp(dir, "/")) { return p[0] == '/' && p[1] && !strchr(p + 1, '/'); }
    return !strncmp(p, dir, dl) && p[dl] == '/' && p[dl + 1] && !strchr(p + dl + 1, '/');
}
bool backend_mem_add_node(backend_mem *m, const char *path, bool dir, uint64_t size, int64_t mtime, uint32_t attrs)
{
    char p[160]; norm_path(path, p, sizeof p);
    if (m->nnode >= MEM_MAX_NODES || node_find(m, p) >= 0) return false;
    int i = m->nnode++;
    strcpy(m->node[i].path, p); m->node[i].dir = dir; m->node[i].size = dir ? 0 : size; m->node[i].mtime = mtime; m->node[i].attrs = attrs;
    return true;
}
static void fill_entry(const backend_mem *m, int i, fs_entry *e)
{
    memset(e, 0, sizeof *e);
    strncpy(e->name, base_name(m->node[i].path), sizeof e->name - 1);
    e->is_dir = m->node[i].dir; e->size = m->node[i].size; e->mtime = m->node[i].mtime; e->attrs = m->node[i].attrs;
}
static int mem_fs_list(void *u, const char *path, fs_entry *out, int max)
{
    backend_mem *m = M(u); char p[160]; norm_path(path, p, sizeof p);
    if (strcmp(p, "/") && (node_find(m, p) < 0 || !m->node[node_find(m, p)].dir)) return -1;
    int n = 0;
    for (int i = 0; i < m->nnode && n < max; i++) if (is_child(p, m->node[i].path)) fill_entry(m, i, &out[n++]);
    return n;
}
static bool mem_fs_stat(void *u, const char *path, fs_entry *out)
{
    backend_mem *m = M(u); char p[160]; norm_path(path, p, sizeof p);
    int i = node_find(m, p);
    if (i < 0) { if (!strcmp(p, "/")) { memset(out, 0, sizeof *out); strcpy(out->name, "/"); out->is_dir = true; return true; } return false; }
    fill_entry(m, i, out);
    return true;
}
static bool parent_is_dir(backend_mem *m, const char *p)
{
    char par[160]; strcpy(par, p);
    char *s = strrchr(par, '/');
    if (!s) return false;
    if (s == par) return true;                                   /* parent is the root */
    *s = 0;
    int i = node_find(m, par);
    return i >= 0 && m->node[i].dir;
}
static bool mem_fs_mkdir(void *u, const char *path)
{
    backend_mem *m = M(u); char p[160]; norm_path(path, p, sizeof p);
    if (!parent_is_dir(m, p)) return false;
    return backend_mem_add_node(m, p, true, 0, 1700000000, 0);
}
static int64_t mem_fs_read(void *u, const char *path, char *out, size_t cap)
{
    backend_mem *m = M(u); char p[160]; norm_path(path, p, sizeof p);
    int n = node_find(m, p);
    if (n < 0 || m->node[n].dir) return -1;
    int f = file_find(m, p);
    size_t len = f < 0 ? 0 : m->file[f].len;
    if (len > cap) len = cap;
    if (len) memcpy(out, m->file[f].data, len);
    return (int64_t)len;
}
static bool mem_fs_write(void *u, const char *path, const char *data, size_t len)
{
    backend_mem *m = M(u); char p[160]; norm_path(path, p, sizeof p);
    if (len > MEM_FILE_BYTES || !parent_is_dir(m, p)) return false;
    int n = node_find(m, p);
    if (n >= 0 && (m->node[n].dir || (m->node[n].attrs & FSA_READONLY))) return false;
    int f = file_find(m, p);
    if (f < 0) { if (m->nfile >= MEM_MAX_FILES) return false; f = m->nfile++; strcpy(m->file[f].path, p); }
    if (n < 0 && !backend_mem_add_node(m, p, false, 0, 1700000000, 0)) { m->nfile--; return false; }
    n = node_find(m, p);
    memcpy(m->file[f].data, data, len); m->file[f].len = (uint32_t)len;
    m->node[n].size = len;
    return true;
}
static bool mem_fs_remove(void *u, const char *path)
{
    backend_mem *m = M(u); char p[160]; norm_path(path, p, sizeof p);
    int i = node_find(m, p);
    if (i < 0 || (m->node[i].attrs & FSA_READONLY)) return false;
    size_t pl = strlen(p);
    for (int k = m->nnode - 1; k >= 0; k--)
        if (!strcmp(m->node[k].path, p) || (!strncmp(m->node[k].path, p, pl) && m->node[k].path[pl] == '/')) m->node[k] = m->node[--m->nnode];
    return true;
}
static bool mem_fs_rename(void *u, const char *from, const char *to)
{
    backend_mem *m = M(u); char f[160], t[160]; norm_path(from, f, sizeof f); norm_path(to, t, sizeof t);
    int i = node_find(m, f);
    if (i < 0 || node_find(m, t) >= 0 || !parent_is_dir(m, t)) return false;
    size_t fl = strlen(f);
    if (m->node[i].dir && !strncmp(t, f, fl) && t[fl] == '/') return false;          /* into itself */
    for (int k = 0; k < m->nnode; k++)
        if (!strcmp(m->node[k].path, f) || (!strncmp(m->node[k].path, f, fl) && m->node[k].path[fl] == '/')) {
            char np[160]; snprintf(np, sizeof np, "%s%s", t, m->node[k].path + fl);
            int fi = file_find(m, m->node[k].path);
            if (fi >= 0) strcpy(m->file[fi].path, np);                               /* contents follow the file */
            strcpy(m->node[k].path, np);
        }
    return true;
}
static bool mem_fs_copy(void *u, const char *from, const char *to)
{
    backend_mem *m = M(u); char f[160], t[160]; norm_path(from, f, sizeof f); norm_path(to, t, sizeof t);
    int i = node_find(m, f);
    if (i < 0 || node_find(m, t) >= 0 || !parent_is_dir(m, t)) return false;
    size_t fl = strlen(f);
    if (m->node[i].dir && !strncmp(t, f, fl) && t[fl] == '/') return false;
    int n0 = m->nnode;
    for (int k = 0; k < n0; k++)
        if (!strcmp(m->node[k].path, f) || (!strncmp(m->node[k].path, f, fl) && m->node[k].path[fl] == '/')) {
            if (m->nnode >= MEM_MAX_NODES) return false;
            char np[160]; snprintf(np, sizeof np, "%s%s", t, m->node[k].path + fl);
            backend_mem_add_node(m, np, m->node[k].dir, m->node[k].size, m->node[k].mtime, m->node[k].attrs & ~(uint32_t)FSA_READONLY);
        }
    return true;
}
static int mem_fs_volumes(void *u, fs_volume *out, int max) { backend_mem *m = M(u); int n = m->nvol < max ? m->nvol : max; memcpy(out, m->vol, (size_t)n * sizeof *out); return n; }
void backend_mem_add_volume(backend_mem *m, const char *mount, const char *label, const char *fstype, uint64_t total, uint64_t avail, bool ro)
{
    if (m->nvol >= 4) return;
    fs_volume *v = &m->vol[m->nvol++];
    memset(v, 0, sizeof *v);
    strncpy(v->mount, mount, 31); strncpy(v->label, label, 23); strncpy(v->fstype, fstype, 11);
    v->total = total; v->avail = avail; v->readonly = ro;
}

/* ---- processes ---- */
static int mem_ps_snapshot(void *u, ps_proc *out, int max) { backend_mem *m = M(u); int n = m->nproc < max ? m->nproc : max; memcpy(out, m->proc, (size_t)n * sizeof *out); return n; }
static bool mem_ps_kill(void *u, uint32_t pid)
{
    backend_mem *m = M(u);
    if (pid <= 4) return false;                                   /* the system process tree is not killable from the UI */
    for (int i = 0; i < m->nproc; i++) if (m->proc[i].pid == pid) {
        for (int k = 0; k < m->nconn; k++) if (m->conn[k].pid == pid) m->conn[k] = m->conn[--m->nconn], k--;
        m->proc[i] = m->proc[--m->nproc];
        for (int k = 0; k < m->nproc; k++) if (m->proc[k].ppid == pid) m->proc[k].ppid = 0;      /* orphans re-parent to the root */
        return true;
    }
    return false;
}
void backend_mem_add_proc(backend_mem *m, uint32_t pid, uint32_t ppid, const char *name, uint32_t threads, uint64_t mem, uint32_t cpu_x10)
{
    if (m->nproc >= MEM_MAX_PROCS) return;
    ps_proc *p = &m->proc[m->nproc++];
    memset(p, 0, sizeof *p);
    p->pid = pid; p->ppid = ppid; strncpy(p->name, name, sizeof p->name - 1);
    p->threads = threads; p->mem = mem; p->cpu_x10 = cpu_x10; p->handles = threads * 12;
}

static int app_index(backend_mem *m, const char *app) { for (int i = 0; i < m->napps; i++) if (!strcmp(m->app[i], app)) return i; return -1; }
static int mem_perm_apps(void *u, char names[][32], int max) { backend_mem *m = M(u); int n = m->napps < max ? m->napps : max; for (int i = 0; i < n; i++) memcpy(names[i], m->app[i], 32); return n; }
static bool mem_perm_get(void *u, const char *app, perm_res r) { backend_mem *m = M(u); int i = app_index(m, app); return i >= 0 && r < RES_COUNT && (m->perm[i] >> r & 1); }
static bool mem_perm_set(void *u, const char *app, perm_res r, bool ok)
{
    backend_mem *m = M(u); int i = app_index(m, app);
    if (i < 0 || r >= RES_COUNT) return false;
    if (ok) m->perm[i] |= (uint8_t)(1u << r); else m->perm[i] &= (uint8_t)~(1u << r);
    m->decided[i] |= (uint8_t)(1u << r);
    return true;
}
static perm_state mem_perm_state(void *u, const char *app, perm_res r)
{
    backend_mem *m = M(u); int i = app_index(m, app);
    if (i < 0 || r >= RES_COUNT || !(m->decided[i] >> r & 1)) return PERM_ASK;
    return (m->perm[i] >> r & 1) ? PERM_ALLOW : PERM_DENY;
}
static bool mem_perm_reset(void *u, const char *app, perm_res r)
{
    backend_mem *m = M(u); int i = app_index(m, app);
    if (i < 0 || r >= RES_COUNT) return false;
    m->perm[i] &= (uint8_t)~(1u << r); m->decided[i] &= (uint8_t)~(1u << r);
    return true;
}
static int cfg_find(backend_mem *m, const char *k) { for (int i = 0; i < m->ncfg; i++) if (!strcmp(m->cfg[i].key, k)) return i; return -1; }
static bool mem_cfg_get(void *u, const char *key, char *out, size_t cap)
{
    backend_mem *m = M(u); int i = cfg_find(m, key);
    if (i < 0) return false;
    strncpy(out, m->cfg[i].val, cap - 1); out[cap - 1] = 0;
    return true;
}
static bool mem_cfg_set(void *u, const char *key, const char *val)
{
    backend_mem *m = M(u); int i = cfg_find(m, key);
    if (strlen(key) >= sizeof m->cfg[0].key || strlen(val) >= sizeof m->cfg[0].val) return false;
    if (i < 0) { if (m->ncfg >= MEM_MAX_CFG) return false; i = m->ncfg++; strcpy(m->cfg[i].key, key); }
    strcpy(m->cfg[i].val, val);
    return true;
}

void backend_mem_init(backend_mem *m, shell_backend *b)
{
    memset(m, 0, sizeof *m);
    memset(b, 0, sizeof *b);
    b->user = m;
    b->net_query = mem_net_query; b->fw_list = mem_fw_list; b->fw_set = mem_fw_set; b->fw_check = mem_fw_check;
    b->net_hard_off_get = mem_hard_get; b->net_hard_off_set = mem_hard_set;
    b->perm_apps = mem_perm_apps; b->perm_get = mem_perm_get; b->perm_set = mem_perm_set;
    b->perm_state_get = mem_perm_state; b->perm_reset = mem_perm_reset;
    b->cfg_get = mem_cfg_get; b->cfg_set = mem_cfg_set;
    b->fs_list = mem_fs_list; b->fs_stat = mem_fs_stat; b->fs_mkdir = mem_fs_mkdir; b->fs_remove = mem_fs_remove;
    b->fs_rename = mem_fs_rename; b->fs_copy = mem_fs_copy; b->fs_read = mem_fs_read; b->fs_write = mem_fs_write; b->fs_volumes = mem_fs_volumes;
    b->ps_snapshot = mem_ps_snapshot; b->ps_kill = mem_ps_kill;
}
void backend_mem_add_conn(backend_mem *m, const char *proc, uint32_t pid, const char *remote, uint16_t port, const char *proto, const char *state)
{
    if (m->nconn >= MEM_MAX_CONN) return;
    net_conn *c = &m->conn[m->nconn++];
    memset(c, 0, sizeof *c);
    strncpy(c->proc, proc, sizeof c->proc - 1); strncpy(c->remote, remote, sizeof c->remote - 1);
    strncpy(c->proto, proto, sizeof c->proto - 1); strncpy(c->state, state, sizeof c->state - 1);
    c->pid = pid; c->port = port;
}
bool backend_mem_add_app(backend_mem *m, const char *name)
{
    if (m->napps >= MEM_MAX_APPS || app_index(m, name) >= 0) return false;
    strncpy(m->app[m->napps], name, 31);
    m->perm[m->napps] = 0;                                    /* default deny: every grant is an explicit user action */
    m->napps++;
    return true;
}

int be_cfg_int(const shell_backend *b, const char *k, int def) { char v[32]; return b->cfg_get(b->user, k, v, sizeof v) ? atoi(v) : def; }
bool be_cfg_bool(const shell_backend *b, const char *k, bool def) { char v[32]; if (!b->cfg_get(b->user, k, v, sizeof v)) return def; return v[0] == '1' || v[0] == 't'; }
void be_cfg_set_int(const shell_backend *b, const char *k, int v) { char s[16]; snprintf(s, sizeof s, "%d", v); b->cfg_set(b->user, k, s); }
void be_cfg_set_bool(const shell_backend *b, const char *k, bool v) { b->cfg_set(b->user, k, v ? "1" : "0"); }

/* ---- persistence: plain text, local only ---- */
size_t backend_mem_save(const backend_mem *m, char *out, size_t cap)
{
    size_t n = 0;
    #define PUT(...) do { int k_ = snprintf(out + n, n < cap ? cap - n : 0, __VA_ARGS__); if (k_ > 0) n += (size_t)k_; } while (0)
    PUT("[config]\n");
    for (int i = 0; i < m->ncfg; i++) PUT("%s=%s\n", m->cfg[i].key, m->cfg[i].val);
    PUT("[firewall]\n");
    for (int i = 0; i < m->nrule; i++) PUT("%s=%s\n", m->rule[i].proc, m->rule[i].action == FW_ALLOW ? "allow" : "deny");
    PUT("[perms]\n");
    for (int i = 0; i < m->napps; i++) PUT("%s=%u\n", m->app[i], (unsigned)m->perm[i] | (unsigned)m->decided[i] << 8);
    #undef PUT
    if (cap) out[n < cap ? n : cap - 1] = 0;
    return n;
}
int backend_mem_load(backend_mem *m, const char *text)
{
    int sect = 0, n = 0;
    while (*text) {
        char line[160];
        size_t len = strcspn(text, "\n");
        if (len >= sizeof line) len = sizeof line - 1;
        memcpy(line, text, len); line[len] = 0;
        text += strcspn(text, "\n"); if (*text) text++;
        if (!*line || *line == '#') continue;
        if (*line == '[') { sect = !strncmp(line, "[config]", 8) ? 1 : !strncmp(line, "[firewall]", 10) ? 2 : !strncmp(line, "[perms]", 7) ? 3 : 0; continue; }
        char *eq = strchr(line, '=');
        if (!eq || !sect) continue;
        *eq = 0;
        const char *k = line, *v = eq + 1;
        if (sect == 1) { if (mem_cfg_set(m, k, v)) n++; }
        else if (sect == 2) { if (mem_fw_set(m, k, !strcmp(v, "allow") ? FW_ALLOW : FW_DENY)) n++; }
        else { backend_mem_add_app(m, k); int i = app_index(m, k); if (i >= 0) { unsigned val = (unsigned)atoi(v); m->perm[i] = (uint8_t)(val & 0xff); m->decided[i] = (uint8_t)((val >> 8) & 0xff); if (!m->decided[i]) m->decided[i] = m->perm[i]; n++; } }
    }
    return n;
}
