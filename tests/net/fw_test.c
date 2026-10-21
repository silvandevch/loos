#include "fw.h"
#include "test.h"
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

static fw_ip V4(int a, int b, int c, int d) { fw_ip i; memset(&i, 0, sizeof i); i.a4 = (ip4_addr){ { (uint8_t)a, (uint8_t)b, (uint8_t)c, (uint8_t)d } }; return i; }
static fw_ip V6(const char *s) { fw_ip i; memset(&i, 0, sizeof i); i.v6 = true; ip6_parse(s, &i.a6); return i; }
static fw_packet pk(fw_dir dir, fw_proto proto, fw_ip local, uint16_t lport, fw_ip remote, uint16_t rport, const char *app, uint8_t flags)
{
    fw_packet p; memset(&p, 0, sizeof p);
    p.dir = dir; p.proto = proto; p.local = local; p.remote = remote; p.lport = lport; p.rport = rport; p.tcp_flags = flags; p.len = 100; p.ifindex = 2;
    strncpy(p.app, app, sizeof p.app - 1);
    return p;
}
static fw_verdict ev(fw_engine *f, fw_packet p, uint64_t t, fw_reason *why) { return fw_eval(f, &p, t, why, NULL); }

int main(void)
{
    fw_engine *f = calloc(1, sizeof *f);
    fw_reason why; fw_ip me = V4(192, 168, 1, 10), web = V4(93, 184, 216, 34), dns = V4(1, 1, 1, 1);
    fw_init(f);

    /* defaults: outbound asks, inbound drops, loopback always passes */
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 50000, web, 443, "browser.exe", TCP_SYN), 1000, &why) == FWV_ASK && why == FWR_DEFAULT);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 22, web, 40000, "sshd", TCP_SYN), 1000, &why) == FWV_DROP && why == FWR_DEFAULT);
    CHECK(ev(f, pk(FW_OUT, FW_UDP, V4(127, 0, 0, 1), 5000, V4(127, 0, 0, 1), 5001, "x", 0), 1000, &why) == FWV_ALLOW && why == FWR_LOOPBACK);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 1, V4(127, 0, 0, 1), 2, "x", TCP_SYN), 1000, &why) == FWV_ASK);               /* only loopback-to-loopback counts */
    CHECK(ev(f, pk(FW_OUT, FW_TCP, V6("::1"), 5, V6("::1"), 6, "x", TCP_SYN), 1000, &why) == FWV_ALLOW && why == FWR_LOOPBACK);
    CHECK(ev(f, pk(FW_OUT, (fw_proto)99, me, 0, web, 0, "x", 0), 1000, &why) == FWV_DROP && why == FWR_INVALID);
    fw_packet mixed = pk(FW_OUT, FW_UDP, me, 1, V6("2001:db8::1"), 2, "x", 0); CHECK(fw_eval(f, &mixed, 1000, &why, NULL) == FWV_DROP && why == FWR_INVALID);

    /* per-process policy: allow creates a flow, replies ride it, strangers do not */
    fw_set_app_policy(f, "browser.exe", FWA_ALLOW);
    CHECK(fw_get_app_policy(f, "browser.exe") == FWA_ALLOW && fw_get_app_policy(f, "other.exe") == FWA_ASK);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 50000, web, 443, "browser.exe", TCP_SYN), 2000, &why) == FWV_ALLOW && why == FWR_APP_POLICY && f->nflow == 1);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 50000, web, 443, "browser.exe", TCP_SYN | TCP_ACK), 2010, &why) == FWV_ALLOW && why == FWR_ESTABLISHED);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 50000, web, 443, "browser.exe", TCP_ACK), 2020, &why) == FWV_ALLOW && why == FWR_ESTABLISHED);
    fw_flow fl[4]; CHECK(fw_flows(f, fl, 4) == 1 && fl[0].state == 2 && fl[0].bytes_in == 100 && fl[0].bytes_out == 200);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 50000, V4(6, 6, 6, 6), 443, "browser.exe", TCP_SYN | TCP_ACK), 2030, &why) == FWV_DROP && why == FWR_INVALID);        /* wrong peer */
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 50000, web, 443, "browser.exe", TCP_ACK), 2040, &why) == FWV_ALLOW);
    /* FIN then RST close the flow; later segments are invalid */
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 50000, web, 443, "browser.exe", TCP_FIN | TCP_ACK), 2050, &why) == FWV_ALLOW);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 50000, web, 443, "browser.exe", TCP_FIN | TCP_ACK), 2060, &why) == FWV_ALLOW);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 50000, web, 443, "browser.exe", TCP_ACK), 2070, &why) == FWV_DROP);
    /* an RST kills a live flow */
    ev(f, pk(FW_OUT, FW_TCP, me, 50001, web, 443, "browser.exe", TCP_SYN), 3000, &why);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 50001, web, 443, "browser.exe", TCP_RST), 3010, &why) == FWV_ALLOW);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 50001, web, 443, "browser.exe", TCP_ACK), 3020, &why) == FWV_DROP);
    /* deny policy blocks and creates nothing */
    int before = f->nflow;
    fw_set_app_policy(f, "ads.exe", FWA_DENY);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 60000, web, 80, "ads.exe", TCP_SYN), 4000, &why) == FWV_DROP && why == FWR_APP_POLICY && f->nflow == before);
    fw_set_app_policy(f, "ads.exe", FWA_ASK);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 60000, web, 80, "ads.exe", TCP_SYN), 4001, &why) == FWV_ASK);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 60000, web, 80, "", TCP_SYN), 4002, &why) == FWV_ASK);                       /* unknown owner asks too */

    /* rules: weight order, deny beats allow on a tie, ports, cidr, app globs, direction, disabled, iface */
    fw_rule r; uint32_t id1 = 0, id2 = 0, id3 = 0;
    CHECK(fw_rule_parse(&r, "allow out udp to 1.1.1.1/32 port 53 app dnsc* weight 10 name dns") && (id1 = fw_add_rule(f, &r)) != 0);
    CHECK(ev(f, pk(FW_OUT, FW_UDP, me, 40000, dns, 53, "dnsclient.exe", 0), 5000, &why) == FWV_ALLOW && why == FWR_RULE);
    CHECK(ev(f, pk(FW_OUT, FW_UDP, me, 40010, dns, 54, "dnsclient.exe", 0), 5000, &why) == FWV_ASK);              /* port mismatch */
    CHECK(ev(f, pk(FW_OUT, FW_UDP, me, 40011, dns, 53, "nodns.exe", 0), 5000, &why) == FWV_ASK);                  /* app glob mismatch */
    CHECK(ev(f, pk(FW_IN, FW_UDP, me, 40000, dns, 53, "dnsclient.exe", 0), 5000, &why) == FWV_ALLOW && why == FWR_ESTABLISHED);   /* the reply */
    CHECK(fw_rule_parse(&r, "deny out udp to 1.1.1.0/24 port 50-60 weight 10") && (id2 = fw_add_rule(f, &r)) != 0);
    CHECK(ev(f, pk(FW_OUT, FW_UDP, me, 40001, dns, 53, "dnsclient.exe", 0), 5100, &why) == FWV_DROP && why == FWR_RULE);            /* tie: deny wins */
    CHECK(fw_rule_parse(&r, "allow out udp to 1.1.1.0/24 port 53 weight 20") && (id3 = fw_add_rule(f, &r)) != 0);
    CHECK(ev(f, pk(FW_OUT, FW_UDP, me, 40002, dns, 53, "dnsclient.exe", 0), 5200, &why) == FWV_ALLOW && why == FWR_RULE);           /* higher weight wins */
    CHECK(fw_set_rule_enabled(f, id3, false) && ev(f, pk(FW_OUT, FW_UDP, me, 40003, dns, 53, "dnsclient.exe", 0), 5300, &why) == FWV_DROP);
    CHECK(fw_remove_rule(f, id2) && fw_remove_rule(f, id3) && !fw_remove_rule(f, id3) && ev(f, pk(FW_OUT, FW_UDP, me, 40004, dns, 53, "dnsclient.exe", 0), 5400, &why) == FWV_ALLOW);
    CHECK(fw_remove_rule(f, id1));
    /* inbound service: a rule opens one port for one app; replies flow out */
    CHECK(fw_rule_parse(&r, "allow in tcp lport 8080,8443 from 192.168.1.0/24 app server.exe") && fw_add_rule(f, &r));
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 8080, V4(192, 168, 1, 50), 41000, "server.exe", TCP_SYN), 6000, &why) == FWV_ALLOW && why == FWR_RULE);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 8080, V4(192, 168, 1, 50), 41000, "server.exe", TCP_SYN | TCP_ACK), 6010, &why) == FWV_ALLOW && why == FWR_ESTABLISHED);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 8080, V4(8, 8, 8, 8), 41000, "server.exe", TCP_SYN), 6020, &why) == FWV_DROP);      /* outside the subnet */
    fw_set_app_policy(f, "server.exe", FWA_ALLOW);                                                                  /* an outbound-allow policy never opens inbound ports */
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 7777, V4(192, 168, 1, 50), 41009, "server.exe", TCP_SYN), 6021, &why) == FWV_DROP);
    fw_set_app_policy(f, "server.exe", FWA_DENY);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 8080, V4(192, 168, 1, 77), 41010, "server.exe", TCP_SYN), 6022, &why) == FWV_ALLOW && why == FWR_RULE);   /* rules outrank the app policy */
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 7778, V4(192, 168, 1, 50), 41011, "server.exe", TCP_SYN), 6023, &why) == FWV_DROP && why == FWR_APP_POLICY);
    fw_set_app_policy(f, "server.exe", FWA_ASK);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 9090, V4(192, 168, 1, 50), 41000, "server.exe", TCP_SYN), 6030, &why) == FWV_DROP);   /* wrong port */
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 8443, V4(192, 168, 1, 51), 41001, "other.exe", TCP_SYN), 6040, &why) == FWV_DROP);    /* wrong app */
    /* ICMP, interface and direction filters, rejects */
    CHECK(fw_rule_parse(&r, "allow in icmp local 192.168.1.10/32 weight 5") && fw_add_rule(f, &r));
    CHECK(ev(f, pk(FW_IN, FW_ICMP, me, 0, web, 0, "", 0), 7000, &why) == FWV_ALLOW && why == FWR_RULE);
    CHECK(fw_rule_parse(&r, "reject out tcp to 10.66.0.0/16 weight 30") && fw_add_rule(f, &r));
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 5555, V4(10, 66, 1, 1), 22, "browser.exe", TCP_SYN), 7100, &why) == FWV_REJECT && f->rejected == 1);
    fw_rule ifr; fw_rule_parse(&ifr, "deny out tcp to 203.0.113.0/24"); ifr.ifindex = 7; fw_add_rule(f, &ifr);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 5556, V4(203, 0, 113, 5), 80, "x", TCP_SYN), 7200, &why) == FWV_ASK);              /* different interface: rule skipped */
    fw_packet on7 = pk(FW_OUT, FW_TCP, me, 5557, V4(203, 0, 113, 5), 80, "x", TCP_SYN); on7.ifindex = 7;
    CHECK(fw_eval(f, &on7, 7300, &why, NULL) == FWV_DROP);

    /* IPv6 rules */
    CHECK(fw_rule_parse(&r, "allow out tcp to 2001:db8::/32 port 443 app curl.exe") && fw_add_rule(f, &r));
    CHECK(ev(f, pk(FW_OUT, FW_TCP, V6("2001:db8:ffff::10"), 40000, V6("2001:db8:1::1"), 443, "curl.exe", TCP_SYN), 8000, &why) == FWV_ALLOW && why == FWR_RULE);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, V6("2001:db8:ffff::10"), 40000, V6("2001:db9::1"), 443, "curl.exe", TCP_SYN), 8010, &why) == FWV_ASK);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 40000, web, 443, "curl.exe", TCP_SYN), 8020, &why) == FWV_ASK);                  /* a v6 rule never matches v4 */

    /* conntrack aging */
    fw_set_app_policy(f, "dnsclient.exe", FWA_ALLOW);
    ev(f, pk(FW_OUT, FW_UDP, me, 41000, dns, 53, "dnsclient.exe", 0), 10000, &why);
    CHECK(ev(f, pk(FW_IN, FW_UDP, me, 41000, dns, 53, "dnsclient.exe", 0), 25000, &why) == FWV_ALLOW && why == FWR_ESTABLISHED);   /* refreshed at 25 s */
    CHECK(ev(f, pk(FW_IN, FW_UDP, me, 41000, dns, 53, "dnsclient.exe", 0), 25000 + 31000, &why) == FWV_DROP);                 /* idle > 30 s: gone */
    int n0 = f->nflow; fw_gc(f, 10000000); CHECK(f->nflow == 0 && n0 > 0);
    ev(f, pk(FW_OUT, FW_TCP, me, 42000, web, 443, "browser.exe", TCP_SYN), 20000, &why);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 42000, web, 443, "browser.exe", TCP_ACK), 20000 + 59000, &why) == FWV_ALLOW);    /* half-open flows get a short timeout... */
    fw_gc(f, 20000 + 59000 + 61000); CHECK(f->nflow == 0);

    /* hard switch overrides everything (even established flows and explicit allows) but not loopback */
    ev(f, pk(FW_OUT, FW_TCP, me, 43000, web, 443, "browser.exe", TCP_SYN), 30000, &why);
    fw_set_hard_off(f, true);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 43000, web, 443, "browser.exe", TCP_SYN | TCP_ACK), 30010, &why) == FWV_DROP && why == FWR_HARD_OFF);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, me, 43001, web, 443, "browser.exe", TCP_SYN), 30020, &why) == FWV_DROP && why == FWR_HARD_OFF);
    CHECK(ev(f, pk(FW_OUT, FW_TCP, V4(127, 0, 0, 1), 1, V4(127, 0, 0, 1), 2, "x", TCP_SYN), 30030, &why) == FWV_ALLOW && why == FWR_LOOPBACK);
    fw_set_hard_off(f, false);
    CHECK(ev(f, pk(FW_IN, FW_TCP, me, 43000, web, 443, "browser.exe", TCP_ACK), 30040, &why) == FWV_ALLOW);               /* flows survive the switch */

    /* conntrack table pressure: thousands of flows, the table stays bounded and keeps working */
    fw_set_app_policy(f, "bulk.exe", FWA_ALLOW);
    for (int i = 0; i < 5000; i++) ev(f, pk(FW_OUT, FW_UDP, me, (uint16_t)(1000 + i % 60000), V4(8, 8, (uint8_t)(i >> 8), (uint8_t)i), 9, "bulk.exe", 0), 40000 + (uint64_t)i, &why);
    CHECK(f->nflow <= FW_CT_SIZE && f->nflow > 400);
    CHECK(ev(f, pk(FW_IN, FW_UDP, me, (uint16_t)(1000 + 4999 % 60000), V4(8, 8, (uint8_t)(4999 >> 8), (uint8_t)4999), 9, "bulk.exe", 0), 45000, &why) == FWV_ALLOW);   /* newest flow is there */
    CHECK(ev(f, pk(FW_IN, FW_UDP, me, 1000, V4(8, 8, 0, 0), 9, "bulk.exe", 0), 45001, &why) == FWV_DROP);                   /* oldest was recycled */

    /* log + counters */
    fw_log_entry log[8]; int nl = fw_log_read(f, log, 8);
    CHECK(nl == 8 && log[0].t >= log[1].t && f->allowed > 100 && f->dropped > 5 && f->asked > 5);
    fw_engine *g = calloc(1, sizeof *g); fw_init(g);
    ev(g, pk(FW_IN, FW_TCP, me, 80, web, 1, "srv", TCP_SYN), 1, &why); ev(g, pk(FW_OUT, FW_TCP, me, 1, web, 80, "cli", TCP_SYN), 2, &why);
    nl = fw_log_read(g, log, 8); CHECK(nl == 2 && log[0].verdict == FWV_ASK && !strcmp(log[0].app, "cli") && log[1].verdict == FWV_DROP && g->dropped == 1 && g->asked == 1);
    for (int i = 0; i < 300; i++) ev(g, pk(FW_IN, FW_TCP, me, 80, web, 1, "srv", TCP_SYN), (uint64_t)i, &why);
    { static fw_log_entry big[FW_LOG_SIZE + 10]; CHECK(fw_log_read(g, big, FW_LOG_SIZE + 10) == FW_LOG_SIZE); }

    /* rule text round trip + validation */
    const char *good[] = { "allow out tcp to 10.0.0.0/8 port 80,443 app browser.exe weight 50 log", "deny in udp lport 5353 from 192.168.0.0/16", "reject out icmp", "allow in tcp lport 1000-2000,3000 app *server* name web disabled", "allow in tcp from 10.0.0.0/8 lport 22", "deny out udp to 8.8.8.8/32 port 53", "allow in udp local 192.168.1.10/32 remote 1.2.3.0/24", "allow out tcp to ::/0 port 443", "deny" };
    for (unsigned i = 0; i < sizeof good / sizeof *good; i++) {
        fw_rule a, b; char out[256];
        CHECK(fw_rule_parse(&a, good[i]));
        fw_rule_format(&a, out, sizeof out);
        CHECK(fw_rule_parse(&b, out));
        char out2[256]; fw_rule_format(&b, out2, sizeof out2); CHECK(!strcmp(out, out2));
    }
    CHECK(fw_rule_parse(&r, "allow out tcp to 10.0.0.0/8 port 80,443 app browser.exe weight 50 log") && r.log && r.weight == 50 && r.nrports == 2 && r.dir == FW_OUT && !strcmp(r.app, "browser.exe"));
    const char *bad[] = { "", "permit out tcp", "allow out tcp to 10.0.0/8", "allow out tcp port 99999", "allow out tcp port 80-70", "allow out port 80", "allow out icmp port 80", "allow out tcp port 1,2,3,4,5", "allow out tcp weight x", "allow out tcp weight 5000", "allow out tcp frobnicate", "allow out tcp to", "allow out tcp app", "allow tcp from 10.0.0.0/8", "allow out tcp local" };
    for (unsigned i = 0; i < sizeof bad / sizeof *bad; i++) CHECK(!fw_rule_parse(&r, bad[i]));
    fw_rule ask = { 0 }; ask.action = FWV_ASK; ask.enabled = true; CHECK(fw_add_rule(f, &ask) == 0);         /* ASK is not a rule action */

    /* glob */
    CHECK(fw_glob("*.exe", "Browser.EXE") && fw_glob("a?c", "abc") && !fw_glob("a?c", "ac") && fw_glob("*", "") && fw_glob("*x*y*", "aaxbbyc") && !fw_glob("*x*y*", "ybx") && !fw_glob("abc", "abcd") && fw_glob("", ""));
    free(f); free(g);
    DONE("fw_test");
}
