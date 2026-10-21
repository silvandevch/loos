#define LOOS_HOST_TEST 1
#include "fake.h"
#include "../../../src/win32/kernel32/k32_internal.h"
#include <stdio.h>
#include <string.h>

static int failures;
extern RTL_USER_PROCESS_PARAMETERS *k32_host_params;
#define CHECK(expr) do { if (!(expr)) { fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #expr); failures++; } } while (0)

int main(void)
{
    char path[260], tiny[4], mapped[16];
    char *part = NULL;
    static WCHAR environment[] = { 'F','O','O','=','b','a','r',0,0 };
    RTL_USER_PROCESS_PARAMETERS parameters;

    fake_reset();
    CHECK(GetFullPathNameA("folder\\file.txt", sizeof path, path, &part) == strlen("C:\\folder\\file.txt"));
    CHECK(!strcmp(path, "C:\\folder\\file.txt"));
    CHECK(part == path + strlen("C:\\folder\\"));
    CHECK(!strcmp(part, "file.txt"));
    CHECK(GetFullPathNameA("folder\\file.txt", sizeof tiny, tiny, NULL) == strlen("C:\\folder\\file.txt") + 1);
    CHECK(CompareStringA(0, NORM_IGNORECASE, "LoOS", -1, "loos", -1) == CSTR_EQUAL);
    CHECK(LCMapStringA(0, LCMAP_UPPERCASE, "LoOS", -1, mapped, sizeof mapped) == 5);
    CHECK(!strcmp(mapped, "LOOS"));

    memset(&parameters, 0, sizeof parameters);
    parameters.Environment = environment;
    parameters.EnvironmentSize = sizeof environment;
    k32_host_params = &parameters;
    CHECK(GetEnvironmentVariableA("foo", mapped, sizeof mapped) == 3);
    CHECK(!strcmp(mapped, "bar"));
    CHECK(GetEnvironmentVariableA("FOO", tiny, 3) == 4);
    CHECK(SetEnvironmentVariableA("BAR", "local"));
    CHECK(GetEnvironmentVariableA("bar", mapped, sizeof mapped) == 5);
    CHECK(!strcmp(mapped, "local"));
    {
        char *block = GetEnvironmentStringsA();
        CHECK(block != NULL);
        if (block) {
            size_t first = strlen(block);
            CHECK(!strcmp(block, "BAR=local") && !strcmp(block + first + 1, "FOO=bar"));
            CHECK(block[first + 1 + strlen(block + first + 1) + 1] == '\0');
            CHECK(FreeEnvironmentStringsA(block));
        }
    }
    k32_host_params = NULL;

    if (failures) return 1;
    puts("ANSI adapter tests passed");
    return 0;
}
