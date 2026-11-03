#include "../../src/kernel/mm.h"

#include <stdio.h>
#include <string.h>

#define CHECK(condition, message) \
    do { \
        if (!(condition)) { \
            fprintf(stderr, "FAIL: %s\n", message); \
            return 1; \
        } \
    } while (0)

struct file_source {
    uint8_t data[4096];
};

static int read_page(void *opaque, uint64_t offset, uint8_t *page)
{
    struct file_source *source = opaque;
    if (offset >= sizeof(source->data)) {
        return -1;
    }
    memcpy(page, source->data + offset, 4096);
    return 0;
}

int main(void)
{
    struct file_source source;
    memset(source.data, 0x3c, sizeof(source.data));
    struct loos_mm_section section;
    CHECK(loos_mm_section_init(&section, sizeof(source.data),
                               LOOS_MM_READ | LOOS_MM_WRITE, true,
                               read_page, &source) == LOOS_MM_OK,
          "section init");
    struct loos_mm_page pages[1];
    struct loos_mm_view view;
    CHECK(loos_mm_map_view(&view, &section, UINT64_C(0x400000), 4096,
                           LOOS_MM_READ | LOOS_MM_WRITE, pages,
                           sizeof(pages)) == LOOS_MM_OK,
          "view map");
    uint8_t *page = NULL;
    CHECK(loos_mm_fault(&view, UINT64_C(0x400123), false, &page) == LOOS_MM_OK &&
              page[0] == 0x3c,
          "demand fault");
    page[0] = 0x7a;
    CHECK(loos_mm_fault(&view, UINT64_C(0x400000), true, &page) == LOOS_MM_OK &&
              page[0] == 0x7a,
           "writable fault");
    loos_mm_unmap_view(&view);

    struct loos_mm_section executable;
    CHECK(loos_mm_section_init(&executable, 4096,
                               LOOS_MM_READ | LOOS_MM_EXECUTE, false, NULL,
                               NULL) == LOOS_MM_OK,
          "executable section init");
    struct loos_mm_page executable_page;
    CHECK(loos_mm_map_view(&view, &executable, UINT64_C(0x500000), 4096,
                           LOOS_MM_READ | LOOS_MM_EXECUTE, &executable_page,
                           sizeof(executable_page)) == LOOS_MM_OK,
          "executable view map");
    CHECK(loos_mm_fault(&view, UINT64_C(0x500000), true, &page) ==
              LOOS_MM_EACCES,
          "read-only executable mapping rejects writes");
    loos_mm_unmap_view(&view);

    CHECK(loos_mm_section_init(&executable, 4096,
                               LOOS_MM_READ | LOOS_MM_WRITE |
                                   LOOS_MM_EXECUTE,
                               false, NULL, NULL) == LOOS_MM_EINVAL,
          "reject writable executable section");
    CHECK(loos_mm_section_init(&executable, UINT64_MAX,
                               LOOS_MM_READ, false, NULL, NULL) ==
              LOOS_MM_OK,
          "accept maximally sized section");
    CHECK(loos_mm_map_view(&view, &executable, UINT64_C(0x600000), UINT64_MAX,
                           LOOS_MM_READ, &executable_page,
                           sizeof(executable_page)) == LOOS_MM_EINVAL,
          "reject overflowing view size");
    return 0;
}
