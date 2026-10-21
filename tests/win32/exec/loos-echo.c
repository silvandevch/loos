/* loos-echo — a real, freestanding Win32 console executable. The LoOS PE
 * loader maps it, binds its single import (host_write from losshost.dll),
 * relocates it, and calls its entry point (pe_main). It does a little real
 * work, prints with its imported call, and returns a distinctive exit code. */
__declspec(dllimport) long host_write(int fd, const void *buf, long len);

static long slen(const char *s) { long n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { host_write(1, s, slen(s)); }

static void put_u64(long v, char *out)
{
    char tmp[24];
    int n = 0, a, b;
    if (v == 0) tmp[n++] = '0';
    while (v > 0) { tmp[n++] = (char)('0' + (v % 10)); v /= 10; }
    for (a = 0, b = n - 1; a < b; a++, b--) { char t = tmp[a]; tmp[a] = tmp[b]; tmp[b] = t; }
    for (a = 0; a < n; a++) out[a] = tmp[a];
    out[n] = 0;
}

int pe_main(void)
{
    char num[24];
    long sum = 0, i;

    say("loos-echo: hello from a real PE32+ image running inside the LoOS loader.\r\n");
    for (i = 1; i <= 100; i++) sum += i;
    say("loos-echo: sum(1..100) = ");
    put_u64(sum, num);
    say(num);
    say("\r\n");
    say("loos-echo: this text came through an imported call bound by the loader.\r\n");
    return 42;
}
