/* Firewall / packet-filter engine (WFP-like). Decides per packet or connection attempt: allow, drop, reject or
 * ask the user. Layers of decision, first match wins:
 *   1. the hard "block all network" switch (loopback stays allowed so local IPC keeps working)
 *   2. loopback
 *   3. established flows (connection tracking: replies to what we allowed are allowed)
 *   4. explicit rules by weight (deny beats allow at equal weight)
 *   5. per-process policy (what the Settings firewall page edits)
 *   6. direction default (inbound deny, outbound ask) */
#ifndef LOOS_FW_H
#define LOOS_FW_H
#include "netutil.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum { FW_IN, FW_OUT } fw_dir;
typedef enum { FW_PROTO_ANY = 0, FW_ICMP = 1, FW_TCP = 6, FW_UDP = 17, FW_ICMP6 = 58 } fw_proto;
typedef enum { FWV_ALLOW, FWV_DROP, FWV_REJECT, FWV_ASK } fw_verdict;
typedef enum { FWR_NONE, FWR_HARD_OFF, FWR_LOOPBACK, FWR_ESTABLISHED, FWR_RULE, FWR_APP_POLICY, FWR_DEFAULT, FWR_INVALID, FWR_TABLE_FULL } fw_reason;
typedef enum { FWA_ASK, FWA_ALLOW, FWA_DENY } fw_app_policy;
enum { TCP_FIN = 1, TCP_SYN = 2, TCP_RST = 4, TCP_ACK = 16 };

typedef struct { bool v6; ip4_addr a4; ip6_addr a6; } fw_ip;
typedef struct {
    fw_dir dir; fw_proto proto;
    fw_ip local, remote; uint16_t lport, rport;     /* from the local machine's point of view, whatever the direction */
    char app[32]; uint32_t pid;                      /* owner of the local socket ("" if unknown) */
    uint32_t ifindex; uint8_t tcp_flags; uint16_t len;
} fw_packet;

typedef struct { bool any; bool v6; ip4_cidr c4; ip6_cidr c6; } fw_addr_match;
typedef struct { uint16_t lo, hi; } fw_port_range;
typedef struct {
    uint32_t id; char name[32]; bool enabled;
    fw_verdict action;                               /* ALLOW, DROP or REJECT */
    int weight;                                      /* higher is evaluated first */
    int dir;                                         /* -1 any, else fw_dir */
    fw_proto proto;
    fw_addr_match local, remote;
    fw_port_range lports[4]; int nlports;            /* none = any */
    fw_port_range rports[4]; int nrports;
    char app[32];                                    /* "" = any; trailing/leading '*' wildcards supported */
    uint32_t ifindex;                                /* 0 = any */
    bool log;
} fw_rule;

#define FW_MAX_RULES 128
#define FW_MAX_APPS 64
#define FW_CT_SIZE 512
#define FW_LOG_SIZE 128
typedef struct { uint64_t t; fw_dir dir; fw_proto proto; fw_ip remote; uint16_t rport; char app[32]; fw_verdict verdict; fw_reason reason; uint32_t rule; } fw_log_entry;

typedef struct {
    bool used; fw_proto proto; fw_ip local, remote; uint16_t lport, rport; fw_dir origin;
    uint8_t state;                                   /* TCP: 0 new, 1 syn sent/recv, 2 established, 3 closing, 4 closed */
    uint64_t last_ms; char app[32];
    uint64_t bytes_in, bytes_out;
} fw_flow;

typedef struct {
    fw_rule rule[FW_MAX_RULES]; int nrule; uint32_t next_id;
    struct { char app[32]; fw_app_policy p; } app[FW_MAX_APPS]; int napp;
    bool hard_off;
    fw_verdict default_in, default_out;
    fw_flow flow[FW_CT_SIZE]; int nflow;
    fw_log_entry log[FW_LOG_SIZE]; unsigned log_head, log_count;
    uint64_t allowed, dropped, rejected, asked;
    uint32_t tcp_est_s, tcp_trans_s, udp_s, icmp_s;     /* conntrack timeouts */
} fw_engine;

void fw_init(fw_engine *f);
uint32_t fw_add_rule(fw_engine *f, const fw_rule *r);                 /* returns the rule id, 0 if full/invalid */
bool fw_remove_rule(fw_engine *f, uint32_t id);
bool fw_set_rule_enabled(fw_engine *f, uint32_t id, bool on);
void fw_set_app_policy(fw_engine *f, const char *app, fw_app_policy p);   /* ASK removes the entry */
fw_app_policy fw_get_app_policy(const fw_engine *f, const char *app);
void fw_set_hard_off(fw_engine *f, bool off);
fw_verdict fw_eval(fw_engine *f, const fw_packet *p, uint64_t now_ms, fw_reason *why, uint32_t *rule_id);
void fw_gc(fw_engine *f, uint64_t now_ms);                           /* expire idle flows */
int  fw_flows(const fw_engine *f, fw_flow *out, int max);             /* live connection list for the UI */
int  fw_log_read(const fw_engine *f, fw_log_entry *out, int max);    /* newest first */
/* "allow out tcp to 10.0.0.0/8 port 80,443 app browser.exe weight 50". from/to mean source/destination and need in|out;
 * local/remote are direction-independent. Output of fw_rule_format always uses local/remote. */
bool fw_rule_parse(fw_rule *r, const char *line);
size_t fw_rule_format(const fw_rule *r, char *out, size_t cap);
bool fw_glob(const char *pattern, const char *text);                 /* case-insensitive, '*' and '?' */

#ifdef __cplusplus
}
#endif
#endif
