#include "dns.h"
#include <string.h>
#include <stdio.h>

static int lc(int c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }
static bool name_eq(const char *a, const char *b) { for (; *a && lc(*a) == lc(*b); a++, b++) { } return !*a && !*b; }

bool dns_name_valid(const char *name)
{
    size_t n = strlen(name);
    if (n && name[n - 1] == '.') n--;
    if (!n || n > 253) return false;
    size_t label = 0;
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)name[i];
        if (c == '.') { if (!label || name[i - 1] == '-') return false; label = 0; continue; }
        bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '-' || c == '_';
        if (!ok || (!label && c == '-') || ++label > 63) return false;
    }
    return label && name[n - 1] != '-';
}
void dns_name_normalize(const char *in, char *out, size_t cap)
{
    size_t n = strlen(in);
    if (n && in[n - 1] == '.') n--;
    if (n >= cap) n = cap - 1;
    for (size_t i = 0; i < n; i++) out[i] = (char)lc((unsigned char)in[i]);
    out[n] = 0;
}

/* ---- wire names ---- */
static int write_name(uint8_t *buf, size_t cap, size_t pos, const char *name, const char (*seen)[DNS_NAME_MAX + 1], const uint16_t *seen_off, int nseen, bool compress, int *wrote_ptr_to)
{
    (void)wrote_ptr_to;
    size_t start = pos;
    const char *p = name;
    if (!*p) { if (pos >= cap) return -1; buf[pos++] = 0; return (int)(pos - start); }
    while (*p) {
        if (compress) {
            for (int i = 0; i < nseen; i++)
                if (name_eq(seen[i], p) && seen_off[i] < 0x3fff) {
                    if (pos + 2 > cap) return -1;
                    net_wr16(buf + pos, (uint16_t)(0xC000 | seen_off[i])); pos += 2;
                    return (int)(pos - start);
                }
        }
        const char *dot = strchr(p, '.');
        size_t len = dot ? (size_t)(dot - p) : strlen(p);
        if (!len || len > 63 || pos + 1 + len + 1 > cap) return -1;
        buf[pos++] = (uint8_t)len; memcpy(buf + pos, p, len); pos += len;
        p += len; if (*p == '.') p++;
    }
    if (pos >= cap) return -1;
    buf[pos++] = 0;
    return (int)(pos - start);
}

/* returns the position after the name at `pos` (in its original place), name decoded into out */
static int read_name(const uint8_t *buf, size_t len, size_t pos, char *out, size_t cap)
{
    size_t outn = 0, p = pos; int jumps = 0; long after = -1;
    for (;;) {
        if (p >= len) return -1;
        uint8_t c = buf[p];
        if ((c & 0xC0) == 0xC0) {
            if (p + 1 >= len) return -1;
            size_t target = (size_t)((c & 0x3f) << 8 | buf[p + 1]);
            if (after < 0) after = (long)p + 2;
            if (target >= p || ++jumps > 32) return -1;                 /* pointers only go backwards: no loops */
            p = target;
            continue;
        }
        if (c & 0xC0) return -1;                                        /* 0x40/0x80 label types are unsupported */
        if (c == 0) { p++; break; }
        if (p + 1 + c > len) return -1;
        if (outn + c + 1 >= cap || outn + c > 253) return -1;
        if (outn) out[outn++] = '.';
        for (uint8_t i = 0; i < c; i++) {
            int ch = lc(buf[p + 1 + i]);
            if (ch == '.' || ch == 0) return -1;
            out[outn++] = (char)ch;
        }
        p += 1 + c;
    }
    out[outn] = 0;
    return (int)(after >= 0 ? (size_t)after : p);
}

int dns_build_query(uint8_t *buf, size_t cap, uint16_t id, const char *name, uint16_t qtype, bool rd, uint16_t edns_udp)
{
    if (cap < 12 || !dns_name_valid(name)) return -1;
    memset(buf, 0, 12);
    net_wr16(buf, id); net_wr16(buf + 2, rd ? DNS_FLAG_RD : 0); net_wr16(buf + 4, 1); net_wr16(buf + 10, edns_udp ? 1 : 0);
    int n = write_name(buf, cap, 12, name, NULL, NULL, 0, false, NULL);
    if (n < 0 || 12 + (size_t)n + 4 + (edns_udp ? 11 : 0) > cap) return -1;
    size_t pos = 12 + (size_t)n;
    net_wr16(buf + pos, qtype); net_wr16(buf + pos + 2, DNS_CLASS_IN); pos += 4;
    if (edns_udp) { buf[pos] = 0; net_wr16(buf + pos + 1, DNS_OPT); net_wr16(buf + pos + 3, edns_udp); net_wr32(buf + pos + 5, 0); net_wr16(buf + pos + 9, 0); pos += 11; }
    return (int)pos;
}

static bool parse_rr(const uint8_t *buf, size_t len, size_t *pos, dns_rr *rr, bool *keep)
{
    int n = read_name(buf, len, *pos, rr->name, sizeof rr->name);
    if (n < 0) return false;
    size_t p = (size_t)n;
    if (p + 10 > len) return false;
    rr->type = net_rd16(buf + p); rr->cls = net_rd16(buf + p + 2); rr->ttl = net_rd32(buf + p + 4);
    if (rr->ttl & 0x80000000u) rr->ttl = 0;                              /* RFC 2181: top bit set = zero */
    size_t rdlen = net_rd16(buf + p + 8); p += 10;
    if (p + rdlen > len) return false;
    const uint8_t *rd = buf + p; size_t end = p + rdlen;
    *keep = true;
    switch (rr->type) {
    case DNS_A: if (rdlen != 4) return false; memcpy(rr->d.a.b, rd, 4); break;
    case DNS_AAAA: if (rdlen != 16) return false; memcpy(rr->d.aaaa.b, rd, 16); break;
    case DNS_CNAME: case DNS_NS: case DNS_PTR:
        if (read_name(buf, len, p, rr->d.target, sizeof rr->d.target) < 0) return false;
        break;
    case DNS_MX:
        if (rdlen < 3) return false;
        rr->d.mx.pref = net_rd16(rd);
        if (read_name(buf, len, p + 2, rr->d.mx.host, sizeof rr->d.mx.host) < 0) return false;
        break;
    case DNS_SRV:
        if (rdlen < 7) return false;
        rr->d.srv.prio = net_rd16(rd); rr->d.srv.weight = net_rd16(rd + 2); rr->d.srv.port = net_rd16(rd + 4);
        if (read_name(buf, len, p + 6, rr->d.srv.host, sizeof rr->d.srv.host) < 0) return false;
        break;
    case DNS_TXT: {
        size_t o = 0, q = 0;
        while (q < rdlen) {
            size_t l = rd[q++];
            if (q + l > rdlen) return false;
            size_t room = sizeof rr->d.txt.text - 1 - o;
            size_t c = l < room ? l : room;
            memcpy(rr->d.txt.text + o, rd + q, c); o += c; q += l;
        }
        rr->d.txt.text[o] = 0;
        break;
    }
    case DNS_SOA: {
        int m = read_name(buf, len, p, rr->d.soa.mname, sizeof rr->d.soa.mname);
        if (m < 0) return false;
        char rname[DNS_NAME_MAX + 1];
        int r = read_name(buf, len, (size_t)m, rname, sizeof rname);
        if (r < 0 || (size_t)r + 20 > end) return false;
        rr->d.soa.serial = net_rd32(buf + r); rr->d.soa.refresh = net_rd32(buf + r + 4); rr->d.soa.retry = net_rd32(buf + r + 8);
        rr->d.soa.expire = net_rd32(buf + r + 12); rr->d.soa.minimum = net_rd32(buf + r + 16);
        break;
    }
    default: *keep = false;
    }
    *pos = end;
    return true;
}

bool dns_parse(const uint8_t *buf, size_t len, dns_msg *m)
{
    if (len < 12) return false;
    memset(m, 0, sizeof *m);
    m->id = net_rd16(buf); m->flags = net_rd16(buf + 2); m->rcode = (uint8_t)(m->flags & 15);
    m->truncated = (m->flags & DNS_FLAG_TC) != 0;
    unsigned qd = net_rd16(buf + 4), an = net_rd16(buf + 6), ns = net_rd16(buf + 8), ar = net_rd16(buf + 10);
    if (qd > 1 || (an + ns + ar) * 11u > len) return false;               /* counts must fit in the packet: cheap DoS guard */
    size_t pos = 12;
    if (qd) {
        int n = read_name(buf, len, pos, m->qname, sizeof m->qname);
        if (n < 0 || (size_t)n + 4 > len) return false;
        m->qtype = net_rd16(buf + n); m->qclass = net_rd16(buf + n + 2);
        pos = (size_t)n + 4;
    }
    for (unsigned i = 0; i < an + ns + ar; i++) {
        dns_rr rr; bool keep;
        memset(&rr, 0, sizeof rr);
        if (!parse_rr(buf, len, &pos, &rr, &keep)) return false;
        if (!keep) continue;
        if (i < an) { if (m->nan < DNS_MAX_RR) m->an[m->nan++] = rr; }
        else if (i < an + ns) { if (m->nns < 4 && rr.type == DNS_SOA) m->ns[m->nns++] = rr; }
    }
    return true;
}

/* ---- response builder (tests, local responder, mDNS-style tools) ---- */
int dns_build_response(uint8_t *buf, size_t cap, const dns_msg *m)
{
    if (cap < 12) return -1;
    char seen[16][DNS_NAME_MAX + 1]; uint16_t soff[16]; int nseen = 0;
    memset(buf, 0, 12);
    net_wr16(buf, m->id); net_wr16(buf + 2, (uint16_t)(DNS_FLAG_QR | DNS_FLAG_RA | DNS_FLAG_RD | (m->truncated ? DNS_FLAG_TC : 0) | (m->rcode & 15)));
    net_wr16(buf + 4, 1); net_wr16(buf + 6, (uint16_t)m->nan); net_wr16(buf + 8, (uint16_t)m->nns);
    size_t pos = 12;
    int n = write_name(buf, cap, pos, m->qname, seen, soff, 0, false, NULL);
    if (n < 0) return -1;
    if (nseen < 16) { strcpy(seen[nseen], m->qname); soff[nseen++] = (uint16_t)pos; }
    pos += (size_t)n;
    if (pos + 4 > cap) return -1;
    net_wr16(buf + pos, m->qtype); net_wr16(buf + pos + 2, m->qclass ? m->qclass : DNS_CLASS_IN); pos += 4;
    const dns_rr *all[DNS_MAX_RR + 4]; int nall = 0;
    for (int i = 0; i < m->nan; i++) all[nall++] = &m->an[i];
    for (int i = 0; i < m->nns; i++) all[nall++] = &m->ns[i];
    for (int i = 0; i < nall; i++) {
        const dns_rr *r = all[i];
        n = write_name(buf, cap, pos, r->name, seen, soff, nseen, true, NULL);
        if (n < 0) return -1;
        if (nseen < 16 && n > 2) { strcpy(seen[nseen], r->name); soff[nseen++] = (uint16_t)pos; }
        pos += (size_t)n;
        if (pos + 10 > cap) return -1;
        net_wr16(buf + pos, r->type); net_wr16(buf + pos + 2, r->cls ? r->cls : DNS_CLASS_IN); net_wr32(buf + pos + 4, r->ttl);
        size_t rdlen_at = pos + 8; pos += 10;
        size_t rd0 = pos;
        switch (r->type) {
        case DNS_A: if (pos + 4 > cap) return -1; memcpy(buf + pos, r->d.a.b, 4); pos += 4; break;
        case DNS_AAAA: if (pos + 16 > cap) return -1; memcpy(buf + pos, r->d.aaaa.b, 16); pos += 16; break;
        case DNS_CNAME: case DNS_NS: case DNS_PTR:
            n = write_name(buf, cap, pos, r->d.target, seen, soff, nseen, true, NULL); if (n < 0) return -1; pos += (size_t)n; break;
        case DNS_MX:
            if (pos + 2 > cap) return -1; net_wr16(buf + pos, r->d.mx.pref); pos += 2;
            n = write_name(buf, cap, pos, r->d.mx.host, seen, soff, nseen, true, NULL); if (n < 0) return -1; pos += (size_t)n; break;
        case DNS_TXT: {
            size_t l = strlen(r->d.txt.text), o = 0;
            do { size_t c = l - o > 255 ? 255 : l - o; if (pos + 1 + c > cap) return -1; buf[pos++] = (uint8_t)c; memcpy(buf + pos, r->d.txt.text + o, c); pos += c; o += c; } while (o < l);
            break;
        }
        case DNS_SOA: {
            n = write_name(buf, cap, pos, r->d.soa.mname, seen, soff, nseen, true, NULL); if (n < 0) return -1; pos += (size_t)n;
            n = write_name(buf, cap, pos, "hostmaster", seen, soff, 0, false, NULL); if (n < 0) return -1; pos += (size_t)n;
            if (pos + 20 > cap) return -1;
            net_wr32(buf + pos, r->d.soa.serial); net_wr32(buf + pos + 4, r->d.soa.refresh); net_wr32(buf + pos + 8, r->d.soa.retry);
            net_wr32(buf + pos + 12, r->d.soa.expire); net_wr32(buf + pos + 16, r->d.soa.minimum); pos += 20;
            break;
        }
        default: return -1;
        }
        net_wr16(buf + rdlen_at, (uint16_t)(pos - rd0));
    }
    return (int)pos;
}

/* ---- cache ---- */
void dns_cache_init(dns_cache *c) { memset(c, 0, sizeof *c); }
void dns_cache_flush(dns_cache *c) { dns_cache_init(c); }

void dns_cache_put(dns_cache *c, const char *name, uint16_t type, uint8_t rcode, const dns_rr *rr, int nrr, uint32_t ttl, uint64_t now)
{
    if (ttl == 0) return;                                                 /* TTL 0 means "do not cache" */
    if (ttl > DNS_CACHE_MAX_TTL) ttl = DNS_CACHE_MAX_TTL;
    if (nrr > 8) nrr = 8;
    int slot = -1;
    for (int i = 0; i < DNS_CACHE_N && slot < 0; i++)                     /* 1: replace the same key */
        if (c->e[i].used && c->e[i].type == type && name_eq(c->e[i].name, name)) slot = i;
    for (int i = 0; i < DNS_CACHE_N && slot < 0; i++)                     /* 2: a free or expired slot */
        if (!c->e[i].used || c->e[i].expire_ms <= now) slot = i;
    if (slot < 0) {                                                       /* 3: all live: evict the entry expiring soonest */
        uint64_t best = UINT64_MAX;
        for (int i = 0; i < DNS_CACHE_N; i++) if (c->e[i].expire_ms < best) { best = c->e[i].expire_ms; slot = i; }
    }
    c->e[slot].used = true;
    dns_name_normalize(name, c->e[slot].name, sizeof c->e[slot].name);
    c->e[slot].type = type; c->e[slot].rcode = rcode; c->e[slot].nrr = nrr;
    for (int i = 0; i < nrr; i++) c->e[slot].rr[i] = rr[i];
    c->e[slot].expire_ms = now + (uint64_t)ttl * 1000u;
}

int dns_cache_get(dns_cache *c, const char *name, uint16_t type, uint64_t now, dns_rr *rr, int max, int *nrr, uint8_t *rcode)
{
    for (int i = 0; i < DNS_CACHE_N; i++) {
        if (!c->e[i].used || c->e[i].type != type || !name_eq(c->e[i].name, name)) continue;
        if (now >= c->e[i].expire_ms) { c->e[i].used = false; return 0; }
        uint32_t left = (uint32_t)((c->e[i].expire_ms - now) / 1000);
        if (!left) left = 1;
        int n = c->e[i].nrr < max ? c->e[i].nrr : max;
        for (int k = 0; k < n; k++) { rr[k] = c->e[i].rr[k]; if (rr[k].ttl > left) rr[k].ttl = left; }
        if (nrr) *nrr = n;
        if (rcode) *rcode = c->e[i].rcode;
        return c->e[i].nrr == 0 ? 2 : 1;
    }
    return 0;
}

/* ---- resolver ---- */
void dns_res_init(dns_resolver *r, const dns_config *cfg, dns_cache *cache)
{
    memset(r, 0, sizeof *r);
    r->cfg = *cfg; r->cache = cache;
    if (!r->cfg.timeout_ms) r->cfg.timeout_ms = 2000;
    if (r->cfg.attempts < 1) r->cfg.attempts = 2;
}

static void fail(dns_resolver *r, dns_error e) { r->state = DNSR_FAILED; r->error = e; r->in_flight = false; }

static void set_qname(dns_resolver *r, const char *n) { strncpy(r->qname, n, DNS_NAME_MAX); r->qname[DNS_NAME_MAX] = 0; r->server = 0; r->attempt = 0; r->in_flight = false; }

static void finish_ok(dns_resolver *r, const dns_rr *rr, int n, const char *canon, uint64_t now)
{
    r->nresult = 0; r->min_ttl = UINT32_MAX;
    for (int i = 0; i < n && r->nresult < DNS_MAX_RR; i++) { r->result[r->nresult++] = rr[i]; if (rr[i].ttl < r->min_ttl) r->min_ttl = rr[i].ttl; }
    strncpy(r->canonical, canon, DNS_NAME_MAX); r->canonical[DNS_NAME_MAX] = 0;
    r->state = DNSR_DONE; r->error = DNSE_NONE; r->in_flight = false;
    (void)now;
}

static void next_candidate(dns_resolver *r, dns_error why, uint64_t now);

/* Look at the cache for the current qname; returns true if it settled the question. */
static bool try_cache(dns_resolver *r, uint64_t now)
{
    if (!r->cache) return false;
    dns_rr rr[8]; int n = 0; uint8_t rc = 0;
    int hit = dns_cache_get(r->cache, r->qname, r->qtype, now, rr, 8, &n, &rc);
    if (hit == 1) { finish_ok(r, rr, n, r->qname, now); return true; }
    if (hit == 2) { next_candidate(r, rc == DNS_RCODE_NXDOMAIN ? DNSE_NXDOMAIN : DNSE_NODATA, now); return true; }
    return false;
}

static void next_candidate(dns_resolver *r, dns_error why, uint64_t now)
{
    if (r->cname_depth > 0 || r->cur + 1 >= r->ncand) { fail(r, why); return; }
    r->cur++;
    set_qname(r, r->cand[r->cur]);
    if (try_cache(r, now)) return;
    r->state = DNSR_WAIT;
}

bool dns_res_start(dns_resolver *r, const char *name, uint16_t qtype, uint64_t now)
{
    dns_config keep = r->cfg; dns_cache *cache = r->cache;
    dns_res_init(r, &keep, cache);
    if (!name || !dns_name_valid(name) || r->cfg.nserver < 1) { r->state = DNSR_FAILED; r->error = DNSE_BAD_NAME; return false; }
    strncpy(r->name, name, DNS_NAME_MAX); r->qtype = qtype;
    bool absolute = name[strlen(name) - 1] == '.';
    char base[DNS_NAME_MAX + 1];
    dns_name_normalize(name, base, sizeof base);
    int dots = 0; for (const char *p = base; *p; p++) dots += *p == '.';
    r->ncand = 0;
    #define ADD_CAND(s) do { if (strlen(s) <= 253) { strcpy(r->cand[r->ncand++], (s)); } } while (0)
    if (absolute) ADD_CAND(base);
    else {
        char with[DNS_MAX_SEARCH][DNS_NAME_MAX + 1];
        int nw = 0;
        for (int i = 0; i < r->cfg.nsearch; i++) {
            char dom[64]; dns_name_normalize(r->cfg.search[i], dom, sizeof dom);
            if (dom[0] && strlen(base) + 1 + strlen(dom) <= 253) { snprintf(with[nw], sizeof with[nw], "%s.%s", base, dom); nw++; }
        }
        if (dots >= r->cfg.ndots) { ADD_CAND(base); for (int i = 0; i < nw; i++) ADD_CAND(with[i]); }
        else { for (int i = 0; i < nw; i++) ADD_CAND(with[i]); ADD_CAND(base); }
    }
    #undef ADD_CAND
    r->cur = 0; r->cname_depth = 0;
    set_qname(r, r->cand[0]);
    r->state = DNSR_WAIT; r->error = DNSE_NONE;
    try_cache(r, now);
    return true;
}

int dns_res_poll(dns_resolver *r, uint64_t now, uint8_t *pkt, size_t cap, int *server_idx)
{
    if (r->state != DNSR_WAIT) return 0;
    if (r->in_flight) {
        if (now - r->sent_ms < r->cfg.timeout_ms) return 0;
        if (++r->attempt >= r->cfg.attempts) { r->attempt = 0; r->server++; }
        if (r->server >= r->cfg.nserver) { fail(r, DNSE_TIMEOUT); return 0; }
    }
    r->txid = r->cfg.rand16 ? r->cfg.rand16(r->cfg.user) : (uint16_t)(now * 2654435761u);
    int n = dns_build_query(pkt, cap, r->txid, r->qname, r->qtype, true, 1232);
    if (n < 0) { fail(r, DNSE_BAD_NAME); return 0; }
    r->in_flight = true; r->sent_ms = now;
    if (server_idx) *server_idx = r->server;
    return n;
}

uint64_t dns_res_next_wakeup(const dns_resolver *r) { return r->state == DNSR_WAIT && r->in_flight ? r->sent_ms + r->cfg.timeout_ms : 0; }

void dns_res_input(dns_resolver *r, const uint8_t *pkt, size_t len, ip4_addr from, uint64_t now)
{
    if ((r->state != DNSR_WAIT && r->state != DNSR_NEED_TCP) || !r->in_flight) return;
    if (!ip4_eq(from, r->cfg.server[r->server])) return;                  /* only the server we asked */
    static dns_msg m;
    if (!dns_parse(pkt, len, &m)) return;
    if (!(m.flags & DNS_FLAG_QR) || m.id != r->txid || m.qtype != r->qtype || !name_eq(m.qname, r->qname)) return;   /* spoof/stale: ignore */
    if (m.truncated && r->state == DNSR_WAIT) { r->state = DNSR_NEED_TCP; return; }
    r->state = DNSR_WAIT;
    switch (m.rcode) {
    case DNS_RCODE_OK: break;
    case DNS_RCODE_NXDOMAIN: {
        uint32_t neg = m.nns ? (m.ns[0].d.soa.minimum < m.ns[0].ttl ? m.ns[0].d.soa.minimum : m.ns[0].ttl) : DNS_CACHE_NEG_TTL;
        if (neg > DNS_CACHE_NEG_TTL * 12) neg = DNS_CACHE_NEG_TTL * 12;
        if (r->cache) dns_cache_put(r->cache, r->qname, r->qtype, DNS_RCODE_NXDOMAIN, NULL, 0, neg, now);
        next_candidate(r, DNSE_NXDOMAIN, now);
        return;
    }
    default:                                                              /* SERVFAIL/REFUSED/...: try the next server */
        r->server++; r->attempt = 0; r->in_flight = false;
        if (r->server >= r->cfg.nserver) fail(r, DNSE_SERVFAIL);
        return;
    }
    /* follow the CNAME chain inside this answer */
    char cur[DNS_NAME_MAX + 1]; strcpy(cur, r->qname);
    dns_rr found[DNS_MAX_RR]; int nf = 0;
    for (int depth = 0; depth < 9; depth++) {
        bool moved = false;
        for (int i = 0; i < m.nan; i++) if (m.an[i].type == DNS_CNAME && name_eq(m.an[i].name, cur)) {
            if (depth == 8) { fail(r, DNSE_LOOP); return; }
            strcpy(cur, m.an[i].d.target); moved = true; break;
        }
        if (!moved) break;
    }
    for (int i = 0; i < m.nan; i++) if ((m.an[i].type == r->qtype || r->qtype == DNS_ANY) && name_eq(m.an[i].name, cur) && nf < DNS_MAX_RR) found[nf++] = m.an[i];
    if (nf) {
        uint32_t ttl = UINT32_MAX;
        for (int i = 0; i < nf; i++) if (found[i].ttl < ttl) ttl = found[i].ttl;
        if (r->cache) dns_cache_put(r->cache, r->qname, r->qtype, DNS_RCODE_OK, found, nf, ttl, now);
        finish_ok(r, found, nf, cur, now);
        return;
    }
    if (!name_eq(cur, r->qname)) {                                        /* chain ends at a name this server did not resolve */
        if (++r->cname_depth > 8) { fail(r, DNSE_LOOP); return; }
        set_qname(r, cur);
        if (try_cache(r, now)) return;
        r->state = DNSR_WAIT;
        return;
    }
    uint32_t neg = m.nns ? (m.ns[0].d.soa.minimum < m.ns[0].ttl ? m.ns[0].d.soa.minimum : m.ns[0].ttl) : DNS_CACHE_NEG_TTL;
    if (r->cache) dns_cache_put(r->cache, r->qname, r->qtype, DNS_RCODE_OK, NULL, 0, neg > DNS_CACHE_NEG_TTL * 12 ? DNS_CACHE_NEG_TTL * 12 : neg, now);
    next_candidate(r, DNSE_NODATA, now);
}
