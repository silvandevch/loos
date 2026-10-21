#include "iphelp.h"
#include <string.h>

void iph_init(iph_table *t)
{
    memset(t, 0, sizeof *t);
    t->next_index = 1;
    static const uint8_t zero[6] = { 0 };
    uint32_t lo = iph_add_iface(t, "lo", zero, 65535);
    iph_get(t, lo)->loopback = true;
    iph_set_up(t, lo, true);
    ip4_addr a = { { 127, 0, 0, 1 } };
    iph_add_addr(t, lo, a, 8);
}

uint32_t iph_add_iface(iph_table *t, const char *name, const uint8_t mac[6], uint32_t mtu)
{
    for (int i = 0; i < IPH_MAX_IFACE; i++) if (t->ifc[i].used && !strcmp(t->ifc[i].name, name)) return 0;
    for (int i = 0; i < IPH_MAX_IFACE; i++) if (!t->ifc[i].used) {
        iph_iface *f = &t->ifc[i];
        memset(f, 0, sizeof *f);
        f->used = true; f->index = t->next_index++; f->mtu = mtu ? mtu : 1500;
        strncpy(f->name, name, sizeof f->name - 1); memcpy(f->mac, mac, 6);
        return f->index;
    }
    return 0;
}
iph_iface *iph_get(iph_table *t, uint32_t index) { for (int i = 0; i < IPH_MAX_IFACE; i++) if (t->ifc[i].used && t->ifc[i].index == index) return &t->ifc[i]; return NULL; }
iph_iface *iph_by_name(iph_table *t, const char *name) { for (int i = 0; i < IPH_MAX_IFACE; i++) if (t->ifc[i].used && !strcmp(t->ifc[i].name, name)) return &t->ifc[i]; return NULL; }
bool iph_set_up(iph_table *t, uint32_t index, bool up) { iph_iface *f = iph_get(t, index); if (!f) return false; f->up = up; return true; }

static int find_route(const iph_table *t, ip4_cidr d, uint32_t ifindex)
{
    for (int i = 0; i < t->nrt; i++)
        if (t->rt[i].ifindex == ifindex && t->rt[i].dest.prefix == d.prefix && ip4_u32(t->rt[i].dest.addr) == ip4_u32(d.addr)) return i;
    return -1;
}
bool iph_add_route(iph_table *t, ip4_cidr d, ip4_addr gw, uint32_t ifindex, uint32_t metric)
{
    if (!iph_get(t, ifindex) || d.prefix > 32) return false;
    d.addr = ip4_from_u32(ip4_u32(d.addr) & ip4_mask(d.prefix));          /* normalise host bits away */
    bool onlink = ip4_u32(gw) == 0;
    if (!onlink) {                                                         /* a gateway must be reachable on that interface */
        iph_iface *f = iph_get(t, ifindex); bool reach = false;
        for (int i = 0; i < f->na4; i++) { ip4_cidr c = { f->a4[i].addr, f->a4[i].prefix }; if (ip4_in_cidr(gw, c)) reach = true; }
        if (!reach) return false;
    }
    int i = find_route(t, d, ifindex);
    if (i < 0) { if (t->nrt >= IPH_MAX_ROUTE) return false; i = t->nrt++; }
    t->rt[i] = (iph_route){ d, gw, ifindex, metric, onlink, false };
    return true;
}
bool iph_del_route(iph_table *t, ip4_cidr d, uint32_t ifindex)
{
    d.addr = ip4_from_u32(ip4_u32(d.addr) & ip4_mask(d.prefix));
    int i = find_route(t, d, ifindex);
    if (i < 0) return false;
    t->rt[i] = t->rt[--t->nrt];
    return true;
}

bool iph_add_addr(iph_table *t, uint32_t index, ip4_addr a, uint8_t prefix)
{
    iph_iface *f = iph_get(t, index);
    if (!f || prefix > 32 || prefix == 0 || f->na4 >= IPH_MAX_ADDR || ip4_u32(a) == 0 || ip4_is_multicast(a)) return false;
    for (int i = 0; i < f->na4; i++) if (ip4_eq(f->a4[i].addr, a)) return false;
    f->a4[f->na4++] = (iph_addr4){ a, prefix, false, ip4_is_link_local(a) };
    ip4_cidr net = { a, prefix };
    ip4_addr none = { { 0, 0, 0, 0 } };
    iph_add_route(t, net, none, index, f->loopback ? 0 : 256);            /* the connected route */
    return true;
}
bool iph_del_addr(iph_table *t, uint32_t index, ip4_addr a)
{
    iph_iface *f = iph_get(t, index);
    if (!f) return false;
    for (int i = 0; i < f->na4; i++) if (ip4_eq(f->a4[i].addr, a)) {
        ip4_cidr net = { f->a4[i].addr, f->a4[i].prefix };
        bool shared = false;                                               /* another address on the same subnet keeps the route */
        for (int k = 0; k < f->na4; k++) if (k != i && f->a4[k].prefix == f->a4[i].prefix && ip4_in_cidr(f->a4[k].addr, net)) shared = true;
        f->a4[i] = f->a4[--f->na4];
        if (!shared) iph_del_route(t, net, index);
        for (int r = 0; r < t->nrt;) {                                     /* routes whose gateway is no longer reachable go too */
            if (t->rt[r].ifindex == index && !t->rt[r].onlink) {
                bool reach = false;
                for (int k = 0; k < f->na4; k++) { ip4_cidr c = { f->a4[k].addr, f->a4[k].prefix }; if (ip4_in_cidr(t->rt[r].gateway, c)) reach = true; }
                if (!reach) { t->rt[r] = t->rt[--t->nrt]; continue; }
            }
            r++;
        }
        return true;
    }
    return false;
}

bool iph_lookup(const iph_table *t, ip4_addr dest, iph_route *out)
{
    int best = -1;
    for (int i = 0; i < t->nrt; i++) {
        const iph_route *r = &t->rt[i];
        const iph_iface *f = NULL;
        for (int k = 0; k < IPH_MAX_IFACE; k++) if (t->ifc[k].used && t->ifc[k].index == r->ifindex) f = &t->ifc[k];
        if (!f || !f->up || !ip4_in_cidr(dest, r->dest)) continue;
        if (best < 0 || r->dest.prefix > t->rt[best].dest.prefix || (r->dest.prefix == t->rt[best].dest.prefix && r->metric < t->rt[best].metric)) best = i;
    }
    if (best < 0) return false;
    if (out) *out = t->rt[best];
    return true;
}
bool iph_next_hop(const iph_table *t, ip4_addr dest, ip4_addr *hop, uint32_t *ifindex)
{
    iph_route r;
    if (!iph_lookup(t, dest, &r)) return false;
    if (hop) *hop = r.onlink ? dest : r.gateway;
    if (ifindex) *ifindex = r.ifindex;
    return true;
}
bool iph_source_for(const iph_table *t, ip4_addr dest, ip4_addr *src)
{
    iph_route r;
    if (!iph_lookup(t, dest, &r)) return false;
    for (int k = 0; k < IPH_MAX_IFACE; k++) {
        const iph_iface *f = &t->ifc[k];
        if (!f->used || f->index != r.ifindex) continue;
        for (int i = 0; i < f->na4; i++) { ip4_cidr c = { f->a4[i].addr, f->a4[i].prefix }; if (ip4_in_cidr(r.onlink ? dest : r.gateway, c)) { *src = f->a4[i].addr; return true; } }
        if (f->na4) { *src = f->a4[0].addr; return true; }
    }
    return false;
}

void iph_clear_lease(iph_table *t, uint32_t index)
{
    iph_iface *f = iph_get(t, index);
    if (!f) return;
    for (int i = 0; i < f->na4;) {
        if (f->a4[i].dhcp) { ip4_addr a = f->a4[i].addr; iph_del_addr(t, index, a); i = 0; } else i++;
    }
    for (int r = 0; r < t->nrt;) if (t->rt[r].ifindex == index && t->rt[r].from_dhcp) t->rt[r] = t->rt[--t->nrt]; else r++;
    f->ndns = 0; f->domain[0] = 0;
}

void iph_apply_lease(iph_table *t, uint32_t index, const dhcp_lease *l)
{
    iph_iface *f = iph_get(t, index);
    if (!f) return;
    iph_clear_lease(t, index);
    int prefix = ip4_prefix_from_mask(l->mask);
    if (prefix <= 0) prefix = 24;
    if (!iph_add_addr(t, index, l->addr, (uint8_t)prefix)) return;
    for (int i = 0; i < f->na4; i++) if (ip4_eq(f->a4[i].addr, l->addr)) { f->a4[i].dhcp = true; f->a4[i].link_local = l->link_local; }
    int r = find_route(t, (ip4_cidr){ l->addr, (uint8_t)prefix }, index);
    if (r >= 0) t->rt[r].from_dhcp = true;
    if (!l->link_local && ip4_u32(l->router)) {
        ip4_cidr def = { { { 0, 0, 0, 0 } }, 0 };
        if (iph_add_route(t, def, l->router, index, 25)) { int d = find_route(t, def, index); if (d >= 0) t->rt[d].from_dhcp = true; }
    }
    f->ndns = l->ndns; for (int i = 0; i < l->ndns; i++) f->dns[i] = l->dns[i];
    strncpy(f->domain, l->domain, sizeof f->domain - 1);
    f->dhcp_enabled = true;
    if (l->mtu) f->mtu = l->mtu;
}

int iph_adapters(const iph_table *t, iph_iface *out, int max) { int n = 0; for (int i = 0; i < IPH_MAX_IFACE && n < max; i++) if (t->ifc[i].used) out[n++] = t->ifc[i]; return n; }
int iph_routes(const iph_table *t, iph_route *out, int max) { int n = t->nrt < max ? t->nrt : max; memcpy(out, t->rt, (size_t)n * sizeof *out); return n; }
