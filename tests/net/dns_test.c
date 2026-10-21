#include "dns.h"
#include "test.h"
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

static uint16_t next_id = 0x1000;
static uint16_t rnd16(void *u) { (void)u; return next_id++; }
static const ip4_addr S1 = { { 10, 0, 0, 1 } }, S2 = { { 10, 0, 0, 2 } };

static dns_rr rr_a(const char *name, uint32_t ttl, int d) { dns_rr r; memset(&r, 0, sizeof r); strcpy(r.name, name); r.type = DNS_A; r.cls = 1; r.ttl = ttl; r.d.a = (ip4_addr){ { 93, 184, 216, (uint8_t)d } }; return r; }
static dns_rr rr_cname(const char *name, const char *to, uint32_t ttl) { dns_rr r; memset(&r, 0, sizeof r); strcpy(r.name, name); r.type = DNS_CNAME; r.cls = 1; r.ttl = ttl; strcpy(r.d.target, to); return r; }

/* craft a reply to the query currently in `q` */
static int reply(const uint8_t *q, size_t ql, uint8_t rcode, const dns_rr *an, int nan, uint32_t soa_min, bool tc, uint8_t *out, size_t cap)
{
    dns_msg qm; dns_parse(q, ql, &qm);
    dns_msg m; memset(&m, 0, sizeof m);
    m.id = qm.id; m.rcode = rcode; m.truncated = tc; strcpy(m.qname, qm.qname); m.qtype = qm.qtype;
    for (int i = 0; i < nan; i++) m.an[m.nan++] = an[i];
    if (soa_min) { dns_rr s; memset(&s, 0, sizeof s); strcpy(s.name, "example.com"); s.type = DNS_SOA; s.cls = 1; s.ttl = 3600; strcpy(s.d.soa.mname, "ns.example.com"); s.d.soa.minimum = soa_min; m.ns[m.nns++] = s; }
    return dns_build_response(out, cap, &m);
}

int main(void)
{
    uint8_t q[512], r[512];
    /* ---- names ---- */
    CHECK(dns_name_valid("example.com") && dns_name_valid("a-b.c1.org.") && dns_name_valid("_srv._tcp.example.com") && dns_name_valid("x"));
    CHECK(!dns_name_valid("") && !dns_name_valid(".") && !dns_name_valid("a..b") && !dns_name_valid("-a.com") && !dns_name_valid("a-.com") && !dns_name_valid("a b.com") && !dns_name_valid("ex%ample.com"));
    char longlabel[80]; memset(longlabel, 'a', 64); strcpy(longlabel + 64, ".com"); CHECK(!dns_name_valid(longlabel));
    memset(longlabel, 'a', 63); strcpy(longlabel + 63, ".com"); CHECK(dns_name_valid(longlabel));
    char longname[300]; longname[0] = 0; for (int i = 0; i < 70; i++) strcat(longname, "abcd."); CHECK(!dns_name_valid(longname));
    char norm[64]; dns_name_normalize("WwW.ExAmPlE.CoM.", norm, sizeof norm); CHECK(!strcmp(norm, "www.example.com"));

    /* ---- codec ---- */
    int n = dns_build_query(q, sizeof q, 0xbeef, "Example.com", DNS_A, true, 1232);
    CHECK(n == 12 + 13 + 4 + 11 && net_rd16(q) == 0xbeef && net_rd16(q + 2) == DNS_FLAG_RD && net_rd16(q + 4) == 1 && net_rd16(q + 10) == 1);
    dns_msg m; CHECK(dns_parse(q, (size_t)n, &m) && !strcmp(m.qname, "example.com") && m.qtype == DNS_A && m.qclass == 1 && m.id == 0xbeef);
    CHECK(dns_build_query(q, 10, 1, "example.com", DNS_A, true, 0) == -1 && dns_build_query(q, sizeof q, 1, "bad..name", DNS_A, true, 0) == -1);
    CHECK(dns_build_query(q, sizeof q, 1, "example.com", DNS_A, false, 0) == 12 + 13 + 4 && net_rd16(q + 2) == 0);

    /* every record type round-trips through the compressing encoder */
    dns_msg src; memset(&src, 0, sizeof src);
    src.id = 7; strcpy(src.qname, "mail.example.com"); src.qtype = DNS_ANY;
    src.an[src.nan++] = rr_a("mail.example.com", 300, 5);
    dns_rr aaaa; memset(&aaaa, 0, sizeof aaaa); strcpy(aaaa.name, "mail.example.com"); aaaa.type = DNS_AAAA; aaaa.ttl = 60; ip6_parse("2001:db8::25", &aaaa.d.aaaa); src.an[src.nan++] = aaaa;
    src.an[src.nan++] = rr_cname("www.example.com", "mail.example.com", 120);
    dns_rr mx; memset(&mx, 0, sizeof mx); strcpy(mx.name, "example.com"); mx.type = DNS_MX; mx.ttl = 3600; mx.d.mx.pref = 10; strcpy(mx.d.mx.host, "mail.example.com"); src.an[src.nan++] = mx;
    dns_rr txt; memset(&txt, 0, sizeof txt); strcpy(txt.name, "example.com"); txt.type = DNS_TXT; txt.ttl = 10; strcpy(txt.d.txt.text, "v=spf1 -all"); src.an[src.nan++] = txt;
    dns_rr ptr; memset(&ptr, 0, sizeof ptr); strcpy(ptr.name, "5.216.184.93.in-addr.arpa"); ptr.type = DNS_PTR; ptr.ttl = 99; strcpy(ptr.d.target, "mail.example.com"); src.an[src.nan++] = ptr;
    int rl = dns_build_response(r, sizeof r, &src);
    CHECK(rl > 0);
    int uncompressed = 0; for (int i = 0; i < src.nan; i++) uncompressed += (int)strlen(src.an[i].name) + 2 + 10 + 16; CHECK(rl < 12 + 20 + uncompressed);   /* compression saved space */
    CHECK(dns_parse(r, (size_t)rl, &m) && m.nan == 6 && m.id == 7 && (m.flags & DNS_FLAG_QR));
    CHECK(m.an[0].type == DNS_A && m.an[0].d.a.b[3] == 5 && m.an[0].ttl == 300);
    char b6[48]; ip6_format(&m.an[1].d.aaaa, b6, sizeof b6); CHECK(m.an[1].type == DNS_AAAA && !strcmp(b6, "2001:db8::25"));
    CHECK(m.an[2].type == DNS_CNAME && !strcmp(m.an[2].name, "www.example.com") && !strcmp(m.an[2].d.target, "mail.example.com"));
    CHECK(m.an[3].type == DNS_MX && m.an[3].d.mx.pref == 10 && !strcmp(m.an[3].d.mx.host, "mail.example.com"));
    CHECK(m.an[4].type == DNS_TXT && !strcmp(m.an[4].d.txt.text, "v=spf1 -all") && m.an[5].type == DNS_PTR && !strcmp(m.an[5].d.target, "mail.example.com"));

    /* hostile inputs never crash or loop */
    uint8_t evil[64]; memset(evil, 0, sizeof evil);
    net_wr16(evil + 4, 1); net_wr16(evil + 6, 1);
    evil[12] = 0xC0; evil[13] = 12;                                           /* compression pointer to itself */
    CHECK(!dns_parse(evil, 30, &m));
    memset(evil, 0, sizeof evil); net_wr16(evil + 4, 1); evil[12] = 5; memcpy(evil + 13, "abcde", 5);       /* label runs past the packet end */
    CHECK(!dns_parse(evil, 16, &m));
    memset(evil, 0, sizeof evil); net_wr16(evil + 4, 1); net_wr16(evil + 6, 60000);                          /* absurd record count */
    evil[12] = 0; CHECK(!dns_parse(evil, 40, &m));
    CHECK(!dns_parse(evil, 11, &m));
    uint8_t forward[64]; memset(forward, 0, sizeof forward); net_wr16(forward + 4, 1); forward[12] = 0xC0; forward[13] = 20; CHECK(!dns_parse(forward, 40, &m));   /* forward pointer */
    for (int t = 0; t < 2000; t++) {                                          /* mutation fuzz of a valid packet */
        uint8_t f[512]; memcpy(f, r, (size_t)rl);
        unsigned s = (unsigned)t * 2654435761u;
        for (int k = 0; k < 4; k++) { s = s * 1664525u + 1013904223u; f[(s >> 8) % (unsigned)rl] = (uint8_t)(s >> 24); }
        dns_parse(f, (size_t)rl - (s % 5), &m);
    }
    CHECK(true);

    /* ---- cache ---- */
    dns_cache *cache = calloc(1, sizeof *cache);
    dns_rr rr = rr_a("host.example.com", 100, 9), out[8]; int cnt; uint8_t rc;
    dns_cache_put(cache, "Host.Example.com", DNS_A, 0, &rr, 1, 100, 1000);
    CHECK(dns_cache_get(cache, "host.example.com", DNS_A, 1000, out, 8, &cnt, &rc) == 1 && cnt == 1 && out[0].ttl == 100);
    CHECK(dns_cache_get(cache, "HOST.example.com", DNS_A, 61000, out, 8, &cnt, &rc) == 1 && out[0].ttl == 40);         /* TTL counts down */
    CHECK(dns_cache_get(cache, "host.example.com", DNS_AAAA, 1000, out, 8, &cnt, &rc) == 0);
    CHECK(dns_cache_get(cache, "host.example.com", DNS_A, 101000, out, 8, &cnt, &rc) == 0);                           /* expired */
    dns_cache_put(cache, "gone.example.com", DNS_A, DNS_RCODE_NXDOMAIN, NULL, 0, 30, 0);
    CHECK(dns_cache_get(cache, "gone.example.com", DNS_A, 1000, out, 8, &cnt, &rc) == 2 && rc == DNS_RCODE_NXDOMAIN);
    dns_cache_put(cache, "zero.example.com", DNS_A, 0, &rr, 1, 0, 0);
    CHECK(dns_cache_get(cache, "zero.example.com", DNS_A, 0, out, 8, &cnt, &rc) == 0);                                  /* TTL 0: never cached */
    dns_cache_put(cache, "long.example.com", DNS_A, 0, &rr, 1, 99999999u, 0);
    CHECK(dns_cache_get(cache, "long.example.com", DNS_A, 86399000u, out, 8, &cnt, &rc) == 1 && dns_cache_get(cache, "long.example.com", DNS_A, 86401000u, out, 8, &cnt, &rc) == 0);   /* capped at one day */
    for (int i = 0; i < 200; i++) { char nm[32]; snprintf(nm, sizeof nm, "h%d.example.com", i); dns_cache_put(cache, nm, DNS_A, 0, &rr, 1, 1000 + (uint32_t)i, 0); }
    CHECK(dns_cache_get(cache, "h199.example.com", DNS_A, 1, out, 8, &cnt, &rc) == 1 && dns_cache_get(cache, "h0.example.com", DNS_A, 1, out, 8, &cnt, &rc) == 0);   /* earliest-expiring evicted */
    dns_cache_flush(cache); CHECK(dns_cache_get(cache, "h199.example.com", DNS_A, 1, out, 8, &cnt, &rc) == 0);

    /* ---- resolver ---- */
    dns_config cfg; memset(&cfg, 0, sizeof cfg);
    cfg.server[0] = S1; cfg.server[1] = S2; cfg.nserver = 2; cfg.ndots = 1; cfg.timeout_ms = 1000; cfg.attempts = 2; cfg.rand16 = rnd16;
    strcpy(cfg.search[0], "corp.example"); strcpy(cfg.search[1], "example.net"); cfg.nsearch = 2;
    dns_resolver *res = calloc(1, sizeof *res);
    dns_cache_init(cache);
    dns_res_init(res, &cfg, cache);
    int si, ql; uint64_t now = 5000;

    /* simple A lookup with an absolute name (a dot is present) */
    CHECK(dns_res_start(res, "www.example.com", DNS_A, now) && res->state == DNSR_WAIT && !strcmp(res->cand[0], "www.example.com") && res->ncand == 3);
    ql = dns_res_poll(res, now, q, sizeof q, &si); CHECK(ql > 0 && si == 0 && res->in_flight);
    CHECK(dns_res_poll(res, now + 500, q, sizeof q, &si) == 0);                                 /* waiting */
    dns_rr ans[2] = { rr_a("www.example.com", 300, 34), rr_a("www.example.com", 120, 35) };
    int rl2 = reply(q, (size_t)ql, 0, ans, 2, 0, false, r, sizeof r);
    dns_res_input(res, r, (size_t)rl2, S2, now + 10); CHECK(res->state == DNSR_WAIT);          /* wrong server: ignored */
    uint8_t bad[512]; memcpy(bad, r, (size_t)rl2); net_wr16(bad, (uint16_t)(res->txid + 1));
    dns_res_input(res, bad, (size_t)rl2, S1, now + 10); CHECK(res->state == DNSR_WAIT);        /* wrong txid: ignored */
    dns_res_input(res, r, (size_t)rl2, S1, now + 20);
    CHECK(res->state == DNSR_DONE && res->nresult == 2 && res->min_ttl == 120 && !strcmp(res->canonical, "www.example.com"));
    /* second lookup is served from the cache without sending anything */
    CHECK(dns_res_start(res, "www.example.com", DNS_A, now + 30000) && res->state == DNSR_DONE && res->nresult == 2 && res->min_ttl <= 120 - 29);
    CHECK(dns_res_poll(res, now + 30000, q, sizeof q, &si) == 0);

    /* search list: a bare name is tried against the domains first; NXDOMAIN moves to the next candidate */
    CHECK(dns_res_start(res, "intranet", DNS_A, now) && res->ncand == 3 && !strcmp(res->cand[0], "intranet.corp.example") && !strcmp(res->cand[2], "intranet"));
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    rl2 = reply(q, (size_t)ql, DNS_RCODE_NXDOMAIN, NULL, 0, 600, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_WAIT && !strcmp(res->qname, "intranet.example.net"));
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    rl2 = reply(q, (size_t)ql, DNS_RCODE_NXDOMAIN, NULL, 0, 600, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    ql = dns_res_poll(res, now, q, sizeof q, &si); CHECK(!strcmp(res->qname, "intranet"));
    dns_rr one = rr_a("intranet", 600, 77);
    rl2 = reply(q, (size_t)ql, 0, &one, 1, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_DONE && res->result[0].d.a.b[3] == 77);
    /* negative answers were cached: the same search fails fast the second time */
    CHECK(dns_cache_get(cache, "intranet.corp.example", DNS_A, now + 1000, out, 8, &cnt, &rc) == 2);

    /* NXDOMAIN everywhere -> failure; trailing dot means no search list */
    dns_cache_init(cache);
    CHECK(dns_res_start(res, "nothere.example.org.", DNS_A, now) && res->ncand == 1);
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    rl2 = reply(q, (size_t)ql, DNS_RCODE_NXDOMAIN, NULL, 0, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_FAILED && res->error == DNSE_NXDOMAIN);
    /* NODATA (NOERROR, no matching records) */
    CHECK(dns_res_start(res, "v4only.example.org.", DNS_AAAA, now));
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    rl2 = reply(q, (size_t)ql, 0, NULL, 0, 120, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_FAILED && res->error == DNSE_NODATA && dns_cache_get(cache, "v4only.example.org", DNS_AAAA, now + 1000, out, 8, &cnt, &rc) == 2);

    /* CNAME chain in one answer, and a chain that must be chased with a second query */
    CHECK(dns_res_start(res, "alias.example.org.", DNS_A, now));
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    dns_rr chain[3] = { rr_cname("alias.example.org", "mid.example.org", 300), rr_cname("mid.example.org", "real.example.org", 300), rr_a("real.example.org", 60, 50) };
    rl2 = reply(q, (size_t)ql, 0, chain, 3, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_DONE && !strcmp(res->canonical, "real.example.org") && res->result[0].d.a.b[3] == 50);
    CHECK(dns_res_start(res, "far.example.org.", DNS_A, now));
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    dns_rr only_cname = rr_cname("far.example.org", "elsewhere.example.net", 300);
    rl2 = reply(q, (size_t)ql, 0, &only_cname, 1, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_WAIT && !strcmp(res->qname, "elsewhere.example.net") && res->cname_depth == 1);
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    dns_rr fin = rr_a("elsewhere.example.net", 90, 61);
    rl2 = reply(q, (size_t)ql, 0, &fin, 1, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_DONE && res->result[0].d.a.b[3] == 61);
    /* a CNAME loop terminates */
    CHECK(dns_res_start(res, "loop.example.org.", DNS_A, now));
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    dns_rr lp[2] = { rr_cname("loop.example.org", "loop2.example.org", 300), rr_cname("loop2.example.org", "loop.example.org", 300) };
    rl2 = reply(q, (size_t)ql, 0, lp, 2, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    for (int i = 0; i < 12 && res->state == DNSR_WAIT; i++) {
        ql = dns_res_poll(res, now, q, sizeof q, &si); if (ql <= 0) break;
        dns_msg qm; dns_parse(q, (size_t)ql, &qm);
        dns_rr back = rr_cname(qm.qname, !strcmp(qm.qname, "loop2.example.org") ? "loop.example.org" : "loop2.example.org", 300);
        rl2 = reply(q, (size_t)ql, 0, &back, 1, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    }
    CHECK(res->state == DNSR_FAILED && res->error == DNSE_LOOP);

    /* retries, failover and timeout */
    dns_cache_init(cache);
    CHECK(dns_res_start(res, "slow.example.org.", DNS_A, now));
    ql = dns_res_poll(res, now, q, sizeof q, &si); CHECK(si == 0);
    CHECK(dns_res_poll(res, now + 999, q, sizeof q, &si) == 0 && dns_res_next_wakeup(res) == now + 1000);
    ql = dns_res_poll(res, now + 1000, q, sizeof q, &si); CHECK(ql > 0 && si == 0);              /* second attempt, same server */
    ql = dns_res_poll(res, now + 2000, q, sizeof q, &si); CHECK(ql > 0 && si == 1);              /* fail over to the second server */
    ql = dns_res_poll(res, now + 3000, q, sizeof q, &si); CHECK(ql > 0 && si == 1);
    CHECK(dns_res_poll(res, now + 4000, q, sizeof q, &si) == 0 && res->state == DNSR_FAILED && res->error == DNSE_TIMEOUT);
    /* SERVFAIL moves to the next server immediately; REFUSED everywhere fails */
    CHECK(dns_res_start(res, "sick.example.org.", DNS_A, now));
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    rl2 = reply(q, (size_t)ql, DNS_RCODE_SERVFAIL, NULL, 0, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    ql = dns_res_poll(res, now, q, sizeof q, &si); CHECK(ql > 0 && si == 1);
    dns_rr okr = rr_a("sick.example.org", 60, 3);
    rl2 = reply(q, (size_t)ql, 0, &okr, 1, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S2, now);
    CHECK(res->state == DNSR_DONE);
    CHECK(dns_res_start(res, "refused.example.org.", DNS_A, now));
    for (int s = 0; s < 2; s++) { ql = dns_res_poll(res, now, q, sizeof q, &si); rl2 = reply(q, (size_t)ql, DNS_RCODE_REFUSED, NULL, 0, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, s ? S2 : S1, now); }
    CHECK(res->state == DNSR_FAILED && res->error == DNSE_SERVFAIL);
    /* truncation asks the caller to retry over TCP, then accepts the TCP reply */
    CHECK(dns_res_start(res, "big.example.org.", DNS_A, now));
    ql = dns_res_poll(res, now, q, sizeof q, &si);
    rl2 = reply(q, (size_t)ql, 0, NULL, 0, 0, true, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_NEED_TCP);
    dns_rr bigr = rr_a("big.example.org", 60, 4);
    rl2 = reply(q, (size_t)ql, 0, &bigr, 1, 0, false, r, sizeof r); dns_res_input(res, r, (size_t)rl2, S1, now);
    CHECK(res->state == DNSR_DONE);
    /* bad names and no servers */
    CHECK(!dns_res_start(res, "bad name", DNS_A, now) && res->error == DNSE_BAD_NAME);
    dns_config none = cfg; none.nserver = 0; dns_resolver *r0 = calloc(1, sizeof *r0); dns_res_init(r0, &none, cache);
    CHECK(!dns_res_start(r0, "example.com", DNS_A, now));
    free(r0); free(res); free(cache);
    DONE("dns_test");
}
