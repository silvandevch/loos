/* ============================================================================
 *  LoTaskMgr - Native LoOS Task Manager
 * ============================================================================
 *  A clean, human-written reconstruction of the classic Windows Task Manager
 *  (Win7/early-Win10 style: tabbed ListViews, status bar, tray icon).
 *  Single source file, like LoOS notepad_rewrite.c. UNICODE throughout.
 *
 *  Tabs: Processes | Performance | Startup apps | Users | Details | Services
 *  Genuine behavior in every tab:
 *    Processes  Toolhelp32 snapshot + GetProcessTimes CPU deltas +
 *               GetProcessMemoryInfo + owner lookup + End Task + Run + Find
 *    Performance CPU/memory history graphs (GDI), uptime, cores, totals
 *    Startup    HKLM/HKCU ...\\Run (+Wow6432Node) and both Startup folders,
 *               with working enable/disable
 *    Users      WTS sessions: log off, disconnect, send message
 *    Details    PID/status/user/CPU/memory/description + priority + affinity
 *    Services   EnumServicesStatusEx: start/stop/pause/continue
 *
 *  STRIPPED OUT (never ported, by design - same policy as LoOS notepad):
 *    - ETW telemetry: EventRegister/EventWriteTransfer/EventUnregister
 *      (TaskMgr_14004c794 and the ~40 wrappers in Taskmgr_readable.c).
 *      This build registers no ETW provider and emits no events, ever.
 *    - XAML/DirectUI UI stack (Windows.UI.Xaml residency in the original).
 *      The UI here is native Win32 (Tab + ListView + GDI graphs), so the
 *      binary has no XAML, DirectUI or WinUI dependency at all.
 *    - "App history" tab (per-process historical usage accounting, an
 *      always-on tracking store in the original). Not rebuilt on purpose.
 *    - Feedback links and online help URLs. Help is About only, offline.
 *
 *  MAPPING to the 1:1 decompile (../decompile/Taskmgr_readable.c):
 *    TaskMgr_14004c794 family  -> dropped (ETW, see above)
 *    NtQuerySystemInformation / Toolhelp / PDH usage -> tm_snapshot_* below
 *    EndTask/IsHungAppWindow/SendMessageTimeout -> tm_end_task()
 *    EnumServicesStatusEx/ControlService -> tm_services_*()
 *    WTSEnumerateSessions (WinStation*) -> tm_users_*()
 *    Run-key enumeration -> tm_startup_*()
 *
 *  BUILD: use the windows-msvc CMake preset. The local build.bat is a
 *  convenience wrapper for hosts with MSVC and the Windows SDK.
 *
 *  Set LOOSDEBUG=1 for verbose developer logging (DebugView / debugger).
 * ============================================================================
 */

#define _CRT_SECURE_NO_WARNINGS
#ifndef UNICODE
#define UNICODE
#endif
#ifndef _UNICODE
#define _UNICODE
#endif
#define WIN32_LEAN_AND_MEAN
#ifndef WINVER
#define WINVER 0x0A00
#endif
#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0A00
#endif
#ifndef _WIN32_IE
#define _WIN32_IE 0x0A00
#endif
#include <winsock2.h> /* address structures only; no sockets or DNS lookups */
#include <windows.h>
#include "brand.h"
#include <windowsx.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>
#include <strsafe.h>
#include <commctrl.h>
#include <tlhelp32.h>
#include <psapi.h>
#include <wtsapi32.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shlwapi.h>
#include <commdlg.h>
#include <iphlpapi.h>
#include <netioapi.h>
#include <tcpestats.h>
#include <pdh.h>
#include <pdhmsg.h>
#include <setupapi.h>
#include <winioctl.h>

#pragma comment(lib, "comctl32.lib")
#pragma comment(lib, "comdlg32.lib")
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "kernel32.lib")
#pragma comment(lib, "gdi32.lib")
#pragma comment(lib, "shell32.lib")
#pragma comment(lib, "advapi32.lib")
#pragma comment(lib, "psapi.lib")
#pragma comment(lib, "wtsapi32.lib")
#pragma comment(lib, "version.lib")
#pragma comment(lib, "shlwapi.lib")
#pragma comment(lib, "iphlpapi.lib")
#pragma comment(lib, "pdh.lib")
#pragma comment(lib, "setupapi.lib")

/* EndTask is missing from current SDK headers; import it explicitly. */
BOOL WINAPI EndTask(HWND hWnd, BOOL fShutDown, BOOL fForce);
/* Per-connection TCP EStats sit behind an `#ifdef WINAPI` gate in some SDK
 * revisions that the compiler sometimes skips; declare them explicitly. */
ULONG WINAPI SetPerTcpConnectionEStats(PMIB_TCPROW Row,
                                       TCP_ESTATS_TYPE EstatsType, PUCHAR Rw,
                                       ULONG RwVersion, ULONG RwSize,
                                       ULONG Offset);
ULONG WINAPI GetPerTcpConnectionEStats(PMIB_TCPROW Row,
                                       TCP_ESTATS_TYPE EstatsType, PUCHAR Rw,
                                       ULONG RwVersion, ULONG RwSize,
                                       PUCHAR Ros, ULONG RosVersion,
                                       ULONG RosSize, PUCHAR Rod,
                                       ULONG RodVersion, ULONG RodSize);

/* Modern DWM bits, guarded so older SDKs still compile (dwmapi.dll itself
 * is loaded dynamically at runtime for the same reason). */
#ifndef DWMWA_USE_IMMERSIVE_DARK_MODE
#define DWMWA_USE_IMMERSIVE_DARK_MODE 20
#endif
typedef HRESULT (WINAPI *DWM_SET_WINDOW_ATTRIBUTE_FN)(HWND, DWORD, LPCVOID, DWORD);
static DWM_SET_WINDOW_ATTRIBUTE_FN g_dwmSetAttr = NULL;

/* Per-monitor-V2 DPI, set programmatically with a manifest fallback. */
typedef DPI_AWARENESS_CONTEXT (WINAPI *SET_PROCESS_DPI_AWARENESS_CONTEXT_FN)(
    DPI_AWARENESS_CONTEXT);
#ifndef DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
#define DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 ((DPI_AWARENESS_CONTEXT)-4)
#endif

/* --------------------------------------------------------------------------
 * Control / menu / dialog / timer IDs
 * -------------------------------------------------------------------------- */
#define IDC_TAB             100
#define IDC_STATUS          101
#define IDC_LV_PROC         110
#define IDC_LV_PERFNET      111 /* (unused placeholder, keeps IDs dense) */
#define IDC_LV_STARTUP      112
#define IDC_LV_USERS        113
#define IDC_LV_DETAILS      114
#define IDC_LV_SERVICES     115
#define IDC_GRAPH_CPU       120
#define IDC_GRAPH_MEM       121
#define IDC_PERF_INFO       122

#define IDT_REFRESH         1

#define WM_TRAY             (WM_APP + 1)
#define TRAY_ICON_ID        1

#define IDM_FILE_RUN        40001
#define IDM_FILE_EXIT       40002
#define IDM_OPT_ONTOP       40101
#define IDM_OPT_MINUSE      40102
#define IDM_OPT_HIDE        40103
#define IDM_VIEW_HIGH       40201
#define IDM_VIEW_NORMAL     40202
#define IDM_VIEW_LOW        40203
#define IDM_VIEW_PAUSED     40204
#define IDM_VIEW_REFRESH    40205
#define IDM_VIEW_FIND       40206
#define IDM_PROC_ENDTASK    40301
#define IDM_PROC_GOTODETAIL 40302
#define IDM_PROC_PRIORITY   40310 /* +0..5: Realtime/High/Above/Normal/Below/Low */
#define IDM_PROC_AFFINITY   40320
#define IDM_PROC_SUSPEND    40321
#define IDM_PROC_RESUME     40322
#define IDM_PROC_ENDTREE    40323
#define IDM_PROC_COPYPATH   40324
#define IDM_PROC_OPENLOC    40325
#define IDM_PROC_WHYHUNG    40328
#define IDM_PROC_CONN       40329
#define IDM_PROC_THREADS    40330
#define IDM_VIEW_GROUPPARENT 40207
#define IDM_VIEW_MINIGRAPH  40208
#define IDM_VIEW_EXPORTCSV  40209
#define IDM_VIEW_DEADLOG    40210
#define IDM_VIEW_ALERTS     40211
#define IDM_VIEW_COLORS     40212
#define IDM_VIEW_CAPS       40213
#define IDM_VIEW_TDREVENTS  40214
#define IDM_SVC_STARTUP     40410 /* +0..2: Automatic/Manual/Disabled */
#define IDM_SVC_START       40401
#define IDM_SVC_STOP        40402
#define IDM_SVC_PAUSE       40403
#define IDM_SVC_CONTINUE    40404
#define IDM_SVC_REFRESH     40405
#define IDM_USER_LOGOFF     40501
#define IDM_USER_DISCONNECT 40502
#define IDM_USER_SENDMSG    40503
#define IDM_HELP_ABOUT      40601

#define IDD_RUN             101
#define IDD_FIND            102
#define IDD_AFFINITY        103
#define IDD_SENDMSG         104
#define IDD_ABOUT           105
#define IDD_WHYHUNG         106
#define IDD_CONN            107
#define IDD_THREADS         108
#define IDD_TDR             109
#define IDD_ALERTS          110
#define IDD_COLORS          111
#define IDD_CAPS            112
#define IDD_DEADLOG         113
#define IDL_LIST            1010
#define IDE_INFO            1011
#define IDE_CPU_TH          1012
#define IDE_RAM_TH          1013
#define IDE_SEC_TH          1014
#define IDB_SWATCH_LO       1015
#define IDB_SWATCH_MID      1016
#define IDB_SWATCH_HI       1017
#define IDE_CAP_NAME        1018
#define IDE_CAP_MB          1019
#define IDC_CAP_ACTION      1020
#define IDB_CAP_ADD         1021
#define IDB_CAP_DEL         1022
#define IDB_COPYLINE        1024
#define IDE_RUN_FILE        1001
#define IDE_FIND_TEXT       1002
#define IDL_AFFINITY        1003
#define IDE_MSG_TITLE       1004
#define IDE_MSG_TEXT        1005

/* Tabs, in order. */
typedef enum {
    TAB_PROCESSES = 0,
    TAB_PERFORMANCE,
    TAB_STARTUP,
    TAB_USERS,
    TAB_DETAILS,
    TAB_SERVICES,
    TAB_COUNT
} TM_TAB;

/* Update speeds (ms). 0 = paused. */
#define SPEED_HIGH_MS   1000
#define SPEED_NORMAL_MS 2000
#define SPEED_LOW_MS    4000

/* History length for the performance graphs. */
#define TM_HIST_N 120

/* --------------------------------------------------------------------------
 * Transcribed UI strings (classic en-US Task Manager wording).
 * -------------------------------------------------------------------------- */
static const wchar_t S_APP[]            = LOOS_BRAND_TASKMGR_W;
static const wchar_t S_CLASS[]          = L"LoOSTaskManager";
static const wchar_t S_FMT_CPU[]        = L"CPU: %u%%";
static const wchar_t S_FMT_MEM[]        = L"Memory: %s";
static const wchar_t S_FMT_PROCS[]      = L"Processes: %u";
static const wchar_t S_ENDTASK_CAP[]    = L"End Task";
static const wchar_t S_ENDTASK_MSG[]    =
    L"Do you want to end \"%s\"?\n\nUnsaved data in this process will be lost.";
static const wchar_t S_RUN_CAP[]        = L"Create New Task";
static const wchar_t S_ABOUT[]          =
    L"LoOS Task Manager\nNative Win32 task manager for LoOS.\n"
    L"No ETW telemetry. No XAML. Offline help only.";
static const wchar_t S_TRAY_TIP_FMT[]   = L"LoOS Task Manager\nCPU %u%%  Mem %u%%";

/* --------------------------------------------------------------------------
 * Row types
 * -------------------------------------------------------------------------- */
typedef struct {
    DWORD   pid;
    DWORD   ppid;
    wchar_t name[MAX_PATH];
    wchar_t user[130];      /* DOMAIN\\User, or "" when unknown */
    wchar_t desc[260];      /* FileDescription from version resource */
    double  cpu;            /* 0..100, last sample */
    SIZE_T  memKB;          /* working set, KB */
    DWORD   threads;
    DWORD   handles;        /* (DWORD)-1 when not queryable */
    DWORD   prioClass;
    DWORD_PTR affinity;
    ULONGLONG prevProc;     /* prev kernel+user ticks (100ns) */
    ULONGLONG prevSys;      /* prev system total for the delta */
    int     alive;          /* set while merging a fresh snapshot */
    int     init;           /* full (slow) queries done once per PID */
    int     suspended;      /* last known suspend state (best effort) */
    /* I/O + network + GPU deltas (per-tick rates, computed in samplers). */
    ULONGLONG prevRIOps, prevWIOps, prevRIoB, prevWioB, prevIoTick;
    double  iopsR, iopsW, ioRBps, ioWBps;
    ULONGLONG cpuTimeTotal; /* kernel+user, 100ns (honest "CPU seconds") */
    ULONGLONG netIn, netOut;      /* session cumulative TCP bytes */
    double  netInBps, netOutBps;
    double  gpuDed, gpuShr;       /* bytes (PDH, 0 when unavailable) */
    int     hasMainWnd;
} TM_PROC;

typedef struct {
    wchar_t name[256];
    wchar_t display[256];
    wchar_t status[32];
    wchar_t startup[32];
    wchar_t desc[260];
    DWORD   pid;
} TM_SVC;

typedef struct {
    wchar_t name[256];
    wchar_t command[1024];
    wchar_t location[260];
    int     enabled;
    int     hive;           /* 0 = HKLM, 1 = HKCU, 2 = folder */
} TM_STARTUP;

typedef struct {
    DWORD   sessionId;
    wchar_t user[130];
    wchar_t state[32];
    wchar_t client[64];
} TM_USER;

/* --------------------------------------------------------------------------
 * Global state
 * -------------------------------------------------------------------------- */
static HINSTANCE g_hInst;
static HWND   g_hwndMain, g_hwndTab, g_hwndStatus;
static HWND   g_pages[TAB_COUNT];       /* one child host per tab */
static HWND   g_lv[TAB_COUNT];          /* the ListView on list tabs (or NULL) */
static HWND   g_graphCpu, g_graphMem, g_perfInfo;
static HMENU  g_menu, g_menuFile, g_menuOpt, g_menuView;

static TM_PROC *g_procs;
static size_t   g_procCount, g_procCap;

static int      g_tab = TAB_PROCESSES;
static int      g_speedMs = SPEED_NORMAL_MS;
static int      g_alwaysTop, g_minOnUse, g_hideMin;
static int      g_devLog = -1;
static int      g_dark = 0;
static int      g_sortCol[TAB_COUNT];
static int      g_sortAsc[TAB_COUNT];

static double   g_cpuHist[TM_HIST_N];
static double   g_memHist[TM_HIST_N];
static double   g_netHist[TM_HIST_N];
static double   g_diskHist[TM_HIST_N];
static int      g_histCount;
static ULONGLONG g_prevIdle, g_prevKernel, g_prevUser;
static ULONGLONG g_prevDiskR, g_prevDiskW;
static ULONGLONG g_prevNetIn, g_prevNetOut;
static double   g_cpuTotal;             /* last system-wide sample */
static double   g_diskBps, g_netBps;    /* system totals, bytes/s */
static double   g_vpnBps, g_lanBps;     /* split by interface class */
static int      g_nCores = 1, g_nLogical = 1;

/* Recently terminated processes (ring). */
#define TM_DEAD_N 64
typedef struct {
    wchar_t name[MAX_PATH];
    DWORD pid;
    SYSTEMTIME when;
    double cpu;
    SIZE_T memKB;
} TM_DEAD;
static TM_DEAD g_dead[TM_DEAD_N];
static int g_deadHead, g_deadCount;

/* Automated caps: suspend/kill/alert a process matching name over MB. */
#define TM_CAPS_N 16
typedef struct {
    wchar_t match[64];
    DWORD mb;
    int action; /* 0 suspend, 1 terminate, 2 alert balloon */
    int used;
} TM_CAP;
static TM_CAP g_caps[TM_CAPS_N];

/* Alerts + heat colors (persisted under HKCU\Software\LoOS\TaskMgr). */
static int g_alertCpuPct = 0, g_alertRamPct = 0, g_alertSecs = 30;
static int g_overCpuSince, g_overRamSince;
static COLORREF g_colLo = RGB(232, 245, 233), g_colMid = RGB(255, 243, 224),
                g_colHi = RGB(255, 235, 238);
static int g_groupParent;
static HWND g_miniGraph; /* detachable always-on-top CPU graph */

/* ntdll suspend/resume (dynamic, no import lib needed). */
typedef LONG (NTAPI *NT_SUSPEND_PROC_FN)(HANDLE);
typedef LONG (NTAPI *NT_RESUME_PROC_FN)(HANDLE);
static NT_SUSPEND_PROC_FN g_ntSuspend;
static NT_RESUME_PROC_FN g_ntResume;

/* --------------------------------------------------------------------------
 * Forward declarations
 * -------------------------------------------------------------------------- */
static void     tm_debug(const wchar_t *fmt, ...);
static void     tm_error(HWND owner, const wchar_t *what);
static int      tm_comma(wchar_t *out, size_t cch, ULONGLONG v);
static void     tm_fmt_bytes(wchar_t *out, size_t cch, ULONGLONG bytes);
static void     tm_uptime(wchar_t *out, size_t cch);
static void     tm_theme_init(HWND hwnd);
static void     tm_dpi_init(void);
static int      tm_is_dark_mode(void);

static int      tm_snapshot_processes(void);
static TM_PROC *tm_find_proc(DWORD pid);
static void     tm_sample_cpu(TM_PROC *p, HANDLE hProc, ULONGLONG sysTotal);
static int      tm_proc_user(HANDLE hProc, wchar_t *out, size_t cch);
static int      tm_proc_desc(const wchar_t *exePath, wchar_t *out, size_t cch);
static HWND     tm_proc_mainwnd(DWORD pid);

static HWND     tm_make_listview(HWND parent, int id);
static void     tm_lv_columns(HWND lv, const wchar_t *const *names,
                              const int *widths, int n);
static int      tm_lv_selected_pid(HWND lv);
static void     tm_lv_select_pid(HWND lv, DWORD pid);
static int CALLBACK tm_lv_compare(LPARAM a, LPARAM b, LPARAM sort);

static void     tm_pages_create(void);
static void     tm_show_tab(int tab);
static void     tm_layout(void);
static void     tm_refresh_all(int force);
static void     tm_refresh_status(void);

static void     tm_procs_refresh(void);
static void     tm_details_refresh(void);
static void     tm_perf_refresh(void);
static void     tm_perf_draw(HWND hwnd, HDC hdc, int isMem);
static void     tm_startup_refresh(void);
static void     tm_users_refresh(void);
static void     tm_services_refresh(void);

static void     tm_end_task(HWND lv);
static void     tm_run_dialog(HWND owner);
static void     tm_find_dialog(HWND owner);
static void     tm_affinity_dialog(HWND owner, DWORD pid);
static void     tm_set_priority(HWND lv, DWORD prioClass);
static void     tm_goto_details(DWORD pid);
static void     tm_svc_command(HWND lv, int cmd);
static void     tm_startup_toggle(HWND lv);
static void     tm_user_command(HWND lv, int cmd);
static void     tm_sendmsg_dialog(HWND owner, DWORD sessionId);

static void     tm_tray_add(void);
static void     tm_tray_del(void);
static void     tm_tray_tip(void);
static HMENU    tm_build_menu(void);
static void     tm_menu_sync(void);
static INT_PTR CALLBACK tm_run_proc(HWND, UINT, WPARAM, LPARAM);
static INT_PTR CALLBACK tm_find_proc_dlg(HWND, UINT, WPARAM, LPARAM);
static INT_PTR CALLBACK tm_affinity_proc(HWND, UINT, WPARAM, LPARAM);
static INT_PTR CALLBACK tm_sendmsg_proc(HWND, UINT, WPARAM, LPARAM);
static INT_PTR CALLBACK tm_about_proc(HWND, UINT, WPARAM, LPARAM);
static LRESULT CALLBACK tm_wnd_proc(HWND, UINT, WPARAM, LPARAM);
static LRESULT CALLBACK tm_graph_proc(HWND, UINT, WPARAM, LPARAM,
                                      UINT_PTR kind);

/* --- Feature round 2 (see bottom of file for implementations) --- */
static void     tm_sample_io(TM_PROC *p, HANDLE hProc);
static void     tm_net_sample(void);
static void     tm_if_sample(void);
static void     tm_gpu_sample(void);
static void     tm_dead_push(const TM_PROC *p);
static void     tm_caps_check(void);
static void     tm_alerts_check(void);
static void     tm_balloon(const wchar_t *title, const wchar_t *text);
static void     tm_nt_init(void);
static int      tm_suspend(DWORD pid);
static int      tm_resume(DWORD pid);
static void     tm_end_tree(DWORD pid);
static DWORD    tm_top_ancestor(DWORD pid);
static void     tm_copy_path(DWORD pid);
static void     tm_open_location(DWORD pid);
static void     tm_whyhung_dialog(HWND owner, DWORD pid);
static void     tm_conn_dialog(HWND owner, DWORD pid);
static void     tm_threads_dialog(HWND owner, DWORD pid);
static void     tm_tdr_dialog(HWND owner);
static void     tm_alerts_dialog(HWND owner);
static void     tm_colors_dialog(HWND owner);
static void     tm_caps_dialog(HWND owner);
static void     tm_dead_dialog(HWND owner);
static void     tm_export_csv(HWND owner);
static void     tm_mini_toggle(void);
static void     tm_prefs_load(void);
static void     tm_prefs_save(void);
static void     tm_heat_draw(HWND lv, LPNMLVCUSTOMDRAW cd, int sub);
static int      tm_npu_present(void);

/* --------------------------------------------------------------------------
 * Helpers: logging, formatting, errors
 * -------------------------------------------------------------------------- */

/* LOOSDEBUG=1 enables verbose logging (DebugView / debugger), like notepad. */
static void tm_debug(const wchar_t *fmt, ...)
{
    wchar_t buf[1024];
    va_list ap;
    if (g_devLog < 0)
        g_devLog = (_wgetenv(L"LOOSDEBUG") != NULL) ? 1 : 0;
    if (!g_devLog)
        return;
    va_start(ap, fmt);
    _vsnwprintf_s(buf, _countof(buf), _TRUNCATE, fmt, ap);
    va_end(ap);
    OutputDebugStringW(L"[taskmgr] ");
    OutputDebugStringW(buf);
    OutputDebugStringW(L"\n");
}

static void tm_error(HWND owner, const wchar_t *what)
{
    DWORD err = GetLastError();
    wchar_t msg[512], sys[256];
    if (err == 0) {
        MessageBoxW(owner, what, S_APP, MB_OK | MB_ICONERROR);
        return;
    }
    FormatMessageW(FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
                   NULL, err, 0, sys, _countof(sys), NULL);
    StringCchPrintfW(msg, _countof(msg), L"%s\n\n(%lu) %s", what, err, sys);
    MessageBoxW(owner, msg, S_APP, MB_OK | MB_ICONERROR);
}

/* 1234567 -> "1,234,567". Returns chars written (not incl. NUL). */
static int tm_comma(wchar_t *out, size_t cch, ULONGLONG v)
{
    wchar_t tmp[32];
    int len = 0, i, first;
    size_t pos = 0;
    if (cch == 0)
        return 0;
    if (v == 0) {
        out[0] = L'0';
        if (cch > 1)
            out[1] = L'\0';
        return 1;
    }
    while (v > 0 && len < (int)_countof(tmp) - 1) {
        tmp[len++] = (wchar_t)(L'0' + (v % 10));
        v /= 10;
    }
    first = len % 3;
    if (first == 0)
        first = 3;
    for (i = len - 1; i >= 0 && pos + 2 < cch; i--) {
        out[pos++] = tmp[i];
        if (i > 0 && ((len - 1 - i + 1 - first) % 3) == 0 && pos + 2 < cch - 1)
            out[pos++] = L',';
    }
    out[pos] = L'\0';
    return (int)pos;
}

/* 1536 -> "1.5 MB". Uses B/KB/MB/GB/TB. */
static void tm_fmt_bytes(wchar_t *out, size_t cch, ULONGLONG bytes)
{
    static const wchar_t *units[] = { L"B", L"KB", L"MB", L"GB", L"TB" };
    double v = (double)bytes;
    int u = 0;
    while (v >= 1024.0 && u < 4) {
        v /= 1024.0;
        u++;
    }
    if (u == 0)
        StringCchPrintfW(out, cch, L"%llu %s", bytes, units[u]);
    else
        StringCchPrintfW(out, cch, L"%.1f %s", v, units[u]);
}

/* "3:14:22:10" (d:h:mm:ss) since boot. */
static void tm_uptime(wchar_t *out, size_t cch)
{
    ULONGLONG s = GetTickCount64() / 1000;
    StringCchPrintfW(out, cch, L"%llu:%02llu:%02llu:%02llu",
                     s / 86400, (s / 3600) % 24, (s / 60) % 60, s % 60);
}

/* --------------------------------------------------------------------------
 * Theme (dark title bar + Mica, dynamic dwmapi) and DPI
 * -------------------------------------------------------------------------- */
static int tm_is_dark_mode(void)
{
    /* Follows the system app theme, like notepad_rewrite.c. */
    DWORD v = 1, cb = sizeof(v);
    HKEY h;
    if (RegOpenKeyExW(HKEY_CURRENT_USER,
                      L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                      0, KEY_READ, &h) == ERROR_SUCCESS) {
        RegQueryValueExW(h, L"AppsUseLightTheme", NULL, NULL,
                         (LPBYTE)&v, &cb);
        RegCloseKey(h);
    }
    return v == 0;
}

static void tm_theme_init(HWND hwnd)
{
    HMODULE dwm;
    BOOL dark;
    DWORD backdrop = 2; /* DWMSBT_MAINWINDOW (Mica) */
    g_dark = tm_is_dark_mode();
    dwm = LoadLibraryW(L"dwmapi.dll");
    if (!dwm)
        return;
    g_dwmSetAttr = (DWM_SET_WINDOW_ATTRIBUTE_FN)
        GetProcAddress(dwm, "DwmSetWindowAttribute");
    if (g_dwmSetAttr) {
        dark = g_dark ? TRUE : FALSE;
        g_dwmSetAttr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE,
                     &dark, sizeof(dark));
        /* Mica is best-effort: ignored on older Windows, like notepad. */
        g_dwmSetAttr(hwnd, 38 /* DWMWA_SYSTEMBACKDROP_TYPE */,
                     &backdrop, sizeof(backdrop));
    }
}

static void tm_dpi_init(void)
{
    HMODULE u32 = LoadLibraryW(L"user32.dll");
    SET_PROCESS_DPI_AWARENESS_CONTEXT_FN fn;
    if (!u32)
        return;
    fn = (SET_PROCESS_DPI_AWARENESS_CONTEXT_FN)GetProcAddress(
        u32, "SetProcessDpiAwarenessContext");
    if (fn)
        fn(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
}

/* --------------------------------------------------------------------------
 * ListView helpers (all tabs share these)
 * -------------------------------------------------------------------------- */
static HWND tm_make_listview(HWND parent, int id)
{
    HWND lv = CreateWindowExW(WS_EX_CLIENTEDGE, WC_LISTVIEWW, L"",
                              WS_CHILD | WS_VISIBLE | LVS_REPORT |
                              LVS_SHOWSELALWAYS | LVS_SINGLESEL,
                              0, 0, 10, 10, parent, (HMENU)(INT_PTR)id,
                              g_hInst, NULL);
    if (lv) {
        ListView_SetExtendedListViewStyle(lv, LVS_EX_FULLROWSELECT |
                                          LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP);
    }
    return lv;
}

static void tm_lv_columns(HWND lv, const wchar_t *const *names,
                          const int *widths, int n)
{
    LVCOLUMNW col;
    int i;
    memset(&col, 0, sizeof(col));
    col.mask = LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM;
    for (i = 0; i < n; i++) {
        col.pszText = (LPWSTR)names[i];
        col.cx = widths[i];
        col.iSubItem = i;
        ListView_InsertColumn(lv, i, &col);
    }
}

/* Text of (item, subitem) into out. */
static void tm_lv_text(HWND lv, int item, int sub,
                       wchar_t *out, size_t cch)
{
    LVITEMW li;
    memset(&li, 0, sizeof(li));
    li.mask = LVIF_TEXT;
    li.iItem = item;
    li.iSubItem = sub;
    li.pszText = out;
    li.cchTextMax = (int)cch;
    out[0] = L'\0';
    ListView_GetItem(lv, &li);
}

/* Set cell text. Takes const (ListView copies on set), so callers can pass
 * literals and const buffers without const-cast warnings. */
static void tm_lv_set(HWND lv, int item, int sub, const wchar_t *text)
{
    ListView_SetItemText(lv, item, sub, (LPWSTR)text);
}

/* PID of the selected row (lParam carries it on proc/detail lists;
 * services/users/startup lists store an index or id instead - see users). */
static int tm_lv_selected_pid(HWND lv)
{
    int sel = ListView_GetNextItem(lv, -1, LVNI_SELECTED);
    LVITEMW li;
    if (sel < 0)
        return -1;
    memset(&li, 0, sizeof(li));
    li.mask = LVIF_PARAM;
    li.iItem = sel;
    if (!ListView_GetItem(lv, &li))
        return -1;
    return (int)(INT_PTR)li.lParam;
}

static void tm_lv_select_pid(HWND lv, DWORD pid)
{
    int n = ListView_GetItemCount(lv), i;
    LVITEMW li;
    for (i = 0; i < n; i++) {
        memset(&li, 0, sizeof(li));
        li.mask = LVIF_PARAM;
        li.iItem = i;
        if (ListView_GetItem(lv, &li) && (DWORD)(INT_PTR)li.lParam == pid) {
            ListView_SetItemState(lv, i, LVIS_SELECTED | LVIS_FOCUSED,
                                  LVIS_SELECTED | LVIS_FOCUSED);
            ListView_EnsureVisible(lv, i, FALSE);
            return;
        }
    }
}

/* Numeric-aware sort used by every tab. lParam = row id, text re-read
 * from the control so one comparator serves all columns. */
static int CALLBACK tm_lv_compare(LPARAM a, LPARAM b, LPARAM sort)
{
    /* sort packs: low word = HWND-proven list id, high = column, sign = dir.
     * We receive the HWND via the packed struct pointer instead. */
    struct {
        HWND hwnd;
        int col;
        int asc;
    } *s = (void *)(INT_PTR)sort;
    wchar_t ta[512], tb[512];
    double da, db;
    wchar_t *ea, *eb;
    int ia = -1, ib = -1, n, i;
    LVFINDINFOW fi;
    LVITEMW li;
    if (!s)
        return 0;
    n = ListView_GetItemCount(s->hwnd);
    memset(&fi, 0, sizeof(fi));
    fi.flags = LVFI_PARAM;
    fi.lParam = a;
    ia = ListView_FindItem(s->hwnd, -1, &fi);
    fi.lParam = b;
    ib = ListView_FindItem(s->hwnd, -1, &fi);
    if (ia < 0 || ib < 0)
        return 0;
    tm_lv_text(s->hwnd, ia, s->col, ta, _countof(ta));
    tm_lv_text(s->hwnd, ib, s->col, tb, _countof(tb));
    /* Numeric when both sides parse (handles "12%", "1.5 MB", "1,024 K"). */
    da = wcstod(ta, &ea);
    db = wcstod(tb, &eb);
    if (ea != ta && eb != tb) {
        int r = (da < db) ? -1 : (da > db) ? 1 : 0;
        /* Unit suffix tie-break: B < KB < MB < GB < TB. */
        if (r == 0) {
            const wchar_t *ua = ea, *ub = eb;
            while (*ua == L' ')
                ua++;
            while (*ub == L' ')
                ub++;
            r = lstrcmpiW(ua, ub);
        }
        if (r == 0) {
            (void)memset(&li, 0, sizeof(li));
            r = lstrcmpiW(ta, tb);
        }
        return s->asc ? r : -r;
    }
    i = lstrcmpiW(ta, tb);
    return s->asc ? i : -i;
}

/* --------------------------------------------------------------------------
 * Process snapshot core (backs the Processes and Details tabs)
 *
 * One Toolhelp32 walk per refresh + per-process queries. CPU% comes from
 * GetProcessTimes deltas against GetSystemTimes, so the first refresh after
 * startup legitimately shows 0% (same as the genuine Task Manager).
 * -------------------------------------------------------------------------- */

/* System-wide (idle+kernel+user) tick total for CPU deltas. */
static ULONGLONG tm_system_total(void)
{
    FILETIME idle, kernel, user;
    ULARGE_INTEGER ki, ke, us, tot;
    if (!GetSystemTimes(&idle, &kernel, &user))
        return 0;
    ki.LowPart = idle.dwLowDateTime;
    ki.HighPart = idle.dwHighDateTime;
    ke.LowPart = kernel.dwLowDateTime;
    ke.HighPart = kernel.dwHighDateTime;
    us.LowPart = user.dwLowDateTime;
    us.HighPart = user.dwHighDateTime;
    tot.QuadPart = ki.QuadPart + ke.QuadPart + us.QuadPart;
    return tot.QuadPart;
}

static TM_PROC *tm_find_proc(DWORD pid)
{
    size_t i;
    for (i = 0; i < g_procCount; i++)
        if (g_procs[i].pid == pid)
            return &g_procs[i];
    return NULL;
}

/* DOMAIN\User owning hProc, or "" when it cannot be determined. */
static int tm_proc_user(HANDLE hProc, wchar_t *out, size_t cch)
{
    HANDLE tok = NULL;
    DWORD len = 0;
    PTOKEN_USER tu = NULL;
    wchar_t name[130], dom[130];
    DWORD cn = _countof(name), dn = _countof(dom), need;
    SID_NAME_USE use;
    out[0] = L'\0';
    if (!OpenProcessToken(hProc, TOKEN_QUERY, &tok))
        return 0;
    GetTokenInformation(tok, TokenUser, NULL, 0, &need);
    tu = (PTOKEN_USER)malloc(need == 0 ? 256 : need);
    if (!tu) {
        CloseHandle(tok);
        return 0;
    }
    if (!GetTokenInformation(tok, TokenUser, tu, need == 0 ? 256 : need,
                             &len)) {
        free(tu);
        CloseHandle(tok);
        return 0;
    }
    if (LookupAccountSidW(NULL, tu->User.Sid, name, &cn, dom, &dn, &use) &&
        dom[0] && name[0])
        StringCchPrintfW(out, cch, L"%s\\%s", dom, name);
    else if (name[0])
        StringCchCopyW(out, cch, name);
    free(tu);
    CloseHandle(tok);
    return out[0] != L'\0';
}

/* FileDescription from the exe's version resource, else the file name. */
static int tm_proc_desc(const wchar_t *exePath, wchar_t *out, size_t cch)
{
    DWORD handle = 0, size;
    BYTE *buf;
    wchar_t *trans;
    UINT transLen;
    wchar_t sub[128];
    const wchar_t *file = exePath;
    const wchar_t *slash = wcsrchr(exePath, L'\\');
    if (slash)
        file = slash + 1;
    StringCchCopyW(out, cch, file);
    size = GetFileVersionInfoSizeW(exePath, &handle);
    if (size == 0 || size > (1 << 20))
        return 0;
    buf = (BYTE *)malloc(size);
    if (!buf)
        return 0;
    if (!GetFileVersionInfoW(exePath, 0, size, buf)) {
        free(buf);
        return 0;
    }
    if (!VerQueryValueW(buf, L"\\VarFileInfo\\Translation",
                        (LPVOID *)&trans, &transLen) ||
        transLen < 4) {
        free(buf);
        return 0;
    }
    StringCchPrintfW(sub, _countof(sub),
                     L"\\StringFileInfo\\%04x%04x\\FileDescription",
                     LOWORD(*(DWORD *)trans), HIWORD(*(DWORD *)trans));
    {
        wchar_t *val;
        UINT valLen;
        if (VerQueryValueW(buf, sub, (LPVOID *)&val, &valLen) && valLen > 1 &&
            val[0]) {
            StringCchCopyW(out, cch, val);
            free(buf);
            return 1;
        }
    }
    free(buf);
    return 0;
}

/* First top-level visible window owned by pid (for End Task status). */
struct TM_WND_CTX {
    DWORD pid;
    HWND hwnd;
};

static BOOL CALLBACK tm_wnd_enum(HWND hwnd, LPARAM lp)
{
    struct TM_WND_CTX *ctx = (struct TM_WND_CTX *)(INT_PTR)lp;
    DWORD pid = 0;
    if (!IsWindowVisible(hwnd) || GetWindow(hwnd, GW_OWNER) != NULL)
        return TRUE;
    GetWindowThreadProcessId(hwnd, &pid);
    if (pid == ctx->pid && ctx->hwnd == NULL)
        ctx->hwnd = hwnd;
    return TRUE;
}

static HWND tm_proc_mainwnd(DWORD pid)
{
    struct TM_WND_CTX ctx;
    ctx.pid = pid;
    ctx.hwnd = NULL;
    EnumWindows(tm_wnd_enum, (LPARAM)(INT_PTR)&ctx);
    return ctx.hwnd;
}

/* Per-process CPU sample: delta of (kernel+user) over system delta. */
static void tm_sample_cpu(TM_PROC *p, HANDLE hProc, ULONGLONG sysTotal)
{
    FILETIME create, exit, kernel, user;
    ULARGE_INTEGER ke, us, tot;
    ULONGLONG dProc, dSys;
    if (!GetProcessTimes(hProc, &create, &exit, &kernel, &user)) {
        p->cpu = 0.0;
        return;
    }
    ke.LowPart = kernel.dwLowDateTime;
    ke.HighPart = kernel.dwHighDateTime;
    us.LowPart = user.dwLowDateTime;
    us.HighPart = user.dwHighDateTime;
    tot.QuadPart = ke.QuadPart + us.QuadPart;
    p->cpuTimeTotal = tot.QuadPart; /* honest "CPU seconds" for startup */
    if (p->prevSys == 0 || sysTotal <= p->prevSys ||
        tot.QuadPart < p->prevProc) {
        /* First sighting (or PID reuse): store, report 0 this round. */
        p->prevProc = tot.QuadPart;
        p->prevSys = sysTotal;
        p->cpu = 0.0;
        return;
    }
    dProc = tot.QuadPart - p->prevProc;
    dSys = sysTotal - p->prevSys;
    p->prevProc = tot.QuadPart;
    p->prevSys = sysTotal;
    if (dSys == 0) {
        p->cpu = 0.0;
        return;
    }
    p->cpu = 100.0 * (double)dProc / (double)dSys;
    if (p->cpu < 0.0)
        p->cpu = 0.0;
    if (p->cpu > 100.0 * g_nLogical)
        p->cpu = 100.0 * g_nLogical;
}

/* Full refresh of g_procs. Returns nonzero on success.
 * Slow queries (version resource, SID, affinity, window scan) run once per
 * PID (p->init) - every tick only does the cheap snapshot + times + memory
 * walk, so the UI thread never stalls. Window presence re-scans every 5th
 * tick to catch newly shown windows. */
static int tm_snapshot_processes(void)
{
    HANDLE snap;
    PROCESSENTRY32W pe;
    ULONGLONG sysTotal = tm_system_total();
    static int tick;
    size_t i;
    tick++;
    /* Mark all dead; the walk below revives the survivors. */
    for (i = 0; i < g_procCount; i++)
        g_procs[i].alive = 0;
    snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if (snap == INVALID_HANDLE_VALUE)
        return 0;
    memset(&pe, 0, sizeof(pe));
    pe.dwSize = sizeof(pe);
    if (Process32FirstW(snap, &pe)) {
        do {
            TM_PROC *p = tm_find_proc(pe.th32ProcessID);
            HANDLE h;
            if (!p) {
                if (g_procCount == g_procCap) {
                    size_t ncap = g_procCap == 0 ? 256 : g_procCap * 2;
                    TM_PROC *np = (TM_PROC *)realloc(g_procs,
                                                     ncap * sizeof(*np));
                    if (!np)
                        break;
                    g_procs = np;
                    g_procCap = ncap;
                }
                p = &g_procs[g_procCount++];
                memset(p, 0, sizeof(*p));
                p->pid = pe.th32ProcessID;
            }
            p->alive = 1;
            p->ppid = pe.th32ParentProcessID;
            StringCchCopyW(p->name, _countof(p->name), pe.szExeFile);
            p->threads = pe.cntThreads;
            if (!p->init || (tick % 5) == 0)
                p->hasMainWnd = (tm_proc_mainwnd(p->pid) != NULL);
            h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE,
                            p->pid);
            if (h) {
                PROCESS_MEMORY_COUNTERS pmc;
                wchar_t path[MAX_PATH];
                DWORD pathLen = _countof(path);
                p->prioClass = GetPriorityClass(h);
                if (p->prioClass == 0)
                    p->prioClass = NORMAL_PRIORITY_CLASS;
                if (GetProcessMemoryInfo(h, &pmc, sizeof(pmc)))
                    p->memKB = pmc.WorkingSetSize / 1024;
                if (!p->init) {
                    if (QueryFullProcessImageNameW(h, 0, path, &pathLen))
                        tm_proc_desc(path, p->desc, _countof(p->desc));
                    else
                        StringCchCopyW(p->desc, _countof(p->desc), p->name);
                }
                if (p->user[0] == L'\0')
                    tm_proc_user(h, p->user, _countof(p->user));
                tm_sample_cpu(p, h, sysTotal);
                tm_sample_io(p, h);
                CloseHandle(h);
                /* Handles need a stronger right; best effort. */
                h = OpenProcess(PROCESS_QUERY_INFORMATION, FALSE, p->pid);
                if (h) {
                    DWORD hc = 0;
                    if (GetProcessHandleCount(h, &hc))
                        p->handles = hc;
                    else
                        p->handles = (DWORD)-1;
                    if (!p->init) {
                        DWORD_PTR mask = 0, sys = 0;
                        if (GetProcessAffinityMask(h, &mask, &sys))
                            p->affinity = mask;
                    }
                    CloseHandle(h);
                } else {
                    p->handles = (DWORD)-1;
                }
                p->init = 1;
            } else {
                /* "System" style entries we may not open: keep name, zero
                 * everything else so the row still shows. */
                if (p->desc[0] == L'\0')
                    StringCchCopyW(p->desc, _countof(p->desc), p->name);
                p->cpu = 0.0;
            }
        } while (Process32NextW(snap, &pe));
    }
    CloseHandle(snap);
    /* Compact out the dead (also kills PID-reuse ghosts). The fallen go
     * to the "what closed?" ring first. */
    for (i = 0; i < g_procCount;) {
        if (!g_procs[i].alive) {
            tm_dead_push(&g_procs[i]);
            g_procs[i] = g_procs[--g_procCount];
        } else {
            i++;
        }
    }
    return 1;
}

/* --------------------------------------------------------------------------
 * Tab pages: hosts + the Processes and Details lists
 * -------------------------------------------------------------------------- */
static const wchar_t *const TAB_NAMES[TAB_COUNT] = {
    L"Processes", L"Performance", L"Startup apps",
    L"Users", L"Details", L"Services"
};

static const wchar_t *const COLS_PROC[] = {
    L"Name", L"PID", L"Status", L"User name", L"CPU", L"Memory",
    L"I/O total", L"GPU", L"Network total", L"Description"
};
static const int COLW_PROC[] = { 180, 70, 110, 130, 70, 90, 90, 110, 110,
                                 260 };

static const wchar_t *const COLS_DETAIL[] = {
    L"Name", L"PID", L"Status", L"User name", L"CPU", L"Memory",
    L"Parent PID", L"I/O total", L"CPU time", L"Description"
};
static const int COLW_DETAIL[] = { 180, 70, 110, 130, 70, 90, 80, 90, 90,
                                   260 };

static const wchar_t *const COLS_SVC[] = {
    L"Name", L"PID", L"Description", L"Status", L"Startup type"
};
static const int COLW_SVC[] = { 200, 70, 300, 100, 110 };

static const wchar_t *const COLS_STARTUP[] = {
    L"Name", L"Command", L"Location", L"Status"
};
static const int COLW_STARTUP[] = { 220, 420, 200, 90 };

static const wchar_t *const COLS_USERS[] = {
    L"User", L"ID", L"Status", L"Client name"
};
static const int COLW_USERS[] = { 200, 60, 130, 220 };

static void tm_pages_create(void)
{
    TCITEMW ti;
    int i;
    memset(&ti, 0, sizeof(ti));
    ti.mask = TCIF_TEXT;
    for (i = 0; i < TAB_COUNT; i++) {
        HWND host;
        ti.pszText = (LPWSTR)TAB_NAMES[i];
        TabCtrl_InsertItem(g_hwndTab, i, &ti);
        host = CreateWindowExW(0, WC_STATICW, L"",
                               WS_CHILD, 0, 0, 10, 10,
                               g_hwndMain, NULL, g_hInst, NULL);
        g_pages[i] = host;
        g_lv[i] = NULL;
        g_sortCol[i] = 0;
        g_sortAsc[i] = 1;
    }
    g_lv[TAB_PROCESSES] = tm_make_listview(g_pages[TAB_PROCESSES],
                                           IDC_LV_PROC);
    tm_lv_columns(g_lv[TAB_PROCESSES], COLS_PROC, COLW_PROC,
                  _countof(COLW_PROC));
    g_lv[TAB_DETAILS] = tm_make_listview(g_pages[TAB_DETAILS],
                                         IDC_LV_DETAILS);
    tm_lv_columns(g_lv[TAB_DETAILS], COLS_DETAIL, COLW_DETAIL,
                  _countof(COLW_DETAIL));
    /* Every tab owns its list from the start (columns are static), so the
     * layout pass always has real controls to size - no lazy creation. */
    g_lv[TAB_STARTUP] = tm_make_listview(g_pages[TAB_STARTUP],
                                         IDC_LV_STARTUP);
    tm_lv_columns(g_lv[TAB_STARTUP], COLS_STARTUP, COLW_STARTUP,
                  _countof(COLW_STARTUP));
    g_lv[TAB_USERS] = tm_make_listview(g_pages[TAB_USERS], IDC_LV_USERS);
    tm_lv_columns(g_lv[TAB_USERS], COLS_USERS, COLW_USERS,
                  _countof(COLW_USERS));
    g_lv[TAB_SERVICES] = tm_make_listview(g_pages[TAB_SERVICES],
                                          IDC_LV_SERVICES);
    tm_lv_columns(g_lv[TAB_SERVICES], COLS_SVC, COLW_SVC,
                  _countof(COLW_SVC));
}

static void tm_show_tab(int tab)
{
    int i;
    if (tab < 0 || tab >= TAB_COUNT)
        return;
    g_tab = tab;
    TabCtrl_SetCurSel(g_hwndTab, tab);
    for (i = 0; i < TAB_COUNT; i++)
        ShowWindow(g_pages[i], i == tab ? SW_SHOW : SW_HIDE);
    tm_layout();
    tm_refresh_all(1);
}

/* Write one process row. detail=0: Processes layout (I/O, GPU, Network);
 * detail=1: Details layout (Parent PID, I/O). */
static void tm_proc_row(HWND lv, int item, const TM_PROC *p, int detail)
{
    wchar_t b[512], cpu[32], mem[32], io[32], gpu[64], net[32];
    LVITEMW li;
    int col = 0;
    memset(&li, 0, sizeof(li));
    li.mask = LVIF_TEXT | LVIF_PARAM;
    li.iItem = item;
    /* Column 0 via insert-or-set. */
    li.iSubItem = 0;
    li.pszText = (LPWSTR)p->name;
    li.lParam = (LPARAM)(INT_PTR)p->pid;
    if ((int)SendMessageW(lv, LVM_GETITEMCOUNT, 0, 0) <= item)
        ListView_InsertItem(lv, &li);
    else
        ListView_SetItem(lv, &li);
    if (g_groupParent && !detail) {
        LVITEMW gi;
        memset(&gi, 0, sizeof(gi));
        gi.mask = LVIF_GROUPID;
        gi.iItem = item;
        gi.iGroupId = (int)tm_top_ancestor(p->pid);
        ListView_SetItem(lv, &gi);
    }
    StringCchPrintfW(b, _countof(b), L"%lu", p->pid);
    tm_lv_set(lv, item, ++col, b);
    tm_lv_set(lv, item, ++col,
              p->hasMainWnd ? L"Running" : L"Background");
    tm_lv_set(lv, item, ++col, p->user[0] ? p->user : L"");
    StringCchPrintfW(cpu, _countof(cpu), L"%.1f%%", p->cpu);
    tm_lv_set(lv, item, ++col, cpu);
    tm_fmt_bytes(mem, _countof(mem), (ULONGLONG)p->memKB * 1024);
    tm_lv_set(lv, item, ++col, mem);
    if (detail) {
        StringCchPrintfW(b, _countof(b), L"%lu", p->ppid);
        tm_lv_set(lv, item, ++col, b);
    }
    /* I/O column shows live IOPS; byte rates feed the Performance tab. */
    StringCchPrintfW(io, _countof(io), L"%.0f/s",
                     p->iopsR + p->iopsW);
    tm_lv_set(lv, item, ++col, io);
    if (detail) {
        /* Honest "CPU seconds since start" (kernel+user). For autostart
         * apps this is their real CPU burn since logon - the only exact
         * boot-impact number Windows exposes without a boot trace. */
        StringCchPrintfW(b, _countof(b), L"%.1fs",
                         p->cpuTimeTotal / 1e7);
        tm_lv_set(lv, item, ++col, b);
    }
    if (!detail) {
        if (p->gpuDed > 0 || p->gpuShr > 0) {
            wchar_t gd[32], gs[32];
            tm_fmt_bytes(gd, _countof(gd), (ULONGLONG)p->gpuDed);
            tm_fmt_bytes(gs, _countof(gs), (ULONGLONG)p->gpuShr);
            StringCchPrintfW(gpu, _countof(gpu), L"%s + %s sh", gd, gs);
        } else {
            StringCchCopyW(gpu, _countof(gpu), L"n/a");
        }
        tm_lv_set(lv, item, ++col, gpu);
        tm_fmt_bytes(net, _countof(net), p->netIn + p->netOut);
        tm_lv_set(lv, item, ++col, net);
    }
    tm_lv_set(lv, item, ++col, p->desc);
}

/* Merge the fresh snapshot into the visible list: update in place, append
 * births, delete deaths. Keeps selection and scroll position. */
/* Group-by-parent for the Processes list (ListView groups). Rebuilt each
 * refresh from the live snapshot; group id = top ancestor PID. */
static void tm_groups_sync(HWND lv)
{
    size_t i;
    int *have = NULL;
    size_t nHave = 0, cap = 0;
    ListView_EnableGroupView(lv, TRUE);
    ListView_RemoveAllGroups(lv);
    for (i = 0; i < g_procCount; i++) {
        DWORD anc = tm_top_ancestor(g_procs[i].pid);
        size_t k;
        int dup = 0;
        LVGROUP g;
        TM_PROC *ap;
        wchar_t header[300];
        for (k = 0; k < nHave; k++)
            if (have[k] == (int)anc)
                dup = 1;
        if (dup)
            continue;
        if (nHave == cap) {
            size_t nc = cap == 0 ? 64 : cap * 2;
            int *nh = (int *)realloc(have, nc * sizeof(*nh));
            if (!nh)
                break;
            have = nh;
            cap = nc;
        }
        have[nHave++] = (int)anc;
        memset(&g, 0, sizeof(g));
        g.cbSize = sizeof(g);
        g.mask = LVGF_HEADER | LVGF_GROUPID;
        g.iGroupId = (int)anc;
        ap = tm_find_proc(anc);
        if (ap)
            StringCchPrintfW(header, _countof(header), L"%s  (%lu)",
                             ap->name, anc);
        else
            StringCchPrintfW(header, _countof(header), L"PID %lu", anc);
        g.pszHeader = header;
        ListView_InsertGroup(lv, -1, &g);
    }
    free(have);
}

static void tm_procs_refresh(void)
{
    HWND lv = g_lv[TAB_PROCESSES];
    size_t i;
    int n, row;
    char *seen;
    if (!lv)
        return;
    SendMessageW(lv, WM_SETREDRAW, FALSE, 0);
    if (g_groupParent)
        tm_groups_sync(lv);
    else
        ListView_EnableGroupView(lv, FALSE);
    n = ListView_GetItemCount(lv);
    seen = (char *)calloc(n == 0 ? 1 : (size_t)n, 1);
    for (i = 0; i < g_procCount; i++) {
        const TM_PROC *p = &g_procs[i];
        int found = -1, k;
        LVITEMW li;
        for (k = 0; k < n; k++) {
            memset(&li, 0, sizeof(li));
            li.mask = LVIF_PARAM;
            li.iItem = k;
            if (ListView_GetItem(lv, &li) &&
                (DWORD)(INT_PTR)li.lParam == p->pid) {
                found = k;
                break;
            }
        }
        if (found >= 0) {
            tm_proc_row(lv, found, p, 0);
            seen[found] = 1;
        } else {
            row = ListView_GetItemCount(lv);
            tm_proc_row(lv, row, p, 0);
        }
    }
    /* Delete dead rows, bottom-up. */
    n = ListView_GetItemCount(lv);
    for (row = n - 1; row >= 0; row--) {
        LVITEMW li;
        memset(&li, 0, sizeof(li));
        li.mask = LVIF_PARAM;
        li.iItem = row;
        if (ListView_GetItem(lv, &li) &&
            !tm_find_proc((DWORD)(INT_PTR)li.lParam))
            ListView_DeleteItem(lv, row);
        else if (row < n && !seen[row]) {
            /* Row survived the PID check above; keep. */
        }
    }
    free(seen);
    SendMessageW(lv, WM_SETREDRAW, TRUE, 0);
    InvalidateRect(lv, NULL, FALSE);
}

static void tm_details_refresh(void)
{
    /* Details shows the same snapshot with the same merge; the lists are
     * independent controls so a user can sort them differently. */
    HWND save = g_lv[TAB_PROCESSES];
    g_lv[TAB_PROCESSES] = g_lv[TAB_DETAILS];
    tm_procs_refresh();
    g_lv[TAB_PROCESSES] = save;
}

/* --------------------------------------------------------------------------
 * Performance tab: CPU + memory history graphs (GDI) and machine facts
 * -------------------------------------------------------------------------- */
static void tm_perf_cores(void)
{
    SYSTEM_INFO si;
    DWORD len = 0;
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION *buf = NULL;
    DWORD count = 0, cores = 0, i;
    GetSystemInfo(&si);
    g_nLogical = (int)si.dwNumberOfProcessors;
    if (!GetLogicalProcessorInformation(NULL, &len) &&
        GetLastError() == ERROR_INSUFFICIENT_BUFFER && len > 0) {
        buf = (SYSTEM_LOGICAL_PROCESSOR_INFORMATION *)malloc(len);
        if (buf && GetLogicalProcessorInformation(buf, &len)) {
            count = len / sizeof(*buf);
            for (i = 0; i < count; i++)
                if (buf[i].Relationship == RelationProcessorCore)
                    cores++;
            if (cores > 0)
                g_nCores = (int)cores;
        }
        free(buf);
    }
    if (g_nCores <= 0)
        g_nCores = g_nLogical;
}

/* Push one system-wide CPU sample (0..100) into the history ring. */
static void tm_perf_sample(void)
{
    FILETIME idle, kernel, user;
    ULARGE_INTEGER id, ke, us;
    ULONGLONG dIdle, dTotal, dBusy;
    if (!GetSystemTimes(&idle, &kernel, &user))
        return;
    id.LowPart = idle.dwLowDateTime;
    id.HighPart = idle.dwHighDateTime;
    ke.LowPart = kernel.dwLowDateTime;
    ke.HighPart = kernel.dwHighDateTime;
    us.LowPart = user.dwLowDateTime;
    us.HighPart = user.dwHighDateTime;
    if (g_prevKernel != 0) {
        dIdle = id.QuadPart - g_prevIdle;
        dTotal = (ke.QuadPart - g_prevKernel) +
                 (us.QuadPart - g_prevUser);
        if (dTotal > 0) {
            dBusy = dTotal > dIdle ? dTotal - dIdle : 0;
            g_cpuTotal = 100.0 * (double)dBusy / (double)dTotal;
        }
    }
    g_prevIdle = id.QuadPart;
    g_prevKernel = ke.QuadPart;
    g_prevUser = us.QuadPart;
}

/* GDI history graph. kind 0 = CPU (green), 1 = memory (orange). */
static void tm_perf_draw(HWND hwnd, HDC hdc, int isMem)
{
    RECT rc;
    HDC mem;
    HBITMAP bmp, old;
    HBRUSH bg, grid;
    HPEN line;
    int x, n;
    GetClientRect(hwnd, &rc);
    mem = CreateCompatibleDC(hdc);
    bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
    old = (HBITMAP)SelectObject(mem, bmp);
    bg = CreateSolidBrush(g_dark ? RGB(LOOS_BRAND_SURFACE_R, LOOS_BRAND_SURFACE_G, LOOS_BRAND_SURFACE_B) : RGB(255, 255, 255));
    FillRect(mem, &rc, bg);
    DeleteObject(bg);
    grid = CreateSolidBrush(g_dark ? RGB(48, 48, 48) : RGB(220, 220, 220));
    {
        RECT g = rc;
        int gx;
        for (gx = g.left; gx < g.right; gx += (rc.right / 10) + 1) {
            RECT v = { gx, g.top, gx + 1, g.bottom };
            FillRect(mem, &v, grid);
        }
    }
    DeleteObject(grid);
    line = CreatePen(PS_SOLID, 2,
                     isMem ? RGB(255, 140, 0) : RGB(0, 160, 0));
    SelectObject(mem, line);
    n = g_histCount < TM_HIST_N ? g_histCount : TM_HIST_N;
    MoveToEx(mem, rc.right, rc.bottom, NULL);
    for (x = 0; x < n; x++) {
        double v = isMem ? g_memHist[(g_histCount - 1 - x + TM_HIST_N * 4) %
                                     TM_HIST_N]
                         : g_cpuHist[(g_histCount - 1 - x + TM_HIST_N * 4) %
                                     TM_HIST_N];
        int px = rc.right - 1 - (x * (rc.right - 1)) / (TM_HIST_N - 1);
        int py = rc.bottom - 1 - (int)((v / 100.0) * (rc.bottom - 4));
        if (x == 0)
            MoveToEx(mem, px, py, NULL);
        else
            LineTo(mem, px, py);
    }
    DeleteObject(line);
    BitBlt(hdc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old);
    DeleteObject(bmp);
    DeleteDC(mem);
}

static LRESULT CALLBACK tm_graph_proc(HWND hwnd, UINT msg, WPARAM wParam,
                                      LPARAM lParam, UINT_PTR kind)
{
    if (msg == WM_PAINT) {
        PAINTSTRUCT ps;
        HDC hdc = BeginPaint(hwnd, &ps);
        tm_perf_draw(hwnd, hdc, (int)kind);
        EndPaint(hwnd, &ps);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

static LRESULT CALLBACK tm_graph_cpu_proc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    return tm_graph_proc(h, m, w, l, 0);
}

static LRESULT CALLBACK tm_graph_mem_proc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    return tm_graph_proc(h, m, w, l, 1);
}

static void tm_perf_refresh(void)
{
    MEMORYSTATUSEX ms;
    PERFORMANCE_INFORMATION pi;
    wchar_t info[1024], mem[64], up[64], procs[64], threads[64], handles[64];
    wchar_t disk[32], net[32], vpn[32], lan[32], npu[128];
    ms.dwLength = sizeof(ms);
    tm_perf_sample();
    if (g_histCount < INT_MAX)
        g_histCount++;
    if (g_histCount > 0) {
        g_cpuHist[(g_histCount - 1) % TM_HIST_N] = g_cpuTotal;
    }
    if (GlobalMemoryStatusEx(&ms) && ms.ullTotalPhys > 0) {
        g_memHist[(g_histCount - 1) % TM_HIST_N] =
            100.0 * (double)(ms.ullTotalPhys - ms.ullAvailPhys) /
            (double)ms.ullTotalPhys;
        tm_fmt_bytes(mem, _countof(mem),
                     ms.ullTotalPhys - ms.ullAvailPhys);
    } else {
        StringCchCopyW(mem, _countof(mem), L"?");
    }
    tm_uptime(up, _countof(up));
    tm_fmt_bytes(disk, _countof(disk), (ULONGLONG)g_diskBps);
    tm_fmt_bytes(net, _countof(net), (ULONGLONG)g_netBps);
    tm_fmt_bytes(vpn, _countof(vpn), (ULONGLONG)g_vpnBps);
    tm_fmt_bytes(lan, _countof(lan), (ULONGLONG)g_lanBps);
    procs[0] = threads[0] = handles[0] = L'\0';
    if (GetPerformanceInfo(&pi, sizeof(pi))) {
        tm_comma(procs, _countof(procs), pi.ProcessCount);
        tm_comma(threads, _countof(threads), pi.ThreadCount);
        tm_comma(handles, _countof(handles), pi.HandleCount);
    }
    StringCchPrintfW(npu, _countof(npu),
                     L"NPU: %s (per-model breakdown needs vendor driver)",
                     tm_npu_present() ? L"present" : L"not detected");
    StringCchPrintfW(info, _countof(info),
                     L"CPU  %.1f%%   (%d cores, %d logical)\r\n"
                     L"In use  %s\r\n"
                     L"Disk  %s/s    Net  %s/s   (VPN %s/s, LAN %s/s)\r\n"
                     L"%s\r\n"
                     L"Up time  %s\r\n"
                     L"Processes  %s    Threads  %s    Handles  %s",
                     g_cpuTotal, g_nCores, g_nLogical, mem,
                     disk, net, vpn, lan, npu, up,
                     procs[0] ? procs : L"?",
                     threads[0] ? threads : L"?",
                     handles[0] ? handles : L"?");
    if (g_perfInfo)
        SetWindowTextW(g_perfInfo, info);
    if (g_graphCpu)
        InvalidateRect(g_graphCpu, NULL, FALSE);
    if (g_graphMem)
        InvalidateRect(g_graphMem, NULL, FALSE);
}

/* --------------------------------------------------------------------------
 * Services tab (EnumServicesStatusExW + ControlService)
 * -------------------------------------------------------------------------- */
/* (column tables live with COLS_PROC near tm_pages_create) */
static const wchar_t *tm_svc_state(DWORD s)
{
    switch (s) {
    case SERVICE_STOPPED:
        return L"Stopped";
    case SERVICE_START_PENDING:
        return L"Starting";
    case SERVICE_STOP_PENDING:
        return L"Stopping";
    case SERVICE_RUNNING:
        return L"Running";
    case SERVICE_CONTINUE_PENDING:
        return L"Resuming";
    case SERVICE_PAUSE_PENDING:
        return L"Pausing";
    case SERVICE_PAUSED:
        return L"Paused";
    default:
        return L"Unknown";
    }
}

static const wchar_t *tm_svc_startup(DWORD s)
{
    switch (s) {
    case SERVICE_AUTO_START:
        return L"Automatic";
    case SERVICE_BOOT_START:
        return L"Boot";
    case SERVICE_SYSTEM_START:
        return L"System";
    case SERVICE_DEMAND_START:
        return L"Manual";
    case SERVICE_DISABLED:
        return L"Disabled";
    default:
        return L"Unknown";
    }
}

static void tm_services_refresh(void)
{
    ENUM_SERVICE_STATUS_PROCESSW *svcs = NULL;
    DWORD need = 0, count = 0, resume = 0;
    SC_HANDLE scm;
    DWORD i;
    HWND lv = g_lv[TAB_SERVICES];
    if (!lv)
        return;
    SendMessageW(lv, WM_SETREDRAW, FALSE, 0);
    ListView_DeleteAllItems(lv);
    scm = OpenSCManagerW(NULL, NULL, SC_MANAGER_ENUMERATE_SERVICE);
    if (!scm) {
        tm_error(g_hwndMain, L"Cannot open the service control manager.");
        SendMessageW(lv, WM_SETREDRAW, TRUE, 0);
        return;
    }
    EnumServicesStatusExW(scm, SC_ENUM_PROCESS_INFO, SERVICE_WIN32,
                          SERVICE_STATE_ALL, NULL, 0, &need, &count,
                          &resume, NULL);
    if (GetLastError() != ERROR_MORE_DATA || need == 0) {
        CloseServiceHandle(scm);
        SendMessageW(lv, WM_SETREDRAW, TRUE, 0);
        return;
    }
    svcs = (ENUM_SERVICE_STATUS_PROCESSW *)malloc(need);
    if (!svcs) {
        CloseServiceHandle(scm);
        SendMessageW(lv, WM_SETREDRAW, TRUE, 0);
        return;
    }
    resume = 0;
    if (!EnumServicesStatusExW(scm, SC_ENUM_PROCESS_INFO, SERVICE_WIN32,
                               SERVICE_STATE_ALL, (LPBYTE)svcs, need,
                               &need, &count, &resume, NULL)) {
        free(svcs);
        CloseServiceHandle(scm);
        SendMessageW(lv, WM_SETREDRAW, TRUE, 0);
        return;
    }
    for (i = 0; i < count; i++) {
        SC_HANDLE svc;
        wchar_t pid[32], desc[260];
        QUERY_SERVICE_CONFIGW *cfg = NULL;
        DWORD cfgNeed = 0;
        LVITEMW li;
        int row;
        memset(&li, 0, sizeof(li));
        li.mask = LVIF_TEXT | LVIF_PARAM;
        li.iItem = (int)i;
        li.pszText = svcs[i].lpServiceName;
        li.lParam = (LPARAM)i;
        row = ListView_InsertItem(lv, &li);
        StringCchPrintfW(pid, _countof(pid), L"%lu",
                         svcs[i].ServiceStatusProcess.dwProcessId);
        tm_lv_set(lv, row, 1, pid);
        /* Description + startup type need the service key. */
        desc[0] = L'\0';
        svc = OpenServiceW(scm, svcs[i].lpServiceName, SERVICE_QUERY_CONFIG);
        if (svc) {
            QueryServiceConfigW(svc, NULL, 0, &cfgNeed);
            if (GetLastError() == ERROR_INSUFFICIENT_BUFFER && cfgNeed > 0) {
                cfg = (QUERY_SERVICE_CONFIGW *)malloc(cfgNeed);
                if (cfg && QueryServiceConfigW(svc, cfg, cfgNeed,
                                               &cfgNeed)) {
                    if (cfg->lpDisplayName)
                        tm_lv_set(lv, row, 0,
                                             cfg->lpDisplayName);
                    if (cfg->lpBinaryPathName) {
                        StringCchCopyW(desc, _countof(desc),
                                       cfg->lpBinaryPathName);
                        tm_lv_set(lv, row, 2, desc);
                    }
                    tm_lv_set(
                        lv, row, 4,
                        (LPWSTR)tm_svc_startup(cfg->dwStartType));
                }
                free(cfg);
            }
            CloseServiceHandle(svc);
        }
        tm_lv_set(
            lv, row, 3,
            (LPWSTR)tm_svc_state(svcs[i].ServiceStatusProcess.dwCurrentState));
    }
    free(svcs);
    CloseServiceHandle(scm);
    SendMessageW(lv, WM_SETREDRAW, TRUE, 0);
    InvalidateRect(lv, NULL, FALSE);
}

static void tm_svc_command(HWND lv, int cmd)
{
    int sel = ListView_GetNextItem(lv, -1, LVNI_SELECTED);
    wchar_t name[256];
    SC_HANDLE scm, svc;
    DWORD access = SERVICE_QUERY_STATUS;
    if (sel < 0)
        return;
    tm_lv_text(lv, sel, 0, name, _countof(name));
    /* Column 0 shows the display name; resolve it back via GetServiceKeyName.
     * We stored the true name in the row text only when no display name
     * existed, so look the service up by display name first. */
    scm = OpenSCManagerW(NULL, NULL, SC_MANAGER_CONNECT);
    if (!scm) {
        tm_error(g_hwndMain, L"Cannot open the service control manager.");
        return;
    }
    {
        wchar_t key[256];
        DWORD keyLen = _countof(key);
        if (GetServiceKeyNameW(scm, name, key, &keyLen))
            StringCchCopyW(name, _countof(name), key);
    }
    if (cmd == IDM_SVC_START)
        access |= SERVICE_START;
    else if (cmd == IDM_SVC_STOP)
        access |= SERVICE_STOP;
    else
        access |= SERVICE_PAUSE_CONTINUE;
    svc = OpenServiceW(scm, name, access);
    if (!svc) {
        tm_error(g_hwndMain, L"Cannot open the service.");
        CloseServiceHandle(scm);
        return;
    }
    if (cmd == IDM_SVC_START) {
        if (!StartServiceW(svc, 0, NULL))
            tm_error(g_hwndMain, L"Cannot start the service.");
    } else {
        SERVICE_STATUS st;
        DWORD code = SERVICE_CONTROL_STOP;
        if (cmd == IDM_SVC_PAUSE)
            code = SERVICE_CONTROL_PAUSE;
        else if (cmd == IDM_SVC_CONTINUE)
            code = SERVICE_CONTROL_CONTINUE;
        if (!ControlService(svc, code, &st))
            tm_error(g_hwndMain, L"Cannot control the service.");
    }
    CloseServiceHandle(svc);
    CloseServiceHandle(scm);
    tm_services_refresh();
}

/* --------------------------------------------------------------------------
 * Startup apps tab (Run keys + Startup folders, with enable/disable)
 * -------------------------------------------------------------------------- */
/* (column tables live with COLS_PROC near tm_pages_create) */
static const wchar_t *const RUN_KEYS[][2] = {
    { L"Software\\Microsoft\\Windows\\CurrentVersion\\Run",
      L"HKLM\\...\\Run" },
    { L"Software\\Microsoft\\Windows\\CurrentVersion\\Run",
      L"HKCU\\...\\Run" },
    { L"Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Run",
      L"HKLM\\...\\Run (32-bit)" },
};

static void tm_startup_add(HWND lv, int *row, const wchar_t *name,
                           const wchar_t *cmd, const wchar_t *loc,
                           int enabled)
{
    LVITEMW li;
    memset(&li, 0, sizeof(li));
    li.mask = LVIF_TEXT | LVIF_PARAM;
    li.iItem = *row;
    li.pszText = (LPWSTR)name;
    li.lParam = (LPARAM)(*row);
    *row = ListView_InsertItem(lv, &li);
    tm_lv_set(lv, *row, 1, (LPWSTR)(cmd ? cmd : L""));
    tm_lv_set(lv, *row, 2, (LPWSTR)loc);
    tm_lv_set(lv, *row, 3,
                         enabled ? L"Enabled" : (LPWSTR)L"Disabled");
    (*row)++;
}

static void tm_startup_read_key(HWND lv, int *row, HKEY hive,
                                const wchar_t *sub, const wchar_t *loc)
{
    HKEY h, hd;
    DWORD i = 0;
    /* NOTE: disabled values live in "<sub>\LoOS-Disabled"; opened below. */
    if (RegOpenKeyExW(hive, sub, 0, KEY_READ, &h) == ERROR_SUCCESS) {
        wchar_t name[256], val[1024];
        DWORD nc, vc, type;
        for (;;) {
            nc = _countof(name);
            vc = sizeof(val);
            if (RegEnumValueW(h, i++, name, &nc, NULL, &type,
                              (LPBYTE)val, &vc) != ERROR_SUCCESS)
                break;
            if (type == REG_SZ || type == REG_EXPAND_SZ)
                tm_startup_add(lv, row, name, val, loc, 1);
        }
        RegCloseKey(h);
    }
    {
        wchar_t dis[320];
        StringCchPrintfW(dis, _countof(dis), L"%s\\LoOS-Disabled", sub);
        if (RegOpenKeyExW(hive, dis, 0, KEY_READ, &hd) == ERROR_SUCCESS) {
            wchar_t name[256], val[1024];
            DWORD nc, vc, type;
            i = 0;
            for (;;) {
                nc = _countof(name);
                vc = sizeof(val);
                if (RegEnumValueW(hd, i++, name, &nc, NULL, &type,
                                  (LPBYTE)val, &vc) != ERROR_SUCCESS)
                    break;
                if (type == REG_SZ || type == REG_EXPAND_SZ)
                    tm_startup_add(lv, row, name, val, loc, 0);
            }
            RegCloseKey(hd);
        }
    }
}

static void tm_startup_read_folder(HWND lv, int *row, int common)
{
    wchar_t dir[MAX_PATH], pat[MAX_PATH];
    WIN32_FIND_DATAW fd;
    HANDLE h;
    if (FAILED(SHGetFolderPathW(NULL,
                                common ? CSIDL_COMMON_STARTUP : CSIDL_STARTUP,
                                NULL, 0, dir)))
        return;
    StringCchPrintfW(pat, _countof(pat), L"%s\\*", dir);
    h = FindFirstFileW(pat, &fd);
    if (h == INVALID_HANDLE_VALUE)
        return;
    do {
        if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
            continue;
        if (lstrcmpiW(fd.cFileName, L"desktop.ini") == 0)
            continue;
        {
            wchar_t full[MAX_PATH];
            StringCchPrintfW(full, _countof(full), L"%s\\%s", dir,
                             fd.cFileName);
            tm_startup_add(lv, row, fd.cFileName, full,
                           common ? L"Common Startup" : L"Startup folder",
                           1);
        }
    } while (FindNextFileW(h, &fd));
    FindClose(h);
}

static void tm_startup_refresh(void)
{
    HWND lv = g_lv[TAB_STARTUP];
    int row = 0;
    if (!lv)
        return;
    SendMessageW(lv, WM_SETREDRAW, FALSE, 0);
    ListView_DeleteAllItems(lv);
    tm_startup_read_key(lv, &row, HKEY_LOCAL_MACHINE, RUN_KEYS[0][0],
                        RUN_KEYS[0][1]);
    tm_startup_read_key(lv, &row, HKEY_CURRENT_USER, RUN_KEYS[1][0],
                        RUN_KEYS[1][1]);
    tm_startup_read_key(lv, &row, HKEY_LOCAL_MACHINE, RUN_KEYS[2][0],
                        RUN_KEYS[2][1]);
    /* RunOnce (runs once at next logon, then self-deletes). */
    tm_startup_read_key(lv, &row, HKEY_LOCAL_MACHINE,
                        L"Software\\Microsoft\\Windows\\CurrentVersion\\"
                        L"RunOnce",
                        L"HKLM\\...\\RunOnce");
    tm_startup_read_key(lv, &row, HKEY_CURRENT_USER,
                        L"Software\\Microsoft\\Windows\\CurrentVersion\\"
                        L"RunOnce",
                        L"HKCU\\...\\RunOnce");
    /* Winlogon Shell/Userinit (read-only here: editing these wrong
     * bricks logon, so they are shown for visibility only). */
    {
        HKEY h;
        wchar_t shell[512], userinit[512];
        DWORD cb;
        if (RegOpenKeyExW(HKEY_LOCAL_MACHINE,
                          L"Software\\Microsoft\\Windows NT\\"
                          L"CurrentVersion\\Winlogon",
                          0, KEY_READ, &h) == ERROR_SUCCESS) {
            cb = sizeof(shell);
            if (RegQueryValueExW(h, L"Shell", NULL, NULL, (LPBYTE)shell,
                                 &cb) == ERROR_SUCCESS &&
                shell[0])
                tm_startup_add(lv, &row, L"Winlogon Shell", shell,
                               L"Winlogon (read-only)", 1);
            cb = sizeof(userinit);
            if (RegQueryValueExW(h, L"Userinit", NULL, NULL,
                                 (LPBYTE)userinit,
                                 &cb) == ERROR_SUCCESS &&
                userinit[0])
                tm_startup_add(lv, &row, L"Winlogon Userinit", userinit,
                               L"Winlogon (read-only)", 1);
            RegCloseKey(h);
        }
    }
    tm_startup_read_folder(lv, &row, 0);
    tm_startup_read_folder(lv, &row, 1);
    SendMessageW(lv, WM_SETREDRAW, TRUE, 0);
    InvalidateRect(lv, NULL, FALSE);
}

/* Enable/disable by moving the value to/from "<key>\\LoOS-Disabled". */
static void tm_startup_toggle(HWND lv)
{
    int sel = ListView_GetNextItem(lv, -1, LVNI_SELECTED);
    wchar_t name[256], loc[260], status[32];
    HKEY hive;
    const wchar_t *sub = NULL;
    int i, enabled;
    if (sel < 0)
        return;
    tm_lv_text(lv, sel, 0, name, _countof(name));
    tm_lv_text(lv, sel, 2, loc, _countof(loc));
    tm_lv_text(lv, sel, 3, status, _countof(status));
    enabled = (lstrcmpiW(status, L"Enabled") == 0);
    hive = wcsstr(loc, L"HKCU") ? HKEY_CURRENT_USER : HKEY_LOCAL_MACHINE;
    for (i = 0; i < 3; i++)
        if (lstrcmpW(loc, RUN_KEYS[i][1]) == 0)
            sub = RUN_KEYS[i][0];
    if (!sub) {
        MessageBoxW(g_hwndMain,
                    L"This entry cannot be toggled here.\n"
                    L"Startup-folder entries live in Explorer; Winlogon "
                    L"values are read-only (editing them can brick logon).",
                    S_APP, MB_OK | MB_ICONINFORMATION);
        return;
    }
    {
        HKEY hSrc, hDst;
        wchar_t dis[320], val[1024];
        DWORD vc = sizeof(val), type = REG_SZ;
        const wchar_t *src = enabled ? sub : NULL;
        wchar_t dstBuf[320];
        StringCchPrintfW(dis, _countof(dis), L"%s\\LoOS-Disabled", sub);
        if (enabled) {
            StringCchCopyW(dstBuf, _countof(dstBuf), dis);
        } else {
            src = dis;
            StringCchCopyW(dstBuf, _countof(dstBuf), sub);
        }
        if (RegOpenKeyExW(hive, src, 0, KEY_READ | KEY_SET_VALUE,
                          &hSrc) != ERROR_SUCCESS) {
            tm_error(g_hwndMain, L"Cannot open the Run key.");
            return;
        }
        if (RegCreateKeyExW(hive, dstBuf, 0, NULL, 0,
                            KEY_READ | KEY_SET_VALUE, NULL, &hDst,
                            NULL) != ERROR_SUCCESS) {
            RegCloseKey(hSrc);
            tm_error(g_hwndMain, L"Cannot open the disabled store.");
            return;
        }
        if (RegQueryValueExW(hSrc, name, NULL, &type, (LPBYTE)val,
                             &vc) != ERROR_SUCCESS) {
            RegCloseKey(hSrc);
            RegCloseKey(hDst);
            return;
        }
        RegSetValueExW(hDst, name, 0, type, (const BYTE *)val, vc);
        RegDeleteValueW(hSrc, name);
        RegCloseKey(hSrc);
        RegCloseKey(hDst);
        tm_debug(L"startup %s: %s", enabled ? L"disabled" : L"enabled",
                 name);
    }
    tm_startup_refresh();
}

/* --------------------------------------------------------------------------
 * Users tab (WTS sessions)
 * -------------------------------------------------------------------------- */
/* (column tables live with COLS_PROC near tm_pages_create) */
static const wchar_t *tm_wts_state(WTS_CONNECTSTATE_CLASS s)
{
    switch (s) {
    case WTSActive:
        return L"Active";
    case WTSConnected:
        return L"Connected";
    case WTSConnectQuery:
        return L"ConnectQuery";
    case WTSShadow:
        return L"Shadow";
    case WTSDisconnected:
        return L"Disconnected";
    case WTSIdle:
        return L"Idle";
    case WTSListen:
        return L"Listen";
    case WTSReset:
        return L"Reset";
    case WTSDown:
        return L"Down";
    case WTSInit:
        return L"Init";
    default:
        return L"Unknown";
    }
}

static void tm_users_refresh(void)
{
    HWND lv = g_lv[TAB_USERS];
    WTS_SESSION_INFOW *si = NULL;
    DWORD count = 0, i;
    if (!lv)
        return;
    SendMessageW(lv, WM_SETREDRAW, FALSE, 0);
    ListView_DeleteAllItems(lv);
    if (WTSEnumerateSessionsW(WTS_CURRENT_SERVER_HANDLE, 0, 1, &si,
                              &count)) {
        for (i = 0; i < count; i++) {
            wchar_t *user = NULL, *client = NULL;
            DWORD ret = 0;
            wchar_t id[32];
            LVITEMW li;
            int row;
            WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE,
                                        si[i].SessionId, WTSUserName,
                                        &user, &ret);
            WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE,
                                        si[i].SessionId, WTSClientName,
                                        &client, &ret);
            memset(&li, 0, sizeof(li));
            li.mask = LVIF_TEXT | LVIF_PARAM;
            li.iItem = (int)i;
            li.pszText = (user && user[0]) ? user : si[i].pWinStationName;
            li.lParam = (LPARAM)(INT_PTR)si[i].SessionId;
            row = ListView_InsertItem(lv, &li);
            StringCchPrintfW(id, _countof(id), L"%lu", si[i].SessionId);
            tm_lv_set(lv, row, 1, id);
            tm_lv_set(lv, row, 2,
                                 (LPWSTR)tm_wts_state(si[i].State));
            tm_lv_set(lv, row, 3,
                                 (client && client[0]) ? client : L"");
            if (user)
                WTSFreeMemory(user);
            if (client)
                WTSFreeMemory(client);
        }
        WTSFreeMemory(si);
    }
    SendMessageW(lv, WM_SETREDRAW, TRUE, 0);
    InvalidateRect(lv, NULL, FALSE);
}

static void tm_user_command(HWND lv, int cmd)
{
    int session = tm_lv_selected_pid(lv);
    if (session < 0)
        return;
    if (cmd == IDM_USER_LOGOFF) {
        wchar_t msg[256], user[130];
        int sel = ListView_GetNextItem(lv, -1, LVNI_SELECTED);
        tm_lv_text(lv, sel, 0, user, _countof(user));
        StringCchPrintfW(msg, _countof(msg),
                         L"Log off \"%s\" (session %d)?", user, session);
        if (MessageBoxW(g_hwndMain, msg, S_APP,
                        MB_YESNO | MB_ICONQUESTION) != IDYES)
            return;
        if (!WTSLogoffSession(WTS_CURRENT_SERVER_HANDLE, (DWORD)session,
                              FALSE))
            tm_error(g_hwndMain, L"Cannot log off the session.");
    } else if (cmd == IDM_USER_DISCONNECT) {
        if (!WTSDisconnectSession(WTS_CURRENT_SERVER_HANDLE,
                                  (DWORD)session, FALSE))
            tm_error(g_hwndMain, L"Cannot disconnect the session.");
    } else {
        tm_sendmsg_dialog(g_hwndMain, (DWORD)session);
        return;
    }
    tm_users_refresh();
}

/* --------------------------------------------------------------------------
 * Actions: End Task, Run, Find, Priority, Affinity, Go-to-details
 * -------------------------------------------------------------------------- */
static void tm_end_task(HWND lv)
{
    int pid = tm_lv_selected_pid(lv);
    TM_PROC *p;
    wchar_t msg[512], name[130];
    int sel;
    HWND wnd;
    if (pid < 0)
        return;
    p = tm_find_proc((DWORD)pid);
    sel = ListView_GetNextItem(lv, -1, LVNI_SELECTED);
    tm_lv_text(lv, sel, 0, name, _countof(name));
    StringCchPrintfW(msg, _countof(msg), S_ENDTASK_MSG, name);
    if (MessageBoxW(g_hwndMain, msg, S_ENDTASK_CAP,
                    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES)
        return;
    /* Graceful first: ask the main window to close (classic EndTask path:
     * IsHungAppWindow + EndTask + SendMessageTimeout in the original). */
    wnd = tm_proc_mainwnd((DWORD)pid);
    if (wnd != NULL) {
        DWORD_PTR res = 0;
        if (IsHungAppWindow(wnd))
            EndTask(wnd, FALSE, TRUE);
        else
            SendMessageTimeoutW(wnd, WM_CLOSE, 0, 0,
                                SMTO_ABORTIFHUNG, 5000, &res);
        Sleep(1200);
        if (!tm_find_proc((DWORD)pid) ||
            tm_proc_mainwnd((DWORD)pid) == NULL) {
            tm_debug(L"end task: %d closed gracefully", pid);
            tm_refresh_all(1);
            return;
        }
    }
    {
        HANDLE h = OpenProcess(PROCESS_TERMINATE, FALSE, (DWORD)pid);
        if (!h) {
            tm_error(g_hwndMain, L"Cannot open the process.");
            return;
        }
        if (!TerminateProcess(h, 1))
            tm_error(g_hwndMain, L"Cannot end the process.");
        else
            tm_debug(L"end task: %d terminated", pid);
        CloseHandle(h);
    }
    if (p)
        (void)p;
    tm_refresh_all(1);
}

static void tm_run_dialog(HWND owner)
{
    DialogBoxW(g_hInst, MAKEINTRESOURCEW(IDD_RUN), owner, tm_run_proc);
}

/* Find: select the first visible row whose name contains the text. */
static void tm_find_apply(HWND lv, const wchar_t *text)
{
    int n = ListView_GetItemCount(lv), i;
    wchar_t name[MAX_PATH];
    for (i = 0; i < n; i++) {
        tm_lv_text(lv, i, 0, name, _countof(name));
        /* Case-insensitive substring. */
        {
            wchar_t *h = NULL;
            wchar_t ln[MAX_PATH], lt[256];
            size_t k;
            for (k = 0; k < _countof(ln) - 1 && name[k]; k++)
                ln[k] = towlower(name[k]);
            ln[k] = L'\0';
            for (k = 0; k < _countof(lt) - 1 && text[k]; k++)
                lt[k] = towlower(text[k]);
            lt[k] = L'\0';
            h = wcsstr(ln, lt);
            if (h) {
                ListView_SetItemState(lv, i,
                                      LVIS_SELECTED | LVIS_FOCUSED,
                                      LVIS_SELECTED | LVIS_FOCUSED);
                ListView_EnsureVisible(lv, i, FALSE);
                SetFocus(lv);
                return;
            }
        }
    }
    MessageBoxW(g_hwndMain, L"No matching process.", S_APP,
                MB_OK | MB_ICONINFORMATION);
}

static void tm_find_dialog(HWND owner)
{
    HWND lv = (g_tab == TAB_DETAILS) ? g_lv[TAB_DETAILS]
                                     : g_lv[TAB_PROCESSES];
    /* Stash the target list in the dialog via GWLP_USERDATA. */
    DialogBoxParamW(g_hInst, MAKEINTRESOURCEW(IDD_FIND), owner,
                    tm_find_proc_dlg, (LPARAM)(INT_PTR)lv);
}

static void tm_set_priority(HWND lv, DWORD prioClass)
{
    int pid = tm_lv_selected_pid(lv);
    HANDLE h;
    if (pid < 0)
        return;
    h = OpenProcess(PROCESS_SET_INFORMATION, FALSE, (DWORD)pid);
    if (!h) {
        tm_error(g_hwndMain, L"Cannot open the process.");
        return;
    }
    if (!SetPriorityClass(h, prioClass))
        tm_error(g_hwndMain, L"Cannot set the priority.");
    else {
        TM_PROC *p = tm_find_proc((DWORD)pid);
        if (p)
            p->prioClass = prioClass;
    }
    CloseHandle(h);
}

static void tm_goto_details(DWORD pid)
{
    tm_show_tab(TAB_DETAILS);
    tm_lv_select_pid(g_lv[TAB_DETAILS], pid);
}

/* --- Run dialog --- */
static INT_PTR CALLBACK tm_run_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    (void)l;
    if (msg == WM_INITDIALOG) {
        SetFocus(GetDlgItem(dlg, IDE_RUN_FILE));
        return FALSE;
    }
    if (msg == WM_COMMAND) {
        WORD id = LOWORD(w);
        if (id == IDOK) {
            wchar_t cmd[MAX_PATH * 2];
            STARTUPINFOW si;
            PROCESS_INFORMATION pi;
            GetDlgItemTextW(dlg, IDE_RUN_FILE, cmd, _countof(cmd));
            if (!cmd[0]) {
                EndDialog(dlg, IDCANCEL);
                return TRUE;
            }
            memset(&si, 0, sizeof(si));
            si.cb = sizeof(si);
            memset(&pi, 0, sizeof(pi));
            if (!CreateProcessW(NULL, cmd, NULL, NULL, FALSE, 0, NULL,
                                NULL, &si, &pi)) {
                tm_error(dlg, L"Cannot start the program.");
                return TRUE;
            }
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
            EndDialog(dlg, IDOK);
            return TRUE;
        }
        if (id == IDCANCEL) {
            EndDialog(dlg, IDCANCEL);
            return TRUE;
        }
    }
    return FALSE;
}

/* --- Find dialog --- */
static INT_PTR CALLBACK tm_find_proc_dlg(HWND dlg, UINT msg, WPARAM w,
                                         LPARAM l)
{
    static HWND s_lv;
    if (msg == WM_INITDIALOG) {
        s_lv = (HWND)(INT_PTR)l;
        SetFocus(GetDlgItem(dlg, IDE_FIND_TEXT));
        return FALSE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDOK) {
        wchar_t text[256];
        GetDlgItemTextW(dlg, IDE_FIND_TEXT, text, _countof(text));
        if (text[0] && s_lv)
            tm_find_apply(s_lv, text);
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDCANCEL) {
        EndDialog(dlg, IDCANCEL);
        return TRUE;
    }
    return FALSE;
}

/* --- Affinity dialog: multi-select listbox filled with CPU 0..N-1 --- */
struct TM_AFF_CTX {
    DWORD pid;
    DWORD_PTR mask;
    DWORD_PTR sys;
};

static INT_PTR CALLBACK tm_affinity_proc(HWND dlg, UINT msg, WPARAM w,
                                         LPARAM l)
{
    static struct TM_AFF_CTX s_ctx;
    if (msg == WM_INITDIALOG) {
        struct TM_AFF_CTX *c = (struct TM_AFF_CTX *)(INT_PTR)l;
        HWND lb;
        int i;
        s_ctx = *c;
        lb = GetDlgItem(dlg, IDL_AFFINITY);
        /* Plain listboxes cannot disable single items, so only system-legal
         * CPUs are listed; the real CPU number rides in the item data. */
        for (i = 0; i < g_nLogical; i++) {
            wchar_t b[32];
            int idx;
            if (!((s_ctx.sys >> i) & 1))
                continue;
            StringCchPrintfW(b, _countof(b), L"CPU %d", i);
            idx = (int)SendMessageW(lb, LB_ADDSTRING, 0,
                                    (LPARAM)(INT_PTR)b);
            SendMessageW(lb, LB_SETITEMDATA, (WPARAM)idx, (LPARAM)i);
            SendMessageW(lb, LB_SETSEL, (s_ctx.mask >> i) & 1 ? TRUE : FALSE,
                         idx);
        }
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDOK) {
        HWND lb = GetDlgItem(dlg, IDL_AFFINITY);
        DWORD_PTR mask = 0;
        int n, i;
        HANDLE h;
        n = (int)SendMessageW(lb, LB_GETCOUNT, 0, 0);
        for (i = 0; i < n; i++)
            if (SendMessageW(lb, LB_GETSEL, (WPARAM)i, 0) > 0)
                mask |= ((DWORD_PTR)1 <<
                         (int)SendMessageW(lb, LB_GETITEMDATA, (WPARAM)i,
                                           0));
        mask &= s_ctx.sys;
        if (mask == 0) {
            MessageBoxW(dlg, L"Select at least one processor.", S_APP,
                        MB_OK | MB_ICONINFORMATION);
            return TRUE;
        }
        h = OpenProcess(PROCESS_SET_INFORMATION, FALSE, s_ctx.pid);
        if (!h) {
            tm_error(dlg, L"Cannot open the process.");
            return TRUE;
        }
        if (!SetProcessAffinityMask(h, mask))
            tm_error(dlg, L"Cannot set the affinity.");
        CloseHandle(h);
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDCANCEL) {
        EndDialog(dlg, IDCANCEL);
        return TRUE;
    }
    return FALSE;
}

static void tm_affinity_dialog(HWND owner, DWORD pid)
{
    HANDLE h = OpenProcess(PROCESS_QUERY_INFORMATION |
                           PROCESS_SET_INFORMATION, FALSE, pid);
    struct TM_AFF_CTX ctx;
    if (!h) {
        tm_error(owner, L"Cannot open the process.");
        return;
    }
    ctx.pid = pid;
    ctx.mask = 0;
    ctx.sys = 0;
    if (!GetProcessAffinityMask(h, &ctx.mask, &ctx.sys)) {
        tm_error(owner, L"Cannot query the affinity.");
        CloseHandle(h);
        return;
    }
    CloseHandle(h);
    DialogBoxParamW(g_hInst, MAKEINTRESOURCEW(IDD_AFFINITY), owner,
                    tm_affinity_proc, (LPARAM)(INT_PTR)&ctx);
}

/* --- Send-message dialog (Users tab) --- */
struct TM_MSG_CTX {
    DWORD session;
};

static INT_PTR CALLBACK tm_sendmsg_proc(HWND dlg, UINT msg, WPARAM w,
                                        LPARAM l)
{
    static struct TM_MSG_CTX s_ctx;
    if (msg == WM_INITDIALOG) {
        s_ctx = *(struct TM_MSG_CTX *)(INT_PTR)l;
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDOK) {
        wchar_t title[128], text[512];
        DWORD resp = 0;
        GetDlgItemTextW(dlg, IDE_MSG_TITLE, title, _countof(title));
        GetDlgItemTextW(dlg, IDE_MSG_TEXT, text, _countof(text));
        if (!WTSSendMessageW(WTS_CURRENT_SERVER_HANDLE, s_ctx.session,
                             (LPWSTR)(title[0] ? title : (wchar_t *)S_APP),
                             (DWORD)(wcslen(title[0] ? title : S_APP) *
                                     sizeof(wchar_t)),
                             text, (DWORD)(wcslen(text) * sizeof(wchar_t)),
                             MB_OK | MB_ICONINFORMATION, 60, &resp,
                             FALSE))
            tm_error(dlg, L"Cannot send the message.");
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDCANCEL) {
        EndDialog(dlg, IDCANCEL);
        return TRUE;
    }
    return FALSE;
}

static void tm_sendmsg_dialog(HWND owner, DWORD sessionId)
{
    struct TM_MSG_CTX ctx;
    ctx.session = sessionId;
    DialogBoxParamW(g_hInst, MAKEINTRESOURCEW(IDD_SENDMSG), owner,
                    tm_sendmsg_proc, (LPARAM)(INT_PTR)&ctx);
}

static INT_PTR CALLBACK tm_about_proc(HWND dlg, UINT msg, WPARAM w,
                                      LPARAM l)
{
    (void)w;
    (void)l;
    if (msg == WM_INITDIALOG)
        return TRUE;
    if (msg == WM_COMMAND && (LOWORD(w) == IDOK || LOWORD(w) == IDCANCEL)) {
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    return FALSE;
}

/* --------------------------------------------------------------------------
 * Layout, refresh orchestration, status bar
 * -------------------------------------------------------------------------- */
static void tm_layout(void)
{
    RECT rc, tab, st;
    int statusH = 0, tabH = 28, i;
    if (!g_hwndMain || !g_hwndTab)
        return;
    GetClientRect(g_hwndMain, &rc);
    if (g_hwndStatus) {
        SendMessageW(g_hwndStatus, WM_SIZE, 0, 0);
        GetWindowRect(g_hwndStatus, &st);
        statusH = st.bottom - st.top;
    }
    SetWindowPos(g_hwndTab, NULL, 4, 4, rc.right - 8, rc.bottom - 8 - statusH,
                 SWP_NOZORDER);
    GetClientRect(g_hwndTab, &tab);
    TabCtrl_AdjustRect(g_hwndTab, FALSE, &tab);
    tabH = tab.top; /* (kept for clarity; hosts use adjusted rect) */
    (void)tabH;
    for (i = 0; i < TAB_COUNT; i++) {
        if (!g_pages[i])
            continue;
        SetWindowPos(g_pages[i], NULL, tab.left + 6, tab.top + 6,
                     tab.right - tab.left - 12, tab.bottom - tab.top - 12,
                     SWP_NOZORDER);
        if (g_lv[i] && GetParent(g_lv[i]) == g_pages[i]) {
            RECT pr;
            GetClientRect(g_pages[i], &pr);
            SetWindowPos(g_lv[i], NULL, 0, 0, pr.right, pr.bottom,
                         SWP_NOZORDER);
        }
    }
    /* Performance page: two graphs side by side, facts below. */
    if (g_pages[TAB_PERFORMANCE]) {
        RECT pr;
        int gw;
        GetClientRect(g_pages[TAB_PERFORMANCE], &pr);
        gw = (pr.right - 12) / 2;
        if (g_graphCpu)
            SetWindowPos(g_graphCpu, NULL, 0, 0, gw, pr.bottom - 150,
                         SWP_NOZORDER);
        if (g_graphMem)
            SetWindowPos(g_graphMem, NULL, gw + 12, 0, pr.right - gw - 12,
                         pr.bottom - 150, SWP_NOZORDER);
        if (g_perfInfo)
            SetWindowPos(g_perfInfo, NULL, 0, pr.bottom - 142, pr.right,
                         142, SWP_NOZORDER);
    }
}

static void tm_refresh_all(int force)
{
    static int first = 1;
    static int slow = 0;
    size_t i;
    (void)force;
    /* Snapshot once per tick; every consumer reads g_procs. */
    tm_snapshot_processes();
    tm_net_sample();
    if ((slow++ % 5) == 0) {
        tm_gpu_sample();
        tm_if_sample();
    }
    /* System disk rate ~= sum of process I/O deltas (honest approximation,
     * labeled as such wherever shown). */
    g_diskBps = 0;
    for (i = 0; i < g_procCount; i++)
        g_diskBps += g_procs[i].ioRBps + g_procs[i].ioWBps;
    if (g_histCount > 0)
        g_diskHist[(g_histCount - 1) % TM_HIST_N] = g_diskBps;
    tm_caps_check();
    tm_alerts_check();
    if (g_tab == TAB_PROCESSES || first)
        tm_procs_refresh();
    if (g_tab == TAB_DETAILS || first)
        tm_details_refresh();
    if (g_tab == TAB_PERFORMANCE)
        tm_perf_refresh();
    if (g_tab == TAB_STARTUP)
        tm_startup_refresh();
    if (g_tab == TAB_USERS)
        tm_users_refresh();
    if (g_tab == TAB_SERVICES)
        tm_services_refresh();
    tm_refresh_status();
    tm_tray_tip();
    first = 0;
}

static void tm_refresh_status(void)
{
    wchar_t a[64], b[64], c[64];
    if (!g_hwndStatus)
        return;
    StringCchPrintfW(a, _countof(a), S_FMT_PROCS, (unsigned)g_procCount);
    StringCchPrintfW(b, _countof(b), S_FMT_CPU, (unsigned)g_cpuTotal);
    {
        MEMORYSTATUSEX ms;
        wchar_t m[32];
        ms.dwLength = sizeof(ms);
        if (GlobalMemoryStatusEx(&ms) && ms.ullTotalPhys > 0) {
            tm_fmt_bytes(m, _countof(m),
                         ms.ullTotalPhys - ms.ullAvailPhys);
            StringCchPrintfW(c, _countof(c), S_FMT_MEM, m);
        } else {
            StringCchCopyW(c, _countof(c), L"Memory: ?");
        }
    }
    SendMessageW(g_hwndStatus, SB_SETTEXTW, 0, (LPARAM)(INT_PTR)a);
    SendMessageW(g_hwndStatus, SB_SETTEXTW, 1, (LPARAM)(INT_PTR)b);
    SendMessageW(g_hwndStatus, SB_SETTEXTW, 2, (LPARAM)(INT_PTR)c);
}

/* --------------------------------------------------------------------------
 * Tray icon
 * -------------------------------------------------------------------------- */
static void tm_tray_add(void)
{
    NOTIFYICONDATAW ni;
    memset(&ni, 0, sizeof(ni));
    ni.cbSize = sizeof(ni);
    ni.hWnd = g_hwndMain;
    ni.uID = TRAY_ICON_ID;
    ni.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    ni.uCallbackMessage = WM_TRAY;
    ni.hIcon = LoadIconW(g_hInst, MAKEINTRESOURCEW(1));
    if (!ni.hIcon)
        ni.hIcon = LoadIconW(NULL, IDI_APPLICATION);
    StringCchCopyW(ni.szTip, _countof(ni.szTip), S_APP);
    Shell_NotifyIconW(NIM_ADD, &ni);
}

static void tm_tray_del(void)
{
    NOTIFYICONDATAW ni;
    memset(&ni, 0, sizeof(ni));
    ni.cbSize = sizeof(ni);
    ni.hWnd = g_hwndMain;
    ni.uID = TRAY_ICON_ID;
    Shell_NotifyIconW(NIM_DELETE, &ni);
}

static void tm_tray_tip(void)
{
    NOTIFYICONDATAW ni;
    MEMORYSTATUSEX ms;
    unsigned memPct = 0;
    ms.dwLength = sizeof(ms);
    if (GlobalMemoryStatusEx(&ms) && ms.ullTotalPhys > 0)
        memPct = (unsigned)(100 * (ms.ullTotalPhys - ms.ullAvailPhys) /
                            ms.ullTotalPhys);
    memset(&ni, 0, sizeof(ni));
    ni.cbSize = sizeof(ni);
    ni.hWnd = g_hwndMain;
    ni.uID = TRAY_ICON_ID;
    ni.uFlags = NIF_TIP;
    StringCchPrintfW(ni.szTip, _countof(ni.szTip), S_TRAY_TIP_FMT,
                     (unsigned)g_cpuTotal, memPct);
    Shell_NotifyIconW(NIM_MODIFY, &ni);
}

/* --------------------------------------------------------------------------
 * Menus
 * -------------------------------------------------------------------------- */
static const wchar_t *const PRIO_NAMES[6] = {
    L"Realtime", L"High", L"Above normal", L"Normal", L"Below normal", L"Low"
};
static const DWORD PRIO_VALS[6] = {
    REALTIME_PRIORITY_CLASS, HIGH_PRIORITY_CLASS, ABOVE_NORMAL_PRIORITY_CLASS,
    NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, IDLE_PRIORITY_CLASS
};

static HMENU tm_build_menu(void)
{
    HMENU bar, proc, prio;
    int i;
    bar = CreateMenu();
    g_menuFile = CreatePopupMenu();
    AppendMenuW(g_menuFile, MF_STRING, IDM_FILE_RUN, L"Run new task");
    AppendMenuW(g_menuFile, MF_SEPARATOR, 0, NULL);
    AppendMenuW(g_menuFile, MF_STRING, IDM_FILE_EXIT, L"E&xit");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)g_menuFile, L"&File");
    g_menuOpt = CreatePopupMenu();
    AppendMenuW(g_menuOpt, MF_STRING, IDM_OPT_ONTOP, L"Always on top");
    AppendMenuW(g_menuOpt, MF_STRING, IDM_OPT_MINUSE, L"Minimize on use");
    AppendMenuW(g_menuOpt, MF_STRING, IDM_OPT_HIDE, L"Hide when minimized");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)g_menuOpt, L"&Options");
    g_menuView = CreatePopupMenu();
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_HIGH, L"Update speed: High");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_NORMAL,
                L"Update speed: Normal");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_LOW, L"Update speed: Low");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_PAUSED, L"Paused");
    AppendMenuW(g_menuView, MF_SEPARATOR, 0, NULL);
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_GROUPPARENT,
                L"Group by parent process");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_MINIGRAPH,
                L"Mini CPU graph (always on top)");
    AppendMenuW(g_menuView, MF_SEPARATOR, 0, NULL);
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_EXPORTCSV,
                L"Export performance CSV...");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_DEADLOG,
                L"Recently terminated...");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_TDREVENTS,
                L"GPU driver events...");
    AppendMenuW(g_menuView, MF_SEPARATOR, 0, NULL);
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_ALERTS,
                L"Performance alerts...");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_COLORS,
                L"Heat-map colors...");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_CAPS,
                L"Automatic process caps...");
    AppendMenuW(g_menuView, MF_SEPARATOR, 0, NULL);
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_FIND, L"&Find...\tCtrl+F");
    AppendMenuW(g_menuView, MF_STRING, IDM_VIEW_REFRESH,
                L"Refresh now\tF5");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)g_menuView, L"&View");
    /* Context "menu" for processes lives in the right-click handler. */
    proc = CreatePopupMenu();
    AppendMenuW(proc, MF_STRING, IDM_PROC_ENDTASK, L"End task");
    AppendMenuW(proc, MF_STRING, IDM_PROC_GOTODETAIL, L"Go to details");
    prio = CreatePopupMenu();
    for (i = 0; i < 6; i++)
        AppendMenuW(prio, MF_STRING, IDM_PROC_PRIORITY + i, PRIO_NAMES[i]);
    AppendMenuW(proc, MF_POPUP, (UINT_PTR)prio, L"Set priority");
    AppendMenuW(proc, MF_STRING, IDM_PROC_AFFINITY, L"Set affinity...");
    DestroyMenu(proc);
    DestroyMenu(prio);
    {
        HMENU help = CreatePopupMenu();
        AppendMenuW(help, MF_STRING, IDM_HELP_ABOUT, L"About LoOS Task Manager");
        AppendMenuW(bar, MF_POPUP, (UINT_PTR)help, L"&Help");
    }
    g_menu = bar;
    return bar;
}

static void tm_menu_sync(void)
{
    CheckMenuItem(g_menuOpt, IDM_OPT_ONTOP,
                  MF_BYCOMMAND | (g_alwaysTop ? MF_CHECKED : MF_UNCHECKED));
    CheckMenuItem(g_menuOpt, IDM_OPT_MINUSE,
                  MF_BYCOMMAND | (g_minOnUse ? MF_CHECKED : MF_UNCHECKED));
    CheckMenuItem(g_menuOpt, IDM_OPT_HIDE,
                  MF_BYCOMMAND | (g_hideMin ? MF_CHECKED : MF_UNCHECKED));
    CheckMenuItem(g_menuView, IDM_VIEW_HIGH,
                  MF_BYCOMMAND |
                  (g_speedMs == SPEED_HIGH_MS ? MF_CHECKED : MF_UNCHECKED));
    CheckMenuItem(g_menuView, IDM_VIEW_NORMAL,
                  MF_BYCOMMAND |
                  (g_speedMs == SPEED_NORMAL_MS ? MF_CHECKED : MF_UNCHECKED));
    CheckMenuItem(g_menuView, IDM_VIEW_LOW,
                  MF_BYCOMMAND |
                  (g_speedMs == SPEED_LOW_MS ? MF_CHECKED : MF_UNCHECKED));
    CheckMenuItem(g_menuView, IDM_VIEW_PAUSED,
                  MF_BYCOMMAND |
                  (g_speedMs == 0 ? MF_CHECKED : MF_UNCHECKED));
    CheckMenuItem(g_menuView, IDM_VIEW_GROUPPARENT,
                  MF_BYCOMMAND |
                  (g_groupParent ? MF_CHECKED : MF_UNCHECKED));
}

/* Right-click menu for the process lists. */
static void tm_proc_context(HWND lv, int detail)
{
    HMENU pop, prio;
    POINT pt;
    int i, pid;
    TM_PROC *p;
    (void)detail;
    pid = tm_lv_selected_pid(lv);
    if (pid < 0)
        return;
    p = tm_find_proc((DWORD)pid);
    pop = CreatePopupMenu();
    AppendMenuW(pop, MF_STRING, IDM_PROC_ENDTASK, L"End task");
    AppendMenuW(pop, MF_STRING, IDM_PROC_ENDTREE, L"End process tree");
    AppendMenuW(pop, MF_STRING, IDM_PROC_SUSPEND, L"Pause (suspend)");
    AppendMenuW(pop, MF_STRING, IDM_PROC_RESUME, L"Resume");
    AppendMenuW(pop, MF_SEPARATOR, 0, NULL);
    if (!detail)
        AppendMenuW(pop, MF_STRING, IDM_PROC_GOTODETAIL, L"Go to details");
    prio = CreatePopupMenu();
    for (i = 0; i < 6; i++) {
        UINT f = MF_STRING;
        if (p && p->prioClass == PRIO_VALS[i])
            f |= MF_CHECKED;
        AppendMenuW(prio, f, IDM_PROC_PRIORITY + i, PRIO_NAMES[i]);
    }
    AppendMenuW(pop, MF_POPUP, (UINT_PTR)prio, L"Set priority");
    AppendMenuW(pop, MF_STRING, IDM_PROC_AFFINITY, L"Set affinity...");
    AppendMenuW(pop, MF_SEPARATOR, 0, NULL);
    AppendMenuW(pop, MF_STRING, IDM_PROC_WHYHUNG, L"Why is this slow?...");
    AppendMenuW(pop, MF_STRING, IDM_PROC_CONN, L"Network connections...");
    AppendMenuW(pop, MF_STRING, IDM_PROC_THREADS, L"Thread history...");
    AppendMenuW(pop, MF_SEPARATOR, 0, NULL);
    AppendMenuW(pop, MF_STRING, IDM_PROC_COPYPATH, L"Copy file path");
    AppendMenuW(pop, MF_STRING, IDM_PROC_OPENLOC, L"Open containing folder");
    GetCursorPos(&pt);
    TrackPopupMenu(pop, TPM_RIGHTBUTTON, pt.x, pt.y, 0, g_hwndMain, NULL);
    DestroyMenu(pop);
}

static void tm_svc_context(HWND lv)
{
    HMENU pop;
    POINT pt;
    if (tm_lv_selected_pid(lv) < 0 && ListView_GetNextItem(lv, -1,
                                                           LVNI_SELECTED) < 0)
        return;
    pop = CreatePopupMenu();
    AppendMenuW(pop, MF_STRING, IDM_SVC_START, L"Start");
    AppendMenuW(pop, MF_STRING, IDM_SVC_STOP, L"Stop");
    AppendMenuW(pop, MF_STRING, IDM_SVC_PAUSE, L"Pause");
    AppendMenuW(pop, MF_STRING, IDM_SVC_CONTINUE, L"Resume");
    {
        HMENU st = CreatePopupMenu();
        AppendMenuW(st, MF_STRING, IDM_SVC_STARTUP + 0, L"Startup: Automatic");
        AppendMenuW(st, MF_STRING, IDM_SVC_STARTUP + 1, L"Startup: Manual");
        AppendMenuW(st, MF_STRING, IDM_SVC_STARTUP + 2, L"Startup: Disabled");
        AppendMenuW(pop, MF_POPUP, (UINT_PTR)st, L"Startup type");
    }
    GetCursorPos(&pt);
    TrackPopupMenu(pop, TPM_RIGHTBUTTON, pt.x, pt.y, 0, g_hwndMain, NULL);
    DestroyMenu(pop);
}

static void tm_user_context(HWND lv)
{
    HMENU pop;
    POINT pt;
    if (tm_lv_selected_pid(lv) < 0)
        return;
    pop = CreatePopupMenu();
    AppendMenuW(pop, MF_STRING, IDM_USER_SENDMSG, L"Send message...");
    AppendMenuW(pop, MF_STRING, IDM_USER_DISCONNECT, L"Disconnect");
    AppendMenuW(pop, MF_STRING, IDM_USER_LOGOFF, L"Log off");
    GetCursorPos(&pt);
    TrackPopupMenu(pop, TPM_RIGHTBUTTON, pt.x, pt.y, 0, g_hwndMain, NULL);
    DestroyMenu(pop);
}

/* --------------------------------------------------------------------------
 * Main window
 * -------------------------------------------------------------------------- */
static void tm_apply_topmost(void)
{
    SetWindowPos(g_hwndMain, g_alwaysTop ? HWND_TOPMOST : HWND_NOTOPMOST,
                 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
}

static void tm_restart_timer(void)
{
    KillTimer(g_hwndMain, IDT_REFRESH);
    if (g_speedMs > 0)
        SetTimer(g_hwndMain, IDT_REFRESH, (UINT)g_speedMs, NULL);
}

static LRESULT CALLBACK tm_wnd_proc(HWND hwnd, UINT msg, WPARAM wParam,
                                    LPARAM lParam)
{
    if (msg == WM_CREATE)
        g_hwndMain = hwnd; /* CreateWindowEx hasn't returned yet; wWinMain
                            * hasn't stored it. Tray + helpers need it now. */
    switch (msg) {
    case WM_CREATE: {
        INITCOMMONCONTROLSEX icc;
        int parts[3] = { 180, 300, -1 };
        icc.dwSize = sizeof(icc);
        icc.dwICC = ICC_TAB_CLASSES | ICC_LISTVIEW_CLASSES |
                    ICC_BAR_CLASSES;
        InitCommonControlsEx(&icc);
        g_hwndTab = CreateWindowExW(0, WC_TABCONTROLW, L"",
                                    WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                                    0, 0, 10, 10, hwnd, (HMENU)IDC_TAB,
                                    g_hInst, NULL);
        SendMessageW(g_hwndTab, WM_SETFONT,
                     (WPARAM)GetStockObject(DEFAULT_GUI_FONT), FALSE);
        g_hwndStatus = CreateWindowExW(0, STATUSCLASSNAMEW, L"",
                                       WS_CHILD | WS_VISIBLE | SBARS_SIZEGRIP,
                                       0, 0, 10, 10, hwnd, (HMENU)IDC_STATUS,
                                       g_hInst, NULL);
        SendMessageW(g_hwndStatus, SB_SETPARTS, 3, (LPARAM)(INT_PTR)parts);
        tm_pages_create();
        /* Performance page children (created once, laid out later). */
        {
            WNDCLASSW wc;
            memset(&wc, 0, sizeof(wc));
            wc.lpfnWndProc = tm_graph_cpu_proc;
            wc.hInstance = g_hInst;
            wc.lpszClassName = L"LoOSTMCpuGraph";
            wc.hCursor = LoadCursorW(NULL, IDC_ARROW);
            RegisterClassW(&wc);
            wc.lpfnWndProc = tm_graph_mem_proc;
            wc.lpszClassName = L"LoOSTMMemGraph";
            RegisterClassW(&wc);
        }
        g_graphCpu = CreateWindowExW(WS_EX_CLIENTEDGE, L"LoOSTMCpuGraph",
                                     L"CPU", WS_CHILD | WS_VISIBLE,
                                     0, 0, 10, 10,
                                     g_pages[TAB_PERFORMANCE], NULL,
                                     g_hInst, NULL);
        g_graphMem = CreateWindowExW(WS_EX_CLIENTEDGE, L"LoOSTMMemGraph",
                                     L"Memory", WS_CHILD | WS_VISIBLE,
                                     0, 0, 10, 10,
                                     g_pages[TAB_PERFORMANCE], NULL,
                                     g_hInst, NULL);
        g_perfInfo = CreateWindowExW(0, L"STATIC", L"",
                                     WS_CHILD | WS_VISIBLE,
                                     0, 0, 10, 10,
                                     g_pages[TAB_PERFORMANCE], NULL,
                                     g_hInst, NULL);
        SendMessageW(g_perfInfo, WM_SETFONT,
                     (WPARAM)GetStockObject(DEFAULT_GUI_FONT), FALSE);
        tm_perf_cores();
        SetMenu(hwnd, tm_build_menu());
        tm_menu_sync();
        tm_theme_init(hwnd);
        tm_tray_add();
        tm_show_tab(TAB_PROCESSES);
        tm_restart_timer();
        return 0;
    }
    case WM_SIZE:
    case WM_WINDOWPOSCHANGED:
        /* WINDOWPOSCHANGED (not just SIZE) fires on the initial show too,
         * so the tab + lists always get real sizes even if no WM_SIZE
         * ever arrives after creation. */
        tm_layout();
        return 0;
    case WM_NOTIFY: {
        NMHDR *nm = (NMHDR *)(INT_PTR)lParam;
        if (nm->idFrom == IDC_TAB && nm->code == TCN_SELCHANGE) {
            tm_show_tab(TabCtrl_GetCurSel(g_hwndTab));
            return 0;
        }
        if (nm->code == LVN_COLUMNCLICK) {
            NMLISTVIEW *nmlv = (NMLISTVIEW *)(INT_PTR)lParam;
            HWND lv = nm->hwndFrom;
            int tab = g_tab, i;
            static struct {
                HWND hwnd;
                int col;
                int asc;
            } s_sort;
            for (i = 0; i < TAB_COUNT; i++)
                if (g_lv[i] == lv)
                    tab = i;
            if (g_sortCol[tab] == nmlv->iSubItem)
                g_sortAsc[tab] = !g_sortAsc[tab];
            else {
                g_sortCol[tab] = nmlv->iSubItem;
                g_sortAsc[tab] = 1;
            }
            s_sort.hwnd = lv;
            s_sort.col = nmlv->iSubItem;
            s_sort.asc = g_sortAsc[tab];
            ListView_SortItems(lv, tm_lv_compare, (LPARAM)(INT_PTR)&s_sort);
            return 0;
        }
        if (nm->code == NM_RCLICK) {
            if (nm->hwndFrom == g_lv[TAB_PROCESSES])
                tm_proc_context(nm->hwndFrom, 0);
            else if (nm->hwndFrom == g_lv[TAB_DETAILS])
                tm_proc_context(nm->hwndFrom, 1);
            else if (nm->hwndFrom == g_lv[TAB_SERVICES])
                tm_svc_context(nm->hwndFrom);
            else if (nm->hwndFrom == g_lv[TAB_USERS])
                tm_user_context(nm->hwndFrom);
            return 0;
        }
        if (nm->code == NM_DBLCLK && nm->hwndFrom == g_lv[TAB_PROCESSES]) {
            int pid = tm_lv_selected_pid(nm->hwndFrom);
            if (pid >= 0)
                tm_goto_details((DWORD)pid);
            return 0;
        }
        if (nm->code == NM_CUSTOMDRAW &&
            (nm->hwndFrom == g_lv[TAB_PROCESSES] ||
             nm->hwndFrom == g_lv[TAB_DETAILS])) {
            /* Heat-map: tint CPU (col 4) and Memory (col 5) cells by
             * value using the user's accent colors. */
            LPNMLVCUSTOMDRAW cd = (LPNMLVCUSTOMDRAW)(INT_PTR)lParam;
            if (cd->nmcd.dwDrawStage == CDDS_PREPAINT)
                return CDRF_NOTIFYITEMDRAW;
            if (cd->nmcd.dwDrawStage == CDDS_ITEMPREPAINT)
                return CDRF_NOTIFYSUBITEMDRAW;
            if ((cd->nmcd.dwDrawStage & CDDS_SUBITEM) &&
                (cd->iSubItem == 4 || cd->iSubItem == 5)) {
                wchar_t t[64];
                double v;
                COLORREF band = g_colLo;
                tm_lv_text(nm->hwndFrom, (int)cd->nmcd.dwItemSpec,
                           cd->iSubItem, t, _countof(t));
                v = wcstod(t, NULL);
                if (cd->iSubItem == 5) {
                    /* Memory "1.2 GB": normalize against total RAM. */
                    MEMORYSTATUSEX ms;
                    const wchar_t *u = t;
                    ms.dwLength = sizeof(ms);
                    while (*u && (*u < L'A' || *u == L' '))
                        u++;
                    if (GlobalMemoryStatusEx(&ms) &&
                        ms.ullTotalPhys > 0) {
                        double bytes = v;
                        if (wcsstr(u, L"TB"))
                            bytes *= 1024.0 * 1024 * 1024 * 1024;
                        else if (wcsstr(u, L"GB"))
                            bytes *= 1024 * 1024 * 1024;
                        else if (wcsstr(u, L"MB"))
                            bytes *= 1024 * 1024;
                        else if (wcsstr(u, L"KB"))
                            bytes *= 1024;
                        v = 100.0 * bytes / (double)ms.ullTotalPhys;
                    } else {
                        v = 0;
                    }
                }
                if (v >= 85.0)
                    band = g_colHi;
                else if (v >= 50.0)
                    band = g_colMid;
                cd->clrTextBk = band;
                return CDRF_NEWFONT;
            }
            return CDRF_DODEFAULT;
        }
        break;
    }
    case WM_COMMAND: {
        WORD id = LOWORD(wParam);
        HWND focusedLv = (g_tab == TAB_DETAILS) ? g_lv[TAB_DETAILS]
                                                : g_lv[TAB_PROCESSES];
        if (id == IDM_FILE_RUN)
            tm_run_dialog(hwnd);
        else if (id == IDM_FILE_EXIT)
            DestroyWindow(hwnd);
        else if (id == IDM_OPT_ONTOP) {
            g_alwaysTop = !g_alwaysTop;
            tm_apply_topmost();
            tm_menu_sync();
        } else if (id == IDM_OPT_MINUSE)
            g_minOnUse = !g_minOnUse, tm_menu_sync();
        else if (id == IDM_OPT_HIDE)
            g_hideMin = !g_hideMin, tm_menu_sync();
        else if (id == IDM_VIEW_HIGH)
            g_speedMs = SPEED_HIGH_MS, tm_restart_timer(), tm_menu_sync();
        else if (id == IDM_VIEW_NORMAL)
            g_speedMs = SPEED_NORMAL_MS, tm_restart_timer(), tm_menu_sync();
        else if (id == IDM_VIEW_LOW)
            g_speedMs = SPEED_LOW_MS, tm_restart_timer(), tm_menu_sync();
        else if (id == IDM_VIEW_PAUSED)
            g_speedMs = 0, tm_restart_timer(), tm_menu_sync();
        else if (id == IDM_VIEW_REFRESH)
            tm_refresh_all(1);
        else if (id == IDM_VIEW_FIND)
            tm_find_dialog(hwnd);
        else if (id == IDM_PROC_ENDTASK)
            tm_end_task(focusedLv);
        else if (id == IDM_PROC_GOTODETAIL) {
            int pid = tm_lv_selected_pid(focusedLv);
            if (pid >= 0)
                tm_goto_details((DWORD)pid);
        } else if (id >= IDM_PROC_PRIORITY && id < IDM_PROC_PRIORITY + 6)
            tm_set_priority(focusedLv, PRIO_VALS[id - IDM_PROC_PRIORITY]);
        else if (id == IDM_PROC_AFFINITY) {
            int pid = tm_lv_selected_pid(focusedLv);
            if (pid >= 0)
                tm_affinity_dialog(hwnd, (DWORD)pid);
        } else if (id == IDM_PROC_SUSPEND || id == IDM_PROC_RESUME) {
            int pid = tm_lv_selected_pid(focusedLv);
            if (pid >= 0) {
                if (id == IDM_PROC_SUSPEND) {
                    if (!tm_suspend((DWORD)pid))
                        tm_error(hwnd, L"Cannot suspend the process.");
                } else if (!tm_resume((DWORD)pid))
                    tm_error(hwnd, L"Cannot resume the process.");
                tm_refresh_all(1);
            }
        } else if (id == IDM_PROC_ENDTREE) {
            int pid = tm_lv_selected_pid(focusedLv);
            if (pid >= 0)
                tm_end_tree((DWORD)pid);
        } else if (id == IDM_PROC_COPYPATH || id == IDM_PROC_OPENLOC) {
            int pid = tm_lv_selected_pid(focusedLv);
            if (pid >= 0) {
                if (id == IDM_PROC_COPYPATH)
                    tm_copy_path((DWORD)pid);
                else
                    tm_open_location((DWORD)pid);
            }
        } else if (id == IDM_PROC_WHYHUNG || id == IDM_PROC_CONN ||
                   id == IDM_PROC_THREADS) {
            int pid = tm_lv_selected_pid(focusedLv);
            if (pid >= 0) {
                if (id == IDM_PROC_WHYHUNG)
                    tm_whyhung_dialog(hwnd, (DWORD)pid);
                else if (id == IDM_PROC_CONN)
                    tm_conn_dialog(hwnd, (DWORD)pid);
                else
                    tm_threads_dialog(hwnd, (DWORD)pid);
            }
        } else if (id == IDM_VIEW_GROUPPARENT) {
            g_groupParent = !g_groupParent;
            tm_menu_sync();
            tm_refresh_all(1);
        } else if (id == IDM_VIEW_MINIGRAPH)
            tm_mini_toggle();
        else if (id == IDM_VIEW_EXPORTCSV)
            tm_export_csv(hwnd);
        else if (id == IDM_VIEW_DEADLOG)
            tm_dead_dialog(hwnd);
        else if (id == IDM_VIEW_TDREVENTS)
            tm_tdr_dialog(hwnd);
        else if (id == IDM_VIEW_ALERTS)
            tm_alerts_dialog(hwnd);
        else if (id == IDM_VIEW_COLORS)
            tm_colors_dialog(hwnd);
        else if (id == IDM_VIEW_CAPS)
            tm_caps_dialog(hwnd);
        else if (id >= IDM_SVC_START && id <= IDM_SVC_REFRESH) {
            if (id == IDM_SVC_REFRESH)
                tm_services_refresh();
            else
                tm_svc_command(g_lv[TAB_SERVICES], id);
        } else if (id >= IDM_SVC_STARTUP && id <= IDM_SVC_STARTUP + 2) {
            int sel = ListView_GetNextItem(g_lv[TAB_SERVICES], -1,
                                           LVNI_SELECTED);
            if (sel >= 0) {
                wchar_t disp[256], key[256];
                DWORD keyLen = _countof(key);
                SC_HANDLE scm, svc;
                DWORD want = (id == IDM_SVC_STARTUP) ? SERVICE_AUTO_START
                             : (id == IDM_SVC_STARTUP + 1)
                                   ? SERVICE_DEMAND_START
                                   : SERVICE_DISABLED;
                tm_lv_text(g_lv[TAB_SERVICES], sel, 0, disp,
                           _countof(disp));
                scm = OpenSCManagerW(NULL, NULL, SC_MANAGER_CONNECT);
                if (scm) {
                    if (GetServiceKeyNameW(scm, disp, key, &keyLen))
                        StringCchCopyW(disp, _countof(disp), key);
                    svc = OpenServiceW(scm, disp, SERVICE_CHANGE_CONFIG);
                    if (svc) {
                        if (!ChangeServiceConfigW(
                                svc, SERVICE_NO_CHANGE, want,
                                SERVICE_NO_CHANGE, NULL, NULL, NULL, NULL,
                                NULL, NULL, NULL))
                            tm_error(hwnd,
                                     L"Cannot change the start type "
                                     L"(elevation may be required).");
                        CloseServiceHandle(svc);
                    } else {
                        tm_error(hwnd, L"Cannot open the service.");
                    }
                    CloseServiceHandle(scm);
                }
                tm_services_refresh();
            }
        } else if (id >= IDM_USER_LOGOFF && id <= IDM_USER_SENDMSG)
            tm_user_command(g_lv[TAB_USERS], id);
        else if (id == IDM_HELP_ABOUT)
            DialogBoxW(g_hInst, MAKEINTRESOURCEW(IDD_ABOUT), hwnd,
                       tm_about_proc);
        return 0;
    }
    case WM_TIMER:
        if (wParam == IDT_REFRESH)
            tm_refresh_all(0);
        return 0;
    case WM_TRAY:
        if (lParam == WM_LBUTTONDBLCLK) {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        } else if (lParam == WM_RBUTTONUP) {
            HMENU pop = CreatePopupMenu();
            POINT pt;
            AppendMenuW(pop, MF_STRING, IDM_VIEW_REFRESH, L"Refresh now");
            AppendMenuW(pop, MF_STRING, IDM_FILE_RUN, L"Run new task");
            AppendMenuW(pop, MF_SEPARATOR, 0, NULL);
            AppendMenuW(pop, MF_STRING, IDM_FILE_EXIT, L"Exit");
            GetCursorPos(&pt);
            SetForegroundWindow(hwnd);
            TrackPopupMenu(pop, TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd,
                           NULL);
            DestroyMenu(pop);
        }
        return 0;
    case WM_SYSCOMMAND:
        if ((wParam & 0xFFF0) == SC_MINIMIZE && g_hideMin) {
            ShowWindow(hwnd, SW_HIDE);
            return 0;
        }
        break;
    case WM_SETTINGCHANGE:
        /* System theme may have flipped; re-apply the title bar. */
        tm_theme_init(hwnd);
        InvalidateRect(hwnd, NULL, TRUE);
        break;
    case WM_DESTROY:
        tm_tray_del();
        KillTimer(hwnd, IDT_REFRESH);
        free(g_procs);
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

/* Headless self-test: LoTaskMgr.exe /debugdump snapshots processes and
 * writes C:\LoOS\out\tmdbg.txt (no window). Lets us verify the data
 * layer over SSH where no desktop exists. */
static int tm_debugdump(void)
{
    FILE *f;
    size_t i, show;
    ULONGLONG sys;
    MEMORYSTATUSEX ms;
    if (_wfopen_s(&f, L"C:\\LoOS\\out\\tmdbg.txt", L"w, ccs=UTF-8") != 0 ||
        !f)
        return 2;
    tm_perf_cores();
    sys = tm_system_total();
    fwprintf(f, L"cores=%d logical=%d sysTotal=%llu\n", g_nCores,
             g_nLogical, sys);
    ms.dwLength = sizeof(ms);
    if (GlobalMemoryStatusEx(&ms))
        fwprintf(f, L"physTotal=%llu physAvail=%llu\n", ms.ullTotalPhys,
                 ms.ullAvailPhys);
    if (!tm_snapshot_processes())
        fwprintf(f, L"SNAPSHOT FAILED err=%lu\n", GetLastError());
    fwprintf(f, L"procs=%zu\n", g_procCount);
    show = g_procCount < 20 ? g_procCount : 20;
    for (i = 0; i < show; i++)
        fwprintf(f, L"%5lu %-28s mem=%8zuKB cpu=%5.1f user=%s\n",
                 g_procs[i].pid, g_procs[i].name, g_procs[i].memKB,
                 g_procs[i].cpu,
                 g_procs[i].user[0] ? g_procs[i].user : L"?");
    fclose(f);
    return 0;
}

int WINAPI wWinMain(HINSTANCE hInst, HINSTANCE hPrev, LPWSTR cmd,
                    int showCmd)
{
    WNDCLASSW wc;
    HWND hwnd;
    MSG msg;
    (void)hPrev;
    if (cmd && wcsstr(cmd, L"/debugdump") != NULL)
        return tm_debugdump();
    g_hInst = hInst;
    tm_prefs_load();
    tm_dpi_init();
    memset(&wc, 0, sizeof(wc));
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = tm_wnd_proc;
    wc.hInstance = hInst;
    wc.hCursor = LoadCursorW(NULL, IDC_ARROW);
    wc.hbrBackground = (HBRUSH)(COLOR_3DFACE + 1);
    wc.lpszClassName = S_CLASS;
    wc.hIcon = LoadIconW(NULL, IDI_APPLICATION);
    if (!RegisterClassW(&wc)) {
        MessageBoxW(NULL, L"Cannot register the window class.", S_APP,
                    MB_OK | MB_ICONERROR);
        return 1;
    }
    hwnd = CreateWindowExW(0, S_CLASS, S_APP,
                           WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                           CW_USEDEFAULT, CW_USEDEFAULT, 900, 600,
                           NULL, NULL, hInst, NULL);
    if (!hwnd) {
        MessageBoxW(NULL, L"Cannot create the main window.", S_APP,
                    MB_OK | MB_ICONERROR);
        return 1;
    }
    g_hwndMain = hwnd;
    if (g_minOnUse) {
        (void)showCmd;
    }
    ShowWindow(hwnd, showCmd);
    UpdateWindow(hwnd);
    while (GetMessageW(&msg, NULL, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    return (int)msg.wParam;
}

/* ============================================================================
 * Feature round 2: IOPS, GPU, network history, audit log, caps, alerts,
 * root-cause, security lookups, safety tools.
 * ============================================================================ */

/* Per-process I/O deltas (2 samples -> ops/s + bytes/s). */
static void tm_sample_io(TM_PROC *p, HANDLE hProc)
{
    IO_COUNTERS io;
    ULONGLONG now = GetTickCount64();
    ULONGLONG dt;
    if (!GetProcessIoCounters(hProc, &io)) {
        p->iopsR = p->iopsW = p->ioRBps = p->ioWBps = 0;
        return;
    }
    if (p->prevIoTick == 0 || now <= p->prevIoTick) {
        p->prevRIOps = io.ReadOperationCount;
        p->prevWIOps = io.WriteOperationCount;
        p->prevRIoB = io.ReadTransferCount;
        p->prevWioB = io.WriteTransferCount;
        p->prevIoTick = now;
        p->iopsR = p->iopsW = p->ioRBps = p->ioWBps = 0;
        return;
    }
    dt = now - p->prevIoTick;
    p->iopsR = 1000.0 * (double)(io.ReadOperationCount - p->prevRIOps) / dt;
    p->iopsW = 1000.0 * (double)(io.WriteOperationCount - p->prevWIOps) / dt;
    p->ioRBps = 1000.0 * (double)(io.ReadTransferCount - p->prevRIoB) / dt;
    p->ioWBps = 1000.0 * (double)(io.WriteTransferCount - p->prevWioB) / dt;
    if (p->iopsR < 0)
        p->iopsR = 0;
    if (p->iopsW < 0)
        p->iopsW = 0;
    p->prevRIOps = io.ReadOperationCount;
    p->prevWIOps = io.WriteOperationCount;
    p->prevRIoB = io.ReadTransferCount;
    p->prevWioB = io.WriteTransferCount;
    p->prevIoTick = now;
}

/* Rolling TCP byte totals per PID via the per-connection EStats table.
 * Only TCP exposes per-connection byte counters (UDP has none); the
 * dialog says so honestly. Runs once per refresh tick. */
static void tm_net_sample(void)
{
    DWORD size = 0;
    MIB_TCPTABLE_OWNER_PID *tab = NULL;
    DWORD i;
    size_t k;
    ULONGLONG now = GetTickCount64();
    static ULONGLONG lastTick;
    ULONGLONG dt = (lastTick && now > lastTick) ? now - lastTick : 0;
    /* Reset per-tick accumulators. */
    for (k = 0; k < g_procCount; k++) {
        g_procs[k].netInBps = g_procs[k].netOutBps = 0;
    }
    GetExtendedTcpTable(NULL, &size, FALSE, AF_INET,
                        TCP_TABLE_OWNER_PID_ALL, 0);
    if (size == 0 || size > (1 << 24))
        return;
    tab = (MIB_TCPTABLE_OWNER_PID *)malloc(size);
    if (!tab)
        return;
    if (GetExtendedTcpTable(tab, &size, FALSE, AF_INET,
                            TCP_TABLE_OWNER_PID_ALL, 0) != NO_ERROR) {
        free(tab);
        return;
    }
    for (i = 0; i < tab->dwNumEntries; i++) {
        MIB_TCPROW_OWNER_PID *r = &tab->table[i];
        TCP_ESTATS_DATA_RW_v0 rw;
        TM_PROC *p;
        /* ESTATS handle per connection: needs the 5-tuple row. */
        {
            MIB_TCPROW row;
            row.dwState = r->dwState;
            row.dwLocalAddr = r->dwLocalAddr;
            row.dwLocalPort = r->dwLocalPort;
            row.dwRemoteAddr = r->dwRemoteAddr;
            row.dwRemotePort = r->dwRemotePort;
            memset(&rw, 0, sizeof(rw));
            rw.EnableCollection = TRUE;
            if (SetPerTcpConnectionEStats(&row, TcpConnectionEstatsData,
                                          (PUCHAR)&rw, 0, sizeof(rw), 0) !=
                NO_ERROR)
                continue;
            memset(&rw, 0, sizeof(rw));
            {
                TCP_ESTATS_DATA_ROD_v0 rod;
                ULONG rodSize = sizeof(rod);
                if (GetPerTcpConnectionEStats(&row, TcpConnectionEstatsData,
                                              NULL, 0, 0, (PUCHAR)&rod, 0,
                                              rodSize, NULL, 0, 0) !=
                    NO_ERROR)
                    continue;
                p = tm_find_proc(r->dwOwningPid);
                if (p) {
                    /* Cumulative counters; session total = max seen. */
                    if (rod.DataBytesIn > p->netIn)
                        p->netIn = rod.DataBytesIn;
                    if (rod.DataBytesOut > p->netOut)
                        p->netOut = rod.DataBytesOut;
                    if (dt > 0) {
                        /* Rate attribution is coarse (shared totals), so
                         * we expose session totals + system rate instead. */
                    }
                }
            }
        }
    }
    free(tab);
    lastTick = now;
    (void)dt;
}

/* Per-interface octets -> VPN vs LAN/WAN split (slow tick is fine;
 * called from the performance refresher every ~5th tick). */
static void tm_if_sample(void)
{
    MIB_IFTABLE *t = NULL;
    DWORD size = 0, i;
    static ULONGLONG lastIn, lastOut, lastVpnIn, lastVpnOut, lastTick;
    ULONGLONG in = 0, out = 0, vin = 0, vout = 0;
    ULONGLONG now = GetTickCount64(), dt;
    GetIfTable(NULL, &size, FALSE);
    if (size == 0 || size > (1 << 24))
        return;
    t = (MIB_IFTABLE *)malloc(size);
    if (!t)
        return;
    if (GetIfTable(t, &size, FALSE) != NO_ERROR) {
        free(t);
        return;
    }
    for (i = 0; i < t->dwNumEntries; i++) {
        MIB_IFROW *r = &t->table[i];
        int isVpn = (r->dwType == IF_TYPE_TUNNEL || r->dwType == 23 /*PPP*/);
        char desc[256];
        if (!isVpn) {
            /* Name-based fallback for virtual adapters. */
            WideCharToMultiByte(CP_ACP, 0, r->wszName, -1, desc,
                                (int)sizeof(desc), NULL, NULL);
            desc[sizeof(desc) - 1] = 0;
            if (strstr(desc, "VPN") || strstr(desc, "vpn") ||
                strstr(desc, "WireGuard") || strstr(desc, "Tailscale") ||
                strstr(desc, "OpenVPN") || strstr(desc, "ZeroTier"))
                isVpn = 1;
        }
        if (r->dwOperStatus != IF_OPER_STATUS_OPERATIONAL)
            continue;
        if (r->dwType == 24 /*loopback*/)
            continue;
        in += r->dwInOctets;
        out += r->dwOutOctets;
        if (isVpn) {
            vin += r->dwInOctets;
            vout += r->dwOutOctets;
        }
    }
    free(t);
    dt = (lastTick && now > lastTick) ? now - lastTick : 0;
    if (dt > 0 && in >= lastIn) {
        g_netBps = 1000.0 * (double)((in - lastIn) + (out - lastOut)) / dt;
        g_vpnBps = 1000.0 * (double)((vin - lastVpnIn) +
                                     (vout - lastVpnOut)) / dt;
        g_lanBps = g_netBps - g_vpnBps;
        if (g_lanBps < 0)
            g_lanBps = 0;
        if (g_histCount > 0)
            g_netHist[(g_histCount - 1) % TM_HIST_N] = g_netBps;
    }
    lastIn = in;
    lastOut = out;
    lastVpnIn = vin;
    lastVpnOut = vout;
    lastTick = now;
    if (g_prevNetIn == 0) {
        g_prevNetIn = in;
        g_prevNetOut = out;
    }
}

/* GPU per-process dedicated/shared via one wildcard PDH query.
 * Missing counters (no GPU / old driver) -> stays 0 = "n/a". */
static void tm_gpu_sample(void)
{
    static HQUERY q;
    static HCOUNTER ded, shr;
    static int inited, noGpu;
    PDH_FMT_COUNTERVALUE_ITEM_W *items = NULL;
    DWORD bufSize = 0, count = 0, i;
    size_t k;
    if (noGpu)
        return;
    if (!inited) {
        if (PdhOpenQueryW(NULL, 0, &q) != ERROR_SUCCESS) {
            noGpu = 1;
            return;
        }
        if (PdhAddCounterW(q, L"\\GPU Process Memory(*)\\Dedicated Usage",
                           0, &ded) != ERROR_SUCCESS ||
            PdhAddCounterW(q, L"\\GPU Process Memory(*)\\Shared Usage", 0,
                           &shr) != ERROR_SUCCESS) {
            noGpu = 1;
            return;
        }
        inited = 1;
    }
    for (k = 0; k < g_procCount; k++) {
        g_procs[k].gpuDed = g_procs[k].gpuShr = 0;
    }
    if (PdhCollectQueryData(q) != ERROR_SUCCESS)
        return;
    /* Dedicated. */
    if (PdhGetFormattedCounterArrayW(ded, PDH_FMT_LARGE, &bufSize, &count,
                                     NULL) == PDH_MORE_DATA &&
        bufSize > 0 && bufSize < (1 << 26)) {
        items = (PDH_FMT_COUNTERVALUE_ITEM_W *)malloc(bufSize);
        if (items &&
            PdhGetFormattedCounterArrayW(ded, PDH_FMT_LARGE, &bufSize,
                                         &count, items) == ERROR_SUCCESS) {
            for (i = 0; i < count; i++) {
                /* Instance: "pid_1234_luid_..._phys_...". */
                unsigned pid = 0;
                if (swscanf_s(items[i].szName, L"pid_%u", &pid) == 1) {
                    TM_PROC *p = tm_find_proc(pid);
                    if (p && items[i].FmtValue.largeValue > 0)
                        p->gpuDed =
                            (double)items[i].FmtValue.largeValue;
                }
            }
        }
        free(items);
        items = NULL;
    }
    bufSize = 0;
    count = 0;
    if (PdhGetFormattedCounterArrayW(shr, PDH_FMT_LARGE, &bufSize, &count,
                                     NULL) == PDH_MORE_DATA &&
        bufSize > 0 && bufSize < (1 << 26)) {
        items = (PDH_FMT_COUNTERVALUE_ITEM_W *)malloc(bufSize);
        if (items &&
            PdhGetFormattedCounterArrayW(shr, PDH_FMT_LARGE, &bufSize,
                                         &count, items) == ERROR_SUCCESS) {
            for (i = 0; i < count; i++) {
                unsigned pid = 0;
                if (swscanf_s(items[i].szName, L"pid_%u", &pid) == 1) {
                    TM_PROC *p = tm_find_proc(pid);
                    if (p && items[i].FmtValue.largeValue > 0)
                        p->gpuShr =
                            (double)items[i].FmtValue.largeValue;
                }
            }
        }
        free(items);
    }
}

/* Push a dying process into the "what closed?" ring. Called for rows that
 * vanished between snapshots (see the compact step). */
static void tm_dead_push(const TM_PROC *p)
{
    TM_DEAD *d = &g_dead[g_deadHead % TM_DEAD_N];
    StringCchCopyW(d->name, _countof(d->name), p->name);
    d->pid = p->pid;
    GetLocalTime(&d->when);
    d->cpu = p->cpu;
    d->memKB = p->memKB;
    g_deadHead++;
    if (g_deadCount < TM_DEAD_N)
        g_deadCount++;
}

/* Automated caps + alerts, evaluated once per refresh tick. */
static void tm_nt_init(void)
{
    static int done;
    HMODULE ntdll;
    if (done)
        return;
    done = 1;
    ntdll = GetModuleHandleW(L"ntdll.dll");
    if (ntdll) {
        g_ntSuspend = (NT_SUSPEND_PROC_FN)GetProcAddress(ntdll,
                                                        "NtSuspendProcess");
        g_ntResume = (NT_RESUME_PROC_FN)GetProcAddress(ntdll,
                                                       "NtResumeProcess");
    }
}

static void tm_balloon(const wchar_t *title, const wchar_t *text)
{
    NOTIFYICONDATAW ni;
    memset(&ni, 0, sizeof(ni));
    ni.cbSize = sizeof(ni);
    ni.hWnd = g_hwndMain;
    ni.uID = TRAY_ICON_ID;
    ni.uFlags = NIF_INFO;
    ni.dwInfoFlags = NIIF_WARNING;
    StringCchCopyW(ni.szInfoTitle, _countof(ni.szInfoTitle), title);
    StringCchCopyW(ni.szInfo, _countof(ni.szInfo), text);
    Shell_NotifyIconW(NIM_MODIFY, &ni);
}

static int tm_suspend(DWORD pid)
{
    HANDLE h;
    LONG st;
    tm_nt_init();
    if (!g_ntSuspend)
        return 0;
    h = OpenProcess(PROCESS_SUSPEND_RESUME, FALSE, pid);
    if (!h)
        return 0;
    st = g_ntSuspend(h);
    CloseHandle(h);
    return st >= 0;
}

static int tm_resume(DWORD pid)
{
    HANDLE h;
    LONG st;
    tm_nt_init();
    if (!g_ntResume)
        return 0;
    h = OpenProcess(PROCESS_SUSPEND_RESUME, FALSE, pid);
    if (!h)
        return 0;
    st = g_ntResume(h);
    CloseHandle(h);
    return st >= 0;
}

static void tm_caps_check(void)
{
    size_t i;
    int c;
    for (i = 0; i < g_procCount; i++) {
        for (c = 0; c < TM_CAPS_N; c++) {
            TM_CAP *cap = &g_caps[c];
            wchar_t ln[MAX_PATH], lm[64], msg[256];
            size_t k;
            if (!cap->used || !cap->match[0])
                continue;
            for (k = 0; k < _countof(ln) - 1 && g_procs[i].name[k]; k++)
                ln[k] = towlower(g_procs[i].name[k]);
            ln[k] = L'\0';
            for (k = 0; k < _countof(lm) - 1 && cap->match[k]; k++)
                lm[k] = towlower(cap->match[k]);
            lm[k] = L'\0';
            if (!wcsstr(ln, lm))
                continue;
            if (g_procs[i].memKB <= (SIZE_T)cap->mb * 1024)
                continue;
            if (cap->action == 0) {
                if (tm_suspend(g_procs[i].pid)) {
                    StringCchPrintfW(msg, _countof(msg),
                                     L"Auto-suspended %s (over %lu MB).",
                                     g_procs[i].name, cap->mb);
                    tm_balloon(L"Process cap", msg);
                    tm_debug(L"cap: suspended %lu", g_procs[i].pid);
                }
            } else if (cap->action == 1) {
                HANDLE h = OpenProcess(PROCESS_TERMINATE, FALSE,
                                       g_procs[i].pid);
                if (h) {
                    TerminateProcess(h, 1);
                    CloseHandle(h);
                    StringCchPrintfW(msg, _countof(msg),
                                     L"Auto-terminated %s (over %lu MB).",
                                     g_procs[i].name, cap->mb);
                    tm_balloon(L"Process cap", msg);
                }
            } else {
                StringCchPrintfW(msg, _countof(msg),
                                 L"%s exceeded %lu MB.", g_procs[i].name,
                                 cap->mb);
                tm_balloon(L"Process cap", msg);
            }
            break; /* one action per process per tick */
        }
    }
}

static void tm_alerts_check(void)
{
    ULONGLONG now = GetTickCount64() / 1000;
    MEMORYSTATUSEX ms;
    unsigned memPct = 0;
    wchar_t msg[256];
    ms.dwLength = sizeof(ms);
    if (GlobalMemoryStatusEx(&ms) && ms.ullTotalPhys > 0)
        memPct = (unsigned)(100 * (ms.ullTotalPhys - ms.ullAvailPhys) /
                            ms.ullTotalPhys);
    if (g_alertCpuPct > 0 && g_cpuTotal >= g_alertCpuPct) {
        if (!g_overCpuSince)
            g_overCpuSince = (int)now;
        else if ((int)now - g_overCpuSince >= g_alertSecs) {
            StringCchPrintfW(msg, _countof(msg),
                             L"CPU at %.0f%% for over %ds.", g_cpuTotal,
                             g_alertSecs);
            tm_balloon(L"Performance alert", msg);
            g_overCpuSince = (int)now; /* re-arm, don't spam */
        }
    } else {
        g_overCpuSince = 0;
    }
    if (g_alertRamPct > 0 && memPct >= (unsigned)g_alertRamPct) {
        if (!g_overRamSince)
            g_overRamSince = (int)now;
        else if ((int)now - g_overRamSince >= g_alertSecs) {
            StringCchPrintfW(msg, _countof(msg),
                             L"Memory at %u%% for over %ds.", memPct,
                             g_alertSecs);
            tm_balloon(L"Performance alert", msg);
            g_overRamSince = (int)now;
        }
    } else {
        g_overRamSince = 0;
    }
}

/* Top-level ancestor (for "group by parent" + tree kill). */
static DWORD tm_top_ancestor(DWORD pid)
{
    DWORD seen = pid, next;
    int depth = 0;
    for (;;) {
        TM_PROC *p = tm_find_proc(seen);
        if (!p || p->ppid == 0 || p->ppid == seen || depth++ > 64)
            return seen;
        next = p->ppid;
        if (!tm_find_proc(next))
            return seen;
        seen = next;
    }
}

/* Recursively terminate pid + all descendants. */
static void tm_end_tree(DWORD pid)
{
    size_t i;
    int again;
    wchar_t msg[256];
    TM_PROC *root = tm_find_proc(pid);
    StringCchPrintfW(msg, _countof(msg),
                     L"End \"%s\" and its entire process tree?",
                     root ? root->name : L"the process");
    if (MessageBoxW(g_hwndMain, msg, L"End process tree",
                    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES)
        return;
    /* Children first, repeat until no child of the set remains. */
    do {
        again = 0;
        for (i = 0; i < g_procCount; i++) {
            DWORD p = g_procs[i].pid;
            DWORD a = tm_top_ancestor(p);
            if (p != pid && (a == pid || g_procs[i].ppid == pid)) {
                HANDLE h = OpenProcess(PROCESS_TERMINATE, FALSE, p);
                if (h) {
                    TerminateProcess(h, 1);
                    CloseHandle(h);
                    again = 1;
                }
            }
        }
        tm_snapshot_processes();
    } while (again);
    {
        HANDLE h = OpenProcess(PROCESS_TERMINATE, FALSE, pid);
        if (h) {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
    tm_refresh_all(1);
}

static int tm_image_path(DWORD pid, wchar_t *out, size_t cch)
{
    HANDLE h;
    DWORD len;
    out[0] = L'\0';
    h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!h)
        return 0;
    len = (DWORD)cch;
    if (!QueryFullProcessImageNameW(h, 0, out, &len))
        out[0] = L'\0';
    CloseHandle(h);
    return out[0] != L'\0';
}

static void tm_copy_path(DWORD pid)
{
    wchar_t path[MAX_PATH];
    HGLOBAL g;
    wchar_t *lock;
    size_t bytes;
    if (!tm_image_path(pid, path, _countof(path))) {
        tm_error(g_hwndMain, L"Cannot query the image path.");
        return;
    }
    bytes = (wcslen(path) + 1) * sizeof(wchar_t);
    g = GlobalAlloc(GMEM_MOVEABLE, bytes);
    if (!g)
        return;
    lock = (wchar_t *)GlobalLock(g);
    if (lock) {
        StringCchCopyW(lock, wcslen(path) + 1, path);
        GlobalUnlock(g);
        if (OpenClipboard(g_hwndMain)) {
            EmptyClipboard();
            SetClipboardData(CF_UNICODETEXT, g);
            CloseClipboard();
            g = NULL; /* owned by the clipboard now */
        }
    }
    if (g)
        GlobalFree(g);
}

static void tm_open_location(DWORD pid)
{
    wchar_t path[MAX_PATH];
    if (!tm_image_path(pid, path, _countof(path))) {
        tm_error(g_hwndMain, L"Cannot query the image path.");
        return;
    }
    if (!PathRemoveFileSpecW(path)) {
        tm_error(g_hwndMain, L"Cannot locate the containing folder.");
        return;
    }
    ShellExecuteW(g_hwndMain, L"open", path, NULL, NULL, SW_SHOWNORMAL);
}

/* Online search and third-party hash lookup are intentionally omitted: process
 * names and image hashes stay on the local machine. */

/* Resetting the desktop shell is omitted until LoOS publishes a shell
 * lifecycle contract. Task Manager must not terminate or guess a shell process.
 */

/* Firewall containment is omitted until LoOS exposes a native policy API.
 * This app must not shell out to Windows-specific netsh commands.
 */

/* Preferences: alerts, heat colors, caps (HKCU\Software\LoOS\TaskMgr). */
static void tm_prefs_load(void)
{
    HKEY h;
    DWORD v, cb;
    int i;
    if (RegOpenKeyExW(HKEY_CURRENT_USER, L"Software\\LoOS\\TaskMgr", 0,
                      KEY_READ, &h) != ERROR_SUCCESS)
        return;
    cb = sizeof(v);
    if (RegQueryValueExW(h, L"AlertCpu", NULL, NULL, (LPBYTE)&v, &cb) ==
        ERROR_SUCCESS)
        g_alertCpuPct = (int)v;
    cb = sizeof(v);
    if (RegQueryValueExW(h, L"AlertRam", NULL, NULL, (LPBYTE)&v, &cb) ==
        ERROR_SUCCESS)
        g_alertRamPct = (int)v;
    cb = sizeof(v);
    if (RegQueryValueExW(h, L"AlertSecs", NULL, NULL, (LPBYTE)&v, &cb) ==
        ERROR_SUCCESS && v >= 5 && v <= 3600)
        g_alertSecs = (int)v;
    cb = sizeof(v);
    if (RegQueryValueExW(h, L"ColLo", NULL, NULL, (LPBYTE)&v, &cb) ==
        ERROR_SUCCESS)
        g_colLo = (COLORREF)v;
    cb = sizeof(v);
    if (RegQueryValueExW(h, L"ColMid", NULL, NULL, (LPBYTE)&v, &cb) ==
        ERROR_SUCCESS)
        g_colMid = (COLORREF)v;
    cb = sizeof(v);
    if (RegQueryValueExW(h, L"ColHi", NULL, NULL, (LPBYTE)&v, &cb) ==
        ERROR_SUCCESS)
        g_colHi = (COLORREF)v;
    for (i = 0; i < TM_CAPS_N; i++) {
        wchar_t key[32], m[64];
        DWORD mb = 0, act = 0;
        cb = sizeof(m);
        StringCchPrintfW(key, _countof(key), L"Cap%dName", i);
        if (RegQueryValueExW(h, key, NULL, NULL, (LPBYTE)m, &cb) !=
            ERROR_SUCCESS || !m[0])
            continue;
        cb = sizeof(mb);
        StringCchPrintfW(key, _countof(key), L"Cap%dMB", i);
        RegQueryValueExW(h, key, NULL, NULL, (LPBYTE)&mb, &cb);
        cb = sizeof(act);
        StringCchPrintfW(key, _countof(key), L"Cap%dAct", i);
        RegQueryValueExW(h, key, NULL, NULL, (LPBYTE)&act, &cb);
        StringCchCopyW(g_caps[i].match, _countof(g_caps[i].match), m);
        g_caps[i].mb = mb;
        g_caps[i].action = (int)act;
        g_caps[i].used = 1;
    }
    RegCloseKey(h);
}

static void tm_prefs_save(void)
{
    HKEY h;
    int i;
    DWORD dw;
    if (RegCreateKeyExW(HKEY_CURRENT_USER, L"Software\\LoOS\\TaskMgr", 0,
                        NULL, 0, KEY_WRITE, NULL, &h,
                        NULL) != ERROR_SUCCESS)
        return;
    dw = (DWORD)g_alertCpuPct;
    RegSetValueExW(h, L"AlertCpu", 0, REG_DWORD, (BYTE *)&dw, sizeof(dw));
    dw = (DWORD)g_alertRamPct;
    RegSetValueExW(h, L"AlertRam", 0, REG_DWORD, (BYTE *)&dw, sizeof(dw));
    dw = (DWORD)g_alertSecs;
    RegSetValueExW(h, L"AlertSecs", 0, REG_DWORD, (BYTE *)&dw, sizeof(dw));
    dw = (DWORD)g_colLo;
    RegSetValueExW(h, L"ColLo", 0, REG_DWORD, (BYTE *)&dw, sizeof(dw));
    dw = (DWORD)g_colMid;
    RegSetValueExW(h, L"ColMid", 0, REG_DWORD, (BYTE *)&dw, sizeof(dw));
    dw = (DWORD)g_colHi;
    RegSetValueExW(h, L"ColHi", 0, REG_DWORD, (BYTE *)&dw, sizeof(dw));
    for (i = 0; i < TM_CAPS_N; i++) {
        wchar_t key[32];
        StringCchPrintfW(key, _countof(key), L"Cap%dName", i);
        if (g_caps[i].used)
            RegSetValueExW(h, key, 0, REG_SZ, (BYTE *)g_caps[i].match,
                           (DWORD)((wcslen(g_caps[i].match) + 1) *
                                   sizeof(wchar_t)));
        else
            RegDeleteValueW(h, key);
        StringCchPrintfW(key, _countof(key), L"Cap%dMB", i);
        dw = g_caps[i].mb;
        RegSetValueExW(h, key, 0, REG_DWORD, (BYTE *)&dw, sizeof(dw));
        StringCchPrintfW(key, _countof(key), L"Cap%dAct", i);
        dw = (DWORD)g_caps[i].action;
        RegSetValueExW(h, key, 0, REG_DWORD, (BYTE *)&dw, sizeof(dw));
    }
    RegCloseKey(h);
}

/* NPU presence via device descriptions (per-model attribution needs the
 * vendor driver, which exposes no public Win32 API - stated in the UI). */
static int tm_npu_present(void)
{
    HDEVINFO devs;
    SP_DEVINFO_DATA did;
    DWORD i = 0;
    int found = 0;
    devs = SetupDiGetClassDevsW(NULL, NULL, NULL,
                                DIGCF_PRESENT | DIGCF_ALLCLASSES);
    if (devs == INVALID_HANDLE_VALUE)
        return 0;
    memset(&did, 0, sizeof(did));
    did.cbSize = sizeof(did);
    while (SetupDiEnumDeviceInfo(devs, i++, &did)) {
        wchar_t desc[256];
        if (SetupDiGetDeviceRegistryPropertyW(devs, &did, SPDRP_DEVICEDESC,
                                              NULL, (PBYTE)desc,
                                              sizeof(desc), NULL)) {
            if (wcsstr(desc, L"NPU") || wcsstr(desc, L"Neural") ||
                wcsstr(desc, L"neural")) {
                found = 1;
                break;
            }
        }
    }
    SetupDiDestroyDeviceInfoList(devs);
    return found;
}

/* --------------------------------------------------------------------------
 * "Why is this slow?" - root-cause analysis for hung/slow processes.
 * Uses real signals: hung-window API, WM_NULL round-trip timing, per-thread
 * CPU totals, memory size. No generic "Not Responding" shrug.
 * -------------------------------------------------------------------------- */
struct TM_WHY_CTX {
    DWORD pid;
};

static INT_PTR CALLBACK tm_why_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    static struct TM_WHY_CTX s_ctx;
    if (msg == WM_INITDIALOG) {
        TM_PROC *p;
        HWND wnd;
        wchar_t info[2048], line[512];
        s_ctx = *(struct TM_WHY_CTX *)(INT_PTR)l;
        p = tm_find_proc(s_ctx.pid);
        if (!p) {
            SetDlgItemTextW(dlg, IDE_INFO, L"Process already gone.");
            return TRUE;
        }
        StringCchPrintfW(info, _countof(info),
                         L"%s  (PID %lu)\r\nCPU %.1f%%   Memory %zu MB   "
                         L"Threads %lu\r\n\r\n",
                         p->name, p->pid, p->cpu, p->memKB / 1024,
                         p->threads);
        wnd = tm_proc_mainwnd(p->pid);
        if (!wnd) {
            StringCchCatW(info, _countof(info),
                          L"DIAGNOSIS: background process (no window).\r\n"
                          L"It cannot \"hang\" a UI; if the system is slow, "
                          L"check its CPU and I/O columns instead.\r\n");
        } else if (IsHungAppWindow(wnd)) {
            DWORD_PTR res = 0;
            LRESULT r = SendMessageTimeoutW(wnd, WM_NULL, 0, 0,
                                            SMTO_ABORTIFHUNG, 2000, &res);
            StringCchCatW(info, _countof(info),
                          L"DIAGNOSIS: message pump stall. The window is "
                          L"not pumping messages");
            if (r == 0)
                StringCchCatW(info, _countof(info),
                              L" (WM_NULL round-trip timed out after 2s");
            StringCchCatW(info, _countof(info),
                          L". Typical causes: blocked file/network I/O, a "
                          L"deadlocked lock, or a modal loop.\r\n");
            if (p->cpu > 20.0)
                StringCchCatW(info, _countof(info),
                              L"NOTE: it is also burning CPU, so a hot "
                              L"loop is likely - see top threads below.\r\n");
            StringCchCatW(info, _countof(info),
                          L"\r\nSUGGESTIONS:\r\n- Wait for a long "
                          L"operation (installers, big saves).\r\n"
                          L"- End task, or Pause it to free the CPU and "
                          L"resume later.\r\n"
                          L"- Set an automatic cap (View menu) if it "
                          L"recurs.\r\n");
        } else if (p->cpu > 50.0) {
            StringCchCatW(info, _countof(info),
                          L"DIAGNOSIS: responsive window but spinning "
                          L"(high CPU). Likely a busy loop or heavy "
                          L"compute, not a freeze.\r\n\r\nSUGGESTIONS:\r\n"
                          L"- Lower its priority or restrict affinity.\r\n"
                          L"- Add an automatic cap to suspend it over a "
                          L"memory/CPU budget.\r\n");
        } else {
            StringCchCatW(info, _countof(info),
                          L"DIAGNOSIS: window is pumping messages and CPU "
                          L"is low - not actually frozen. Slowness likely "
                          L"comes from disk, network, or another "
                          L"process.\r\n");
        }
        if (p->memKB / 1024 > 2000) {
            StringCchPrintfW(line, _countof(line),
                             L"\r\nWARNING: %zu MB working set - under "
                             L"memory pressure this process will stutter.",
                             p->memKB / 1024);
            StringCchCatW(info, _countof(info), line);
        }
        /* Top threads by total CPU time (Toolhelp + GetThreadTimes). */
        {
            HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            THREADENTRY32 te;
            struct {
                DWORD tid;
                ULONGLONG t;
            } top[5] = { { 0, 0 } };
            int ti, tj;
            if (snap != INVALID_HANDLE_VALUE) {
                memset(&te, 0, sizeof(te));
                te.dwSize = sizeof(te);
                if (Thread32First(snap, &te)) {
                    do {
                        if (te.th32OwnerProcessID == p->pid) {
                            HANDLE th = OpenThread(THREAD_QUERY_INFORMATION,
                                                   FALSE, te.th32ThreadID);
                            if (th) {
                                FILETIME c, e, k, u;
                                ULARGE_INTEGER ke, us;
                                if (GetThreadTimes(th, &c, &e, &k, &u)) {
                                    ULONGLONG t;
                                    ke.LowPart = k.dwLowDateTime;
                                    ke.HighPart = k.dwHighDateTime;
                                    us.LowPart = u.dwLowDateTime;
                                    us.HighPart = u.dwHighDateTime;
                                    t = ke.QuadPart + us.QuadPart;
                                    for (ti = 0; ti < 5; ti++)
                                        if (t > top[ti].t) {
                                            for (tj = 4; tj > ti; tj--)
                                                top[tj] = top[tj - 1];
                                            top[ti].tid = te.th32ThreadID;
                                            top[ti].t = t;
                                            break;
                                        }
                                }
                                CloseHandle(th);
                            }
                        }
                    } while (Thread32Next(snap, &te));
                }
                CloseHandle(snap);
            }
            StringCchCatW(info, _countof(info),
                          L"\r\nTop threads (CPU seconds, lifetime):\r\n");
            for (ti = 0; ti < 5 && top[ti].tid; ti++) {
                StringCchPrintfW(line, _countof(line), L"  TID %lu: %.1fs\r\n",
                                 top[ti].tid, top[ti].t / 1e7);
                StringCchCatW(info, _countof(info), line);
            }
        }
        SetDlgItemTextW(dlg, IDE_INFO, info);
        return TRUE;
    }
    if (msg == WM_COMMAND && (LOWORD(w) == IDOK || LOWORD(w) == IDCANCEL)) {
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    return FALSE;
}

static void tm_whyhung_dialog(HWND owner, DWORD pid)
{
    struct TM_WHY_CTX ctx;
    ctx.pid = pid;
    DialogBoxParamW(g_hInst, MAKEINTRESOURCEW(IDD_WHYHUNG), owner,
                    tm_why_proc, (LPARAM)(INT_PTR)&ctx);
}

/* --------------------------------------------------------------------------
 * Local TCP/UDP connection table for one process. Addresses remain numeric;
 * resolving them would issue DNS queries outside the machine.
 * -------------------------------------------------------------------------- */
struct TM_CONN_CTX {
    DWORD pid;
};

static void tm_format_ipv4(DWORD address, wchar_t *out, size_t cch)
{
    StringCchPrintfW(out, cch, L"%u.%u.%u.%u",
                     (unsigned)(address & 0xff),
                     (unsigned)((address >> 8) & 0xff),
                     (unsigned)((address >> 16) & 0xff),
                     (unsigned)((address >> 24) & 0xff));
}

static unsigned short tm_port_host_order(unsigned short port)
{
    return (unsigned short)((port >> 8) | (port << 8));
}

static void tm_conn_fill(HWND dlg, DWORD pid)
{
    HWND lb = GetDlgItem(dlg, IDL_LIST);
    DWORD size = 0, i;
    MIB_TCPTABLE_OWNER_PID *tcp = NULL;
    MIB_UDPTABLE_OWNER_PID *udp = NULL;
    SendMessageW(lb, LB_RESETCONTENT, 0, 0);
    GetExtendedTcpTable(NULL, &size, FALSE, AF_INET,
                        TCP_TABLE_OWNER_PID_ALL, 0);
    if (size > 0 && size < (1 << 24)) {
        tcp = (MIB_TCPTABLE_OWNER_PID *)malloc(size);
        if (tcp && GetExtendedTcpTable(tcp, &size, FALSE, AF_INET,
                                       TCP_TABLE_OWNER_PID_ALL, 0) != NO_ERROR) {
            free(tcp);
            tcp = NULL;
        }
    }
    if (tcp) {
        for (i = 0; i < tcp->dwNumEntries; i++) {
            MIB_TCPROW_OWNER_PID *r = &tcp->table[i];
            wchar_t line[512], lip[64], rip[64];
            const wchar_t *state = L"?";
            if (r->dwOwningPid != pid)
                continue;
            tm_format_ipv4(r->dwLocalAddr, lip, _countof(lip));
            tm_format_ipv4(r->dwRemoteAddr, rip, _countof(rip));
            switch (r->dwState) {
            case MIB_TCP_STATE_ESTAB: state = L"ESTABLISHED"; break;
            case MIB_TCP_STATE_LISTEN: state = L"LISTEN"; break;
            case MIB_TCP_STATE_TIME_WAIT: state = L"TIME_WAIT"; break;
            case MIB_TCP_STATE_CLOSE_WAIT: state = L"CLOSE_WAIT"; break;
            case MIB_TCP_STATE_SYN_SENT: state = L"SYN_SENT"; break;
            default: break;
            }
            StringCchPrintfW(line, _countof(line), L"TCP %s:%u -> %s:%u  %s",
                             lip, tm_port_host_order((unsigned short)r->dwLocalPort),
                             rip, tm_port_host_order((unsigned short)r->dwRemotePort), state);
            SendMessageW(lb, LB_ADDSTRING, 0, (LPARAM)(INT_PTR)line);
        }
        free(tcp);
    }
    size = 0;
    GetExtendedUdpTable(NULL, &size, FALSE, AF_INET, UDP_TABLE_OWNER_PID, 0);
    if (size > 0 && size < (1 << 24)) {
        udp = (MIB_UDPTABLE_OWNER_PID *)malloc(size);
        if (udp && GetExtendedUdpTable(udp, &size, FALSE, AF_INET,
                                       UDP_TABLE_OWNER_PID, 0) != NO_ERROR) {
            free(udp);
            udp = NULL;
        }
    }
    if (udp) {
        for (i = 0; i < udp->dwNumEntries; i++) {
            MIB_UDPROW_OWNER_PID *r = &udp->table[i];
            wchar_t line[256], lip[64];
            if (r->dwOwningPid != pid)
                continue;
            tm_format_ipv4(r->dwLocalAddr, lip, _countof(lip));
            StringCchPrintfW(line, _countof(line), L"UDP %s:%u", lip,
                             tm_port_host_order((unsigned short)r->dwLocalPort));
            SendMessageW(lb, LB_ADDSTRING, 0, (LPARAM)(INT_PTR)line);
        }
        free(udp);
    }
    if (SendMessageW(lb, LB_GETCOUNT, 0, 0) == 0)
        SendMessageW(lb, LB_ADDSTRING, 0,
                     (LPARAM)(INT_PTR)L"(no TCP/UDP endpoints)");
}

static INT_PTR CALLBACK tm_conn_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    static struct TM_CONN_CTX s_ctx;
    if (msg == WM_INITDIALOG) {
        TM_PROC *p;
        wchar_t cap[300];
        s_ctx = *(struct TM_CONN_CTX *)(INT_PTR)l;
        p = tm_find_proc(s_ctx.pid);
        StringCchPrintfW(cap, _countof(cap), L"Connections - %s (%lu)",
                         p ? p->name : L"?", s_ctx.pid);
        SetWindowTextW(dlg, cap);
        tm_conn_fill(dlg, s_ctx.pid);
        return TRUE;
    }
    if (msg == WM_COMMAND) {
        WORD id = LOWORD(w);
        if (id == IDB_COPYLINE) {
            HWND lb = GetDlgItem(dlg, IDL_LIST);
            int sel = (int)SendMessageW(lb, LB_GETCURSEL, 0, 0);
            if (sel >= 0) {
                int len = (int)SendMessageW(lb, LB_GETTEXTLEN, (WPARAM)sel,
                                            0);
                HGLOBAL g = GlobalAlloc(GMEM_MOVEABLE,
                                        ((size_t)len + 1) * sizeof(wchar_t));
                if (g) {
                    wchar_t *lock = (wchar_t *)GlobalLock(g);
                    if (lock) {
                        SendMessageW(lb, LB_GETTEXT, (WPARAM)sel,
                                     (LPARAM)(INT_PTR)lock);
                        GlobalUnlock(g);
                        if (OpenClipboard(dlg)) {
                            EmptyClipboard();
                            SetClipboardData(CF_UNICODETEXT, g);
                            CloseClipboard();
                            g = NULL;
                        }
                    }
                    if (g)
                        GlobalFree(g);
                }
            }
            return TRUE;
        }
        if (id == IDOK || id == IDCANCEL) {
            EndDialog(dlg, IDOK);
            return TRUE;
        }
    }
    return FALSE;
}

static void tm_conn_dialog(HWND owner, DWORD pid)
{
    struct TM_CONN_CTX ctx;
    ctx.pid = pid;
    DialogBoxParamW(g_hInst, MAKEINTRESOURCEW(IDD_CONN), owner,
                    tm_conn_proc, (LPARAM)(INT_PTR)&ctx);
}

/* --------------------------------------------------------------------------
 * Thread-count timeline for one process (catches creation spikes/leaks).
 * -------------------------------------------------------------------------- */
struct TM_THR_CTX {
    DWORD pid;
    int hist[120];
    int n;
};

static int tm_thread_count(DWORD pid)
{
    HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
    THREADENTRY32 te;
    int c = 0;
    if (snap == INVALID_HANDLE_VALUE)
        return -1;
    memset(&te, 0, sizeof(te));
    te.dwSize = sizeof(te);
    if (Thread32First(snap, &te)) {
        do {
            if (te.th32OwnerProcessID == pid)
                c++;
        } while (Thread32Next(snap, &te));
    }
    CloseHandle(snap);
    return c;
}

static INT_PTR CALLBACK tm_thr_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    static struct TM_THR_CTX s_ctx;
    if (msg == WM_INITDIALOG) {
        TM_PROC *p;
        wchar_t cap[300];
        s_ctx = *(struct TM_THR_CTX *)(INT_PTR)l;
        memset(s_ctx.hist, 0, sizeof(s_ctx.hist));
        s_ctx.n = 0;
        p = tm_find_proc(s_ctx.pid);
        StringCchPrintfW(cap, _countof(cap), L"Threads - %s (%lu)",
                         p ? p->name : L"?", s_ctx.pid);
        SetWindowTextW(dlg, cap);
        SetTimer(dlg, 7, 500, NULL);
        return TRUE;
    }
    if (msg == WM_TIMER) {
        int c = tm_thread_count(s_ctx.pid);
        HWND lb;
        wchar_t line[128];
        if (c < 0) {
            KillTimer(dlg, 7);
            SetDlgItemTextW(dlg, IDE_INFO, L"Process exited.");
            return TRUE;
        }
        if (s_ctx.n < 120)
            s_ctx.hist[s_ctx.n++] = c;
        else {
            memmove(s_ctx.hist, s_ctx.hist + 1, sizeof(int) * 119);
            s_ctx.hist[119] = c;
        }
        lb = GetDlgItem(dlg, IDL_LIST);
        StringCchPrintfW(line, _countof(line), L"threads: %d", c);
        SetDlgItemTextW(dlg, IDE_INFO, line);
        SendMessageW(lb, LB_ADDSTRING, 0, (LPARAM)(INT_PTR)line);
        while (SendMessageW(lb, LB_GETCOUNT, 0, 0) > 40)
            SendMessageW(lb, LB_DELETESTRING, 0, 0);
        SendMessageW(lb, LB_SETCARETINDEX,
                     SendMessageW(lb, LB_GETCOUNT, 0, 0) - 1, FALSE);
        InvalidateRect(dlg, NULL, FALSE);
        return TRUE;
    }
    if (msg == WM_PAINT) {
        PAINTSTRUCT ps;
        HDC hdc = BeginPaint(dlg, &ps);
        RECT rc, gr;
        int i, max = 1;
        GetClientRect(dlg, &rc);
        gr.left = 12;
        gr.right = rc.right - 12;
        gr.top = rc.bottom - 130;
        gr.bottom = rc.bottom - 12;
        FillRect(hdc, &gr, GetStockBrush(BLACK_BRUSH));
        for (i = 0; i < s_ctx.n; i++)
            if (s_ctx.hist[i] > max)
                max = s_ctx.hist[i];
        if (s_ctx.n > 1) {
            HPEN pen = CreatePen(PS_SOLID, 2, RGB(LOOS_BRAND_ACCENT_R, LOOS_BRAND_ACCENT_G, LOOS_BRAND_ACCENT_B));
            HPEN old = (HPEN)SelectObject(hdc, pen);
            for (i = 0; i < s_ctx.n; i++) {
                int x = gr.left + (i * (gr.right - gr.left)) / 119;
                int y = gr.bottom - 2 -
                        (s_ctx.hist[i] * (gr.bottom - gr.top - 4)) / max;
                if (i == 0)
                    MoveToEx(hdc, x, y, NULL);
                else
                    LineTo(hdc, x, y);
            }
            SelectObject(hdc, old);
            DeleteObject(pen);
        }
        EndPaint(dlg, &ps);
        return TRUE;
    }
    if (msg == WM_COMMAND && (LOWORD(w) == IDOK || LOWORD(w) == IDCANCEL)) {
        KillTimer(dlg, 7);
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    return FALSE;
}

static void tm_threads_dialog(HWND owner, DWORD pid)
{
    struct TM_THR_CTX ctx;
    ctx.pid = pid;
    DialogBoxParamW(g_hInst, MAKEINTRESOURCEW(IDD_THREADS), owner,
                    tm_thr_proc, (LPARAM)(INT_PTR)&ctx);
}

/* --------------------------------------------------------------------------
 * GPU driver (TDR) events from the System event log. Answers "which display
 * driver failed and why did it reload" with real log records.
 * -------------------------------------------------------------------------- */
static INT_PTR CALLBACK tm_tdr_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    (void)l;
    if (msg == WM_INITDIALOG) {
        HANDLE log;
        HWND lb = GetDlgItem(dlg, IDL_LIST);
        DWORD read = 0, need = 0, flags;
        BYTE fixed[65536];
        int shown = 0;
        (void)flags;
        log = OpenEventLogW(NULL, L"System");
        if (!log) {
            SendMessageW(lb, LB_ADDSTRING, 0,
                         (LPARAM)(INT_PTR)L"Cannot open the System log.");
            return TRUE;
        }
        GetNumberOfEventLogRecords(log, &need);
        while (shown < 60) {
            EVENTLOGRECORD *r = (EVENTLOGRECORD *)fixed;
            if (!ReadEventLogW(log,
                               EVENTLOG_BACKWARDS_READ |
                                   EVENTLOG_SEQUENTIAL_READ,
                               0, fixed, sizeof(fixed), &read, &need) ||
                read == 0)
                break;
            {
                BYTE *p = fixed;
                DWORD left = read;
                while (left >= sizeof(EVENTLOGRECORD)) {
                    r = (EVENTLOGRECORD *)p;
                    if (r->Length < sizeof(EVENTLOGRECORD) ||
                        r->Length > left)
                        break;
                    {
                        const wchar_t *src =
                            (const wchar_t *)(p + sizeof(*r));
                        wchar_t line[512];
                        SYSTEMTIME st;
                        FILETIME ft;
                        ULARGE_INTEGER ull;
                        ull.LowPart = r->TimeGenerated;
                        ull.HighPart = 0;
                        /* TimeGenerated is seconds since 1970. */
                        ft.dwLowDateTime = 0;
                        ft.dwHighDateTime = 0;
                        {
                            /* 11644473600 = FILETIME epoch delta. */
                            ULONGLONG t = (ULONGLONG)r->TimeGenerated +
                                          11644473600ULL;
                            t *= 10000000ULL;
                            ft.dwLowDateTime = (DWORD)t;
                            ft.dwHighDateTime = (DWORD)(t >> 32);
                            FileTimeToLocalFileTime(&ft, &ft);
                            FileTimeToSystemTime(&ft, &st);
                        }
                        if (wcsstr(src, L"nvlddmkm") ||
                            wcsstr(src, L"amdkmdap") ||
                            wcsstr(src, L"amduw") ||
                            wcsstr(src, L"igfx") ||
                            wcsstr(src, L"Display") ||
                            wcsstr(src, L"dxgkrnl") ||
                            r->EventID == 4101 || r->EventID == 4109 ||
                            r->EventID == 4115 || r->EventID == 14 ||
                            r->EventID == 13) {
                            const wchar_t *why = L"";
                            if (r->EventID == 4101)
                                why = L" - driver stopped responding "
                                      L"(TDR recovery)";
                            else if (r->EventID == 4109)
                                why = L" - driver preempted (long "
                                      L"operation)";
                            StringCchPrintfW(
                                line, _countof(line),
                                L"%02u:%02u %s  id=%lu%s", st.wHour,
                                st.wMinute, src,
                                r->EventID & 0xFFFF, why);
                            SendMessageW(lb, LB_ADDSTRING, 0,
                                         (LPARAM)(INT_PTR)line);
                            if (++shown >= 60)
                                break;
                        }
                    }
                    p += r->Length;
                    left -= r->Length;
                }
            }
            if (need == 0)
                break;
        }
        CloseEventLog(log);
        if (shown == 0)
            SendMessageW(lb, LB_ADDSTRING, 0,
                         (LPARAM)(INT_PTR)L"No display-driver events found "
                         L"(healthy driver, or log rotated).");
        return TRUE;
    }
    if (msg == WM_COMMAND && (LOWORD(w) == IDOK || LOWORD(w) == IDCANCEL)) {
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    return FALSE;
}

static void tm_tdr_dialog(HWND owner)
{
    DialogBoxW(g_hInst, MAKEINTRESOURCEW(IDD_TDR), owner, tm_tdr_proc);
}

/* --------------------------------------------------------------------------
 * Alerts config, heat colors, caps editor, dead log, CSV export, mini graph.
 * -------------------------------------------------------------------------- */
static INT_PTR CALLBACK tm_alerts_proc(HWND dlg, UINT msg, WPARAM w,
                                       LPARAM l)
{
    (void)l;
    if (msg == WM_INITDIALOG) {
        SetDlgItemInt(dlg, IDE_CPU_TH, (UINT)g_alertCpuPct, FALSE);
        SetDlgItemInt(dlg, IDE_RAM_TH, (UINT)g_alertRamPct, FALSE);
        SetDlgItemInt(dlg, IDE_SEC_TH, (UINT)g_alertSecs, FALSE);
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDOK) {
        g_alertCpuPct = (int)GetDlgItemInt(dlg, IDE_CPU_TH, NULL, FALSE);
        g_alertRamPct = (int)GetDlgItemInt(dlg, IDE_RAM_TH, NULL, FALSE);
        g_alertSecs = (int)GetDlgItemInt(dlg, IDE_SEC_TH, NULL, FALSE);
        if (g_alertCpuPct < 0 || g_alertCpuPct > 100)
            g_alertCpuPct = 0;
        if (g_alertRamPct < 0 || g_alertRamPct > 100)
            g_alertRamPct = 0;
        if (g_alertSecs < 5)
            g_alertSecs = 5;
        if (g_alertSecs > 3600)
            g_alertSecs = 3600;
        g_overCpuSince = g_overRamSince = 0;
        tm_prefs_save();
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDCANCEL) {
        EndDialog(dlg, IDCANCEL);
        return TRUE;
    }
    return FALSE;
}

static void tm_alerts_dialog(HWND owner)
{
    DialogBoxW(g_hInst, MAKEINTRESOURCEW(IDD_ALERTS), owner, tm_alerts_proc);
}

static INT_PTR CALLBACK tm_colors_proc(HWND dlg, UINT msg, WPARAM w,
                                       LPARAM l)
{
    static COLORREF s_custom[16];
    (void)l;
    if (msg == WM_INITDIALOG)
        return TRUE;
    if (msg == WM_CTLCOLORSTATIC) {
        HDC hdc = (HDC)w;
        HWND ctl = (HWND)(INT_PTR)l;
        int id = GetDlgCtrlID(ctl);
        static HBRUSH br;
        if (id == IDB_SWATCH_LO || id == IDB_SWATCH_MID ||
            id == IDB_SWATCH_HI) {
            COLORREF c = (id == IDB_SWATCH_LO) ? g_colLo
                         : (id == IDB_SWATCH_MID) ? g_colMid
                                                  : g_colHi;
            if (br)
                DeleteObject(br);
            br = CreateSolidBrush(c);
            SetBkColor(hdc, c);
            return (INT_PTR)br;
        }
    }
    if (msg == WM_COMMAND && (LOWORD(w) == IDB_SWATCH_LO ||
                              LOWORD(w) == IDB_SWATCH_MID ||
                              LOWORD(w) == IDB_SWATCH_HI)) {
        CHOOSECOLORW cc;
        static COLORREF cust[16];
        memset(&cc, 0, sizeof(cc));
        cc.lStructSize = sizeof(cc);
        cc.hwndOwner = dlg;
        cc.lpCustColors = cust;
        cc.Flags = CC_FULLOPEN | CC_RGBINIT;
        cc.rgbResult = (LOWORD(w) == IDB_SWATCH_LO) ? g_colLo
                       : (LOWORD(w) == IDB_SWATCH_MID) ? g_colMid
                                                       : g_colHi;
        if (ChooseColorW(&cc)) {
            if (LOWORD(w) == IDB_SWATCH_LO)
                g_colLo = cc.rgbResult;
            else if (LOWORD(w) == IDB_SWATCH_MID)
                g_colMid = cc.rgbResult;
            else
                g_colHi = cc.rgbResult;
            InvalidateRect(dlg, NULL, FALSE);
        }
        (void)s_custom;
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDOK) {
        tm_prefs_save();
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDCANCEL) {
        EndDialog(dlg, IDCANCEL);
        return TRUE;
    }
    return FALSE;
}

static void tm_colors_dialog(HWND owner)
{
    DialogBoxW(g_hInst, MAKEINTRESOURCEW(IDD_COLORS), owner, tm_colors_proc);
}

static void tm_caps_fill(HWND dlg)
{
    HWND lb = GetDlgItem(dlg, IDL_LIST);
    int i;
    static const wchar_t *acts[] = { L"suspend", L"terminate", L"alert" };
    SendMessageW(lb, LB_RESETCONTENT, 0, 0);
    for (i = 0; i < TM_CAPS_N; i++) {
        if (!g_caps[i].used)
            continue;
        {
            wchar_t line[160];
            StringCchPrintfW(line, _countof(line), L"%s  over %lu MB  -> %s",
                             g_caps[i].match, g_caps[i].mb,
                             acts[g_caps[i].action % 3]);
            SendMessageW(lb, LB_ADDSTRING, 0, (LPARAM)(INT_PTR)line);
        }
    }
}

static INT_PTR CALLBACK tm_caps_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    (void)l;
    if (msg == WM_INITDIALOG) {
        HWND cb = GetDlgItem(dlg, IDC_CAP_ACTION);
        SendMessageW(cb, CB_ADDSTRING, 0, (LPARAM)(INT_PTR)L"Suspend");
        SendMessageW(cb, CB_ADDSTRING, 0, (LPARAM)(INT_PTR)L"Terminate");
        SendMessageW(cb, CB_ADDSTRING, 0, (LPARAM)(INT_PTR)L"Alert only");
        SendMessageW(cb, CB_SETCURSEL, 0, 0);
        tm_caps_fill(dlg);
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDB_CAP_ADD) {
        wchar_t name[64];
        int i, slot = -1;
        DWORD mb = GetDlgItemInt(dlg, IDE_CAP_MB, NULL, FALSE);
        int act = (int)SendMessageW(GetDlgItem(dlg, IDC_CAP_ACTION),
                                    CB_GETCURSEL, 0, 0);
        GetDlgItemTextW(dlg, IDE_CAP_NAME, name, _countof(name));
        if (!name[0] || mb == 0) {
            MessageBoxW(dlg, L"Enter a name fragment and an MB limit.",
                        S_APP, MB_OK | MB_ICONINFORMATION);
            return TRUE;
        }
        for (i = 0; i < TM_CAPS_N; i++)
            if (!g_caps[i].used) {
                slot = i;
                break;
            }
        if (slot < 0) {
            MessageBoxW(dlg, L"Rule list is full.", S_APP,
                        MB_OK | MB_ICONINFORMATION);
            return TRUE;
        }
        StringCchCopyW(g_caps[slot].match, _countof(g_caps[slot].match),
                       name);
        g_caps[slot].mb = mb;
        g_caps[slot].action = act < 0 ? 0 : act;
        g_caps[slot].used = 1;
        tm_prefs_save();
        tm_caps_fill(dlg);
        return TRUE;
    }
    if (msg == WM_COMMAND && LOWORD(w) == IDB_CAP_DEL) {
        HWND lb = GetDlgItem(dlg, IDL_LIST);
        int sel = (int)SendMessageW(lb, LB_GETCURSEL, 0, 0), k = -1, i;
        if (sel < 0)
            return TRUE;
        for (i = 0; i < TM_CAPS_N; i++) {
            if (!g_caps[i].used)
                continue;
            if (++k == sel) {
                g_caps[i].used = 0;
                break;
            }
        }
        tm_prefs_save();
        tm_caps_fill(dlg);
        return TRUE;
    }
    if (msg == WM_COMMAND && (LOWORD(w) == IDOK || LOWORD(w) == IDCANCEL)) {
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    return FALSE;
}

static void tm_caps_dialog(HWND owner)
{
    DialogBoxW(g_hInst, MAKEINTRESOURCEW(IDD_CAPS), owner, tm_caps_proc);
}

static INT_PTR CALLBACK tm_dead_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    (void)l;
    if (msg == WM_INITDIALOG) {
        HWND lb = GetDlgItem(dlg, IDL_LIST);
        int i;
        for (i = 0; i < g_deadCount; i++) {
            const TM_DEAD *d =
                &g_dead[(g_deadHead - 1 - i + TM_DEAD_N * 2) % TM_DEAD_N];
            wchar_t line[320];
            StringCchPrintfW(line, _countof(line),
                             L"%02u:%02u:%02u  %-24s pid=%-6lu cpu=%4.1f%% "
                             L"mem=%zuMB",
                             d->when.wHour, d->when.wMinute,
                             d->when.wSecond, d->name, d->pid, d->cpu,
                             d->memKB / 1024);
            SendMessageW(lb, LB_ADDSTRING, 0, (LPARAM)(INT_PTR)line);
        }
        if (g_deadCount == 0)
            SendMessageW(lb, LB_ADDSTRING, 0,
                         (LPARAM)(INT_PTR)L"(nothing terminated while "
                         L"Task Manager was watching)");
        return TRUE;
    }
    if (msg == WM_COMMAND && (LOWORD(w) == IDOK || LOWORD(w) == IDCANCEL)) {
        EndDialog(dlg, IDOK);
        return TRUE;
    }
    return FALSE;
}

static void tm_dead_dialog(HWND owner)
{
    DialogBoxW(g_hInst, MAKEINTRESOURCEW(IDD_DEADLOG), owner, tm_dead_proc);
}

/* One-click CSV: history rings + full process table (Save dialog). */
static void tm_export_csv(HWND owner)
{
    wchar_t path[MAX_PATH] = L"taskmgr-log.csv";
    OPENFILENAMEW ofn;
    FILE *f;
    size_t i;
    int k;
    memset(&ofn, 0, sizeof(ofn));
    ofn.lStructSize = sizeof(ofn);
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = L"CSV files (*.csv)\0*.csv\0All files (*.*)\0*.*\0";
    ofn.lpstrFile = path;
    ofn.nMaxFile = _countof(path);
    ofn.lpstrDefExt = L"csv";
    ofn.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
    if (!GetSaveFileNameW(&ofn))
        return;
    if (_wfopen_s(&f, path, L"w, ccs=UTF-8") != 0 || !f) {
        tm_error(owner, L"Cannot write the file.");
        return;
    }
    fwprintf(f, L"# LoOS Task Manager export (%d samples, ~%ds cadence)\n",
             g_histCount < TM_HIST_N ? g_histCount : TM_HIST_N,
             g_speedMs / 1000);
    fwprintf(f, L"sample,cpu_pct,mem_pct,disk_Bps,net_Bps,vpn_Bps\n");
    for (k = 0; k < (g_histCount < TM_HIST_N ? g_histCount : TM_HIST_N);
         k++) {
        int idx = g_histCount <= TM_HIST_N
                      ? k
                      : (g_histCount - TM_HIST_N + k) % TM_HIST_N;
        fwprintf(f, L"%d,%.1f,%.1f,%.0f,%.0f,%.0f\n", k, g_cpuHist[idx],
                 g_memHist[idx], g_diskHist[idx], g_netHist[idx],
                 (g_histCount - 1 + TM_HIST_N) % TM_HIST_N == idx
                     ? g_vpnBps
                     : 0.0);
    }
    fwprintf(f, L"\nname,pid,ppid,cpu_pct,mem_KB,threads,iops_r,iops_s,"
                L"net_in_B,net_out_B,gpu_ded_B,gpu_shr_B,user,desc\n");
    for (i = 0; i < g_procCount; i++) {
        TM_PROC *p = &g_procs[i];
        fwprintf(f, L"\"%s\",%lu,%lu,%.1f,%zu,%lu,%.0f,%.0f,%llu,%llu,"
                    L"%.0f,%.0f,\"%s\",\"%s\"\n",
                 p->name, p->pid, p->ppid, p->cpu, p->memKB, p->threads,
                 p->iopsR, p->iopsW, p->netIn, p->netOut, p->gpuDed,
                 p->gpuShr, p->user, p->desc);
    }
    fclose(f);
    tm_balloon(L"Exported", L"Performance log written to CSV.");
}

/* Detachable always-on-top mini CPU graph. */
static LRESULT CALLBACK tm_mini_proc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    if (m == WM_PAINT) {
        PAINTSTRUCT ps;
        HDC hdc = BeginPaint(h, &ps);
        RECT rc;
        wchar_t t[64];
        int n, x;
        GetClientRect(h, &rc);
        FillRect(hdc, &rc, GetStockBrush(BLACK_BRUSH));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, RGB(0, 255, 128));
        StringCchPrintfW(t, _countof(t), L"CPU %.0f%%", g_cpuTotal);
        DrawTextW(hdc, t, -1, &rc, DT_TOP | DT_LEFT | DT_SINGLELINE);
        n = g_histCount < TM_HIST_N ? g_histCount : TM_HIST_N;
        if (n > 1) {
            HPEN pen = CreatePen(PS_SOLID, 2, RGB(0, 220, 120));
            HPEN old = (HPEN)SelectObject(hdc, pen);
            for (x = 0; x < n; x++) {
                double v =
                    g_cpuHist[(g_histCount - 1 - x + TM_HIST_N * 4) %
                              TM_HIST_N];
                int px = rc.right - 2 -
                         (x * (rc.right - 4)) / (TM_HIST_N - 1);
                int py = rc.bottom - 2 -
                         (int)((v / 100.0) * (rc.bottom - 28));
                if (x == 0)
                    MoveToEx(hdc, px, py, NULL);
                else
                    LineTo(hdc, px, py);
            }
            SelectObject(hdc, old);
            DeleteObject(pen);
        }
        EndPaint(h, &ps);
        return 0;
    }
    if (m == WM_TIMER) {
        InvalidateRect(h, NULL, FALSE);
        return 0;
    }
    if (m == WM_CLOSE) {
        tm_mini_toggle(); /* toggle off */
        return 0;
    }
    return DefWindowProcW(h, m, w, l);
}

static void tm_mini_toggle(void)
{
    if (g_miniGraph) {
        DestroyWindow(g_miniGraph);
        g_miniGraph = NULL;
        return;
    }
    {
        WNDCLASSW wc;
        memset(&wc, 0, sizeof(wc));
        wc.lpfnWndProc = tm_mini_proc;
        wc.hInstance = g_hInst;
        wc.lpszClassName = L"LoOSTMMini";
        wc.hCursor = LoadCursorW(NULL, IDC_ARROW);
        RegisterClassW(&wc); /* once is fine; re-register is harmless */
    }
    g_miniGraph = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW |
                                      WS_EX_NOACTIVATE,
                                  L"LoOSTMMini", L"CPU",
                                  WS_POPUP | WS_CAPTION | WS_VISIBLE |
                                      WS_SYSMENU,
                                  100, 100, 260, 150, NULL, NULL, g_hInst,
                                  NULL);
    if (g_miniGraph)
        SetTimer(g_miniGraph, 9, 1000, NULL);
}











