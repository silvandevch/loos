#ifndef LOOS_LOADER_BASE_H
#define LOOS_LOADER_BASE_H
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#ifdef LOADER_HOST_TEST
#include <string.h>
#else
#include "rt.h"
#endif
#define PAGE_SIZE 4096ull
#define ALIGN_UP(v, a)   (((v) + (a) - 1) & ~((uint64_t)(a) - 1))
#define ALIGN_DOWN(v, a) ((v) & ~((uint64_t)(a) - 1))
#endif
