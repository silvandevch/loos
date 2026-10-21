/* DHCPv4 (RFC 2131/2132): message codec and an I/O-free client state machine. The caller delivers packets
 * (UDP 67/68) and a clock; the client returns what to send. Falls back to a link-local 169.254/16 address
 * (RFC 3927) when no server answers. */
#ifndef LOOS_DHCP_H
#define LOOS_DHCP_H
#include "netutil.h"

#ifdef __cplusplus
extern "C" {
#endif

enum { DHCP_DISCOVER = 1, DHCP_OFFER = 2, DHCP_REQUEST = 3, DHCP_DECLINE = 4, DHCP_ACK = 5, DHCP_NAK = 6, DHCP_RELEASE = 7, DHCP_INFORM = 8 };
enum { DHCP_OPT_SUBNET = 1, DHCP_OPT_ROUTER = 3, DHCP_OPT_DNS = 6, DHCP_OPT_HOSTNAME = 12, DHCP_OPT_DOMAIN = 15, DHCP_OPT_MTU = 26,
       DHCP_OPT_BROADCAST = 28, DHCP_OPT_NTP = 42, DHCP_OPT_REQUESTED_IP = 50, DHCP_OPT_LEASE = 51, DHCP_OPT_MSGTYPE = 53,
       DHCP_OPT_SERVER_ID = 54, DHCP_OPT_PARAM_REQ = 55, DHCP_OPT_MAXMSG = 57, DHCP_OPT_T1 = 58, DHCP_OPT_T2 = 59, DHCP_OPT_CLIENT_ID = 61,
       DHCP_OPT_SEARCH = 119, DHCP_OPT_END = 255 };
#define DHCP_MIN_PACKET 300

typedef struct {
    uint8_t op, htype, hlen; uint32_t xid; uint16_t secs, flags;
    ip4_addr ciaddr, yiaddr, siaddr, giaddr;
    uint8_t chaddr[16];
    uint8_t msgtype;
    bool has_subnet, has_server, has_requested, has_lease, has_t1, has_t2, has_mtu;
    ip4_addr subnet, server_id, requested_ip, broadcast;
    ip4_addr router[4]; int nrouter;
    ip4_addr dns[4]; int ndns;
    ip4_addr ntp[2]; int nntp;
    uint32_t lease, t1, t2; uint16_t mtu;
    char domain[64], hostname[64], search[128];
    uint8_t client_id[32]; int client_id_len;
    uint8_t param_req[16]; int nparam;
} dhcp_msg;

int  dhcp_build(uint8_t *buf, size_t cap, const dhcp_msg *m);       /* BOOTP header + magic cookie + options; length >= 300 or -1 */
bool dhcp_parse(const uint8_t *buf, size_t len, dhcp_msg *out);

typedef enum { DHCPC_INIT, DHCPC_SELECTING, DHCPC_REQUESTING, DHCPC_BOUND, DHCPC_RENEWING, DHCPC_REBINDING, DHCPC_FALLBACK } dhcp_state;
typedef struct {
    ip4_addr addr, mask, router, dns[4], server; int ndns;
    char domain[64]; uint32_t lease_s, t1_s, t2_s; uint16_t mtu;
    bool link_local;
} dhcp_lease;
typedef struct {
    uint8_t mac[6]; char hostname[32];
    dhcp_state state;
    uint32_t xid; uint64_t state_since_ms, next_tx_ms, bound_at_ms, start_ms;
    int retries; uint32_t backoff_ms;
    dhcp_msg offer; dhcp_lease lease; bool have_lease;
    uint32_t (*rand32)(void *user); void *user;
    void (*on_lease)(const dhcp_lease *l, bool acquired, void *user);   /* acquired=false: lost / released */
    int discover_failures;
} dhcp_client;

void dhcp_client_init(dhcp_client *c, const uint8_t mac[6], const char *hostname, uint32_t (*rand32)(void *), void *user);
void dhcp_client_start(dhcp_client *c, uint64_t now_ms);
/* Next packet to transmit at `now`, or 0. dest is the IPv4 destination (255.255.255.255 broadcast or the server). */
int  dhcp_client_poll(dhcp_client *c, uint64_t now_ms, uint8_t *pkt, size_t cap, ip4_addr *dest);
void dhcp_client_input(dhcp_client *c, const uint8_t *pkt, size_t len, uint64_t now_ms);
int  dhcp_client_release(dhcp_client *c, uint8_t *pkt, size_t cap, ip4_addr *dest);
uint32_t dhcp_client_remaining_s(const dhcp_client *c, uint64_t now_ms);   /* seconds until the lease expires */
ip4_addr dhcp_link_local_for(const uint8_t mac[6], unsigned attempt);

#ifdef __cplusplus
}
#endif
#endif
