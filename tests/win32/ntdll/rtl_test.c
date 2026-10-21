#define LOOS_HOST_TEST 1
#include "../kernel32/fake.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_EQ(a, b) do { unsigned long long a_ = (unsigned long long)(a), b_ = (unsigned long long)(b); \
    if (a_ != b_) { printf("FAIL %s:%d: %s = %llu, want %llu\n", __FILE__, __LINE__, #a, a_, b_); fails++; } } while (0)

static const WCHAR *W(const char *s) { static WCHAR b[4][256]; static int i; WCHAR *o = b[i++ & 3]; fake_wset(o, s); return o; }

static void test_strings(void)
{
    UNICODE_STRING u, v; ANSI_STRING a, b;
    RtlInitUnicodeString(&u, W("Hello")); CHECK(u.Length == 10 && u.MaximumLength == 12);
    RtlInitUnicodeString(&u, NULL); CHECK(u.Length == 0 && u.MaximumLength == 0 && u.Buffer == NULL);
    RtlInitAnsiString(&a, "abc"); CHECK(a.Length == 3 && a.MaximumLength == 4);
    RtlInitUnicodeString(&u, W("MiXed")); RtlInitUnicodeString(&v, W("mixed"));
    CHECK(!RtlEqualUnicodeString(&u, &v, FALSE) && RtlEqualUnicodeString(&u, &v, TRUE));
    CHECK(RtlCompareUnicodeString(&u, &v, FALSE) < 0 && RtlCompareUnicodeString(&u, &v, TRUE) == 0);
    RtlInitUnicodeString(&v, W("mixed!")); CHECK(RtlCompareUnicodeString(&u, &v, TRUE) < 0 && !RtlEqualUnicodeString(&u, &v, TRUE));
    CHECK(RtlUpcaseUnicodeChar(0xE9) == 0xC9 && RtlDowncaseUnicodeChar(0x41F) == 0x43F && RtlUpcaseUnicodeChar('z') == 'Z');
    RtlInitAnsiString(&a, "caf\xe9 \x80");
    CHECK(NT_SUCCESS(RtlAnsiStringToUnicodeString(&u, &a, TRUE)) && u.Length == 12 && u.Buffer[3] == 0xE9 && u.Buffer[5] == 0x20AC && u.Buffer[6] == 0 && u.MaximumLength == 14);
    CHECK(NT_SUCCESS(RtlUnicodeStringToAnsiString(&b, &u, TRUE)) && b.Length == 6 && !memcmp(b.Buffer, "caf\xe9 \x80", 7));
    RtlFreeAnsiString(&b); CHECK(b.Buffer == NULL);
    { WCHAR cjk[2] = { 0x4E2D, 0 }; UNICODE_STRING c; c.Buffer = cjk; c.Length = 2; c.MaximumLength = 4;
      CHECK(NT_SUCCESS(RtlUnicodeStringToAnsiString(&b, &c, TRUE)) && b.Buffer[0] == '?'); RtlFreeAnsiString(&b); }
    { WCHAR fixed[3]; UNICODE_STRING f; f.Buffer = fixed; f.Length = 0; f.MaximumLength = 6;           /* caller buffer too small */
      CHECK(RtlAnsiStringToUnicodeString(&f, &a, FALSE) == STATUS_BUFFER_OVERFLOW); }
    RtlFreeUnicodeString(&u); CHECK(u.Buffer == NULL && u.Length == 0);
}

static void test_bitmaps(void)
{
    RTL_BITMAP bm; ULONG buf[4]; ULONG r;
    RtlInitializeBitMap(&bm, buf, 100);
    RtlClearAllBits(&bm);
    CHECK_EQ(RtlNumberOfSetBits(&bm), 0); CHECK_EQ(RtlNumberOfClearBits(&bm), 100);
    RtlSetBits(&bm, 3, 5);
    CHECK(RtlTestBit(&bm, 3) && RtlTestBit(&bm, 7) && !RtlTestBit(&bm, 8) && !RtlTestBit(&bm, 2) && !RtlTestBit(&bm, 100));
    CHECK(RtlAreBitsSet(&bm, 3, 5) && !RtlAreBitsSet(&bm, 3, 6) && RtlAreBitsClear(&bm, 8, 92) && !RtlAreBitsClear(&bm, 0, 4) && !RtlAreBitsClear(&bm, 90, 20));
    CHECK_EQ(RtlFindClearBits(&bm, 3, 0), 0);                                       /* bits 0..2 */
    CHECK_EQ(RtlFindClearBits(&bm, 4, 0), 8);                                       /* first run of 4 starts after the set block */
    CHECK_EQ(RtlFindClearBits(&bm, 4, 20), 20);                                     /* honours the hint */
    CHECK_EQ(RtlFindClearBits(&bm, 95, 0), 0xFFFFFFFFu);                            /* no such run */
    CHECK_EQ(RtlFindSetBits(&bm, 5, 0), 3); CHECK_EQ(RtlFindSetBits(&bm, 6, 0), 0xFFFFFFFFu);
    r = RtlFindClearBitsAndSet(&bm, 10, 0); CHECK(r == 8 && RtlAreBitsSet(&bm, 8, 10));
    r = RtlFindSetBitsAndClear(&bm, 15, 0); CHECK(r == 3 && RtlAreBitsClear(&bm, 3, 15));
    RtlSetBits(&bm, 95, 5); CHECK_EQ(RtlNumberOfSetBits(&bm), 5);
    RtlSetBits(&bm, 99, 5);                                                          /* runs past the end: ignored */
    CHECK_EQ(RtlNumberOfSetBits(&bm), 5);
    CHECK_EQ(RtlFindClearBits(&bm, 1, 98), 0);                                       /* hint near the end wraps around */
    RtlSetAllBits(&bm); CHECK(RtlNumberOfSetBits(&bm) >= 100);
    CHECK_EQ(RtlFindClearBits(&bm, 1, 0), 0xFFFFFFFFu);
}

static void test_image(void)
{
    static uint8_t img[0x400];
    uint8_t *nt; ULONG size = 0;
    memset(img, 0, sizeof img);
    img[0] = 'M'; img[1] = 'Z'; img[0x3c] = 0x80;
    nt = img + 0x80; memcpy(nt, "PE\0\0", 4);
    nt[24] = 0x0b; nt[25] = 0x02; nt[24 + 108] = 16;                                 /* PE32+ optional header, 16 data directories */
    { uint8_t *e = nt + 24 + 112 + 1 * 8; e[0] = 0x00; e[1] = 0x02; e[4] = 0x20; }   /* import directory: rva 0x200, size 0x20 */
    CHECK(RtlImageNtHeader(img) == nt);
    CHECK(RtlImageDirectoryEntryToData(img, TRUE, 1, &size) == img + 0x200 && size == 0x20);
    CHECK(RtlImageDirectoryEntryToData(img, TRUE, 2, &size) == NULL);                // empty entry
    CHECK(RtlImageDirectoryEntryToData(img, TRUE, 16, &size) == NULL);
    img[0] = 'X'; CHECK(RtlImageNtHeader(img) == NULL); img[0] = 'M';
    img[0x3c] = 0xff; img[0x3d] = 0xff; img[0x3e] = 0xff; img[0x3f] = 0x7f; CHECK(RtlImageNtHeader(img) == NULL);
    CHECK(RtlImageNtHeader(NULL) == NULL);
}

static void test_time_and_memory(void)
{
    LARGE_INTEGER t; TIME_FIELDS f; ULONG s;
    t.QuadPart = 125911584000000000ll;                                               /* 2000-01-01 */
    CHECK(RtlTimeToSecondsSince1970(&t, &s) && s == 946684800u);
    RtlSecondsSince1970ToTime(946684800u, &t); CHECK(t.QuadPart == 125911584000000000ll);
    RtlTimeToTimeFields(&t, &f); CHECK(f.Year == 2000 && f.Month == 1 && f.Day == 1 && f.Hour == 0 && f.Weekday == 6);
    t.QuadPart = 133536836967890000ll; RtlTimeToTimeFields(&t, &f);
    CHECK(f.Year == 2024 && f.Month == 2 && f.Day == 29 && f.Hour == 12 && f.Minute == 34 && f.Second == 56 && f.Milliseconds == 789 && f.Weekday == 4);
    { LARGE_INTEGER back; CHECK(RtlTimeFieldsToTime(&f, &back) && back.QuadPart == 133536836967890000ll); }
    f.Day = 30; { LARGE_INTEGER b; CHECK(!RtlTimeFieldsToTime(&f, &b)); }            /* Feb 30 */
    t.QuadPart = 0; CHECK(!RtlTimeToSecondsSince1970(&t, &s));                       /* before 1970 */
    t.QuadPart = (0x100000000ll + 11644473600ll) * 10000000ll; CHECK(!RtlTimeToSecondsSince1970(&t, &s));    /* beyond 2106 */
    { char a[9] = "abcdefgh", b[9] = "abcdXfgh";
      CHECK_EQ(RtlCompareMemory(a, b, 8), 4); CHECK_EQ(RtlCompareMemory(a, a, 8), 8); CHECK_EQ(RtlCompareMemory(a, b, 0), 0);
      RtlFillMemory(a, 3, 'z'); CHECK(!memcmp(a, "zzzdefgh", 8)); RtlZeroMemory(a + 2, 2); CHECK(a[2] == 0 && a[3] == 0 && a[4] == 'e');
      RtlMoveMemory(a + 1, a, 4); CHECK(a[1] == 'z' && a[2] == 'z' && a[3] == 0); RtlCopyMemory(b, "12345678", 8); CHECK(!memcmp(b, "12345678", 8)); }
}

/* ---------- delay-load resolver on a hand-built mapped image ---------- */
typedef struct { ULONG Attributes, DllNameRVA, ModuleHandleRVA, ImportAddressTableRVA, ImportNameTableRVA, Bound, Unload, Time; } DESC;
static void test_delayload(void)
{
    static uint8_t img[0x1000] __attribute__((aligned(16)));
    DESC *d = (DESC *)(img + 0x400);
    PVOID *iat = (PVOID *)(img + 0x240), *slot = (PVOID *)(img + 0x280);
    uint64_t *nt = (uint64_t *)(img + 0x200);
    PVOID r;
    fake_reset();
    memset(img, 0, sizeof img);
    strcpy((char *)img + 0x100, "known.dll");
    *(uint16_t *)(img + 0x300) = 7; strcpy((char *)img + 0x302, "Func");
    nt[0] = 0x300; nt[1] = 0x8000000000000001ull; nt[2] = 0x300;                      /* by name, by ordinal 1, by name again */
    d->Attributes = 1; d->DllNameRVA = 0x100; d->ModuleHandleRVA = 0x280; d->ImportAddressTableRVA = 0x240; d->ImportNameTableRVA = 0x200;

    r = LdrResolveDelayLoadedAPI(img, d, NULL, NULL, &iat[0], 0);
    CHECK(r == (PVOID)0x7ff700001000ull && iat[0] == r);                                  /* resolved and stored */
    CHECK(*slot == (PVOID)0x7ff700000000ull); CHECK_EQ(F.module_refs, 1);                  /* module handle cached */
    r = LdrResolveDelayLoadedAPI(img, d, NULL, NULL, &iat[1], 0);
    CHECK(r == (PVOID)0x7ff700001000ull && iat[1] == r); CHECK_EQ(F.module_refs, 1);       /* ordinal import; DLL not loaded again */
    r = LdrResolveDelayLoadedAPI(img, d, NULL, NULL, &iat[2], 0); CHECK(iat[2] == r);

    /* failures end the process with the Windows delay-load statuses */
    if (!setjmp(fake_exit_jmp)) { strcpy((char *)img + 0x100, "nothere.dll"); *slot = NULL; LdrResolveDelayLoadedAPI(img, d, NULL, NULL, &iat[0], 0); CHECK(0); }
    CHECK_EQ(F.exit_code, 0xC06D007Eu);
    if (!setjmp(fake_exit_jmp)) { strcpy((char *)img + 0x100, "known.dll"); *slot = NULL; strcpy((char *)img + 0x302, "Missing"); LdrResolveDelayLoadedAPI(img, d, NULL, NULL, &iat[0], 0); CHECK(0); }
    CHECK_EQ(F.exit_code, 0xC06D007Fu);
    d->Attributes = 0;                                                                      /* VA-based descriptors are not accepted */
    if (!setjmp(fake_exit_jmp)) { LdrResolveDelayLoadedAPI(img, d, NULL, NULL, &iat[0], 0); CHECK(0); }
    CHECK_EQ(F.exit_code, 0xC06D007Eu);
    /* kernelbase's wrappers */
    d->Attributes = 1; strcpy((char *)img + 0x302, "Func"); *slot = NULL;
    CHECK(ResolveDelayLoadedAPI(img, d, NULL, NULL, &iat[0], 0) == (PVOID)0x7ff700001000ull);
    CHECK(DelayLoadFailureHook("x.dll", "y") == NULL);
}

static void test_format_message(void)
{
    WCHAR buf[256]; char ab[256]; DWORD n; PVOID args[3]; WCHAR *alloc = NULL;
    n = FormatMessageW(FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS, NULL, 2, 0, buf, 256, NULL);
    CHECK_EQ(n, 44); CHECK(buf[0] == 'T' && buf[n - 2] == '\r' && buf[n - 1] == '\n' && buf[n] == 0);
    n = FormatMessageW(FORMAT_MESSAGE_FROM_SYSTEM | 0xFF, NULL, 5, 0, buf, 256, NULL);
    CHECK_EQ(n, 17);                                      /* width mask: no trailing CR LF */
    args[0] = (PVOID)W("world"); args[1] = (PVOID)(DWORD_PTR)42; args[2] = (PVOID)(DWORD_PTR)0xBEEF;
    n = FormatMessageW(FORMAT_MESSAGE_FROM_STRING | FORMAT_MESSAGE_ARGUMENT_ARRAY, W("hi %1 %2!05d! %3!#x!%%%n"), 0, 0, buf, 256, args);
    CHECK(n > 0); { char o[64]; unsigned i; for (i = 0; i <= n; i++) o[i] = (char)buf[i]; CHECK(!strcmp(o, "hi world 00042 0xbeef%\r\n") || !strcmp(o, "hi world 00042 beef%\r\n")); }
    n = FormatMessageW(FORMAT_MESSAGE_FROM_STRING | FORMAT_MESSAGE_ARGUMENT_ARRAY | FORMAT_MESSAGE_ALLOCATE_BUFFER, W("x%1!-4s!|"), 0, 0, (LPWSTR)&alloc, 0, args);
    CHECK(n == 7 && alloc && alloc[1] == 'w' && alloc[6] == '|'); RtlFreeHeap(RtlGetProcessHeap(), 0, alloc);
    n = FormatMessageA(FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS, NULL, 5, 0, ab, 256, NULL);
    CHECK(n == 17 + 2 && !strncmp(ab, "Access is denied.", 17));
    CHECK(FormatMessageW(FORMAT_MESSAGE_FROM_SYSTEM, NULL, 0xDEAD, 0, buf, 256, NULL) == 0 && GetLastError() == ERROR_MR_MID_NOT_FOUND);
    CHECK(FormatMessageW(FORMAT_MESSAGE_FROM_SYSTEM, NULL, 2, 0, buf, 4, NULL) == 0 && GetLastError() == ERROR_INSUFFICIENT_BUFFER);
}

int main(void)
{
    fake_reset();
    test_strings(); test_bitmaps(); test_image(); test_time_and_memory(); test_delayload(); test_format_message();
    fake_reset();
    if (fails) printf("rtl: %d FAILED\n", fails); else printf("rtl: all passed\n");
    return fails != 0;
}
