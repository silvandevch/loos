/* ============================================================================
 *  LoOS Notepad plugin SDK (np_plugin.h) - version 3 (v1/v2 compatible)
 * ============================================================================
 *  A plugin is a plain C DLL in %APPDATA%\LoOS\Notepad\plugins\*.dll that
 *  exports NpPluginVersion + NpPluginCommands, plus executors:
 *
 *    int NpPluginVersion(void);                       // return 1, 2 or 3
 *    int NpPluginCommands(const NP_PLUGIN_CMD **out); // static array + count
 *    void NpPluginExec(int id, HWND edit, HWND main);  // ABI v1 executor
 *    void NpPluginExec2(int id, const NP_HOST *host);  // ABI v2 executor
 *    void NpPluginExec3(int id, const NP_HOST3 *host); // ABI v3 executor
 *
 *  Optional event/idle entry points (any ABI version):
 *    void NpPluginEvent(int event, const NP_HOST *host);
 *      NP_EVENT_STARTUP/SHUTDOWN/OPEN/SAVE/TAB/CHANGE (1..6, notify only)
 *    void NpPluginIdle(const NP_HOST *host);  // ~2 s cadence, keep it fast
 *
 *  ABI negotiation: the host calls the best executor your version allows
 *  (Exec3 > Exec2 > Exec). NP_HOST3 starts with the exact v2 layout, so a
 *  v2 plugin reads it as NP_HOST safely. NEVER reorder the shared prefix.
 *
 *  NP_HOST services (v2): editBox, mainWnd, getSel/setSel,
 *  getTextLen/getText, replaceSel/replaceAll, getPath, setStatus,
 *  getConfigDir, confirm.
 *  NP_HOST3 adds: getLineCount/getLine/getCaretLine/gotoLine/getSelText,
 *  inputBox/messageBox, getPrivateDir/readPrivate/writePrivate,
 *  getEncoding/isDirty/openFile.
 *
 *  Rules: ids 0..11 per plugin, names are copied into the Tools menu,
 *  everything runs on the UI thread - keep it fast, never block.
 *  Build with gcc (see build_plugin.bat) or any C compiler:
 *    gcc -shared -O2 -o MyPlugin.dll myplugin.c
 * ============================================================================
 */
#ifndef NP_PLUGIN_H
#define NP_PLUGIN_H

#include <windows.h>

#define NP_PLUGIN_ABI_VERSION 3
#define NP_PLUGIN_MAX_COMMANDS 12

typedef struct {
    int id;                  /* 0..11, local to this plugin */
    const wchar_t *name;     /* shown verbatim in the Tools menu */
} NP_PLUGIN_CMD;

typedef struct {
    int version;             /* always 2 here */
    HWND editBox;
    HWND mainWnd;
    void (*getSel)(int *s, int *e);
    void (*setSel)(int s, int e);
    int (*getTextLen)(void);
    void (*getText)(WCHAR *out, int outChars);
    void (*replaceSel)(const WCHAR *text);
    void (*replaceAll)(const WCHAR *text);
    void (*getPath)(WCHAR *out, int outChars);
    void (*setStatus)(const WCHAR *text);
    void (*getConfigDir)(WCHAR *out, int outChars);
    int (*confirm)(const WCHAR *text);   /* IDYES / IDNO */
} NP_HOST;

typedef struct {
    int version;             /* always 3 here */
    HWND editBox;
    HWND mainWnd;
    void (*getSel)(int *s, int *e);
    void (*setSel)(int s, int e);
    int (*getTextLen)(void);
    void (*getText)(WCHAR *out, int outChars);
    void (*replaceSel)(const WCHAR *text);
    void (*replaceAll)(const WCHAR *text);
    void (*getPath)(WCHAR *out, int outChars);
    void (*setStatus)(const WCHAR *text);
    void (*getConfigDir)(WCHAR *out, int outChars);
    int (*confirm)(const WCHAR *text);
    int (*getLineCount)(void);
    int (*getLine)(int line, WCHAR *out, int outChars);
    int (*getCaretLine)(void);
    void (*gotoLine)(int line);
    void (*getSelText)(WCHAR *out, int outChars);
    int (*inputBox)(const WCHAR *title, const WCHAR *prompt,
                    const WCHAR *def, WCHAR *out, int outChars);
    int (*messageBox)(const WCHAR *text, int buttons);
    void (*getPrivateDir)(WCHAR *out, int outChars);
    int (*readPrivate)(const WCHAR *name, WCHAR *out, int outChars);
    int (*writePrivate)(const WCHAR *name, const WCHAR *value);
    int (*getEncoding)(void);   /* 0 ANSI, 1 UTF-8, 2 UTF-16LE, 3 UTF-16BE */
    int (*isDirty)(void);
    void (*openFile)(const WCHAR *path);
} NP_HOST3;

enum {
    NP_EVENT_STARTUP = 1, NP_EVENT_SHUTDOWN, NP_EVENT_OPEN,
    NP_EVENT_SAVE, NP_EVENT_TAB, NP_EVENT_CHANGE
};

#ifdef __cplusplus
extern "C" {
#endif

__declspec(dllexport) int NpPluginVersion(void);
__declspec(dllexport) int NpPluginCommands(const NP_PLUGIN_CMD **out);
__declspec(dllexport) void NpPluginExec(int id, HWND edit, HWND main);
__declspec(dllexport) void NpPluginExec2(int id, const NP_HOST *host);
__declspec(dllexport) void NpPluginExec3(int id, const NP_HOST3 *host);
__declspec(dllexport) void NpPluginEvent(int event, const NP_HOST *host);
__declspec(dllexport) void NpPluginIdle(const NP_HOST *host);

#ifdef __cplusplus
}
#endif

#endif /* NP_PLUGIN_H */
