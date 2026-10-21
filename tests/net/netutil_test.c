#include "netutil.h"
#include "test.h"
#include <string.h>

static const char *fmt6(const char *in) { static char b[48]; ip6_addr a; if (!ip6_parse(in, &a)) return "(bad)"; ip6_format(&a, b, sizeof b); return b; }

int main(void)
{
    ip4_addr a; char b[32];
    CHECK(ip4_parse("192.168.1.10", &a) && a.b[0] == 192 && a.b[3] == 10 && ip4_format(a, b, sizeof b) == 12 && !strcmp(b, "192.168.1.10"));
    const char *bad4[] = { "", "1.2.3", "1.2.3.4.5", "256.1.1.1", "1.2.3.04", "1.2.3.4 ", "a.b.c.d", "1..2.3", "-1.2.3.4", "1.2.3.", "0x7f.0.0.1", "1.2.3.4/8" };
    for (unsigned i = 0; i < sizeof bad4 / sizeof *bad4; i++) CHECK(!ip4_parse(bad4[i], &a));
    CHECK(ip4_parse("0.0.0.0", &a) && ip4_parse("255.255.255.255", &a) && ip4_parse("10.0.0.0", &a));

    /* IPv6: canonical output (RFC 5952) */
    CHECK(!strcmp(fmt6("::"), "::") && !strcmp(fmt6("::1"), "::1") && !strcmp(fmt6("2001:db8::1"), "2001:db8::1"));
    CHECK(!strcmp(fmt6("2001:0db8:0000:0000:0000:0000:0000:0001"), "2001:db8::1") && !strcmp(fmt6("2001:db8:0:0:1:0:0:1"), "2001:db8::1:0:0:1"));
    CHECK(!strcmp(fmt6("1:2:3:4:5:6:7:8"), "1:2:3:4:5:6:7:8") && !strcmp(fmt6("1:0:3:4:5:6:7:8"), "1:0:3:4:5:6:7:8"));       /* a single zero word is not compressed */
    CHECK(!strcmp(fmt6("fe80::"), "fe80::") && !strcmp(fmt6("::ffff:192.0.2.128"), "::ffff:c000:280") && !strcmp(fmt6("FF02::1:FF00:1"), "ff02::1:ff00:1"));
    const char *bad6[] = { "", ":", ":::", "1::2::3", "12345::", "1:2:3:4:5:6:7", "1:2:3:4:5:6:7:8:9", "g::1", "1:2:3:4:5:6:7::8", "::1.2.3", ":1::2", "1:2:3:4:5:6:1.2.3.4:5" };
    ip6_addr a6;
    for (unsigned i = 0; i < sizeof bad6 / sizeof *bad6; i++) CHECK(!ip6_parse(bad6[i], &a6));
    CHECK(ip6_parse("1:2:3:4:5:6:7::", &a6) && ip6_parse("::2:3:4:5:6:7:8", &a6));

    /* CIDR */
    ip4_cidr c;
    CHECK(ip4_cidr_parse("10.1.2.3/8", &c) && c.prefix == 8 && ip4_in_cidr((ip4_addr){ { 10, 255, 0, 1 } }, c) && !ip4_in_cidr((ip4_addr){ { 11, 0, 0, 1 } }, c));
    CHECK(ip4_cidr_parse("1.2.3.4", &c) && c.prefix == 32 && !ip4_cidr_parse("1.2.3.4/33", &c) && !ip4_cidr_parse("1.2.3.4/", &c) && !ip4_cidr_parse("1.2.3.4/x", &c));
    CHECK(ip4_cidr_parse("0.0.0.0/0", &c) && ip4_in_cidr((ip4_addr){ { 8, 8, 8, 8 } }, c));
    CHECK(ip4_mask(0) == 0 && ip4_mask(24) == 0xffffff00u && ip4_mask(32) == 0xffffffffu && ip4_mask(1) == 0x80000000u);
    CHECK(ip4_prefix_from_mask((ip4_addr){ { 255, 255, 255, 0 } }) == 24 && ip4_prefix_from_mask((ip4_addr){ { 255, 255, 255, 255 } }) == 32 && ip4_prefix_from_mask((ip4_addr){ { 0, 0, 0, 0 } }) == 0 && ip4_prefix_from_mask((ip4_addr){ { 255, 0, 255, 0 } }) == -1);
    ip6_cidr c6;
    CHECK(ip6_cidr_parse("2001:db8::/32", &c6) && c6.prefix == 32);
    ip6_addr in6, out6; ip6_parse("2001:db8:1::5", &in6); ip6_parse("2001:db9::1", &out6);
    CHECK(ip6_in_cidr(&in6, c6) && !ip6_in_cidr(&out6, c6));
    ip6_cidr_parse("fe80::/10", &c6); ip6_parse("febf::1", &in6); ip6_parse("fec0::1", &out6);
    CHECK(ip6_in_cidr(&in6, c6) && !ip6_in_cidr(&out6, c6) && !ip6_cidr_parse("::/129", &c6));

    /* classes */
    CHECK(ip4_is_private((ip4_addr){ { 10, 1, 1, 1 } }) && ip4_is_private((ip4_addr){ { 172, 16, 0, 1 } }) && ip4_is_private((ip4_addr){ { 172, 31, 255, 1 } }) && !ip4_is_private((ip4_addr){ { 172, 32, 0, 1 } }) && ip4_is_private((ip4_addr){ { 192, 168, 0, 1 } }) && !ip4_is_private((ip4_addr){ { 8, 8, 8, 8 } }));
    CHECK(ip4_is_loopback((ip4_addr){ { 127, 1, 2, 3 } }) && ip4_is_link_local((ip4_addr){ { 169, 254, 9, 9 } }) && ip4_is_multicast((ip4_addr){ { 224, 0, 0, 251 } }) && !ip4_is_multicast((ip4_addr){ { 240, 0, 0, 1 } }));

    /* RFC 1071 example: 00 01 f2 03 f4 f5 f6 f7 -> checksum 0x220d */
    const uint8_t ex[] = { 0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7 };
    CHECK(net_checksum(ex, sizeof ex, 0) == 0x220d);
    /* a packet with its checksum inserted verifies to 0; odd length pads with a zero byte */
    uint8_t ip[20] = { 0x45, 0, 0, 0x73, 0, 0, 0x40, 0, 0x40, 0x11, 0, 0, 0xc0, 0xa8, 0, 1, 0xc0, 0xa8, 0, 0xc7 };
    uint16_t sum = net_checksum(ip, 20, 0); CHECK(sum == 0xb861);
    net_wr16(ip + 10, sum); CHECK(net_checksum(ip, 20, 0) == 0);
    CHECK(net_checksum("\x01", 1, 0) == (uint16_t)~0x0100u);
    uint32_t acc = net_checksum_add(ex, 4, 0); acc = net_checksum_add(ex + 4, 4, acc); CHECK(net_checksum_finish(acc) == 0x220d);
    uint8_t w[4]; net_wr32(w, 0x01020304u); CHECK(net_rd32(w) == 0x01020304u && net_rd16(w + 1) == 0x0203 && ip4_u32(ip4_from_u32(0xc0a80001u)) == 0xc0a80001u);
    DONE("netutil_test");
}
