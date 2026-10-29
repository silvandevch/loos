#include "core.h"

#include <string.h>

static bool valid_thread(const struct loos_thread *thread)
{
    return thread != NULL && thread->state != LOOS_THREAD_DEAD &&
           thread->priority <= 31U;
}

int loos_run_queue_init(struct loos_run_queue *queue)
{
    if (queue == NULL) {
        return -1;
    }
    memset(queue, 0, sizeof(*queue));
    return 0;
}

int loos_run_queue_enqueue(struct loos_run_queue *queue,
                           struct loos_thread *thread)
{
    if (queue == NULL || !valid_thread(thread) || thread->on_run_queue) {
        return -1;
    }
    if (queue->count == LOOS_KERNEL_MAX_THREADS) {
        return -2;
    }
    queue->threads[queue->count++] = thread;
    thread->state = LOOS_THREAD_READY;
    thread->on_run_queue = true;
    if (thread->remaining_ticks == 0) {
        thread->remaining_ticks = thread->quantum_ticks == 0
                                      ? 1U
                                      : thread->quantum_ticks;
    }
    return 0;
}

int loos_run_queue_remove(struct loos_run_queue *queue,
                          struct loos_thread *thread)
{
    if (queue == NULL || thread == NULL || !thread->on_run_queue) {
        return -1;
    }
    size_t index = queue->count;
    for (size_t i = 0; i < queue->count; ++i) {
        if (queue->threads[i] == thread) {
            index = i;
            break;
        }
    }
    if (index == queue->count) {
        return -1;
    }
    memmove(&queue->threads[index], &queue->threads[index + 1U],
            (queue->count - index - 1U) * sizeof(queue->threads[0]));
    --queue->count;
    if (queue->cursor > index && queue->cursor != 0) {
        --queue->cursor;
    }
    if (queue->cursor >= queue->count && queue->count != 0) {
        queue->cursor = 0;
    }
    thread->on_run_queue = false;
    return 0;
}

struct loos_thread *loos_scheduler_pick(struct loos_run_queue *queue)
{
    if (queue == NULL || queue->count == 0) {
        return NULL;
    }
    struct loos_thread *best = NULL;
    size_t best_index = queue->cursor;
    for (size_t pass = 0; pass < queue->count; ++pass) {
        const size_t index = (queue->cursor + pass) % queue->count;
        struct loos_thread *candidate = queue->threads[index];
        if (!valid_thread(candidate) || candidate->suspended ||
            candidate->state == LOOS_THREAD_BLOCKED) {
            continue;
        }
        if (best == NULL || candidate->priority > best->priority) {
            best = candidate;
            best_index = index;
        }
    }
    if (best != NULL) {
        queue->cursor = (best_index + 1U) % queue->count;
        best->state = LOOS_THREAD_RUNNING;
    }
    return best;
}

size_t loos_scheduler_sync(struct loos_run_queue *queue,
                           loos_sched_query_fn query, void *opaque)
{
    size_t n = 0;
    if (queue == NULL || query == NULL) {
        return 0;
    }
    for (size_t i = 0; i < queue->count; ++i) {
        struct loos_thread *t = queue->threads[i];
        uint32_t priority;
        bool runnable;
        if (t != NULL && query(opaque, t->id, &priority, &runnable) &&
            priority <= 31U) {
            t->priority = priority;
            t->suspended = !runnable;
            ++n;
        }
    }
    return n;
}

int loos_scheduler_tick(struct loos_run_queue *queue,
                        struct loos_thread *current,
                        bool *preempt)
{
    if (queue == NULL || current == NULL || preempt == NULL ||
        !current->on_run_queue || current->state != LOOS_THREAD_RUNNING) {
        return -1;
    }
    if (current->remaining_ticks > 0) {
        --current->remaining_ticks;
    }
    *preempt = current->remaining_ticks == 0;
    if (*preempt) {
        current->remaining_ticks = current->quantum_ticks == 0
                                       ? 1U
                                       : current->quantum_ticks;
        current->state = LOOS_THREAD_READY;
    }
    return 0;
}

int loos_syscall_register(struct loos_syscall_table *table,
                          uint32_t number,
                          loos_syscall_fn function,
                          void *opaque)
{
    if (table == NULL || number >= LOOS_KERNEL_MAX_SYSCALLS ||
        function == NULL || table->entries[number].function != NULL) {
        return -1;
    }
    table->entries[number].function = function;
    table->entries[number].opaque = opaque;
    return 0;
}

int loos_syscall_dispatch(const struct loos_syscall_table *table,
                          uint32_t number,
                          const uint64_t args[6],
                          uint64_t *result)
{
    if (table == NULL || args == NULL || result == NULL ||
        number >= LOOS_KERNEL_MAX_SYSCALLS ||
        table->entries[number].function == NULL) {
        return -1;
    }
    return table->entries[number].function(args, result,
                                           table->entries[number].opaque);
}

int loos_validate_user_range(loos_user_range_check_fn checker,
                             void *opaque,
                             uint64_t address,
                             size_t length,
                             bool write)
{
    if (checker == NULL || (length != 0 && address > UINT64_MAX - length)) {
        return -1;
    }
    return checker(opaque, address, length, write) ? 0 : -2;
}
