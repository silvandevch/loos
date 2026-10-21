#include "kernel.h"

void serial_putc(char c)
{
    while (!(inb(0x3fd) & 0x20)) {}
    outb(0x3f8, (uint8_t)c);
}

void serial_puts(const char *s) { while (*s) serial_putc(*s++); }

void serial_hex(uint64_t v)
{
    serial_puts("0x");
    for (int i = 60; i >= 0; i -= 4) serial_putc("0123456789abcdef"[(v >> i) & 15]);
}

void serial_dec(uint64_t v)
{
    char b[21]; int n = 0;
    do { b[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    while (n) serial_putc(b[--n]);
}
