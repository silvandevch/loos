#include "core.h"

#include <stdio.h>
#include <string.h>

#define CHECK(condition, message) \
    do { \
        if (!(condition)) { \
            fprintf(stderr, "FAIL: %s\n", message); \
            return 1; \
        } \
    } while (0)

static int add(const uint64_t args[6], uint64_t *result, void *opaque)
{
    (void)opaque;
    *result = args[0] + args[1];
    return 0;
}

static bool range_ok(void *opaque, uint64_t address, size_t length, bool write)
{
    (void)opaque;
    (void)write;
    return address >= UINT64_C(0x1000) && length <= 0x1000 &&
           address <= UINT64_C(0x8000) && address + length <= 0x9000;
}

int main(void)
{
    struct loos_run_queue queue;
    CHECK(loos_run_queue_init(&queue) == 0, "queue init");
    struct loos_thread low = {.id = 1, .priority = 1, .quantum_ticks = 2};
    struct loos_thread high = {.id = 2, .priority = 5, .quantum_ticks = 1};
    CHECK(loos_run_queue_enqueue(&queue, &low) == 0, "enqueue low");
    CHECK(loos_run_queue_enqueue(&queue, &high) == 0, "enqueue high");
    CHECK(loos_scheduler_pick(&queue) == &high, "priority pick");
    bool preempt = false;
    CHECK(loos_scheduler_tick(&queue, &high, &preempt) == 0 && preempt,
          "quantum preemption");

    struct loos_syscall_table table = {0};
    CHECK(loos_syscall_register(&table, 7, add, NULL) == 0,
          "syscall registration");
    const uint64_t args[6] = {4, 8, 0, 0, 0, 0};
    uint64_t result = 0;
    CHECK(loos_syscall_dispatch(&table, 7, args, &result) == 0 && result == 12,
          "syscall dispatch");
    CHECK(loos_validate_user_range(range_ok, NULL, 0x2000, 16, false) == 0,
          "user range");
    CHECK(loos_validate_user_range(range_ok, NULL, UINT64_MAX, 2, false) < 0,
          "range overflow");
    return 0;
}
