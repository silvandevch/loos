#include "../../../src/win32/apisets/apiset.h"
#include <stdio.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

static void expect(const char *in, apiset_status st, const char *host)
{
    char out[64] = "";
    apiset_status r = apiset_resolve(in, out, sizeof out);
    if (r != st || (host && strcmp(out, host))) {
        printf("FAIL resolve(\"%s\") = %d \"%s\", want %d \"%s\"\n", in, r, out, st, host ? host : "");
        fails++;
    }
}

int main(void)
{
    size_t i;
    const char *prev = NULL;

    expect("api-ms-win-core-file-l1-2-1.dll", APISET_OK, "kernelbase.dll");
    expect("API-MS-WIN-CORE-FILE-L1-2-1.DLL", APISET_OK, "kernelbase.dll");
    expect("api-ms-win-core-file-l1-2-0", APISET_OK, "kernelbase.dll");     /* no .dll */
    expect("api-ms-win-core-file-l1-2-99.dll", APISET_OK, "kernelbase.dll");/* revision ignored */
    expect("api-ms-win-crt-stdio-l1-1-0.dll", APISET_OK, "ucrtbase.dll");
    expect("api-ms-win-core-com-l1-1-3.dll", APISET_OK, "combase.dll");
    expect("ext-ms-win-ntuser-window-l1-1-4.dll", APISET_OK, "user32.dll");
    expect("api-ms-win-core-file-l9-9-0.dll", APISET_UNKNOWN, NULL);        /* unknown major */
    expect("api-ms-win-nonexistent-l1-1-0.dll", APISET_UNKNOWN, NULL);
    expect("kernel32.dll", APISET_NOT_AN_APISET, NULL);
    expect("", APISET_BAD_NAME, NULL);
    expect("api-ms-win-core-file-l1-2.dll", APISET_UNKNOWN, NULL);          /* stripped to "...l1" */
    expect("api-ms-win-core-file", APISET_BAD_NAME, NULL);                  /* no revision */
    expect("api-ms-", APISET_BAD_NAME, NULL);

    { char tiny[4]; CHECK(apiset_resolve("api-ms-win-core-file-l1-2-1.dll", tiny, sizeof tiny) == APISET_BUFFER_TOO_SMALL); }
    { char big[300]; memset(big, 'a', 299); big[299] = 0; memcpy(big, "api-ms-", 7);
      char out[8]; CHECK(apiset_resolve(big, out, sizeof out) == APISET_BAD_NAME); }

    /* schema invariants: strictly sorted (binary search relies on it), lowercase, no revision */
    CHECK(apiset_count() > 100);
    for (i = 0; i < apiset_count(); i++) {
        const char *host, *c = apiset_contract_at(i, &host);
        const char *p;
        CHECK(c && host && strstr(host, ".dll"));
        CHECK(apiset_is_apiset(c));
        if (prev) CHECK(strcmp(prev, c) < 0);
        for (p = c; *p; p++) CHECK(!(*p >= 'A' && *p <= 'Z'));
        /* every table entry must resolve to itself when given a full 3-part name */
        { char full[160], out[64]; snprintf(full, sizeof full, "%s-0.dll", c);
          CHECK(apiset_resolve(full, out, sizeof out) == APISET_OK && !strcmp(out, host)); }
        prev = c;
    }
    CHECK(apiset_contract_at(apiset_count(), NULL) == NULL);

    if (fails) printf("apiset: %d FAILED\n", fails);
    else printf("apiset: all passed (%zu contracts)\n", apiset_count());
    return fails != 0;
}
