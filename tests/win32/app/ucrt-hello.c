/*
 * A normal PE32+ console program whose C runtime imports come from the LoOS
 * ucrtbase image.  It intentionally uses no host or MinGW startup code: the
 * entry point and imports are still ordinary Win32 PE contracts.
 */
typedef unsigned long long size_t;

__declspec(dllimport) int printf(const char *, ...);
__declspec(dllimport) size_t strlen(const char *);
__declspec(dllimport) int _stricmp(const char *, const char *);
__declspec(dllimport) long strtol(const char *, char **, int);

int app_main(void)
{
    const char *word = "ucrtbase";
    char *end;
    long value = strtol("2a", &end, 16);
    int same = _stricmp(word, "UCRTBASE") == 0;

    printf("ucrt-hello: %s (%u bytes), parse=0x%x, compare=%s\r\n",
           word, (unsigned)strlen(word), (unsigned)value, same ? "ok" : "bad");
    return (same && value == 42 && *end == 0) ? 13 : 1;
}
