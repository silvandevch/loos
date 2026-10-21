/* IP helper: the interface and routing tables (GetAdaptersAddresses / GetIpForwardTable / GetBestRoute
 * equivalents) plus the glue that applies DHCP leases to them. */
#ifndef LOOS_IPHELP_H
#define LOOS_IPHELP_H
#include "netutil.h"
#include "dhcp.h"

#ifdef __cplusplus
extern "C" {
#endif

#define IPH_MAX_IFACE 8
#define IPH_MAX_ADDR 6
#define IPH_MAX_ROUTE 64
typedef struct { ip4_addr addr; uint8_t prefix; bool dhcp, link_local; } iph_addr4;
typedef struct {
    bool used; uint32_t index; char name[16]; uint8_t mac[6]; uint32_t mtu; bool up, loopback;
    iph_addr4 a4[IPH_MAX_ADDR]; int na4;
    ip6_cidr a6[IPH_MAX_ADDR]; int na6;
    ip4_addr dns[4]; int ndns; char domain[64];
    bool dhcp_enabled;
    uint64_t rx_bytes, tx_bytes;
} iph_iface;
typedef struct { ip4_cidr dest; ip4_addr gateway; uint32_t ifindex; uint32_t metric; bool onlink, from_dhcp; } iph_route;
typedef struct { iph_iface ifc[IPH_MAX_IFACE]; iph_route rt[IPH_MAX_ROUTE]; int nrt; uint32_t next_index; } iph_table;

void iph_init(iph_table *t);                                         /* creates the loopback interface (127.0.0.1/8) */
uint32_t iph_add_iface(iph_table *t, const char *name, const uint8_t mac[6], uint32_t mtu);   /* returns the interface index, 0 on failure */
iph_iface *iph_get(iph_table *t, uint32_t index);
iph_iface *iph_by_name(iph_table *t, const char *name);
bool iph_set_up(iph_table *t, uint32_t index, bool up);
bool iph_add_addr(iph_table *t, uint32_t index, ip4_addr a, uint8_t prefix);    /* also adds the connected route */
bool iph_del_addr(iph_table *t, uint32_t index, ip4_addr a);
bool iph_add_route(iph_table *t, ip4_cidr dest, ip4_addr gw, uint32_t ifindex, uint32_t metric);
bool iph_del_route(iph_table *t, ip4_cidr dest, uint32_t ifindex);
/* best route: up interface, longest prefix, lowest metric (interface order breaks ties). Fills the chosen route. */
bool iph_lookup(const iph_table *t, ip4_addr dest, iph_route *out);
bool iph_source_for(const iph_table *t, ip4_addr dest, ip4_addr *src);      /* the address a packet to dest would leave with */
bool iph_next_hop(const iph_table *t, ip4_addr dest, ip4_addr *hop, uint32_t *ifindex);
void iph_apply_lease(iph_table *t, uint32_t index, const dhcp_lease *l);
void iph_clear_lease(iph_table *t, uint32_t index);
int  iph_adapters(const iph_table *t, iph_iface *out, int max);
int  iph_routes(const iph_table *t, iph_route *out, int max);

#ifdef __cplusplus
}
#endif
#endif
