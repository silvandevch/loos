#include "dhcp.h"
#include "iphelp.h"
#include "test.h"
#include <string.h>
#include <stdlib.h>

static uint32_t seed = 12345;
static uint32_t rnd32(void *u) { (void)u; seed = seed * 1664525u + 1013904223u; return seed; }
static const uint8_t MAC[6] = { 0x02, 0x11, 0x22, 0x33, 0x44, 0x55 };
static const ip4_addr SRV = { { 192, 168, 1, 1 } };
static int leases_up, leases_down; static dhcp_lease last_lease;
static void on_lease(const dhcp_lease *l, bool acq, void *u) { (void)u; if (acq) { leases_up++; last_lease = *l; } else leases_down++; }

static int server_reply(const uint8_t *req, size_t rl, uint8_t type, ip4_addr yi, uint32_t lease, uint8_t *out, size_t cap, bool with_t)
{
    dhcp_msg q; dhcp_parse(req, rl, &q);
    dhcp_msg m; memset(&m, 0, sizeof m);
    m.op = 2; m.htype = 1; m.hlen = 6; m.xid = q.xid; m.msgtype = type; memcpy(m.chaddr, q.chaddr, 16);
    m.yiaddr = yi; m.has_server = true; m.server_id = SRV;
    if (type != DHCP_NAK) {
        m.has_subnet = true; m.subnet = (ip4_addr){ { 255, 255, 255, 0 } };
        m.router[m.nrouter++] = SRV; m.dns[m.ndns++] = (ip4_addr){ { 192, 168, 1, 53 } }; m.dns[m.ndns++] = (ip4_addr){ { 1, 1, 1, 1 } };
        m.has_lease = true; m.lease = lease; strcpy(m.domain, "home.lan");
        if (with_t) { m.has_t1 = true; m.t1 = lease / 4; m.has_t2 = true; m.t2 = lease / 2; }
    }
    return dhcp_build(out, cap, &m);
}

/* drive the client until it sends something */
static int pump(dhcp_client *c, uint64_t *now, uint8_t *pkt, ip4_addr *dest, int max_steps)
{
    for (int i = 0; i < max_steps; i++) { int n = dhcp_client_poll(c, *now, pkt, 600, dest); if (n > 0) return n; *now += 500; }
    return 0;
}

int main(void)
{
    /* ---- codec ---- */
    dhcp_msg m; memset(&m, 0, sizeof m);
    m.op = 1; m.xid = 0xdeadbeef; m.msgtype = DHCP_DISCOVER; memcpy(m.chaddr, MAC, 6);
    strcpy(m.hostname, "loos-box"); m.nparam = 3; m.param_req[0] = 1; m.param_req[1] = 3; m.param_req[2] = 6;
    m.client_id[0] = 1; memcpy(m.client_id + 1, MAC, 6); m.client_id_len = 7;
    uint8_t buf[600];
    int n = dhcp_build(buf, sizeof buf, &m);
    CHECK(n >= DHCP_MIN_PACKET && buf[0] == 1 && buf[1] == 1 && buf[2] == 6 && net_rd32(buf + 4) == 0xdeadbeef && net_rd32(buf + 236) == 0x63825363u);
    dhcp_msg p; CHECK(dhcp_parse(buf, (size_t)n, &p) && p.msgtype == DHCP_DISCOVER && p.xid == 0xdeadbeef && !strcmp(p.hostname, "loos-box") && p.nparam == 3 && p.client_id_len == 7 && !memcmp(p.chaddr, MAC, 6));
    CHECK(dhcp_build(buf, 100, &m) == -1);
    uint8_t trunc[600]; memcpy(trunc, buf, 244); CHECK(!dhcp_parse(trunc, 100, &p));
    uint8_t badmagic[600]; memcpy(badmagic, buf, 300); badmagic[236] = 0; CHECK(!dhcp_parse(badmagic, 300, &p));
    uint8_t badopt[300]; memcpy(badopt, buf, 300); badopt[240] = DHCP_OPT_LEASE; badopt[241] = 9; CHECK(!dhcp_parse(badopt, 245, &p));    /* option runs past the packet */
    uint8_t badlen[300]; memcpy(badlen, buf, 300); badlen[240] = DHCP_OPT_LEASE; badlen[241] = 3; badlen[242] = 1; badlen[243] = 2; badlen[244] = 3; badlen[245] = 255; CHECK(!dhcp_parse(badlen, 300, &p));   /* wrong-size lease */
    for (int t = 0; t < 3000; t++) { uint8_t f[600]; memcpy(f, buf, 300); uint32_t s = (uint32_t)t * 2654435761u; for (int k = 0; k < 6; k++) { s = s * 1664525u + 1013904223u; f[(s >> 8) % 300] = (uint8_t)(s >> 24); } dhcp_parse(f, 300, &p); }

    /* ---- client: DISCOVER -> OFFER -> REQUEST -> ACK ---- */
    dhcp_client c; dhcp_client_init(&c, MAC, "loos-box", rnd32, NULL); c.on_lease = on_lease;
    uint64_t now = 1000; uint8_t pkt[600], rep[600]; ip4_addr dest; int pl;
    dhcp_client_start(&c, now);
    pl = pump(&c, &now, pkt, &dest, 20);
    CHECK(pl > 0 && ip4_u32(dest) == 0xffffffffu && c.state == DHCPC_SELECTING);
    dhcp_msg d; dhcp_parse(pkt, (size_t)pl, &d);
    CHECK(d.msgtype == DHCP_DISCOVER && d.xid == c.xid && (d.flags & 0x8000) && !memcmp(d.chaddr, MAC, 6) && d.nparam >= 8 && d.client_id[0] == 1);
    ip4_addr mine = { { 192, 168, 1, 77 } };
    /* a reply for another client / another xid is ignored */
    int rl = server_reply(pkt, (size_t)pl, DHCP_OFFER, mine, 7200, rep, sizeof rep, false);
    uint8_t other[600]; memcpy(other, rep, (size_t)rl); other[28] ^= 0xff; dhcp_client_input(&c, other, (size_t)rl, now); CHECK(c.state == DHCPC_SELECTING);
    memcpy(other, rep, (size_t)rl); other[7] ^= 0x01; dhcp_client_input(&c, other, (size_t)rl, now); CHECK(c.state == DHCPC_SELECTING);
    uint8_t insane[600]; { dhcp_msg bad; dhcp_parse(rep, (size_t)rl, &bad); bad.yiaddr = (ip4_addr){ { 127, 0, 0, 1 } }; dhcp_build(insane, sizeof insane, &bad); }
    dhcp_client_input(&c, insane, 300, now); CHECK(c.state == DHCPC_SELECTING);
    dhcp_client_input(&c, rep, (size_t)rl, now + 50); CHECK(c.state == DHCPC_REQUESTING);
    now += 100;
    pl = pump(&c, &now, pkt, &dest, 20);
    dhcp_parse(pkt, (size_t)pl, &d);
    CHECK(d.msgtype == DHCP_REQUEST && d.has_requested && ip4_eq(d.requested_ip, mine) && d.has_server && ip4_eq(d.server_id, SRV) && ip4_u32(dest) == 0xffffffffu);
    rl = server_reply(pkt, (size_t)pl, DHCP_ACK, mine, 7200, rep, sizeof rep, false);
    dhcp_client_input(&c, rep, (size_t)rl, now);
    CHECK(c.state == DHCPC_BOUND && c.have_lease && leases_up == 1 && ip4_eq(last_lease.addr, mine) && ip4_u32(last_lease.mask) == 0xffffff00u);
    CHECK(ip4_eq(last_lease.router, SRV) && last_lease.ndns == 2 && !strcmp(last_lease.domain, "home.lan") && last_lease.lease_s == 7200 && last_lease.t1_s == 3600 && last_lease.t2_s == 6300);
    CHECK(dhcp_client_remaining_s(&c, now + 1000000) == 7200 - 1000);
    CHECK(dhcp_client_poll(&c, now + 1000, pkt, sizeof pkt, &dest) == 0);                         /* nothing to do while bound */

    /* ---- IP helper applies the lease ---- */
    iph_table *t = calloc(1, sizeof *t); iph_init(t);
    uint32_t eth = iph_add_iface(t, "eth0", MAC, 1500); iph_set_up(t, eth, true);
    CHECK(iph_add_iface(t, "eth0", MAC, 1500) == 0 && eth == 2 && iph_by_name(t, "eth0") && iph_get(t, 1)->loopback);
    iph_apply_lease(t, eth, &last_lease);
    iph_iface *f = iph_get(t, eth);
    CHECK(f->na4 == 1 && f->a4[0].dhcp && f->a4[0].prefix == 24 && f->ndns == 2 && !strcmp(f->domain, "home.lan") && f->dhcp_enabled);
    iph_route rt; ip4_addr hop, src; uint32_t idx;
    CHECK(iph_lookup(t, (ip4_addr){ { 192, 168, 1, 200 } }, &rt) && rt.dest.prefix == 24 && rt.onlink);
    CHECK(iph_lookup(t, (ip4_addr){ { 8, 8, 8, 8 } }, &rt) && rt.dest.prefix == 0 && ip4_eq(rt.gateway, SRV) && rt.metric == 25);
    CHECK(iph_next_hop(t, (ip4_addr){ { 8, 8, 8, 8 } }, &hop, &idx) && ip4_eq(hop, SRV) && idx == eth);
    CHECK(iph_next_hop(t, (ip4_addr){ { 192, 168, 1, 9 } }, &hop, &idx) && ip4_eq(hop, (ip4_addr){ { 192, 168, 1, 9 } }));
    CHECK(iph_source_for(t, (ip4_addr){ { 8, 8, 8, 8 } }, &src) && ip4_eq(src, mine) && iph_source_for(t, (ip4_addr){ { 127, 0, 0, 9 } }, &src) && ip4_eq(src, (ip4_addr){ { 127, 0, 0, 1 } }));
    CHECK(iph_lookup(t, (ip4_addr){ { 127, 5, 5, 5 } }, &rt) && rt.ifindex == 1);
    /* interface down removes it from lookups; a more specific / lower-metric route wins */
    iph_set_up(t, eth, false); CHECK(!iph_lookup(t, (ip4_addr){ { 8, 8, 8, 8 } }, &rt)); iph_set_up(t, eth, true);
    uint8_t mac2[6] = { 2, 0, 0, 0, 0, 2 }; uint32_t wlan = iph_add_iface(t, "wlan0", mac2, 1500); iph_set_up(t, wlan, true);
    iph_add_addr(t, wlan, (ip4_addr){ { 10, 9, 0, 5 } }, 16);
    ip4_cidr def = { { { 0, 0, 0, 0 } }, 0 };
    CHECK(!iph_add_route(t, def, (ip4_addr){ { 172, 16, 0, 1 } }, wlan, 5));                         /* gateway not on-link for that interface */
    CHECK(iph_add_route(t, def, (ip4_addr){ { 10, 9, 0, 1 } }, wlan, 10));
    CHECK(iph_lookup(t, (ip4_addr){ { 1, 1, 1, 1 } }, &rt) && rt.ifindex == wlan && rt.metric == 10);   /* metric 10 beats the DHCP route's 25 */
    ip4_cidr sub = { { { 10, 50, 0, 0 } }, 16 };
    CHECK(iph_add_route(t, sub, (ip4_addr){ { 192, 168, 1, 1 } }, eth, 100) && iph_lookup(t, (ip4_addr){ { 10, 50, 3, 3 } }, &rt) && rt.ifindex == eth && rt.dest.prefix == 16);   /* longest prefix wins over metric */
    CHECK(iph_del_route(t, sub, eth) && !iph_del_route(t, sub, eth));
    CHECK(!iph_add_addr(t, wlan, (ip4_addr){ { 10, 9, 0, 5 } }, 16) && !iph_add_addr(t, wlan, (ip4_addr){ { 224, 0, 0, 1 } }, 8) && !iph_add_addr(t, 99, (ip4_addr){ { 1, 2, 3, 4 } }, 8));
    /* removing the address that carries the gateway drops the dependent default route */
    CHECK(iph_del_addr(t, wlan, (ip4_addr){ { 10, 9, 0, 5 } }));
    CHECK(iph_lookup(t, (ip4_addr){ { 1, 1, 1, 1 } }, &rt) && rt.ifindex == eth);
    iph_iface list[8]; iph_route rts[64]; CHECK(iph_adapters(t, list, 8) == 3 && iph_routes(t, rts, 64) >= 3);
    iph_clear_lease(t, eth);
    CHECK(iph_get(t, eth)->na4 == 0 && iph_get(t, eth)->ndns == 0 && !iph_lookup(t, (ip4_addr){ { 8, 8, 8, 8 } }, &rt));
    free(t);

    /* ---- renewal at T1 (unicast), then rebinding, then expiry back to INIT ---- */
    uint64_t t0 = now;
    now = t0 + 3600 * 1000ull;
    pl = dhcp_client_poll(&c, now, pkt, sizeof pkt, &dest);
    CHECK(pl > 0 && c.state == DHCPC_RENEWING && ip4_eq(dest, SRV));
    dhcp_parse(pkt, (size_t)pl, &d);
    CHECK(d.msgtype == DHCP_REQUEST && ip4_eq(d.ciaddr, mine) && !d.has_requested && !d.has_server);
    rl = server_reply(pkt, (size_t)pl, DHCP_ACK, mine, 7200, rep, sizeof rep, true);
    dhcp_client_input(&c, rep, (size_t)rl, now + 20);
    CHECK(c.state == DHCPC_BOUND && leases_up == 1 && c.lease.t1_s == 1800 && c.lease.t2_s == 3600 && c.bound_at_ms == now + 20);      /* same address: no new lease event */
    /* server vanishes: T1 passes, renewing gets no answer, T2 moves to rebinding (broadcast), then the lease expires */
    uint64_t b = c.bound_at_ms;
    now = b + 1800 * 1000ull; pl = dhcp_client_poll(&c, now, pkt, sizeof pkt, &dest); CHECK(pl > 0 && c.state == DHCPC_RENEWING);
    now = b + 3600 * 1000ull; pl = dhcp_client_poll(&c, now, pkt, sizeof pkt, &dest); CHECK(pl > 0 && c.state == DHCPC_REBINDING && ip4_u32(dest) == 0xffffffffu);
    CHECK(leases_down == 0);
    now = b + 7200 * 1000ull + 1; pl = dhcp_client_poll(&c, now, pkt, sizeof pkt, &dest);
    CHECK(leases_down == 1 && !c.have_lease && (c.state == DHCPC_INIT || c.state == DHCPC_SELECTING) && (pl == 0 || ip4_u32(dest) == 0xffffffffu));

    /* ---- NAK while renewing drops the lease and restarts discovery ---- */
    dhcp_client c2; dhcp_client_init(&c2, MAC, "h", rnd32, NULL); c2.on_lease = on_lease;
    uint64_t n2 = 0; dhcp_client_start(&c2, n2);
    pl = pump(&c2, &n2, pkt, &dest, 20); rl = server_reply(pkt, (size_t)pl, DHCP_OFFER, mine, 600, rep, sizeof rep, false); dhcp_client_input(&c2, rep, (size_t)rl, n2);
    pl = pump(&c2, &n2, pkt, &dest, 20); rl = server_reply(pkt, (size_t)pl, DHCP_ACK, mine, 600, rep, sizeof rep, false); dhcp_client_input(&c2, rep, (size_t)rl, n2);
    CHECK(c2.state == DHCPC_BOUND && c2.lease.t1_s == 300);
    int up = leases_up, down = leases_down;
    n2 += 301000; pl = dhcp_client_poll(&c2, n2, pkt, sizeof pkt, &dest); CHECK(c2.state == DHCPC_RENEWING && pl > 0);
    rl = server_reply(pkt, (size_t)pl, DHCP_NAK, (ip4_addr){ { 0, 0, 0, 0 } }, 0, rep, sizeof rep, false); dhcp_client_input(&c2, rep, (size_t)rl, n2);
    CHECK(c2.state == DHCPC_INIT && leases_down == down + 1 && leases_up == up);
    /* release */
    dhcp_client c3; dhcp_client_init(&c3, MAC, "h", rnd32, NULL);
    CHECK(dhcp_client_release(&c3, pkt, sizeof pkt, &dest) == 0);                                  /* nothing to release */
    c2.have_lease = true; c2.lease = last_lease; c2.state = DHCPC_BOUND;
    pl = dhcp_client_release(&c2, pkt, sizeof pkt, &dest); dhcp_parse(pkt, (size_t)pl, &d);
    CHECK(pl > 0 && d.msgtype == DHCP_RELEASE && ip4_eq(d.ciaddr, last_lease.addr) && ip4_eq(dest, last_lease.server) && !c2.have_lease);

    /* ---- nobody answers: retransmit with growing backoff, then fall back to link-local ---- */
    dhcp_client c4; dhcp_client_init(&c4, MAC, "h", rnd32, NULL); c4.on_lease = on_lease;
    uint64_t n4 = 0; dhcp_client_start(&c4, n4);
    int sends = 0; uint64_t last_send = 0, min_gap = 0, prev_gap = 0; bool grew = true;
    up = leases_up;
    for (int step = 0; step < 400 && c4.state != DHCPC_FALLBACK; step++, n4 += 500) {
        pl = dhcp_client_poll(&c4, n4, pkt, sizeof pkt, &dest);
        if (pl > 0) { if (sends) { uint64_t gap = n4 - last_send; if (prev_gap && gap + 2500 < prev_gap) grew = false; prev_gap = gap; if (!min_gap || gap < min_gap) min_gap = gap; } last_send = n4; sends++; }
    }
    CHECK(sends == 5 && grew && min_gap >= 2000 && c4.state == DHCPC_FALLBACK && c4.lease.link_local && leases_up == up + 1);
    CHECK(c4.lease.addr.b[0] == 169 && c4.lease.addr.b[1] == 254 && c4.lease.addr.b[2] >= 1 && c4.lease.addr.b[2] <= 254 && ip4_u32(c4.lease.mask) == 0xffff0000u);
    ip4_addr l1 = dhcp_link_local_for(MAC, 0), l2 = dhcp_link_local_for(MAC, 0), l3 = dhcp_link_local_for(MAC, 1);
    CHECK(ip4_eq(l1, l2) && !ip4_eq(l1, l3) && ip4_is_link_local(l3));
    CHECK(dhcp_client_remaining_s(&c4, n4) == 0);
    /* the fallback address lands on the interface without a gateway */
    iph_table *t2 = calloc(1, sizeof *t2); iph_init(t2); uint32_t e2 = iph_add_iface(t2, "eth0", MAC, 1500); iph_set_up(t2, e2, true);
    iph_apply_lease(t2, e2, &c4.lease);
    CHECK(iph_get(t2, e2)->a4[0].link_local && !iph_lookup(t2, (ip4_addr){ { 8, 8, 8, 8 } }, &rt) && iph_lookup(t2, (ip4_addr){ { 169, 254, 7, 7 } }, &rt));
    /* ... and a real server turning up later still wins: after the quiet minute it probes again and binds */
    n4 += 61000; pl = dhcp_client_poll(&c4, n4, pkt, sizeof pkt, &dest);
    CHECK(pl > 0 && c4.state == DHCPC_SELECTING);
    rl = server_reply(pkt, (size_t)pl, DHCP_OFFER, mine, 3600, rep, sizeof rep, false); dhcp_client_input(&c4, rep, (size_t)rl, n4);
    pl = pump(&c4, &n4, pkt, &dest, 10); rl = server_reply(pkt, (size_t)pl, DHCP_ACK, mine, 3600, rep, sizeof rep, false); dhcp_client_input(&c4, rep, (size_t)rl, n4);
    CHECK(c4.state == DHCPC_BOUND && !c4.lease.link_local && ip4_eq(c4.lease.addr, mine));
    iph_apply_lease(t2, e2, &c4.lease);
    CHECK(iph_get(t2, e2)->na4 == 1 && !iph_get(t2, e2)->a4[0].link_local && iph_lookup(t2, (ip4_addr){ { 8, 8, 8, 8 } }, &rt));
    free(t2);
    DONE("dhcp_test");
}
