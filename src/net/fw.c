#include "fw.h"
#include <string.h>
#include <stdio.h>
#include <stdlib.h>

static int lc(int c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }
bool fw_glob(const char *p, const char *t)
{
    const char *star = NULL, *mark = NULL;
    while (*t) {
        if (*p == '*') { star = p++; mark = t; }
        else if (*p == '?' || lc((unsigned char)*p) == lc((unsigned char)*t)) { p++; t++; }
        else if (star) { p = star + 1; t = ++mark; }
        else return false;
    }
    while (*p == '*') p++;
    return !*p;
}

void fw_init(fw_engine *f)
{
    memset(f, 0, sizeof *f);
    f->next_id = 1; f->default_in = FWV_DROP; f->default_out = FWV_ASK;
    f->tcp_est_s = 7200; f->tcp_trans_s = 60; f->udp_s = 30; f->icmp_s = 10;
}

uint32_t fw_add_rule(fw_engine *f, const fw_rule *r)
{
    if (f->nrule >= FW_MAX_RULES || r->action == FWV_ASK || r->nlports > 4 || r->nrports > 4) return 0;
    fw_rule *n = &f->rule[f->nrule++];
    *n = *r; n->id = f->next_id++;
    return n->id;
}
bool fw_remove_rule(fw_engine *f, uint32_t id)
{
    for (int i = 0; i < f->nrule; i++) if (f->rule[i].id == id) { memmove(&f->rule[i], &f->rule[i + 1], (size_t)(f->nrule - i - 1) * sizeof f->rule[0]); f->nrule--; return true; }
    return false;
}
bool fw_set_rule_enabled(fw_engine *f, uint32_t id, bool on) { for (int i = 0; i < f->nrule; i++) if (f->rule[i].id == id) { f->rule[i].enabled = on; return true; } return false; }

void fw_set_app_policy(fw_engine *f, const char *app, fw_app_policy p)
{
    for (int i = 0; i < f->napp; i++)
        if (!strcmp(f->app[i].app, app)) { if (p == FWA_ASK) f->app[i] = f->app[--f->napp]; else f->app[i].p = p; return; }
    if (p == FWA_ASK || f->napp >= FW_MAX_APPS) return;
    strncpy(f->app[f->napp].app, app, 31); f->app[f->napp].p = p; f->napp++;
}
fw_app_policy fw_get_app_policy(const fw_engine *f, const char *app)
{
    for (int i = 0; i < f->napp; i++) if (!strcmp(f->app[i].app, app)) return f->app[i].p;
    return FWA_ASK;
}
void fw_set_hard_off(fw_engine *f, bool off) { f->hard_off = off; }

/* ---- matching ---- */
static bool ip_is_loopback(const fw_ip *a) { if (!a->v6) return a->a4.b[0] == 127; static const uint8_t l[16] = { [15] = 1 }; return !memcmp(a->a6.b, l, 16); }
static bool addr_match(const fw_addr_match *m, const fw_ip *a)
{
    if (m->any) return true;
    if (m->v6 != a->v6) return false;
    return a->v6 ? ip6_in_cidr(&a->a6, m->c6) : ip4_in_cidr(a->a4, m->c4);
}
static bool ports_match(const fw_port_range *r, int n, uint16_t port)
{
    if (!n) return true;
    for (int i = 0; i < n; i++) if (port >= r[i].lo && port <= r[i].hi) return true;
    return false;
}
static bool rule_match(const fw_rule *r, const fw_packet *p)
{
    if (!r->enabled) return false;
    if (r->dir >= 0 && r->dir != (int)p->dir) return false;
    if (r->proto != FW_PROTO_ANY && r->proto != p->proto) return false;
    if (r->ifindex && r->ifindex != p->ifindex) return false;
    if (r->app[0] && !fw_glob(r->app, p->app)) return false;
    if (!addr_match(&r->local, &p->local) || !addr_match(&r->remote, &p->remote)) return false;
    bool ports = p->proto == FW_TCP || p->proto == FW_UDP;
    if (ports && (!ports_match(r->lports, r->nlports, p->lport) || !ports_match(r->rports, r->nrports, p->rport))) return false;
    if (!ports && (r->nlports || r->nrports)) return false;
    return true;
}

/* ---- connection tracking ---- */
static bool ip_eq(const fw_ip *a, const fw_ip *b) { return a->v6 == b->v6 && (a->v6 ? !memcmp(a->a6.b, b->a6.b, 16) : ip4_eq(a->a4, b->a4)); }
static fw_flow *ct_find(fw_engine *f, const fw_packet *p)
{
    for (int i = 0; i < FW_CT_SIZE; i++) {
        fw_flow *c = &f->flow[i];
        if (!c->used || c->proto != p->proto || c->lport != p->lport || c->rport != p->rport) continue;
        if (ip_eq(&c->local, &p->local) && ip_eq(&c->remote, &p->remote)) return c;
    }
    return NULL;
}
static bool flow_expired(const fw_engine *f, const fw_flow *c, uint64_t now)
{
    uint64_t age = now - c->last_ms, lim;
    switch (c->proto) {
    case FW_TCP: lim = (c->state == 2 ? f->tcp_est_s : f->tcp_trans_s) * 1000ull; if (c->state == 4) lim = 5000; break;
    case FW_UDP: lim = f->udp_s * 1000ull; break;
    default: lim = f->icmp_s * 1000ull;
    }
    return age > lim;
}
static fw_flow *ct_new(fw_engine *f, const fw_packet *p, uint64_t now)
{
    fw_flow *slot = NULL, *oldest = NULL;
    for (int i = 0; i < FW_CT_SIZE; i++) {
        fw_flow *c = &f->flow[i];
        if (!c->used || flow_expired(f, c, now)) { slot = c; break; }
        if (!oldest || c->last_ms < oldest->last_ms) oldest = c;
    }
    if (!slot) slot = oldest;                                                          /* table full: recycle the least recently used */
    if (slot->used) f->nflow--;
    memset(slot, 0, sizeof *slot);
    slot->used = true; slot->proto = p->proto; slot->local = p->local; slot->remote = p->remote;
    slot->lport = p->lport; slot->rport = p->rport; slot->origin = p->dir; slot->last_ms = now;
    strncpy(slot->app, p->app, sizeof slot->app - 1);
    f->nflow++;
    return slot;
}
void fw_gc(fw_engine *f, uint64_t now)
{
    for (int i = 0; i < FW_CT_SIZE; i++) if (f->flow[i].used && flow_expired(f, &f->flow[i], now)) { f->flow[i].used = false; f->nflow--; }
}
int fw_flows(const fw_engine *f, fw_flow *out, int max) { int n = 0; for (int i = 0; i < FW_CT_SIZE && n < max; i++) if (f->flow[i].used) out[n++] = f->flow[i]; return n; }

/* update TCP state on a packet that belongs to flow c */
static void tcp_track(fw_flow *c, const fw_packet *p)
{
    if (p->tcp_flags & TCP_RST) { c->state = 4; return; }
    if (p->tcp_flags & TCP_FIN) { c->state = c->state >= 3 ? 4 : 3; return; }
    if (c->state == 0 && (p->tcp_flags & TCP_SYN)) c->state = 1;
    else if (c->state == 1 && p->dir != c->origin && (p->tcp_flags & (TCP_SYN | TCP_ACK)) == (TCP_SYN | TCP_ACK)) c->state = 2;   /* the peer answered the SYN */
}

static void log_it(fw_engine *f, const fw_packet *p, uint64_t now, fw_verdict v, fw_reason why, uint32_t rule)
{
    fw_log_entry *e = &f->log[f->log_head];
    e->t = now; e->dir = p->dir; e->proto = p->proto; e->remote = p->remote; e->rport = p->rport; e->verdict = v; e->reason = why; e->rule = rule;
    strncpy(e->app, p->app, sizeof e->app - 1); e->app[sizeof e->app - 1] = 0;
    f->log_head = (f->log_head + 1) % FW_LOG_SIZE;
    if (f->log_count < FW_LOG_SIZE) f->log_count++;
}
int fw_log_read(const fw_engine *f, fw_log_entry *out, int max)
{
    int n = (int)f->log_count < max ? (int)f->log_count : max;
    for (int i = 0; i < n; i++) out[i] = f->log[(f->log_head + FW_LOG_SIZE - 1 - (unsigned)i) % FW_LOG_SIZE];
    return n;
}

static bool is_initiator(const fw_packet *p)
{
    if (p->proto == FW_TCP) return (p->tcp_flags & TCP_SYN) && !(p->tcp_flags & TCP_ACK);
    return true;
}

fw_verdict fw_eval(fw_engine *f, const fw_packet *p, uint64_t now, fw_reason *why_out, uint32_t *rule_out)
{
    fw_reason why = FWR_DEFAULT; uint32_t rid = 0; fw_verdict v;
    bool log = false;
    if (p->proto != FW_ICMP && p->proto != FW_ICMP6 && p->proto != FW_TCP && p->proto != FW_UDP) { v = FWV_DROP; why = FWR_INVALID; goto done; }
    if (p->local.v6 != p->remote.v6) { v = FWV_DROP; why = FWR_INVALID; goto done; }
    if (ip_is_loopback(&p->local) && ip_is_loopback(&p->remote)) { v = FWV_ALLOW; why = FWR_LOOPBACK; goto done; }   /* local IPC survives the hard switch */
    if (f->hard_off) { v = FWV_DROP; why = FWR_HARD_OFF; goto done; }
    {
        fw_flow *c = ct_find(f, p);
        if (c && !flow_expired(f, c, now) && c->state != 4) {
            if (c->proto == FW_TCP) tcp_track(c, p);
            c->last_ms = now;
            if (p->dir == FW_IN) c->bytes_in += p->len; else c->bytes_out += p->len;
            v = FWV_ALLOW; why = FWR_ESTABLISHED; goto done;
        }
        if (c) { c->used = false; f->nflow--; c = NULL; }
        if (p->proto == FW_TCP && !is_initiator(p)) { v = FWV_DROP; why = FWR_INVALID; goto done; }       /* mid-stream segment with no flow */
    }
    {
        int best = -1;
        for (int i = 0; i < f->nrule; i++) {
            if (!rule_match(&f->rule[i], p)) continue;
            if (best < 0 || f->rule[i].weight > f->rule[best].weight || (f->rule[i].weight == f->rule[best].weight && f->rule[best].action == FWV_ALLOW && f->rule[i].action != FWV_ALLOW)) best = i;
        }
        if (best >= 0) { v = f->rule[best].action; why = FWR_RULE; rid = f->rule[best].id; log = f->rule[best].log; goto track; }
    }
    {
        fw_app_policy ap = p->app[0] ? fw_get_app_policy(f, p->app) : FWA_ASK;
        /* "allow" lets a process make outbound connections; it never opens inbound ones (that takes an explicit rule).
         * "deny" blocks the process in both directions. */
        if (ap == FWA_ALLOW && p->dir == FW_OUT) { v = FWV_ALLOW; why = FWR_APP_POLICY; goto track; }
        if (ap == FWA_DENY) { v = FWV_DROP; why = FWR_APP_POLICY; goto track; }
    }
    v = p->dir == FW_IN ? f->default_in : f->default_out; why = FWR_DEFAULT;
track:
    if (v == FWV_ALLOW && is_initiator(p)) {
        fw_flow *c = ct_new(f, p, now);
        if (p->proto == FW_TCP) tcp_track(c, p);
        if (p->dir == FW_IN) c->bytes_in += p->len; else c->bytes_out += p->len;
    }
done:
    switch (v) { case FWV_ALLOW: f->allowed++; break; case FWV_DROP: f->dropped++; break; case FWV_REJECT: f->rejected++; break; default: f->asked++; }
    if (log || (why != FWR_ESTABLISHED && why != FWR_LOOPBACK)) log_it(f, p, now, v, why, rid);       /* every decision except bulk traffic of known flows */
    if (why_out) *why_out = why;
    if (rule_out) *rule_out = rid;
    return v;
}

/* ---- rule text ---- */
static bool parse_ports(const char *s, fw_port_range *r, int *n)
{
    *n = 0;
    while (*s) {
        if (*n >= 4) return false;
        unsigned lo = 0, hi; int digits = 0;
        while (*s >= '0' && *s <= '9') { lo = lo * 10 + (unsigned)(*s++ - '0'); digits++; if (lo > 65535) return false; }
        if (!digits) return false;
        hi = lo;
        if (*s == '-') { s++; hi = 0; digits = 0; while (*s >= '0' && *s <= '9') { hi = hi * 10 + (unsigned)(*s++ - '0'); digits++; if (hi > 65535) return false; } if (!digits || hi < lo) return false; }
        r[*n].lo = (uint16_t)lo; r[*n].hi = (uint16_t)hi; (*n)++;
        if (*s == ',') s++; else if (*s) return false;
    }
    return *n > 0;
}
static bool parse_addr(const char *s, fw_addr_match *m)
{
    memset(m, 0, sizeof *m);
    if (!strcmp(s, "any")) { m->any = true; return true; }
    if (strchr(s, ':')) { m->v6 = true; return ip6_cidr_parse(s, &m->c6); }
    return ip4_cidr_parse(s, &m->c4);
}

bool fw_rule_parse(fw_rule *r, const char *line)
{
    memset(r, 0, sizeof *r);
    r->enabled = true; r->dir = -1; r->local.any = r->remote.any = true;
    char buf[200]; strncpy(buf, line, sizeof buf - 1); buf[sizeof buf - 1] = 0;
    char *tok[40]; int nt = 0;
    for (char *p = buf; *p;) { while (*p == ' ' || *p == '\t') p++; if (!*p) break; if (nt >= 40) return false; tok[nt++] = p; while (*p && *p != ' ' && *p != '\t') p++; if (*p) *p++ = 0; }
    int i = 0;
    if (i >= nt) return false;
    if (!strcmp(tok[i], "allow")) r->action = FWV_ALLOW; else if (!strcmp(tok[i], "deny") || !strcmp(tok[i], "drop")) r->action = FWV_DROP; else if (!strcmp(tok[i], "reject")) r->action = FWV_REJECT; else return false;
    i++;
    while (i < nt) {
        const char *k = tok[i++];
        if (!strcmp(k, "in")) r->dir = FW_IN; else if (!strcmp(k, "out")) r->dir = FW_OUT;
        else if (!strcmp(k, "tcp")) r->proto = FW_TCP; else if (!strcmp(k, "udp")) r->proto = FW_UDP; else if (!strcmp(k, "icmp")) r->proto = FW_ICMP; else if (!strcmp(k, "icmp6")) r->proto = FW_ICMP6;
        else if (!strcmp(k, "log")) r->log = true; else if (!strcmp(k, "disabled")) r->enabled = false;
        else if (i < nt && !strcmp(k, "local")) { if (!parse_addr(tok[i++], &r->local)) return false; }
        else if (i < nt && !strcmp(k, "remote")) { if (!parse_addr(tok[i++], &r->remote)) return false; }
        else if (i < nt && (!strcmp(k, "from") || !strcmp(k, "to"))) {          /* source/destination: needs a direction to mean local or remote */
            if (r->dir < 0) return false;
            bool src = !strcmp(k, "from"), local = (r->dir == FW_OUT) == src;
            if (!parse_addr(tok[i++], local ? &r->local : &r->remote)) return false;
        }
        else if (i < nt && (!strcmp(k, "port") || !strcmp(k, "rport"))) { if (!parse_ports(tok[i++], r->rports, &r->nrports)) return false; }
        else if (i < nt && !strcmp(k, "lport")) { if (!parse_ports(tok[i++], r->lports, &r->nlports)) return false; }
        else if (i < nt && !strcmp(k, "app")) { if (strlen(tok[i]) >= sizeof r->app) return false; strcpy(r->app, tok[i++]); }
        else if (i < nt && !strcmp(k, "weight")) { char *e; long w = strtol(tok[i], &e, 10); if (*e || w < -1000 || w > 1000) return false; r->weight = (int)w; i++; }
        else if (i < nt && !strcmp(k, "name")) { if (strlen(tok[i]) >= sizeof r->name) return false; strcpy(r->name, tok[i++]); }
        else return false;
    }
    if ((r->nlports || r->nrports) && r->proto != FW_TCP && r->proto != FW_UDP) return false;     /* ports need tcp or udp */
    return true;
}

static void fmt_cidr(const fw_addr_match *m, char *out, size_t cap)
{
    if (m->any) { snprintf(out, cap, "any"); return; }
    char a[48];
    if (m->v6) { ip6_format(&m->c6.addr, a, sizeof a); snprintf(out, cap, "%s/%u", a, m->c6.prefix); }
    else { ip4_format(m->c4.addr, a, sizeof a); snprintf(out, cap, "%s/%u", a, m->c4.prefix); }
}
size_t fw_rule_format(const fw_rule *r, char *out, size_t cap)
{
    size_t n = 0; char t[64];
    #define PUT(...) do { int k_ = snprintf(out + n, n < cap ? cap - n : 0, __VA_ARGS__); if (k_ > 0) n += (size_t)k_; } while (0)
    PUT("%s", r->action == FWV_ALLOW ? "allow" : r->action == FWV_REJECT ? "reject" : "deny");
    if (r->dir == FW_IN) PUT(" in"); else if (r->dir == FW_OUT) PUT(" out");
    if (r->proto) PUT(" %s", r->proto == FW_TCP ? "tcp" : r->proto == FW_UDP ? "udp" : r->proto == FW_ICMP ? "icmp" : "icmp6");
    if (!r->local.any) { fmt_cidr(&r->local, t, sizeof t); PUT(" local %s", t); }
    if (!r->remote.any) { fmt_cidr(&r->remote, t, sizeof t); PUT(" remote %s", t); }
    if (r->nlports) { PUT(" lport "); for (int i = 0; i < r->nlports; i++) { if (r->lports[i].lo == r->lports[i].hi) PUT("%s%u", i ? "," : "", r->lports[i].lo); else PUT("%s%u-%u", i ? "," : "", r->lports[i].lo, r->lports[i].hi); } }
    if (r->nrports) { PUT(" port "); for (int i = 0; i < r->nrports; i++) { if (r->rports[i].lo == r->rports[i].hi) PUT("%s%u", i ? "," : "", r->rports[i].lo); else PUT("%s%u-%u", i ? "," : "", r->rports[i].lo, r->rports[i].hi); } }
    if (r->app[0]) PUT(" app %s", r->app);
    if (r->weight) PUT(" weight %d", r->weight);
    if (r->log) PUT(" log");
    if (!r->enabled) PUT(" disabled");
    if (r->name[0]) PUT(" name %s", r->name);
    #undef PUT
    return n;
}
