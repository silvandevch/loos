#include "ui.h"
#include <stdio.h>

void fmt_size(uint64_t b, char *out, size_t cap)
{
    static const char *const unit[] = { "B", "KB", "MB", "GB", "TB" };
    int u = 0;
    uint64_t whole = b, rem = 0;
    while (whole >= 1024 && u < 4) { rem = whole % 1024; whole /= 1024; u++; }
    if (u == 0) snprintf(out, cap, "%llu B", (unsigned long long)b);
    else snprintf(out, cap, "%llu.%llu %s", (unsigned long long)whole, (unsigned long long)(rem * 10 / 1024), unit[u]);
}

void fmt_time(int64_t t, char *out, size_t cap)
{
    int64_t days = t / 86400, rem = t % 86400;
    if (rem < 0) { rem += 86400; days--; }
    int64_t z = days + 719468, era = (z >= 0 ? z : z - 146096) / 146097;
    unsigned doe = (unsigned)(z - era * 146097), yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int64_t y = (int64_t)yoe + era * 400;
    unsigned doy = doe - (365 * yoe + yoe / 4 - yoe / 100), mp = (5 * doy + 2) / 153;
    unsigned d = doy - (153 * mp + 2) / 5 + 1, m = mp < 10 ? mp + 3 : mp - 9;
    y += m <= 2;
    snprintf(out, cap, "%04lld-%02u-%02u %02d:%02d", (long long)y, m, d, (int)(rem / 3600), (int)(rem % 3600 / 60));
}

void fmt_pct10(uint32_t x10, char *out, size_t cap) { snprintf(out, cap, "%u.%u%%", x10 / 10, x10 % 10); }
