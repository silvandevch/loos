#include "dhcp.h"
#include <string.h>

#define MAGIC 0x63825363u
#define FIXED 240

static int put_opt(uint8_t *b, size_t cap, size_t *pos, uint8_t code, const void *data, size_t n)
{
    if (*pos + 2 + n > cap || n > 255) return -1;
    b[(*pos)++] = code; b[(*pos)++] = (uint8_t)n;
    if (n) memcpy(b + *pos, data, n);
    *pos += n;
    return 0;
}
static int put_u8(uint8_t *b, size_t cap, size_t *pos, uint8_t code, uint8_t v) { return put_opt(b, cap, pos, code, &v, 1); }
static int put_u32(uint8_t *b, size_t cap, size_t *pos, uint8_t code, uint32_t v) { uint8_t t[4]; net_wr32(t, v); return put_opt(b, cap, pos, code, t, 4); }

int dhcp_build(uint8_t *buf, size_t cap, const dhcp_msg *m)
{
    if (cap < DHCP_MIN_PACKET) return -1;
    memset(buf, 0, cap);
    buf[0] = m->op; buf[1] = m->htype ? m->htype : 1; buf[2] = m->hlen ? m->hlen : 6;
    net_wr32(buf + 4, m->xid); net_wr16(buf + 8, m->secs); net_wr16(buf + 10, m->flags);
    memcpy(buf + 12, m->ciaddr.b, 4); memcpy(buf + 16, m->yiaddr.b, 4); memcpy(buf + 20, m->siaddr.b, 4); memcpy(buf + 24, m->giaddr.b, 4);
    memcpy(buf + 28, m->chaddr, 16);
    net_wr32(buf + 236, MAGIC);
    size_t pos = FIXED;
    int rc = put_u8(buf, cap, &pos, DHCP_OPT_MSGTYPE, m->msgtype);
    if (m->has_subnet) rc |= put_opt(buf, cap, &pos, DHCP_OPT_SUBNET, m->subnet.b, 4);
    if (m->nrouter) { uint8_t t[16]; for (int i = 0; i < m->nrouter && i < 4; i++) memcpy(t + 4 * i, m->router[i].b, 4); rc |= put_opt(buf, cap, &pos, DHCP_OPT_ROUTER, t, 4 * (size_t)(m->nrouter > 4 ? 4 : m->nrouter)); }
    if (m->ndns) { uint8_t t[16]; for (int i = 0; i < m->ndns && i < 4; i++) memcpy(t + 4 * i, m->dns[i].b, 4); rc |= put_opt(buf, cap, &pos, DHCP_OPT_DNS, t, 4 * (size_t)(m->ndns > 4 ? 4 : m->ndns)); }
    if (m->hostname[0]) rc |= put_opt(buf, cap, &pos, DHCP_OPT_HOSTNAME, m->hostname, strlen(m->hostname));
    if (m->domain[0]) rc |= put_opt(buf, cap, &pos, DHCP_OPT_DOMAIN, m->domain, strlen(m->domain));
    if (m->has_mtu) { uint8_t t[2]; net_wr16(t, m->mtu); rc |= put_opt(buf, cap, &pos, DHCP_OPT_MTU, t, 2); }
    if (m->has_requested) rc |= put_opt(buf, cap, &pos, DHCP_OPT_REQUESTED_IP, m->requested_ip.b, 4);
    if (m->has_lease) rc |= put_u32(buf, cap, &pos, DHCP_OPT_LEASE, m->lease);
    if (m->has_server) rc |= put_opt(buf, cap, &pos, DHCP_OPT_SERVER_ID, m->server_id.b, 4);
    if (m->nparam) rc |= put_opt(buf, cap, &pos, DHCP_OPT_PARAM_REQ, m->param_req, (size_t)m->nparam);
    if (m->has_t1) rc |= put_u32(buf, cap, &pos, DHCP_OPT_T1, m->t1);
    if (m->has_t2) rc |= put_u32(buf, cap, &pos, DHCP_OPT_T2, m->t2);
    if (m->client_id_len) rc |= put_opt(buf, cap, &pos, DHCP_OPT_CLIENT_ID, m->client_id, (size_t)m->client_id_len);
    if (m->op == 1) { uint8_t t[2]; net_wr16(t, 1500); rc |= put_opt(buf, cap, &pos, DHCP_OPT_MAXMSG, t, 2); }
    if (rc || pos + 1 > cap) return -1;
    buf[pos++] = DHCP_OPT_END;
    return (int)(pos < DHCP_MIN_PACKET ? DHCP_MIN_PACKET : pos);
}

bool dhcp_parse(const uint8_t *b, size_t len, dhcp_msg *m)
{
    if (len < FIXED || net_rd32(b + 236) != MAGIC) return false;
    memset(m, 0, sizeof *m);
    m->op = b[0]; m->htype = b[1]; m->hlen = b[2];
    if (m->hlen > 16) return false;
    m->xid = net_rd32(b + 4); m->secs = net_rd16(b + 8); m->flags = net_rd16(b + 10);
    memcpy(m->ciaddr.b, b + 12, 4); memcpy(m->yiaddr.b, b + 16, 4); memcpy(m->siaddr.b, b + 20, 4); memcpy(m->giaddr.b, b + 24, 4);
    memcpy(m->chaddr, b + 28, 16);
    size_t p = FIXED;
    while (p < len) {
        uint8_t code = b[p++];
        if (code == DHCP_OPT_END) break;
        if (code == 0) continue;
        if (p >= len) return false;
        size_t n = b[p++];
        if (p + n > len) return false;
        const uint8_t *d = b + p;
        switch (code) {
        case DHCP_OPT_MSGTYPE: if (n != 1) return false; m->msgtype = d[0]; break;
        case DHCP_OPT_SUBNET: if (n != 4) return false; memcpy(m->subnet.b, d, 4); m->has_subnet = true; break;
        case DHCP_OPT_ROUTER: if (n % 4) return false; for (size_t i = 0; i < n / 4 && m->nrouter < 4; i++) memcpy(m->router[m->nrouter++].b, d + 4 * i, 4); break;
        case DHCP_OPT_DNS: if (n % 4) return false; for (size_t i = 0; i < n / 4 && m->ndns < 4; i++) memcpy(m->dns[m->ndns++].b, d + 4 * i, 4); break;
        case DHCP_OPT_NTP: if (n % 4) return false; for (size_t i = 0; i < n / 4 && m->nntp < 2; i++) memcpy(m->ntp[m->nntp++].b, d + 4 * i, 4); break;
        case DHCP_OPT_BROADCAST: if (n == 4) memcpy(m->broadcast.b, d, 4); break;
        case DHCP_OPT_HOSTNAME: { size_t c = n < sizeof m->hostname - 1 ? n : sizeof m->hostname - 1; memcpy(m->hostname, d, c); break; }
        case DHCP_OPT_DOMAIN: { size_t c = n < sizeof m->domain - 1 ? n : sizeof m->domain - 1; memcpy(m->domain, d, c); break; }
        case DHCP_OPT_MTU: if (n != 2) return false; m->mtu = net_rd16(d); m->has_mtu = true; break;
        case DHCP_OPT_REQUESTED_IP: if (n != 4) return false; memcpy(m->requested_ip.b, d, 4); m->has_requested = true; break;
        case DHCP_OPT_LEASE: if (n != 4) return false; m->lease = net_rd32(d); m->has_lease = true; break;
        case DHCP_OPT_T1: if (n != 4) return false; m->t1 = net_rd32(d); m->has_t1 = true; break;
        case DHCP_OPT_T2: if (n != 4) return false; m->t2 = net_rd32(d); m->has_t2 = true; break;
        case DHCP_OPT_SERVER_ID: if (n != 4) return false; memcpy(m->server_id.b, d, 4); m->has_server = true; break;
        case DHCP_OPT_CLIENT_ID: m->client_id_len = (int)(n < sizeof m->client_id ? n : sizeof m->client_id); memcpy(m->client_id, d, (size_t)m->client_id_len); break;
        case DHCP_OPT_PARAM_REQ: m->nparam = (int)(n < sizeof m->param_req ? n : sizeof m->param_req); memcpy(m->param_req, d, (size_t)m->nparam); break;
        case DHCP_OPT_SEARCH: { size_t c = n < sizeof m->search - 1 ? n : sizeof m->search - 1; memcpy(m->search, d, c); break; }
        default: break;
        }
        p += n;
    }
    return m->msgtype != 0;
}

/* ---- client ---- */
static uint32_t rnd(dhcp_client *c) { return c->rand32 ? c->rand32(c->user) : 0x9e3779b9u * (uint32_t)(c->xid + 1); }

void dhcp_client_init(dhcp_client *c, const uint8_t mac[6], const char *hostname, uint32_t (*r)(void *), void *user)
{
    memset(c, 0, sizeof *c);
    memcpy(c->mac, mac, 6); c->rand32 = r; c->user = user;
    if (hostname) { strncpy(c->hostname, hostname, sizeof c->hostname - 1); }
    c->state = DHCPC_INIT;
}

static void enter(dhcp_client *c, dhcp_state s, uint64_t now) { c->state = s; c->state_since_ms = now; c->retries = 0; c->backoff_ms = 4000; c->next_tx_ms = now; }

void dhcp_client_start(dhcp_client *c, uint64_t now)
{
    c->xid = rnd(c); c->start_ms = now; c->discover_failures = 0;
    enter(c, DHCPC_INIT, now);
    c->next_tx_ms = now + (rnd(c) % 1000);                                /* RFC 2131: small random delay before the first DISCOVER */
}

ip4_addr dhcp_link_local_for(const uint8_t mac[6], unsigned attempt)
{
    uint32_t h = 2166136261u;
    for (int i = 0; i < 6; i++) { h ^= mac[i]; h *= 16777619u; }
    h += attempt * 0x9e3779b9u;
    ip4_addr a = { { 169, 254, (uint8_t)(1 + (h >> 8) % 254), (uint8_t)(1 + (h & 0xff) % 254) } };
    return a;                                                              /* 169.254.1.0-169.254.254.255 is the usable range */
}

static void base_msg(dhcp_client *c, dhcp_msg *m, uint8_t type, uint64_t now)
{
    memset(m, 0, sizeof *m);
    m->op = 1; m->htype = 1; m->hlen = 6; m->xid = c->xid; m->msgtype = type;
    m->secs = (uint16_t)((now - c->start_ms) / 1000 > 0xffff ? 0xffff : (now - c->start_ms) / 1000);
    memcpy(m->chaddr, c->mac, 6);
    m->client_id[0] = 1; memcpy(m->client_id + 1, c->mac, 6); m->client_id_len = 7;
    strncpy(m->hostname, c->hostname, sizeof m->hostname - 1);
    static const uint8_t want[] = { DHCP_OPT_SUBNET, DHCP_OPT_ROUTER, DHCP_OPT_DNS, DHCP_OPT_DOMAIN, DHCP_OPT_MTU, DHCP_OPT_BROADCAST, DHCP_OPT_NTP, DHCP_OPT_LEASE, DHCP_OPT_T1, DHCP_OPT_T2, DHCP_OPT_SEARCH };
    memcpy(m->param_req, want, sizeof want); m->nparam = (int)sizeof want;
}

static void backoff(dhcp_client *c, uint64_t now)
{
    uint32_t jitter = rnd(c) % 2001;                                       /* +/- 1 s, RFC 2131 4.1 */
    c->next_tx_ms = now + c->backoff_ms - 1000 + jitter;
    if (c->backoff_ms < 64000) c->backoff_ms *= 2;
    c->retries++;
}

int dhcp_client_poll(dhcp_client *c, uint64_t now, uint8_t *pkt, size_t cap, ip4_addr *dest)
{
    dhcp_msg m;
    ip4_addr bcast = { { 255, 255, 255, 255 } };
    if (c->state == DHCPC_BOUND) {                                         /* timers */
        uint64_t elapsed = (now - c->bound_at_ms) / 1000;
        if (elapsed >= c->lease.lease_s) {
            dhcp_lease lost = c->lease;
            c->have_lease = false;
            if (c->on_lease) c->on_lease(&lost, false, c->user);
            c->xid = rnd(c); enter(c, DHCPC_INIT, now);
        } else if (elapsed >= c->lease.t2_s) { enter(c, DHCPC_REBINDING, now); }
        else if (elapsed >= c->lease.t1_s) { c->xid = rnd(c); enter(c, DHCPC_RENEWING, now); }
        else return 0;
    }
    if (c->state == DHCPC_RENEWING || c->state == DHCPC_REBINDING) {
        uint64_t elapsed = (now - c->bound_at_ms) / 1000;
        if (c->state == DHCPC_RENEWING && elapsed >= c->lease.t2_s) { c->xid = rnd(c); enter(c, DHCPC_REBINDING, now); }
        if (elapsed >= c->lease.lease_s) {
            dhcp_lease lost = c->lease; c->have_lease = false;
            if (c->on_lease) c->on_lease(&lost, false, c->user);
            c->xid = rnd(c); enter(c, DHCPC_INIT, now);
        }
    }
    if (now < c->next_tx_ms) return 0;
    switch (c->state) {
    case DHCPC_INIT: case DHCPC_SELECTING:
        if (c->retries >= 5) {                                              /* nobody answered: self-assign a link-local address */
            c->discover_failures++;
            dhcp_lease ll; memset(&ll, 0, sizeof ll);
            ll.addr = dhcp_link_local_for(c->mac, (unsigned)c->discover_failures); ll.mask = ip4_from_u32(0xffff0000u); ll.link_local = true;
            ll.lease_s = 0xffffffffu; c->lease = ll; c->have_lease = true;
            if (c->on_lease) c->on_lease(&ll, true, c->user);
            enter(c, DHCPC_FALLBACK, now);
            c->next_tx_ms = now + 60000;                                    /* keep probing for a real server every minute */
            return 0;
        }
        base_msg(c, &m, DHCP_DISCOVER, now);
        c->state = DHCPC_SELECTING;
        *dest = bcast; m.flags = 0x8000;
        backoff(c, now);
        return dhcp_build(pkt, cap, &m);
    case DHCPC_FALLBACK:
        c->xid = rnd(c); c->retries = 0; c->backoff_ms = 4000; c->state = DHCPC_INIT; c->next_tx_ms = now;
        return dhcp_client_poll(c, now, pkt, cap, dest);
    case DHCPC_REQUESTING:
        if (c->retries >= 4) { c->xid = rnd(c); enter(c, DHCPC_INIT, now); return 0; }
        base_msg(c, &m, DHCP_REQUEST, now);
        m.has_requested = true; m.requested_ip = c->offer.yiaddr; m.has_server = true; m.server_id = c->offer.server_id; m.flags = 0x8000;
        *dest = bcast; backoff(c, now);
        return dhcp_build(pkt, cap, &m);
    case DHCPC_RENEWING: case DHCPC_REBINDING:
        base_msg(c, &m, DHCP_REQUEST, now);
        m.ciaddr = c->lease.addr;
        *dest = c->state == DHCPC_RENEWING ? c->lease.server : bcast;
        { uint64_t left_s = c->lease.lease_s - (now - c->bound_at_ms) / 1000; c->backoff_ms = left_s > 120 ? 60000 : 20000; }
        c->next_tx_ms = now + c->backoff_ms / 2;
        return dhcp_build(pkt, cap, &m);
    default: return 0;
    }
}

static bool sane_addr(ip4_addr a) { return ip4_u32(a) != 0 && ip4_u32(a) != 0xffffffffu && !ip4_is_multicast(a) && !ip4_is_loopback(a); }

static void bind_lease(dhcp_client *c, const dhcp_msg *ack, uint64_t now)
{
    dhcp_lease l; memset(&l, 0, sizeof l);
    l.addr = ack->yiaddr;
    int pre = ack->has_subnet ? ip4_prefix_from_mask(ack->subnet) : -1;
    l.mask = pre >= 0 ? ack->subnet : ip4_from_u32(ip4_mask(ip4_is_private(l.addr) && l.addr.b[0] == 10 ? 8 : 24));
    if (ack->nrouter && sane_addr(ack->router[0])) l.router = ack->router[0];
    for (int i = 0; i < ack->ndns && l.ndns < 4; i++) if (sane_addr(ack->dns[i])) l.dns[l.ndns++] = ack->dns[i];
    strncpy(l.domain, ack->domain, sizeof l.domain - 1);
    l.server = ack->has_server ? ack->server_id : c->offer.server_id;
    l.lease_s = ack->has_lease && ack->lease >= 60 ? ack->lease : 3600;
    l.t1_s = ack->has_t1 && ack->t1 > 0 && ack->t1 < l.lease_s ? ack->t1 : l.lease_s / 2;
    l.t2_s = ack->has_t2 && ack->t2 > l.t1_s && ack->t2 < l.lease_s ? ack->t2 : (uint32_t)((uint64_t)l.lease_s * 7 / 8);
    l.mtu = ack->has_mtu && ack->mtu >= 576 ? ack->mtu : 0;
    bool had = c->have_lease && !c->lease.link_local;
    bool changed = !had || !ip4_eq(c->lease.addr, l.addr);
    c->lease = l; c->have_lease = true; c->bound_at_ms = now;
    enter(c, DHCPC_BOUND, now);
    if (changed && c->on_lease) c->on_lease(&l, true, c->user);
}

void dhcp_client_input(dhcp_client *c, const uint8_t *pkt, size_t len, uint64_t now)
{
    dhcp_msg m;
    if (!dhcp_parse(pkt, len, &m) || m.op != 2 || m.xid != c->xid || memcmp(m.chaddr, c->mac, 6)) return;
    switch (c->state) {
    case DHCPC_SELECTING:
        if (m.msgtype == DHCP_OFFER && sane_addr(m.yiaddr) && m.has_server) {
            c->offer = m; enter(c, DHCPC_REQUESTING, now);
        }
        break;
    case DHCPC_REQUESTING: case DHCPC_RENEWING: case DHCPC_REBINDING:
        if (m.msgtype == DHCP_ACK && sane_addr(m.yiaddr)) bind_lease(c, &m, now);
        else if (m.msgtype == DHCP_NAK) {
            if (c->have_lease && !c->lease.link_local && c->on_lease) { dhcp_lease lost = c->lease; c->have_lease = false; c->on_lease(&lost, false, c->user); }
            c->xid = rnd(c); enter(c, DHCPC_INIT, now);
        }
        break;
    default: break;
    }
}

int dhcp_client_release(dhcp_client *c, uint8_t *pkt, size_t cap, ip4_addr *dest)
{
    if (!c->have_lease || c->lease.link_local) return 0;
    dhcp_msg m;
    base_msg(c, &m, DHCP_RELEASE, c->bound_at_ms);
    m.nparam = 0; m.ciaddr = c->lease.addr; m.has_server = true; m.server_id = c->lease.server;
    *dest = c->lease.server;
    dhcp_lease gone = c->lease; c->have_lease = false;
    if (c->on_lease) c->on_lease(&gone, false, c->user);
    c->state = DHCPC_INIT;
    return dhcp_build(pkt, cap, &m);
}

uint32_t dhcp_client_remaining_s(const dhcp_client *c, uint64_t now)
{
    if (!c->have_lease || c->lease.link_local) return 0;
    uint64_t e = (now - c->bound_at_ms) / 1000;
    return e >= c->lease.lease_s ? 0 : (uint32_t)(c->lease.lease_s - e);
}
