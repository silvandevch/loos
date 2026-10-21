/* Task monitor: process tree with per-process sockets and I/O, from Ps.snapshot()/NetMonitor. Local only. */
#ifndef LOOS_SHELL_TASKMON_H
#define LOOS_SHELL_TASKMON_H
#include "ui.h"
#include "backend.h"

#ifdef __cplusplus
extern "C" {
#endif

#define TM_MAX 64
typedef enum { TM_SORT_NAME, TM_SORT_PID, TM_SORT_CPU, TM_SORT_MEM, TM_SORT_THREADS, TM_SORT_SOCKETS } tm_sort;
typedef struct {
    const shell_backend *be;
    ps_proc p[TM_MAX]; int n;
    int view[TM_MAX], depth[TM_MAX], nview;
    net_conn conns[32]; int nconns;
    int sel, scroll;                       /* selection is a view row; selected_pid tracks it across refreshes */
    uint32_t selected_pid;
    tm_sort sort; bool desc;
    bool tree;
    char filter[32]; int filter_caret;
    bool confirm_kill;
    char status[96];
    uint32_t total_cpu_x10; uint64_t total_mem; uint32_t total_sockets;
} shell_taskmon;

void tm_init(shell_taskmon *t, const shell_backend *be);
void tm_refresh(shell_taskmon *t);
void tm_set_sort(shell_taskmon *t, tm_sort s);
const ps_proc *tm_selected(const shell_taskmon *t);
bool tm_kill_selected(shell_taskmon *t);
int  tm_proc_sockets(const shell_taskmon *t, uint32_t pid, net_conn *out, int max);
void tm_frame(shell_taskmon *t, ui_ctx *ui, int w, int h);

#ifdef __cplusplus
}
#endif
#endif
