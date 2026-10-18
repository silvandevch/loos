/* ============================================================================
 *  np_demo.c - sample LoOS Notepad plugin, ABI v3 (uses every host call)
 * ============================================================================
 *  Build: build_plugin.bat demo
 *  (needs gcc - see the error message there for install steps)
 *
 *  Commands and the host calls they exercise:
 *    0 UPPERCASE Selection ..... getSel/getText/replaceSel/setSel
 *    1 Insert Timestamp ........ replaceSel
 *    2 Wrap Document [...] ..... getTextLen/getText/replaceAll
 *    3 Go to Line... ........... inputBox/gotoLine/getLineCount
 *    4 Current Line to Status .. getCaretLine/getLine/setStatus
 *    5 File Info ............... getPath/getEncoding/isDirty/messageBox
 *    6 Config Dir to Status .... getConfigDir/setStatus
 *    7 Run Counter ............. readPrivate/writePrivate/getPrivateDir
 *    8 Confirm Demo ............ confirm/setStatus
 *    9 Flash + Focus ........... mainWnd/editBox members
 *  Events: counts opens/saves/changes/tabs (persisted on shutdown).
 *  Idle: bumps an idle-tick counter visible in Event Stats.
 * ============================================================================
 */
#include "np_plugin.h"
#include <stdlib.h>
#include <wchar.h>
#include <wctype.h>

static const NP_PLUGIN_CMD g_cmds[] = {
    { 0, L"UPPERCASE Selection" },
    { 1, L"Insert Timestamp" },
    { 2, L"Wrap Document [...]" },
    { 3, L"Go to Line..." },
    { 4, L"Current Line to Status" },
    { 5, L"File Info" },
    { 6, L"Config Dir to Status" },
    { 7, L"Run Counter" },
    { 8, L"Confirm Demo" },
    { 9, L"Event Stats" },
    { 10, L"Flash + Focus" },
};

static int g_opens, g_saves, g_changes, g_tabs, g_idles;

__declspec(dllexport) int NpPluginVersion(void)
{
    return NP_PLUGIN_ABI_VERSION; /* 3 */
}

__declspec(dllexport) int NpPluginCommands(const NP_PLUGIN_CMD **out)
{
    *out = g_cmds;
    return (int)(sizeof(g_cmds) / sizeof(g_cmds[0]));
}

/* v1 fallback (kept working if the host ever negotiates down). */
__declspec(dllexport) void NpPluginExec(int id, HWND editBox, HWND mainWnd)
{
    (void)mainWnd;
    if(id != 0) return;
    DWORD s = 0, e = 0;
    SendMessageW(editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    if(s == e) return;
    int full = GetWindowTextLengthW(editBox);
    WCHAR *all = malloc(((size_t)full + 1) * sizeof(WCHAR));
    if(!all) return;
    GetWindowTextW(editBox, all, full + 1);
    int n = (int)(e - s);
    WCHAR *upper = malloc(((size_t)n + 1) * sizeof(WCHAR));
    if(upper){
        for(int i = 0; i < n && (int)s + i <= full; i++)
            upper[i] = towupper(all[s + i]);
        upper[n] = L'\0';
        SendMessageW(editBox, EM_SETSEL, s, e);
        SendMessageW(editBox, EM_REPLACESEL, TRUE, (LPARAM)upper);
        free(upper);
    }
    free(all);
}

/* v2 entry (kept for mixed-version hosts). */
__declspec(dllexport) void NpPluginExec2(int id, const NP_HOST *host)
{
    const NP_HOST *h = host;
    if(!h || h->version < 2) return;
    if(id == 0){
        int s = 0, e = 0;
        h->getSel(&s, &e);
        if(s == e){ h->setStatus(L"Select something first"); return; }
        int len = h->getTextLen();
        WCHAR *all = malloc(((size_t)len + 1) * sizeof(WCHAR));
        if(!all) return;
        h->getText(all, len + 1);
        int n = e - s;
        WCHAR *upper = malloc(((size_t)n + 1) * sizeof(WCHAR));
        if(upper){
            for(int i = 0; i < n && s + i <= len; i++)
                upper[i] = towupper(all[s + i]);
            upper[n] = L'\0';
            h->setSel(s, e);
            h->replaceSel(upper);
            h->setSel(s, s + n);
            h->setStatus(L"Uppercased selection");
            free(upper);
        }
        free(all);
    }else if(id == 1){
        SYSTEMTIME st;
        GetLocalTime(&st);
        wchar_t date[64], time[64], out[160];
        GetDateFormatW(LOCALE_USER_DEFAULT, 0, &st, NULL, date, 64);
        GetTimeFormatW(LOCALE_USER_DEFAULT, 0, &st, NULL, time, 64);
        swprintf_s(out, 160, L"%s %s", date, time);
        h->replaceSel(out);
    }
}

static void demo_stats(const NP_HOST3 *h)
{
    int len = h->getTextLen();
    int lines = h->getLineCount();
    int line = h->getCaretLine();
    wchar_t msg[256];
    swprintf_s(msg, 256, L"%d chars, %d lines, caret on line %d",
               len, lines, line + 1);
    h->setStatus(msg);
}

__declspec(dllexport) void NpPluginExec3(int id, const NP_HOST3 *host)
{
    const NP_HOST3 *h = host;
    if(!h || h->version != 3) return;
    switch(id){
    case 0: {
        /* Reuse the v2 path (same prefix layout). */
        NpPluginExec2(0, (const NP_HOST *)h);
        break;
    }
    case 1: {
        SYSTEMTIME st;
        GetLocalTime(&st);
        wchar_t date[64], time[64], out[160];
        GetDateFormatW(LOCALE_USER_DEFAULT, 0, &st, NULL, date, 64);
        GetTimeFormatW(LOCALE_USER_DEFAULT, 0, &st, NULL, time, 64);
        swprintf_s(out, 160, L"%s %s", date, time);
        h->replaceSel(out);
        break;
    }
    case 2: {
        int len = h->getTextLen();
        WCHAR *all = malloc(((size_t)len + 3) * sizeof(WCHAR));
        if(!all) break;
        h->getText(all, len + 1);
        WCHAR *out = malloc(((size_t)len + 3) * sizeof(WCHAR));
        if(out){
            out[0] = L'[';
            memcpy(out + 1, all, ((size_t)len + 1) * sizeof(WCHAR));
            out[len + 1] = L']';
            out[len + 2] = L'\0';
            h->replaceAll(out);
            h->setStatus(L"Wrapped document in [...]");
            free(out);
        }
        free(all);
        break;
    }
    case 3: {
        wchar_t num[32] = {0};
        if(h->inputBox(L"Go to Line", L"Line number (1-based):", L"1",
                       num, 32) != IDOK)
            break;
        int want = _wtoi(num);
        if(want >= 1) h->gotoLine(want - 1);
        break;
    }
    case 4: {
        int line = h->getCaretLine();
        wchar_t buf[512] = {0};
        h->getLine(line, buf, 512);
        h->setStatus(buf[0] ? buf : L"(empty line)");
        break;
    }
    case 5: {
        wchar_t path[1024] = {0};
        h->getPath(path, 1024);
        static const wchar_t *encs[] =
            { L"ANSI", L"UTF-8", L"UTF-16 LE", L"UTF-16 BE" };
        int enc = h->getEncoding();
        wchar_t msg[1200];
        swprintf_s(msg, 1200, L"%s | %s | %s",
                   path[0] ? path : L"Untitled",
                   (enc >= 0 && enc <= 3) ? encs[enc] : L"?",
                   h->isDirty() ? L"modified" : L"saved");
        h->messageBox(msg, 0);
        break;
    }
    case 6: {
        wchar_t dir[1024] = {0};
        h->getConfigDir(dir, 1024);
        wchar_t msg[1150];
        swprintf_s(msg, 1150, L"Config: %s", dir);
        h->setStatus(msg);
        break;
    }
    case 7: {
        wchar_t val[32] = {0};
        int runs = 0;
        if(h->readPrivate(L"runs", val, 32)) runs = _wtoi(val);
        runs++;
        wchar_t out[32];
        swprintf_s(out, 32, L"%d", runs);
        h->writePrivate(L"runs", out);
        wchar_t dir[1024] = {0};
        h->getPrivateDir(dir, 1024);
        wchar_t msg[1200];
        swprintf_s(msg, 1200, L"Run #%d (stored in %s)", runs, dir);
        h->setStatus(msg);
        break;
    }
    case 8:
        if(h->confirm(L"Do you like this plugin API?") == IDYES)
            h->setStatus(L"Glad to hear it");
        else
            h->setStatus(L"Noted");
        break;
    case 9: {
        wchar_t msg[256];
        swprintf_s(msg, 256,
                   L"opens=%d saves=%d changes=%d tabs=%d idles=%d",
                   g_opens, g_saves, g_changes, g_tabs, g_idles);
        h->setStatus(msg);
        break;
    }
    case 10:
        FlashWindow(h->mainWnd, TRUE);
        SetFocus(h->editBox);
        h->setStatus(L"Flashed + focused");
        break;
    default:
        break;
    }
}

__declspec(dllexport) void NpPluginEvent(int event, const NP_HOST *host)
{
    (void)host;
    /* Keep it cheap: this fires on every keystroke (CHANGE). */
    switch(event){
    case 3: g_opens++; break;   /* NP_EVENT_OPEN */
    case 4: g_saves++; break;   /* NP_EVENT_SAVE */
    case 5: g_tabs++; break;    /* NP_EVENT_TAB */
    case 6: g_changes++; break; /* NP_EVENT_CHANGE */
    default: break;
    }
}

__declspec(dllexport) void NpPluginIdle(const NP_HOST *host)
{
    (void)host;
    g_idles++; /* visible via Event Stats; no UI churn here */
}
