/* Console self-test: Winsock over the LoOS TCP/IP stack. HTTP GET to 10.0.2.2:8099 (the QEMU host), then DNS. */
#include <winsock2.h>
#include <ws2tcpip.h>
static HANDLE out;
static void say(const char *s) { DWORD n; int l = 0; while (s[l]) l++; WriteFile(out, s, l, &n, 0); }
static void num(int v) { char b[16]; int i = 15; b[i] = 0; if (!v) b[--i] = '0'; while (v) { b[--i] = '0' + v % 10; v /= 10; } say(b + i); }
void start(void)
{
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    int fail = 0; WSADATA wd;
    say("net: WSAStartup "); num(WSAStartup(0x0202, &wd)); say("\n");
    SOCKET s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    struct sockaddr_in a = {0}; a.sin_family = AF_INET; a.sin_port = htons(8099); a.sin_addr.s_addr = inet_addr("10.0.2.2");
    int r = connect(s, (struct sockaddr *)&a, sizeof a);
    say("net: connect 10.0.2.2:8099 -> "); num(r); say("\n"); if (r) fail++;
    const char *req = "GET /hello.txt HTTP/1.0\r\nHost: loos\r\n\r\n";
    int sent = send(s, req, 41, 0); say("net: sent "); num(sent); say(" bytes\n"); if (sent != 41) fail++;
    char buf[1024]; int total = 0, n;
    while ((n = recv(s, buf + total, sizeof buf - 1 - total, 0)) > 0) total += n;
    buf[total] = 0;
    say("net: received "); num(total); say(" bytes\n");
    char *body = buf; for (int i = 0; i + 3 < total; i++) if (buf[i] == '\r' && buf[i+1] == '\n' && buf[i+2] == '\r' && buf[i+3] == '\n') { body = buf + i + 4; break; }
    say("net: status line "); for (int i = 0; i < total && buf[i] != '\r'; i++) { char c[2] = { buf[i], 0 }; say(c); } say("\n");
    say("net: body="); say(body); say("\n");
    if (total < 20 || body[0] != 'L') fail++;
    closesocket(s);
    struct hostent *h = gethostbyname("example.com");
    say("net: DNS example.com -> "); if (h) { unsigned char *ip = (unsigned char *)h->h_addr_list[0]; num(ip[0]); say("."); num(ip[1]); say("."); num(ip[2]); say("."); num(ip[3]); } else say("unresolved (host offline?)"); say("\n");
    say(fail ? "net: FAILED\n" : "net: PASS\n");
    ExitProcess(fail);
}
