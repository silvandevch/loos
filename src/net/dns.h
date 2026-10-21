/* DNS (RFC 1035 / 2181 / 6891): message codec, TTL cache with negative caching, and an I/O-free stub resolver
 * state machine (search list, retries, server failover, CNAME chasing, response validation). The caller moves
 * packets: it sends what dns_res_poll() returns and feeds replies to dns_res_input(). */
#ifndef LOOS_DNS_H
#define LOOS_DNS_H
#include "netutil.h"

#ifdef __cplusplus
extern "C" {
#endif

enum { DNS_A = 1, DNS_NS = 2, DNS_CNAME = 5, DNS_SOA = 6, DNS_PTR = 12, DNS_MX = 15, DNS_TXT = 16, DNS_AAAA = 28, DNS_SRV = 33, DNS_OPT = 41, DNS_ANY = 255 };
enum { DNS_CLASS_IN = 1 };
enum { DNS_RCODE_OK = 0, DNS_RCODE_FORMERR = 1, DNS_RCODE_SERVFAIL = 2, DNS_RCODE_NXDOMAIN = 3, DNS_RCODE_NOTIMP = 4, DNS_RCODE_REFUSED = 5 };
enum { DNS_FLAG_QR = 0x8000, DNS_FLAG_AA = 0x0400, DNS_FLAG_TC = 0x0200, DNS_FLAG_RD = 0x0100, DNS_FLAG_RA = 0x0080 };
#define DNS_NAME_MAX 255
#define DNS_MAX_RR 24

typedef struct {
    char name[DNS_NAME_MAX + 1];        /* lower-cased, no trailing dot */
    uint16_t type, cls;
    uint32_t ttl;
    union {
        ip4_addr a; ip6_addr aaaa;
        char target[DNS_NAME_MAX + 1];  /* CNAME / NS / PTR */
        struct { uint16_t pref; char host[DNS_NAME_MAX + 1]; } mx;
        struct { uint16_t prio, weight, port; char host[DNS_NAME_MAX + 1]; } srv;
        struct { char text[256]; } txt;
        struct { char mname[DNS_NAME_MAX + 1]; uint32_t serial, refresh, retry, expire, minimum; } soa;
    } d;
} dns_rr;

typedef struct {
    uint16_t id, flags;
    uint8_t rcode;
    char qname[DNS_NAME_MAX + 1]; uint16_t qtype, qclass;
    dns_rr an[DNS_MAX_RR]; int nan;
    dns_rr ns[4]; int nns;              /* authority section (SOA for negative answers) */
    bool truncated;
} dns_msg;

bool dns_name_valid(const char *name);                          /* LDH labels of 1..63, total <= 253, no empty labels */
void dns_name_normalize(const char *in, char *out, size_t cap); /* lower-case, strip one trailing dot */
int  dns_build_query(uint8_t *buf, size_t cap, uint16_t id, const char *name, uint16_t qtype, bool rd, uint16_t edns_udp);   /* length or -1 */
bool dns_parse(const uint8_t *buf, size_t len, dns_msg *out);   /* false on malformed/hostile input (loops, overruns, bad counts) */
int  dns_build_response(uint8_t *buf, size_t cap, const dns_msg *m);   /* for tests and the local responder; uses name compression */

/* ---- cache ---- */
#define DNS_CACHE_N 64
#define DNS_CACHE_MAX_TTL 86400u
#define DNS_CACHE_NEG_TTL 300u
typedef struct {
    struct { bool used; char name[DNS_NAME_MAX + 1]; uint16_t type; uint8_t rcode; dns_rr rr[8]; int nrr; uint64_t expire_ms; } e[DNS_CACHE_N];
} dns_cache;
void dns_cache_init(dns_cache *c);
void dns_cache_put(dns_cache *c, const char *name, uint16_t type, uint8_t rcode, const dns_rr *rr, int nrr, uint32_t ttl_s, uint64_t now_ms);
/* returns 1 hit (rr filled, TTLs reduced to the time left), 2 negative hit, 0 miss/expired */
int  dns_cache_get(dns_cache *c, const char *name, uint16_t type, uint64_t now_ms, dns_rr *rr, int max, int *nrr, uint8_t *rcode);
void dns_cache_flush(dns_cache *c);

/* ---- stub resolver ---- */
#define DNS_MAX_SERVERS 4
#define DNS_MAX_SEARCH 4
typedef struct {
    ip4_addr server[DNS_MAX_SERVERS]; int nserver;
    char search[DNS_MAX_SEARCH][64]; int nsearch;
    int ndots;                          /* names with at least this many dots are tried as absolute first */
    uint32_t timeout_ms;                /* per attempt */
    int attempts;                       /* tries per server per candidate */
    uint16_t (*rand16)(void *user); void *user;
} dns_config;
typedef enum { DNSR_IDLE, DNSR_WAIT, DNSR_DONE, DNSR_FAILED, DNSR_NEED_TCP } dns_res_state;
typedef enum { DNSE_NONE, DNSE_NXDOMAIN, DNSE_NODATA, DNSE_SERVFAIL, DNSE_TIMEOUT, DNSE_BAD_NAME, DNSE_LOOP } dns_error;
typedef struct {
    dns_config cfg;
    dns_res_state state; dns_error error;
    char name[DNS_NAME_MAX + 1]; uint16_t qtype;
    char cand[DNS_MAX_SEARCH + 2][DNS_NAME_MAX + 1]; int ncand, cur;     /* search-list expansion of `name` */
    char qname[DNS_NAME_MAX + 1]; int cname_depth;                       /* name currently being asked (differs after a CNAME) */
    int server, attempt; uint16_t txid; uint64_t sent_ms; bool in_flight;
    dns_rr result[DNS_MAX_RR]; int nresult; char canonical[DNS_NAME_MAX + 1]; uint32_t min_ttl;
    dns_cache *cache;
} dns_resolver;
void dns_res_init(dns_resolver *r, const dns_config *cfg, dns_cache *cache);
bool dns_res_start(dns_resolver *r, const char *name, uint16_t qtype, uint64_t now_ms);
/* If a packet should be sent now, fills it and returns its length (>0) and the server index; 0 = nothing to send. */
int  dns_res_poll(dns_resolver *r, uint64_t now_ms, uint8_t *pkt, size_t cap, int *server_idx);
void dns_res_input(dns_resolver *r, const uint8_t *pkt, size_t len, ip4_addr from, uint64_t now_ms);
uint64_t dns_res_next_wakeup(const dns_resolver *r);

#ifdef __cplusplus
}
#endif
#endif
