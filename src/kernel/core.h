#ifndef LOOS_KERNEL_CORE_H
#define LOOS_KERNEL_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LOOS_KERNEL_MAX_CPUS 256U
#define LOOS_KERNEL_MAX_THREADS 4096U
#define LOOS_KERNEL_MAX_SYSCALLS 512U
#define LOOS_KERNEL_PAGE_SIZE UINT64_C(4096)

enum loos_thread_state {
    LOOS_THREAD_READY = 0,
    LOOS_THREAD_RUNNING = 1,
    LOOS_THREAD_BLOCKED = 2,
    LOOS_THREAD_DEAD = 3,
};

struct loos_thread {
    uint64_t id;
    uint32_t priority;
    uint32_t quantum_ticks;
    uint32_t remaining_ticks;
    enum loos_thread_state state;
    bool on_run_queue;
    bool suspended; /* set by loos_scheduler_sync; pick skips it */
};

struct loos_run_queue {
    struct loos_thread *threads[LOOS_KERNEL_MAX_THREADS];
    size_t count;
    size_t cursor;
};

int loos_run_queue_init(struct loos_run_queue *queue);
int loos_run_queue_enqueue(struct loos_run_queue *queue,
                           struct loos_thread *thread);
int loos_run_queue_remove(struct loos_run_queue *queue,
                          struct loos_thread *thread);
struct loos_thread *loos_scheduler_pick(struct loos_run_queue *queue);
/* Executive view of one thread: base priority (1..31) and suspend state. */
typedef bool (*loos_sched_query_fn)(void *opaque, uint64_t thread_id,
                                    uint32_t *priority, bool *runnable);

/* Refreshes priority/suspended of every queued thread; returns updated count. */
size_t loos_scheduler_sync(struct loos_run_queue *queue,
                           loos_sched_query_fn query, void *opaque);
int loos_scheduler_tick(struct loos_run_queue *queue,
                        struct loos_thread *current,
                        bool *preempt);

typedef int (*loos_syscall_fn)(const uint64_t args[6], uint64_t *result,
                               void *opaque);

struct loos_syscall_entry {
    loos_syscall_fn function;
    void *opaque;
};

struct loos_syscall_table {
    struct loos_syscall_entry entries[LOOS_KERNEL_MAX_SYSCALLS];
};

int loos_syscall_register(struct loos_syscall_table *table,
                          uint32_t number,
                          loos_syscall_fn function,
                          void *opaque);
int loos_syscall_dispatch(const struct loos_syscall_table *table,
                          uint32_t number,
                          const uint64_t args[6],
                          uint64_t *result);

typedef bool (*loos_user_range_check_fn)(void *opaque,
                                         uint64_t address,
                                         size_t length,
                                         bool write);

int loos_validate_user_range(loos_user_range_check_fn checker,
                             void *opaque,
                             uint64_t address,
                             size_t length,
                             bool write);

#ifdef __cplusplus
}
#endif

#endif
