#include "session.h"
#include "../security/kdf.h"
#include <string.h>
#include <stdio.h>

void accounts_init(shell_accounts *db, uint32_t iters) { memset(db, 0, sizeof *db); db->default_iters = iters ? iters : 100000; }

bool account_valid_name(const char *n)
{
    size_t len = strlen(n);
    if (!len || len >= ACC_NAME_MAX) return false;
    for (size_t i = 0; i < len; i++) {
        char c = n[i];
        if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' || c == '-')) return false;
    }
    return true;
}
shell_account *account_find(shell_accounts *db, const char *name)
{
    for (int i = 0; i < db->n; i++) if (!strcmp(db->acc[i].name, name)) return &db->acc[i];
    return NULL;
}

static void derive(const shell_account *a, const char *pw, uint8_t out[32])
{
    loos_pbkdf2_sha256(pw, strlen(pw), a->salt, sizeof a->salt, a->iters, out, 32);
}
static bool set_secret(shell_accounts *db, shell_account *a, const char *pw, const uint8_t *salt16)
{
    if (!pw[0]) { a->has_password = false; memset(a->hash, 0, sizeof a->hash); memset(a->salt, 0, sizeof a->salt); a->iters = db->default_iters; return true; }
    if (strlen(pw) >= ACC_PW_MAX) return false;
    if (salt16) memcpy(a->salt, salt16, 16);
    else if (db->random) db->random(a->salt, 16, db->random_user);
    else return false;                                           /* never fall back to a fixed salt */
    a->iters = db->default_iters; a->has_password = true;
    derive(a, pw, a->hash);
    return true;
}

bool account_create(shell_accounts *db, const char *name, const char *display, const char *pw, const uint8_t *salt16, bool admin)
{
    if (db->n >= ACC_MAX || !account_valid_name(name) || account_find(db, name)) return false;
    shell_account *a = &db->acc[db->n];
    memset(a, 0, sizeof *a);
    strcpy(a->name, name);
    strncpy(a->display, display && *display ? display : name, sizeof a->display - 1);
    a->admin = admin || db->n == 0;                              /* the first account is always an administrator */
    if (!set_secret(db, a, pw, salt16)) return false;
    db->n++;
    return true;
}

bool account_set_password(shell_accounts *db, const char *name, const char *old_pw, const char *new_pw, const uint8_t *salt16)
{
    shell_account *a = account_find(db, name);
    if (!a) return false;
    if (a->has_password) {
        uint8_t h[32];
        if (!old_pw) return false;
        derive(a, old_pw, h);
        if (!loos_constant_time_equal(h, a->hash, 32)) return false;
    }
    return set_secret(db, a, new_pw, salt16);
}

bool account_delete(shell_accounts *db, const char *name)
{
    shell_account *a = account_find(db, name);
    if (!a) return false;
    if (a->admin) {
        int admins = 0;
        for (int i = 0; i < db->n; i++) admins += db->acc[i].admin;
        if (admins <= 1) return false;
    }
    if (!strcmp(db->autologin, name)) db->autologin[0] = 0;
    *a = db->acc[--db->n];
    return true;
}

auth_result account_verify(shell_accounts *db, const char *name, const char *pw, uint64_t now, uint32_t *retry_ms)
{
    shell_account *a = account_find(db, name);
    if (retry_ms) *retry_ms = 0;
    if (!a) return AUTH_NO_SUCH_USER;
    if (a->locked_until_ms > now) { if (retry_ms) *retry_ms = (uint32_t)(a->locked_until_ms - now); return AUTH_LOCKED_OUT; }
    bool ok;
    if (!a->has_password) ok = pw[0] == 0;
    else { uint8_t h[32]; derive(a, pw, h); ok = loos_constant_time_equal(h, a->hash, 32); }
    if (ok) { a->failures = 0; a->locked_until_ms = 0; return AUTH_OK; }
    a->failures++;
    if (a->failures >= ACC_FREE_TRIES) {
        uint32_t shift = a->failures - ACC_FREE_TRIES;
        uint64_t d = shift >= 16 ? ACC_LOCK_MAX_MS : (uint64_t)ACC_LOCK_BASE_MS << shift;
        if (d > ACC_LOCK_MAX_MS) d = ACC_LOCK_MAX_MS;
        a->locked_until_ms = now + d;
        if (retry_ms) *retry_ms = (uint32_t)d;
    }
    return AUTH_BAD_PASSWORD;
}

bool accounts_set_autologin(shell_accounts *db, const char *name)
{
    if (!name || !*name) { db->autologin[0] = 0; return true; }
    if (!account_find(db, name)) return false;
    strcpy(db->autologin, name);
    return true;
}

static void to_hex(const uint8_t *b, size_t n, char *out) { static const char d[] = "0123456789abcdef"; for (size_t i = 0; i < n; i++) { out[2 * i] = d[b[i] >> 4]; out[2 * i + 1] = d[b[i] & 15]; } out[2 * n] = 0; }
static bool from_hex(const char *s, uint8_t *out, size_t n)
{
    for (size_t i = 0; i < n; i++) {
        int v = 0;
        for (int k = 0; k < 2; k++) {
            char c = s[2 * i + k]; int d = c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : -1;
            if (d < 0) return false;
            v = v * 16 + d;
        }
        out[i] = (uint8_t)v;
    }
    return true;
}

size_t accounts_save(const shell_accounts *db, char *out, size_t cap)
{
    size_t n = 0;
    #define PUT(...) do { int k_ = snprintf(out + n, n < cap ? cap - n : 0, __VA_ARGS__); if (k_ > 0) n += (size_t)k_; } while (0)
    PUT("autologin %s\n", db->autologin[0] ? db->autologin : "-");
    for (int i = 0; i < db->n; i++) {
        const shell_account *a = &db->acc[i];
        char salt[33], hash[65];
        to_hex(a->salt, 16, salt); to_hex(a->hash, 32, hash);
        PUT("account %s %u %d %d %s %s %s\n", a->name, a->iters, a->admin, a->has_password, salt, hash, a->display);
    }
    #undef PUT
    if (cap) out[n < cap ? n : cap - 1] = 0;
    return n;
}

/* "name iters admin haspw salt hash rest": space separated tokens (no sscanf: the kernel has no scanf family) */
static bool token(const char **p, char *out, size_t cap)
{
    const char *s = *p;
    while (*s == ' ') s++;
    size_t n = 0;
    while (s[n] && s[n] != ' ') n++;
    if (!n || n >= cap) return false;
    memcpy(out, s, n); out[n] = 0;
    *p = s + n;
    return true;
}
static bool number(const char *t, unsigned *v)
{
    if (!*t) return false;
    unsigned r = 0;
    for (; *t; t++) { if (*t < '0' || *t > '9' || r > 429496729u) return false; r = r * 10 + (unsigned)(*t - '0'); }
    *v = r;
    return true;
}
static bool parse_account_fields(const char *s, char *name, size_t ncap, unsigned *iters, int *admin, int *haspw, char *salt, size_t scap, char *hash, size_t hcap, int *off)
{
    const char *p = s; char t[72]; unsigned a, h;
    if (!token(&p, name, ncap) || !token(&p, t, sizeof t) || !number(t, iters)) return false;
    if (!token(&p, t, sizeof t) || !number(t, &a) || !token(&p, t, sizeof t) || !number(t, &h)) return false;
    if (!token(&p, salt, scap) || !token(&p, hash, hcap)) return false;
    *admin = (int)a; *haspw = (int)h;
    if (*p == ' ') p++;
    *off = (int)(p - s);
    return true;
}

int accounts_load(shell_accounts *db, const char *text)
{
    int n = 0;
    while (*text) {
        char line[200];
        size_t len = strcspn(text, "\n");
        if (len >= sizeof line) len = sizeof line - 1;
        memcpy(line, text, len); line[len] = 0;
        text += strcspn(text, "\n"); if (*text) text++;
        if (!strncmp(line, "autologin ", 10)) { if (strcmp(line + 10, "-")) strncpy(db->autologin, line + 10, ACC_NAME_MAX - 1); continue; }
        if (strncmp(line, "account ", 8) || db->n >= ACC_MAX) continue;
        char name[ACC_NAME_MAX], salt[40], hash[72]; unsigned iters; int admin, haspw, off = 0;
        if (!parse_account_fields(line + 8, name, sizeof name, &iters, &admin, &haspw, salt, sizeof salt, hash, sizeof hash, &off)) continue;
        if (!account_valid_name(name) || account_find(db, name) || iters == 0 || iters > 10000000u) continue;
        shell_account *a = &db->acc[db->n];
        memset(a, 0, sizeof *a);
        strcpy(a->name, name); a->iters = iters; a->admin = admin != 0; a->has_password = haspw != 0;
        if (strlen(salt) != 32 || strlen(hash) != 64 || !from_hex(salt, a->salt, 16) || !from_hex(hash, a->hash, 32)) continue;
        strncpy(a->display, line + 8 + off, sizeof a->display - 1);
        db->n++; n++;
    }
    return n;
}

/* ---- session state machine ---- */
void session_init(shell_session *s, shell_accounts *db) { memset(s, 0, sizeof *s); s->db = db; s->idle_lock_ms = 0; }
static void fire(shell_session *s, sess_event e) { if (s->on_event) s->on_event(e, s->user, s->ctx); }

bool session_boot(shell_session *s, uint64_t now)
{
    if (s->db->autologin[0] && account_find(s->db, s->db->autologin)) {
        shell_account *a = account_find(s->db, s->db->autologin);
        /* auto sign-in is the user's explicit choice; the password still guards unlocking the screen */
        strcpy(s->user, a->name); s->state = SESS_ACTIVE; s->login_count++; s->last_activity_ms = now;
        fire(s, SESS_EV_LOGIN);
        return true;
    }
    s->state = SESS_SIGNED_OUT;
    return false;
}

auth_result session_login(shell_session *s, const char *user, const char *pw, uint64_t now, uint32_t *retry)
{
    auth_result r = account_verify(s->db, user, pw, now, retry);
    if (r != AUTH_OK) return r;
    strcpy(s->user, user); s->state = SESS_ACTIVE; s->login_count++; s->last_activity_ms = now;
    fire(s, SESS_EV_LOGIN);
    return r;
}
void session_lock(shell_session *s) { if (s->state == SESS_ACTIVE) { s->state = SESS_LOCKED; fire(s, SESS_EV_LOCK); } }
auth_result session_unlock(shell_session *s, const char *pw, uint64_t now, uint32_t *retry)
{
    if (s->state != SESS_LOCKED) return AUTH_NO_SUCH_USER;
    auth_result r = account_verify(s->db, s->user, pw, now, retry);
    if (r == AUTH_OK) { s->state = SESS_ACTIVE; s->last_activity_ms = now; fire(s, SESS_EV_UNLOCK); }
    return r;
}
void session_logout(shell_session *s)
{
    if (s->state == SESS_SIGNED_OUT) return;
    fire(s, SESS_EV_LOGOUT);
    s->state = SESS_SIGNED_OUT; s->user[0] = 0;
}
void session_activity(shell_session *s, uint64_t now) { s->last_activity_ms = now; }
void session_tick(shell_session *s, uint64_t now)
{
    if (s->state == SESS_ACTIVE && s->idle_lock_ms && now - s->last_activity_ms >= s->idle_lock_ms) {
        s->state = SESS_LOCKED; fire(s, SESS_EV_AUTOLOCK);
    }
}

/* ---- login screen ---- */
void login_init(shell_login *l, shell_session *s) { memset(l, 0, sizeof *l); l->session = s; }

static void tile(ui_ctx *u, shell_rect r, const shell_account *a, bool sel, bool *clicked)
{
    const shell_theme *t = u->theme;
    bool over = ui_hovered(u, r);
    fb_fill_round(u->fb, u->clip, r, 10, sel ? color_mix(t->surface_hi, t->accent, 90) : over ? t->surface_hi : t->surface);
    shell_rect av = { r.x + r.w / 2 - 24, r.y + 14, 48, 48 };
    unsigned h = 0; for (const char *p = a->name; *p; p++) h = h * 31 + (unsigned char)*p;
    fb_fill_round(u->fb, u->clip, av, 24, color_mix(t->accent, 0xFFFFFFFFu, (int)(h % 120)));
    char ini[2] = { (char)((a->display[0] >= 'a' && a->display[0] <= 'z') ? a->display[0] - 32 : a->display[0]), 0 };
    fb_text(u->fb, u->clip, av.x + 20, av.y + 16, ini, t->accent_text);
    int tw = shell_text_width(a->display);
    fb_text(u->fb, u->clip, r.x + (r.w - tw) / 2, r.y + 74, a->display, t->text);
    *clicked = u->pressed && over;
}

void login_frame(shell_login *l, ui_ctx *u, int w, int h)
{
    shell_session *s = l->session;
    shell_accounts *db = s->db;
    const shell_theme *t = u->theme;
    ui_backdrop(u, w, h);
    char clock[8] = { (char)('0' + l->hour / 10 % 10), (char)('0' + l->hour % 10), ':', (char)('0' + l->minute / 10 % 10), (char)('0' + l->minute % 10), 0 };
    shell_font big = { &loos_font_semibold, 64 };
    fb_text_ex(u->fb, u->clip, w / 2 - shell_text_width_ex(clock, big) / 2, 36, clock, 0xFFF3F4F6u, big);
    ui_label(u, w / 2 - shell_text_width("Local only.") / 2, 112, "Local only.", t->text_dim);

    if (db->n == 0) {                                              /* first run: create the administrator */
        shell_rect card = { w / 2 - 200, h / 2 - 150, 400, 330 };
        ui_panel(u, card, t->surface);
        ui_label(u, card.x + 20, card.y + 16, "Create your local account", t->text);
        ui_label(u, card.x + 20, card.y + 38, "It stays on this device. No cloud account.", t->text_dim);
        ui_textfield(u, (shell_rect){ card.x + 20, card.y + 70, 360, 30 }, l->su_name, sizeof l->su_name, &l->su_c1, "login name");
        ui_textfield(u, (shell_rect){ card.x + 20, card.y + 110, 360, 30 }, l->su_disp, sizeof l->su_disp, &l->su_c2, "display name");
        ui_textfield_password(u, (shell_rect){ card.x + 20, card.y + 150, 360, 30 }, l->su_pw, sizeof l->su_pw, &l->su_c3, "password");
        ui_textfield_password(u, (shell_rect){ card.x + 20, card.y + 190, 360, 30 }, l->su_pw2, sizeof l->su_pw2, &l->su_c4, "repeat password");
        if (ui_button_accent(u, (shell_rect){ card.x + 20, card.y + 240, 160, 34 }, "Create account")) {
            if (!account_valid_name(l->su_name)) strcpy(l->msg, "Login name: a-z, 0-9, - and _ only.");
            else if (strcmp(l->su_pw, l->su_pw2)) strcpy(l->msg, "The passwords do not match.");
            else if (strlen(l->su_pw) < 4 && l->su_pw[0]) strcpy(l->msg, "Use at least 4 characters, or leave it empty.");
            else if (account_create(db, l->su_name, l->su_disp, l->su_pw, NULL, true)) { l->msg[0] = 0; memset(l->su_pw, 0, sizeof l->su_pw); memset(l->su_pw2, 0, sizeof l->su_pw2); }
            else strcpy(l->msg, "Could not create the account.");
        }
        ui_label(u, card.x + 20, card.y + 290, l->msg, t->danger);
        return;
    }

    if (s->state == SESS_LOCKED) {                                 /* only the locked user can unlock */
        shell_account *a = account_find(db, s->user);
        for (int i = 0; i < db->n; i++) if (&db->acc[i] == a) l->sel = i;
    }
    if (l->sel >= db->n) l->sel = 0;
    int shown = s->state == SESS_LOCKED ? 1 : db->n;
    int tw = 120, gap = 12, x0 = w / 2 - (shown * (tw + gap) - gap) / 2, y0 = 170;
    for (int i = 0; i < shown; i++) {
        int idx = s->state == SESS_LOCKED ? l->sel : i;
        bool click;
        tile(u, (shell_rect){ x0 + i * (tw + gap), y0, tw, 110 }, &db->acc[idx], idx == l->sel, &click);
        if (click && s->state != SESS_LOCKED && l->sel != idx) { l->sel = idx; l->pw[0] = 0; l->pw_caret = 0; l->msg[0] = 0; }
    }
    shell_account *cur = &db->acc[l->sel];
    int py = y0 + 130;
    bool submit = false;
    if (cur->has_password) {
        shell_rect pr = { w / 2 - 150, py, 300, 34 };
        u->focus = u->focus ? u->focus : ui_textfield_id("password", pr);
        submit = ui_textfield_password(u, pr, l->pw, sizeof l->pw, &l->pw_caret, "password");
        py += 44;
    } else ui_label(u, w / 2 - 92, py + 8, "No password set", t->text_dim);
    if (ui_button_accent(u, (shell_rect){ w / 2 - 150, py, 300, 34 }, s->state == SESS_LOCKED ? "Unlock" : "Sign in")) submit = true;
    if (submit) {
        uint32_t retry = 0;
        auth_result r = s->state == SESS_LOCKED ? session_unlock(s, l->pw, l->now_ms, &retry) : session_login(s, cur->name, l->pw, l->now_ms, &retry);
        switch (r) {
        case AUTH_OK: l->msg[0] = 0; break;
        case AUTH_BAD_PASSWORD:
            if (retry) snprintf(l->msg, sizeof l->msg, "Wrong password. Try again in %u s.", (retry + 999) / 1000);
            else strcpy(l->msg, "Wrong password.");
            break;
        case AUTH_LOCKED_OUT: snprintf(l->msg, sizeof l->msg, "Too many attempts. Try again in %u s.", (retry + 999) / 1000); break;
        default: strcpy(l->msg, "No such user."); break;
        }
        memset(l->pw, 0, sizeof l->pw); l->pw_caret = 0;           /* never keep a typed password around */
    }
    ui_label(u, w / 2 - 150, py + 44, l->msg, t->danger);
    if (cur->locked_until_ms > l->now_ms) {
        char b[64]; snprintf(b, sizeof b, "Locked for %u more seconds", (unsigned)((cur->locked_until_ms - l->now_ms + 999) / 1000));
        ui_label(u, w / 2 - 150, py + 66, b, t->text_dim);
    }
}
