#ifndef LOOS_SHELL_TEST_H
#define LOOS_SHELL_TEST_H
#include <stdio.h>
static int t_fail, t_n;
#define CHECK(c) do { t_n++; if (!(c)) { t_fail++; printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); } } while (0)
#define DONE(name) do { printf("%s: %d checks, %d failed\n", name, t_n, t_fail); return t_fail != 0; } while (0)
#endif
