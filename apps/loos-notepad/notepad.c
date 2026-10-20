/* ============================================================================
 *  LoOS/notepad.exe  -  Windows Notepad - Native LoOS Editor
 * ============================================================================
 *  This is the native text editor for LoOS. On LoOS there is no separate
 *  "standard" notepad.exe - THIS IS notepad.exe.
 *
 *  A clean, human-written reconstruction of Windows Notepad. UI data is
 *  transcribed 1:1 from the genuine Windows 11 resources
 *  (notepad.exe.mui: menu 1, dialogs 11/12/14/15/1540/1541, string tables,
 *  GLOBALACC/MAINACC/SLIPUPACC):
 *    - notepad.rc  : exact dialog templates (built with rc.exe)
 *    - notepad.ico : ORIGINAL LoOS logo (not Microsoft art)
 *  This file carries the same menu/accelerator/string tables compiled in.
 *
 *  STRIPPED OUT (never ported, by design):
 *    - ETW telemetry: EventRegister/EventWriteTransfer/EventUnregister
 *      (the decompile's Telemetry_* helpers). This build registers no ETW
 *      provider and emits no events, ever.
 *    - Windows Error Reporting calls and SetWinEventHook (only local
 *      NotifyWinEvent screen-reader announcements remain; nothing leaves
 *      the machine).
 *    - "Send Feedback" menu item (feedback-hub data collection). Help is
 *      View Help + About Notepad only.
 *    - All Copilot/AI features (Summarize/Rewrite). Those live only in the
 *      separate XAML Store app, which is not our target; the classic UI we
 *      rebuild never had them.
 *  Also stripped as optional extras: Bing search, the Store promo banner
 *  (BannerClass) and its ShowStoreBanner value. What remains online is only
 *  ever opened by an explicit click (View Help).
 *
 *  BUILD (on LoOS VM 192.168.105.128):
 *    rc /fo C:\LoOS\out\notepad.res notepad.rc
 *    call "C:\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
 *    cl /EHsc /O2 /W4 /DUNICODE /D_UNICODE notepad_rewrite.c ^
 *       C:\LoOS\out\notepad.res /Fe:C:\LoOS\out\notepad.exe ^
 *       comdlg32.lib shell32.lib comctl32.lib winspool.lib gdi32.lib ^
 *       ole32.lib UxTheme.lib user32.lib kernel32.lib advapi32.lib ^
 *       /link /SUBSYSTEM:WINDOWS
 *
 *  SUPERVISED RUN: notepad.exe --watch [files...] supervises itself
 *  (relaunch on abnormal exit, crash-loop guard) - same single binary.
 *
 *  Set LOOSDEBUG=1 for verbose developer logging (DebugView / debugger).
 *
 *  MODERNIZATION (post-1:1, this file only -- the .rc/.ico stay genuine):
 *    - View > Theme > System / Light / Dark (persisted as iTheme). System
 *      follows HKCU\...\Personalize\AppsUseLightTheme live (WM_SETTINGCHANGE).
 *    - Dark title bar + Mica backdrop via dwmapi (loaded dynamically, so the
 *      binary still runs on older Windows which simply ignore it).
 *    - DarkMode_Explorer window themes for dark scrollbars/buttons/combos,
 *      themed edit/status/banner colors via WM_CTLCOLOR*.
 *    - PerMonitorV2 DPI awareness set programmatically at startup.
 *    - Status bar appends the selection character count, like modern editors.
 *  Known Win32 limits (same as every native dark-mode app): classic menus,
 *  caret/selection colors and common dialogs stay light; only the frame,
 *  edit surface, status bar and banner go dark.
 *
 *  POWER EDITION (this era, all fully working, no stubs):
 *    Tabs with per-tab state, session save/restore, per-tab crash recovery,
 *    fuzzy Ctrl+P palette (tabs + recent files).
 *    Regex find/replace (own engine: ^$. *+?{m,n} |() [] \d\w\s\b, $1-$9 in
 *    replacements, step-capped against pathological patterns), live match
 *    count in the Find dialogs, resizable Find/Replace windows.
 *    Sort Lines (alpha/numeric/length/desc/unique/trim), math evaluator
 *    (Ctrl+M), URL auto-detect with click-open + hover tooltip, auto-close
 *    pairs + smart backspace, autocomplete popup (Ctrl+Space + automatic),
 *    persistent macro recorder (Ctrl+Shift+R/E).
 *    Markdown preview pane (RichEdit RTF renderer, debounced live update),
 *    version history with restore dialog, sticky markdown header bar,
 *    hex color hover + ChooseColor edit, dialog-driven column ops,
 *    Ctrl+click multi-caret editing, custom .theme files, distraction-free
 *    (F11), always-on-top, sticky-note mode, side-by-side split with scroll
 *    sync, minimal Tools> plugin API (*.dll, documented below), large-file
 *    mode (>32 MB: wrap locked off, autosave skipped, undo cleared).
 *  Honestly out of scope on a native EDIT control (no fake UI for these):
 *    minimap, font ligatures, letter-spacing, per-tab process isolation,
 *    full syntax highlighting. The plugin API exposes text access so
 *    external tools can still integrate.
 *
 *  PLUGIN ABI (Tools menu, %APPDATA%\LoOS\Notepad\plugins\*.dll):
 *    int NpPluginVersion(void);                    // must return 1
 *    int NpPluginCommands(const NP_PLUGIN_CMD **); // static array + count
 *    void NpPluginExec(int id, HWND edit, HWND main);
 *    typedef struct { int id; const wchar_t *name; } NP_PLUGIN_CMD;
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
#include <windows.h>
#include "brand.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>
#include <wctype.h>
#include <stdbool.h>
#include <stdarg.h>
#include <stdint.h>
#include <commdlg.h>
#include <commctrl.h>
#include <shellapi.h>
#include <shlobj.h>
#include <winspool.h>   /* OpenPrinterW, DocumentPropertiesW, GetDefaultPrinterW */
#include <uxtheme.h>    /* SetWindowTheme for DarkMode_Explorer scrollbars */
#include <dwmapi.h>     /* struct defs only; the DLL is loaded dynamically */
#pragma warning(push)
#pragma warning(disable:4201) /* nameless struct/union inside richedit.h */
#include <richedit.h>   /* RichEdit constants for the Markdown preview pane */
#pragma warning(pop)

#pragma comment(lib, "comdlg32.lib")
#pragma comment(lib, "shell32.lib")
#pragma comment(lib, "comctl32.lib")
#pragma comment(lib, "winspool.lib")
#pragma comment(lib, "gdi32.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "UxTheme.lib")

/* Modern DWM bits, guarded so older SDKs still compile (dwmapi.dll itself
 * is loaded dynamically at runtime for the same reason). */
#ifndef DWMWA_USE_IMMERSIVE_DARK_MODE
#define DWMWA_USE_IMMERSIVE_DARK_MODE 20
#endif
#ifndef DWMWA_SYSTEMBACKDROP_TYPE
#define DWMWA_SYSTEMBACKDROP_TYPE 38
#endif
#ifndef DWMSBT_MAINWINDOW
#define DWMSBT_MAINWINDOW 2   /* Mica */
#endif
typedef HRESULT (WINAPI *DWM_SET_WINDOW_ATTRIBUTE_FN)(HWND, DWORD, LPCVOID, DWORD);
static DWM_SET_WINDOW_ATTRIBUTE_FN g_dwmSetAttr = NULL;
/* NOTE: the application manifest (comctl32 v6, DPI, long paths) lives in
 * notepad.exe.manifest and is linked via notepad.rc (ID 1), so the single
 * exe carries everything. No linker pragma here on purpose. */

#ifndef IDC_STATIC
#define IDC_STATIC (-1)
#endif
#ifndef EM_SETBKGNDCOLOR
#define EM_SETBKGNDCOLOR (WM_USER + 67) /* from richedit.h; EDIT honors it */
#endif
#ifndef EM_SETCARETWIDTH
#define EM_SETCARETWIDTH (WM_USER + 220) /* EDIT caret width in pixels */
#endif
#ifndef PDERR_NODEFAULTPRN
#define PDERR_NODEFAULTPRN 0x100A   /* no-printer error from CommDlgExtendedError */
#endif

/* --------------------------------------------------------------------------
 *  Constants
 * -------------------------------------------------------------------------- */
#define NP_MAX_PATH       32767
#define NP_FIND_MAX       256
#define NP_TITLE_MAX      512
#define NP_MSG_MAX        2048

#define NP_ZOOM_MIN       10
#define NP_ZOOM_MAX       500
#define NP_ZOOM_STEP      10
#define NP_ZOOM_DEFAULT   100

#define NP_AUTOSAVE_MS    30000
#define NP_STATUS_MSG_MS  4000
#define NP_LARGE_FILE     (32UL * 1024UL * 1024UL)
#define NP_TABSTRIP_H     26
#define NP_STICKY_H       22
#define NP_MAX_TABS       64
#define NP_MAX_CARETS     16
#define NP_MAX_PLUGINS    32
#define NP_PLUGIN_MAX_COMMANDS 12
#define NP_MAX_MACRO_OPS  4096
#define NP_RX_MAXGROUPS   10
#define NP_RX_STEP_LIMIT  2000000

#define NP_CLASS_MAIN     L"Notepad"
#define NP_REG_KEY        L"Software\\Microsoft\\Notepad"
#define NP_HELP_URL       L"loos-help://notepad"   /* handled locally by the shell; never a web address */
#define NP_VERSION        LOOS_BRAND_VERSION_W

/* Exact command IDs from the genuine menu resource (menu 1). The top-level
 * popups are stored as (id-char + text): '&'+'File', '&'+'Edit',
 * 'F'+'&ormat' -> "F&ormat", '&'+'View', '&'+'Zoom', '&'+'Help'. */
enum {
    CMD_FILE_NEW = 1, CMD_FILE_NEW_WINDOW = 8, CMD_FILE_OPEN = 2,
    CMD_FILE_SAVE = 3, CMD_FILE_SAVEAS = 4, CMD_FILE_PAGESETUP = 5,
    CMD_FILE_PRINT = 6, CMD_FILE_EXIT = 7,
    CMD_EDIT_UNDO = 16, CMD_EDIT_CUT = 768, CMD_EDIT_COPY = 769,
    CMD_EDIT_PASTE = 770, CMD_EDIT_DELETE = 771,
    CMD_EDIT_FIND = 21, CMD_EDIT_FINDNEXT = 22, CMD_EDIT_FINDPREV = 29,
    CMD_EDIT_REPLACE = 23, CMD_EDIT_GOTO = 24, CMD_EDIT_SELECTALL = 25,
    CMD_EDIT_TIMEDATE = 26,
    CMD_FORMAT_WORDWRAP = 32, CMD_FORMAT_FONT = 33,
    CMD_VIEW_ZOOMIN = 34, CMD_VIEW_ZOOMOUT = 35, CMD_VIEW_ZOOMRESTORE = 36,
    CMD_VIEW_STATUSBAR = 27,
    CMD_HELP_TOPIC = 64, CMD_HELP_ABOUT = 65,
    /* 66 was "Send &Feedback" (feedback-hub telemetry): stripped, see above */
    CMD_THEME_SYSTEM = 501, CMD_THEME_LIGHT, CMD_THEME_DARK,
    /* POWER additions (Tools/Edit/View extensions, plugin range 600+) */
    CMD_FILE_HISTORY = 509,
    CMD_SORT_ALPHA = 510, CMD_SORT_NUMERIC, CMD_SORT_LENGTH, CMD_SORT_DESC,
    CMD_SORT_UNIQUE, CMD_SORT_TRIM,
    CMD_EDIT_MATH = 517,
    CMD_MACRO_RECORD = 518, CMD_MACRO_PLAY = 520,
    CMD_COL_INSERT = 521, CMD_COL_DELETE, CMD_COL_COPY,
    CMD_VIEW_MD = 524, CMD_VIEW_STICKYSCROLL, CMD_VIEW_SPLITV, CMD_VIEW_SPLITH,
    CMD_VIEW_SPLITSYNC, CMD_VIEW_DISTRACT, CMD_VIEW_TOPMOST, CMD_VIEW_STICKYNOTE,
    CMD_FILE_CLOSETAB = 533, CMD_FILE_NEWTAB,
    CMD_TOOLS_PALETTE = 532,
    CMD_VIEW_TOOLBAR = 535,
    CMD_FORMAT_PAIRS = 536,
    CMD_VIEW_OVERVIEW = 537,
    CMD_EDIT_DUP = 538, CMD_EDIT_DELLINE, CMD_EDIT_REOPEN,
    CMD_PLUGIN_BASE = 600, CMD_PLUGIN_END = 720,
    CMD_THEME_CUSTOM = 700, CMD_THEME_CUSTOM_END = 732,
    IDC_EDITBOX = 15, IDC_STATUSBAR = 102,
    IDT_AUTOSAVE = 201, IDT_STATUSMSG, IDT_MDUPDATE, IDT_STICKY, IDT_CARET,
    IDT_PLUGINIDLE
};

/* Genuine dialog/control IDs (see notepad.rc for the templates). */
enum {
    DLG_PRINTING = 11, DLG_PAGESETUP = 12, DLG_GOTO = 14, DLG_ENCHOOK = 15,
    DLG_FIND = 1540, DLG_REPLACE = 1541,
    CTL_FIND_EDIT = 1152, CTL_REPLACE_EDIT = 1153,
    CTL_WHOLEWORD = 1040, CTL_MATCHCASE = 1041, CTL_WRAPAROUND = 1042,
    CTL_DIRGROUP = 1072, CTL_DIR_UP = 1056, CTL_DIR_DOWN = 1057,
    CTL_FIND_HELP = 1038, CTL_REPLACE_BTN = 1024, CTL_REPLACE_ALL = 1025,
    CTL_REGEX = 1043, CTL_MATCHCOUNT = 1089,
    CTL_GOTO_EDIT = 258, CTL_GOTO_LABEL = 1200,
    CTL_ENC_LABEL = 259, CTL_ENC_COMBO = 257,
    CTL_PRINT_NAME = 20, CTL_PRINT_PAGE = 21,
    CTL_PS_PAPER_LBL = 1073, CTL_PS_SIZE_LBL = 1089, CTL_PS_PAPER = 1137,
    CTL_PS_SRC_LBL = 1090, CTL_PS_SOURCE = 1138,
    CTL_PS_ORIENT = 1072, CTL_PS_PORTRAIT = 1056, CTL_PS_LANDSCAPE = 1057,
    CTL_PS_MARGINS = 1075,
    CTL_PS_LEFT_LBL = 1102, CTL_PS_LEFT = 1155,
    CTL_PS_RIGHT_LBL = 1103, CTL_PS_RIGHT = 1157,
    CTL_PS_TOP_LBL = 1104, CTL_PS_TOP = 1156,
    CTL_PS_BOTTOM_LBL = 1105, CTL_PS_BOTTOM = 1158,
    CTL_PS_HDR_LBL = 32, CTL_PS_HEADER = 30,
    CTL_PS_FTR_LBL = 33, CTL_PS_FOOTER = 31,
    CTL_PS_VALUESLINK = 34, CTL_PS_PRINTER = 1026,
    CTL_PS_PREVIEW_PAGE = 1080, CTL_PS_PREVIEW_R = 1081, CTL_PS_PREVIEW_B = 1082
};

/* Genuine UI strings (notepad.exe.mui string tables; IDs in comments). */
static const wchar_t S_APP[]          = LOOS_BRAND_NOTEPAD_W;                 /* 9, 3000 */
static const wchar_t S_UNTITLED[]     = L"Untitled";                       /* 4 */
static const wchar_t S_TITLE_FMT[]    = L"%s%s - LoOS Notepad";               /* 5: %1 %2 */
static const wchar_t S_SAVE_TASK[]    = L"Do you want to save changes to %1?"; /* 3001 */
static const wchar_t S_SAVE_BTN[]     = L"&Save";                          /* 3002 */
static const wchar_t S_DONTSAVE_BTN[] = L"Do&n't Save";                    /* 3003 */
static const wchar_t S_SAVE_CLASSIC[] = L"The text in the %s file has changed.\n\nDo you want to save the changes?"; /* 3 */
static const wchar_t S_CANT_FIND[]    = L"Cannot find \"%s\"";             /* 6 */
static const wchar_t S_GOTO_RANGE[]   = L"The line number is beyond the total number of lines"; /* 31 */
static const wchar_t S_NOMEM[]        = L"Not enough memory available to complete this operation. Quit one or more applications to increase available memory, and then try again."; /* 7 */
static const wchar_t S_TOO_LARGE[]    = L"The %s file is too large for Notepad.\n\nUse another editor to edit the file."; /* 8 */
static const wchar_t S_CANT_OPEN[]    = L"Cannot open the %s file.\n\nMake sure a disk is in the drive you specified."; /* 1 */
static const wchar_t S_NOTFOUND_NEW[] = L"Cannot find the %s file.\n\nDo you want to create a new file?"; /* 2 */
static const wchar_t S_BAD_NAME[]     = L"Not a valid file name.";        /* 13 */
static const wchar_t S_CANT_CREATE[]  = L"Cannot create the %s file.\n\nMake sure that the path and file name are correct."; /* 14 */
static const wchar_t S_UNICODE_LOSS[] = L"%s\n This file contains characters in Unicode format which will be lost if you save this file as an ANSI encoded text file. To keep the Unicode information, click Cancel below and then select one of the Unicode options from the Encoding drop down list. Continue?"; /* 27 */
static const wchar_t S_SHUTDOWN[]     = L"You cannot shut down or log off LoOS because\nthe Save As dialog box in LoOS Notepad is open. Switch to\nLoOS Notepad, close this dialog box, and then try shutting\ndown or logging off LoOS again."; /* 24 */
static const wchar_t S_NO_PRINTER[]   = L"Cannot access your printer.\nBe sure that your printer is connected properly and use Control Panel to verify that the printer is configured properly."; /* 25 */
static const wchar_t S_PAGE_SMALL[]   = L"Page too small to print one line.\nTry printing using smaller font."; /* 28 */
static const wchar_t S_FILTER_TXT[]   = L"Text Documents (*.txt)";        /* 20 */
static const wchar_t S_FILTER_ALL[]   = L"All Files (*.*)";               /* 21 + "(*.*)" */
static const wchar_t S_OPEN_TITLE[]   = L"Open";                           /* 22 */
static const wchar_t S_SAVEAS_TITLE[] = L"Save As";                        /* 23 */
static const wchar_t S_GOTO_CAP[]     = L"Go To Line";                     /* dlg 14 */
static const wchar_t S_ENC_AUTO[]     = L"Auto-Detect";                    /* 57 */
static const wchar_t S_ENC_UTF8BOM[]  = L"UTF-8 with BOM";                 /* 58 */
static const wchar_t S_FOUND_NEXT[]   = L"Found the next occurrence of text";       /* 2305 */
static const wchar_t S_NOT_FOUND[]    = L"Unable to find the next occurrence of text"; /* 2306 */
static const wchar_t S_REPLACED[]     = L"Replaced the next occurrence of text";     /* 2307 */
static const wchar_t S_NOT_REPLACED[] = L"Unable to replace the next occurrence of text"; /* 2308 */
static const wchar_t S_REPLACED_ALL[] = L"Replaced all occurrences of text";        /* 2309 */
static const wchar_t S_WRAP_BOTTOM[]  = L" Found next from the bottom";    /* 54 */
static const wchar_t S_WRAP_TOP[]     = L" Found next from the top";       /* 55 */

/* Document encoding (status bar shows the real one). */
typedef enum { ENC_ANSI = 0, ENC_UTF8, ENC_UTF16LE, ENC_UTF16BE } NP_ENCODING;
typedef enum { EOL_CRLF = 0, EOL_LF, EOL_CR } NP_EOL;

/* --------------------------------------------------------------------------
 *  Global State
 * -------------------------------------------------------------------------- */
/* One document tab. The ACTIVE tab's text lives in the EDIT control; idle
 * tabs keep theirs in a malloc'd buffer. Undo history does not survive a
 * tab switch (EDIT control limit) -- the text, selection, scroll, encoding
 * and dirty flag do. */
typedef struct {
    wchar_t  path[NP_MAX_PATH];
    WCHAR   *text;
    int      textLen;
    NP_ENCODING enc;
    int      bom;
    NP_EOL   eol;
    int      dirty;
    int      selS;
    int      selE;
    int      firstLine;
    int      largeFile;
} NP_TAB;

/* Closed-tab stack for Ctrl+Shift+T (storage here so the window procedure
 * can free it; push/reopen live with the shortcut code at the end). */
#define NP_CLOSED_MAX 10

typedef struct {
    wchar_t path[NP_MAX_PATH];
    WCHAR *text;
    NP_ENCODING enc;
    int bom;
    NP_EOL eol;
    int selS;
    int selE;
    int largeFile;
} NP_CLOSED;

static NP_CLOSED g_closed[NP_CLOSED_MAX];
static int g_closedCount;

/* (NP_HOST + g_npHost live just below with the plugin types.) */

typedef struct { int kind; int key; WCHAR *text; } NP_MACRO_OP;
/* kind: 0 = insert text, 1 = backspace, 2 = delete, 3 = enter, 4 = tab,
 * 5..10 = left/right/up/down/home/end */

typedef struct { int id; const wchar_t *name; } NP_PLUGIN_CMD;
/* ABI v2 host table: everything a plugin may call without touching
 * window messages directly. Passed to NpPluginExec2 when the plugin
 * reports version 2 (v1 plugins keep using NpPluginExec). */
typedef struct {
    int version;
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
} NP_HOST;
/* ABI v3 host table: exact v2 prefix (so a v2 plugin can safely read it
 * as NP_HOST) plus new services. Never reorder the shared prefix. */
typedef struct {
    int version;
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
    int (*getEncoding)(void);
    int (*isDirty)(void);
    void (*openFile)(const WCHAR *path);
} NP_HOST3;

/* v2 host table instance (filled by np_plugins_load, used on dispatch). */
static NP_HOST g_npHost;

/* v3 host table instance (filled alongside; prefix-compatible). */
static NP_HOST3 g_host3;

/* Active plugin context for private-dir calls (set around dispatch/events). */
static int s_activePlugin;

typedef struct {
    HMODULE  lib;
    int      baseId;
    int      count;
    int      abiVersion;
    void (WINAPI *exec)(int id, HWND editBox, HWND mainWnd);
    void (WINAPI *exec2)(int id, const NP_HOST *host);
    void (WINAPI *exec3)(int id, const NP_HOST3 *host);
    void (WINAPI *onEvent)(int event, const NP_HOST *host);
    void (WINAPI *onIdle)(const NP_HOST *host);
    wchar_t  fileName[64];
} NP_PLUGIN;

/* ABI v3 events (NpPluginEvent, optional, notifications only). */
enum {
    NP_EVENT_STARTUP = 1, NP_EVENT_SHUTDOWN, NP_EVENT_OPEN,
    NP_EVENT_SAVE, NP_EVENT_TAB, NP_EVENT_CHANGE
};

/* Menus are built owner-drawn from birth in dark/custom themes (no
 * post-hoc type flipping): AppendMenuW(MF_OWNERDRAW) stores the label
 * literal as item data. Top-level bar items stay native MFT_STRING. */

typedef struct _NOTEPAD_STATE {
    HWND     mainWnd;
    HWND     editBox;
    HWND     statusBar;
    HWND     findDlg;
    int      findIsReplace;
    HMENU    editMenu;       /* Edit submenu handle for INITMENUPOPUP */
    HACCEL   globalAcc;
    HACCEL   mainAcc;
    HACCEL   slipUpAcc;
    wchar_t  path[NP_MAX_PATH];
    int      dirty;
    int      wrap;
    int      showBar;
    int      zoom;
    LOGFONTW font;
    int      haveFont;
    NP_ENCODING encoding;
    int      bomPresent;
    NP_EOL   eolMode;
    int      matchCase;
    int      wholeWord;
    int      wrapAround;
    int      findDown;
    /* Save/Open encoding hook results */
    int      ofnMode;        /* 0 none, 1 open, 2 save */
    int      saveDlgOpen;
    int      useSaveChoice;  /* Save As committed an encoding choice */
    int      saveEnc;        /* pending Save As encoding */
    int      saveBom;
    int      forceEnc;       /* pending Open forced encoding, -1 = auto */
    int      forceBom;
    /* Page Setup (thousandths of an inch + header/footer + printer) */
    DWORD    marginLeft;
    DWORD    marginTop;
    DWORD    marginRight;
    DWORD    marginBottom;
    wchar_t  header[256];
    wchar_t  footer[256];
    int      savedPaper;
    int      savedOrient;
    HGLOBAL  hDevMode;
    HGLOBAL  hDevNames;
    int      saveWinPos;
    int      winX;
    int      winY;
    int      winDX;
    int      winDY;
    int      setupMode;      /* /.SETUP chromeless mode (uses SlipUpAcc) */
    int      abortPrint;
    HWND     abortDlg;
    wchar_t  statusMsg[256];
    int      statusMsgActive;
    /* Modernization state */
    int      theme;          /* 0 = system, 1 = light, 2 = dark */
    int      darkActive;
    HBRUSH   themeBrush;     /* owned; recreated on every theme switch */
    /* Power state */
    HWND     tabBar;
    NP_TAB  *tabs;
    int      tabCount;
    int      tabCap;
    int      tabActive;
    int      mdPreview;
    HWND     mdView;
    HMODULE  richLib;
    UINT     mdTimerOn;
    int      stickyOn;
    HWND     stickyBar;
    int      distractOn;
    HMENU    savedMenu;
    WINDOWPLACEMENT savedPlace;
    DWORD    savedStyle;
    DWORD    savedExStyle;
    int      topmost;
    int      stickyNote;
    int      splitMode;      /* 0 off, 1 vertical, 2 horizontal */
    HWND     splitEdit;
    int      splitTab;
    int      splitSync;
    int      splitGuard;
    int      autoPairs;
    int      carets[NP_MAX_CARETS];
    int      caretCount;
    int      editGuard;
    int      recording;
    NP_MACRO_OP *macro;
    int      macroCount;
    int      macroCap;
    int      findRegex;
    HWND     acPopup;
    HWND     acList;
    int      acActive;
    HWND     tipWnd;
    int      tipArmed;
    DWORD    tipTick;
    HMENU    toolsMenu;
    NP_PLUGIN plugins[NP_MAX_PLUGINS];
    int      pluginCount;
    wchar_t  themeName[64];
    COLORREF themeBg;
    COLORREF themeFg;
    wchar_t  customThemes[32][64];
    int      customThemeCount;
    /* Polish state */
    HFONT    menuFont;
    int      menuDark;       /* menu currently built owner-drawn (dark) */
    HWND     toolBar;
    int      showToolbar;
    int      tabHover;
    HWND     overviewBar;
    int      showOverview;
    wchar_t  sbText[4][96];   /* owner-drawn status content */
    int      caretFocus;
    int      caretPhase;
    RECT     caretRect;
} NOTEPAD_STATE;

static NOTEPAD_STATE g_np;
static wchar_t g_findWhat[NP_FIND_MAX];
static wchar_t g_replaceWith[NP_FIND_MAX];
static int g_devLog = -1;       /* LOOSDEBUG=1 enables verbose logging */

/* Exact accelerator tables from the genuine resources. */
static const ACCEL g_globalAccTable[] = {
    { FVIRTKEY | FCONTROL, 'N', CMD_FILE_NEW },
    { FVIRTKEY | FCONTROL | FSHIFT, 'N', CMD_FILE_NEW_WINDOW },
    { FVIRTKEY | FCONTROL, 'O', CMD_FILE_OPEN },
    { FVIRTKEY | FCONTROL, 'S', CMD_FILE_SAVE },
    { FVIRTKEY | FCONTROL | FSHIFT, 'S', CMD_FILE_SAVEAS },
    { FVIRTKEY | FCONTROL, 'P', CMD_FILE_PRINT },
    { FVIRTKEY | FCONTROL, 'W', CMD_FILE_CLOSETAB },
    { FVIRTKEY | FCONTROL, 'Q', CMD_FILE_EXIT },
    { FVIRTKEY, VK_F1, CMD_HELP_TOPIC },
    { FVIRTKEY, VK_F3, CMD_EDIT_FINDNEXT },
    { FVIRTKEY | FSHIFT, VK_F3, CMD_EDIT_FINDPREV },
    { FVIRTKEY | FCONTROL, 'G', CMD_EDIT_GOTO },
    { FVIRTKEY | FCONTROL, 'F', CMD_EDIT_FIND },
    { FVIRTKEY | FCONTROL, 'H', CMD_EDIT_REPLACE },
    { FVIRTKEY | FCONTROL, 187, CMD_VIEW_ZOOMIN },
    { FVIRTKEY | FCONTROL, VK_ADD, CMD_VIEW_ZOOMIN },
    { FVIRTKEY | FCONTROL, 189, CMD_VIEW_ZOOMOUT },
    { FVIRTKEY | FCONTROL, VK_SUBTRACT, CMD_VIEW_ZOOMOUT },
    { FVIRTKEY | FCONTROL, VK_NUMPAD0, CMD_VIEW_ZOOMRESTORE },
    { FVIRTKEY | FCONTROL, '0', CMD_VIEW_ZOOMRESTORE },
};
static const ACCEL g_mainAccTable[] = {
    { FVIRTKEY | FCONTROL, 189, CMD_EDIT_COPY },
    { FVIRTKEY, VK_F5, CMD_EDIT_TIMEDATE },
    { FVIRTKEY | FALT, VK_BACK, CMD_EDIT_UNDO },
    { FVIRTKEY | FCONTROL, 'Z', CMD_EDIT_UNDO },
    { FVIRTKEY | FCONTROL, 'X', CMD_EDIT_CUT },
    { FVIRTKEY | FCONTROL, 'C', CMD_EDIT_COPY },
    { FVIRTKEY | FCONTROL, 'V', CMD_EDIT_PASTE },
    { FVIRTKEY | FCONTROL, 'A', CMD_EDIT_SELECTALL },
};
static const ACCEL g_slipUpAccTable[] = {
    { FVIRTKEY | FCONTROL, 189, CMD_EDIT_COPY },
    { FVIRTKEY, VK_F5, CMD_EDIT_TIMEDATE },
    { FVIRTKEY | FALT, VK_BACK, CMD_EDIT_UNDO },
    { FVIRTKEY | FCONTROL, 'Z', CMD_EDIT_UNDO },
    { FVIRTKEY | FCONTROL, 'X', CMD_EDIT_CUT },
    { FVIRTKEY | FCONTROL, 'C', CMD_EDIT_COPY },
    { FVIRTKEY | FCONTROL, 'V', CMD_EDIT_PASTE },
    { FVIRTKEY | FCONTROL, 'A', CMD_EDIT_SELECTALL },
    { FVIRTKEY, VK_ESCAPE, CMD_FILE_EXIT },
    { FVIRTKEY | FCONTROL, 'C', CMD_FILE_EXIT },
    { FVIRTKEY | FCONTROL, 'D', CMD_FILE_EXIT },
    { FVIRTKEY | FCONTROL, 'Z', CMD_FILE_EXIT },
};

/* Per-dialog working copy for Page Setup so Cancel discards everything. */
typedef struct {
    DEVMODEW dm;
    int      hasDm;
    wchar_t  driver[64];
    wchar_t  device[NP_MAX_PATH];
    WORD    *papers;
    wchar_t (*paperNames)[64];
    POINT   *paperSizes;   /* tenths of mm */
    int      paperCount;
    WORD    *bins;
    wchar_t (*binNames)[64];
    int      binCount;
    DWORD    margins[4];   /* L, T, R, B thousandths */
    wchar_t  header[256];
    wchar_t  footer[256];
} PAGESETUP_CTX;

/* Default papers when no printer is installed (DMPAPER_* + size in inches). */
static const struct { const wchar_t *name; WORD id; double w; double h; } g_fallbackPapers[] = {
    { L"Letter", 1, 8.5, 11.0 }, { L"Legal", 5, 8.5, 14.0 },
    { L"Executive", 7, 7.25, 10.5 }, { L"A3", 8, 11.69, 16.54 },
    { L"A4", 9, 8.27, 11.69 }, { L"A5", 11, 5.83, 8.27 },
};

/* Forward declarations */
static void     np_debug(const wchar_t *fmt, ...);
static void     np_menu_measure(MEASUREITEMSTRUCT *mis);
static void     np_menu_draw(const DRAWITEMSTRUCT *dis);
static HFONT    np_get_menu_font(void);
static void     np_menu_append(HMENU pop, UINT flags, UINT_PTR id,
                               const wchar_t *text);
static void     np_menu_append_bar(HMENU bar, UINT_PTR id,
                                   const wchar_t *text);
static void     np_rebuild_menu(void);
static void     np_menu_init_states(HMENU bar);
static void     np_plugins_reappend(void);
static void     np_toolbar_create(void);
static void     np_toolbar_update(void);
static void     np_effective_colors(COLORREF *bg, COLORREF *fg, int *dark);
static LRESULT  np_dark_ctl(HDC hdc, HWND ctl);
static void     np_dark_dlg_init(HWND dlg);
static void     np_status_draw(const DRAWITEMSTRUCT *dis);
static void     np_caret_tick(void);
static void     np_caret_pos(RECT *out);
static INT_PTR CALLBACK np_about_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l);
static LRESULT  np_toolbar_customdraw(LPNMTBCUSTOMDRAW tb);
static void     np_tab_draw(const DRAWITEMSTRUCT *dis);
static LRESULT CALLBACK np_tab_subproc(HWND h, UINT m, WPARAM w, LPARAM l,
                                       UINT_PTR id, DWORD_PTR ref);
static void     np_tab_x_rect(HWND tab, int item, RECT *out);
static LRESULT CALLBACK np_overview_proc(HWND h, UINT m, WPARAM w, LPARAM l);
static void     np_overview_toggle(void);
static WCHAR   *np_format1(const wchar_t *templ, const wchar_t *arg1);
static void     np_skip_exe_name(const wchar_t **cmd);
static int      np_startup_init(void);
static HWND     np_create_main_window(const wchar_t *fileToOpen, int showCmd);
static HMENU    np_build_menu(void);
static void     np_build_accelerators(void);
static int      np_message_loop(void);
static LRESULT CALLBACK np_wnd_proc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam);
static void     np_on_size(void);
static void     np_on_command(int id);
static void     np_init_edit_popup(HMENU editMenu);
static void     np_refresh_title(void);
static void     np_set_dirty(int dirty);
static const wchar_t *np_base_name(void);
static int      np_ask_save(void);
static int      np_file_new(void);
static int      np_file_open(void);
static int      np_file_save(void);
static int      np_file_save_as(void);
static int      np_load_file(const wchar_t *path, int forceEnc, int forceBom, int askCreate);
static int      np_save_file(const wchar_t *path);
static void     np_file_error(const wchar_t *path);
static WCHAR   *np_normalize_eol(const WCHAR *text, NP_EOL *detected);
static WCHAR   *np_denormalize_eol(const WCHAR *text, NP_EOL mode);
static const wchar_t *np_encoding_name(NP_ENCODING enc);
static const wchar_t *np_eol_name(NP_EOL eol);
static UINT_PTR CALLBACK np_ofn_hook(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam);
static void     np_ofn_fill_encodings(HWND dlg, int forSave);
static int      np_check_unicode_loss(const WCHAR *text);
static void     np_insert_time_date(void);
static void     np_find_or_replace(int replace);
static INT_PTR CALLBACK np_find_proc(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam);
static INT_PTR CALLBACK np_replace_proc(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam);
static void     np_find_dlg_init(HWND dlg, int replace);
static int      np_match_at(const WCHAR *text, int pos);
static int      np_find_pos(const WCHAR *text, int textLen, int from, int down);
static void     np_select_match(int pos, int len, int focus);
static int      np_do_find(int fromDlg);
static int      np_do_replace(void);
static int      np_do_replace_all(void);
static void     np_announce(const wchar_t *msg);
static void     np_goto_line(void);
static INT_PTR CALLBACK np_goto_proc(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam);
static void     np_toggle_wrap(void);
static void     np_pick_font(void);
static UINT_PTR CALLBACK np_cf_hook(HWND dlg, UINT msg, WPARAM wParam,
                                    LPARAM lParam);
static void     np_toggle_statusbar(void);
static void     np_zoom_by(int delta);
static void     np_zoom_reset(void);
static void     np_apply_zoom(void);
static void     np_update_status(void);
static void     np_enable_dpi_awareness(void);
static int      np_system_uses_dark(void);
static void     np_apply_theme(void);
static void     np_set_theme(int theme);
static void     np_show_about(void);
static void     np_fire_event(int event);
static int      np_any_idle(void);
static void     np_idle_tick(void);
static void     np_host_init(void);
static void     np_page_setup(void);
static INT_PTR CALLBACK np_pagesetup_proc(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam);
static void     np_ps_fill_lists(HWND dlg, PAGESETUP_CTX *ctx);
static void     np_ps_show_margins(HWND dlg, PAGESETUP_CTX *ctx);
static void     np_ps_paper_thou(PAGESETUP_CTX *ctx, int paperSel,
                                 DWORD *w, DWORD *h);
static int      np_ps_read_margins(HWND dlg, PAGESETUP_CTX *ctx, int complain);
static void     np_ps_update_preview(HWND dlg, PAGESETUP_CTX *ctx);
static void     np_print(void);
static UINT_PTR CALLBACK np_print_hook(HWND dlg, UINT msg, WPARAM wParam,
                                       LPARAM lParam);
static BOOL CALLBACK np_abort_proc(HDC hdc, int code);
static INT_PTR CALLBACK np_abort_dlg_proc(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam);
static int      np_print_document(HDC hdc);
static void     np_expand_codes(const WCHAR *templ, int page, WCHAR *out, size_t outChars,
                                WCHAR *left, size_t leftChars,
                                WCHAR *center, size_t centerChars,
                                WCHAR *right, size_t rightChars);
static int      np_ensure_devmode(void);
static void     np_free_devmode(void);
static int      np_printer_name(WCHAR *out, size_t outChars);
static int      np_autosave_dir(wchar_t *out, size_t outChars);
static void     np_autosave_write(void);
static void     np_autosave_clear_tab(int idx);
static void     np_autosave_clear(void);
static int      np_autosave_offer(void);
static void     np_load_settings(void);
static void     np_save_settings(void);
static void     np_default_font(void);
static void     np_reg_dword(HKEY key, const wchar_t *name, DWORD value);
static int      np_reg_get_dword(HKEY key, const wchar_t *name, DWORD *out);
static int      np_setup_open(const wchar_t *path);
static int      np_watchdog(const WCHAR *args);
/* Power: tabs + session */
static int      np_tabs_init(void);
static int      np_tab_stash(void);
static int      np_tab_show(int idx);
static int      np_tab_new(void);
static int      np_tab_close(int idx);
static void     np_tab_label(int idx);
static void     np_session_dir(wchar_t *out, size_t outChars);
static void     np_session_save(void);
static void     np_session_restore(void);
static void     np_recent_add(const wchar_t *path);
static int      np_recent_list(wchar_t out[][1024], int max);
/* Power: regex */
static int      np_regex_attempt(const WCHAR *pat, int from,
                                 const WCHAR *text, int tlen, int matchCase,
                                 int groups[][2], int maxGroups,
                                 int *steps);
static int      np_regex_match(const WCHAR *pat, const WCHAR *text, int matchCase,
                               int *matchLen, int groups[][2], int maxGroups,
                               int *steps);
static int      np_regex_find_pos(const WCHAR *text, int textLen, int from,
                                  int down, int *matchLen, int groups[][2]);
static void     np_regex_expand(const WCHAR *repl, const WCHAR *src,
                                int groups[][2], int ngroups,
                                WCHAR *out, size_t outChars);
static int      np_find_count(void);
static void     np_find_count_show(HWND dlg);
/* Power: edit tools */
static void     np_sort_lines(int mode);
static void     np_eval_math(void);
static long long np_math_expr(const WCHAR **pp, int *ok);
static void     np_edit_wire(HWND edit);
static void     np_edit_wire(HWND edit);
static LRESULT CALLBACK np_edit_subproc(HWND h, UINT m, WPARAM w, LPARAM l,
                                        UINT_PTR id, DWORD_PTR ref);
static void     np_auto_complete(int force);
static void     np_ac_accept(void);
static void     np_ac_cancel(void);
static void     np_macro_toggle(void);
static void     np_macro_play(void);
static void     np_macro_record_char(WCHAR ch);
static void     np_macro_record_key(int vk);
static void     np_macro_save(void);
static void     np_macro_load(void);
static void     np_macro_free(void);
static void     np_column_op(int op);
static void     np_multicaret_clear(void);
static void     np_multicaret_apply(int vk, WCHAR ch);
static void     np_closed_push(const NP_TAB *t);
static int      np_closed_reopen(void);
static void     np_edit_dupline(void);
static void     np_edit_delline(void);
/* Power: panes + history + palette + plugins + themes + modes */
static void     np_md_toggle(void);
static void     np_md_update(void);
static void     np_md_render(const WCHAR *text, char **rtfOut);
static void     np_md_set_rtf(const char *rtf);
static void     np_sticky_toggle(void);
static void     np_sticky_update(void);
static void     np_color_hover(int idx);
static void     np_color_at(int idx, int edit);
static void     np_tip_hide(void);
static void     np_tip_ensure(void);
static void     np_tip_recolor(void);
static void     np_tip_show(const WCHAR *text, int sx, int sy);
static void     np_link_notify(NMHDR *hdr, LPARAM lParam);
static int      np_get_line_text(HWND edit, int line, WCHAR *buf, int bufChars);
static int      np_hex_at(const WCHAR *lineText, int colInLine, COLORREF *rgb);
static int      np_input_box(const WCHAR *title, const WCHAR *label1,
                             const WCHAR *def1, const WCHAR *label2,
                             const WCHAR *def2, WCHAR *out1, WCHAR *out2);
static void     np_color_edit(void);
static void     np_history_open(void);
static INT_PTR CALLBACK np_history_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l);
static void     np_history_write(void);
static void     np_history_dir(wchar_t *out, size_t outChars,
                               const wchar_t *path);
static void     np_palette_open(void);
static LRESULT CALLBACK np_palette_proc(HWND h, UINT m, WPARAM w, LPARAM l);
static LRESULT CALLBACK np_palette_edit_subproc(HWND h, UINT m, WPARAM w,
                                                LPARAM l, UINT_PTR id,
                                                DWORD_PTR ref);
static int      np_fuzzy_score(const WCHAR *pat, const WCHAR *text);
static void     np_split_set(int mode);
static void     np_split_refresh(void);
static void     np_split_stash(void);
static void     np_split_sync_from(HWND src, UINT msg, WPARAM wParam);
static void     np_distract_toggle(void);
static void     np_stickynote_toggle(void);
static void     np_topmost_toggle(void);
static void     np_theme_menu(HMENU viewMenu);
static void     np_theme_file_dir(wchar_t *out, size_t outChars);
static int      np_theme_parse_color(const wchar_t *s, COLORREF *out);
static void     np_theme_apply_custom(const wchar_t *name);
static void     np_theme_load(const wchar_t *name, COLORREF *bg, COLORREF *fg);
static void     np_effective_colors(COLORREF *bg, COLORREF *fg, int *dark);
static void     np_plugins_load(void);
static void     np_plugins_free(void);
static void     np_largefile_check(DWORD size);

/* ==========================================================================
 *  Helpers
 * ========================================================================== */
static void np_debug(const wchar_t *fmt, ...)
{
    if(g_devLog < 0){
        wchar_t v[8] = {0};
        GetEnvironmentVariableW(L"LOOSDEBUG", v, 8);
        g_devLog = (v[0] == L'1') ? 1 : 0;
    }
    if(!g_devLog) return;
    wchar_t buf[NP_MSG_MAX];
    va_list args;
    va_start(args, fmt);
    _vsnwprintf_s(buf, NP_MSG_MAX, _TRUNCATE, fmt, args);
    va_end(args);
    OutputDebugStringW(L"[notepad] ");
    OutputDebugStringW(buf);
    OutputDebugStringW(L"\n");
}

/* FormatMessageW %1-style templating (like the original). Caller LocalFrees. */
static WCHAR *np_format1(const wchar_t *templ, const wchar_t *arg1)
{
    const wchar_t *args[1];
    WCHAR *out = NULL;
    args[0] = arg1;
    FormatMessageW(FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_STRING |
                   FORMAT_MESSAGE_ARGUMENT_ARRAY,
                   templ, 0, 0, (LPWSTR)&out, 0, (va_list *)args);
    return out;
}

/* ==========================================================================
 *  Startup
 * ========================================================================== */

/* Skip `"C:\path\notepad.exe" ` at the front of the command line, leaving the
 * file to open. Handles quoted and unquoted exe names. */
static void np_skip_exe_name(const wchar_t **cmd)
{
    const wchar_t *p = *cmd;
    int inQuotes = 0;
    while(*p){
        if(*p == L'"') inQuotes = !inQuotes;
        else if(!inQuotes && (*p == L' ' || *p == L'\t')) break;
        p = CharNextW(p);
    }
    while(*p == L' ' || *p == L'\t') p++;
    *cmd = p;
}

int WINAPI wWinMain(HINSTANCE hInstance, HINSTANCE hPrev, PWSTR cmdLine, int showCmd)
{
    (void)hInstance; /* all handles come from GetModuleHandleW(NULL) */
    (void)hPrev;
    (void)cmdLine; /* re-read below, like real Notepad does */
    np_enable_dpi_awareness();
    memset(&g_np, 0, sizeof(g_np));
    g_np.wrap = 1;
    g_np.zoom = NP_ZOOM_DEFAULT;
    g_np.encoding = ENC_ANSI;
    g_np.eolMode = EOL_CRLF;
    g_np.wrapAround = 1;
    g_np.findDown = 1;
    g_np.saveWinPos = 1;
    g_np.showToolbar = 1;
    g_np.autoPairs = 1;
    g_np.tabHover = -1;
    g_np.forceEnc = -1;
    g_np.marginLeft = 750; g_np.marginTop = 750;
    g_np.marginRight = 750; g_np.marginBottom = 750;
    wcsncpy_s(g_np.header, 256, L"&f", _TRUNCATE);
    wcsncpy_s(g_np.footer, 256, L"Page &p", _TRUNCATE);

    const wchar_t *fileArg = GetCommandLineW();
    np_skip_exe_name(&fileArg);

    /* Supervisor mode: notepad.exe --watch [args...] runs itself as a
     * watched child and relaunches it on abnormal exits. Same binary,
     * no separate watchdog.exe. */
    {
        const WCHAR *tok = fileArg;
        while(*tok == L' ' || *tok == L'\t') tok++;
        const WCHAR *end = tok;
        while(*end && *end != L' ' && *end != L'\t') end++;
        size_t n = (size_t)(end - tok);
        if((n == 7 && _wcsnicmp(tok, L"--watch", 7) == 0) ||
           (n == 6 && _wcsnicmp(tok, L"/watch", 6) == 0) ||
           (n == 2 && _wcsnicmp(tok, L"-w", 2) == 0)){
            while(*end == L' ' || *end == L'\t') end++;
            return np_watchdog(end);
        }
    }

    CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
    if(!np_startup_init()){ CoUninitialize(); return 0; }
    /* NO ETW here: the original registers an event provider first thing.
     * This build deliberately does not (see header). */
    if(!np_create_main_window(fileArg, showCmd)){ CoUninitialize(); return 0; }
    int exitCode = np_message_loop();
    np_save_settings();
    np_free_devmode();
    if(g_np.globalAcc) DestroyAcceleratorTable(g_np.globalAcc);
    if(g_np.mainAcc) DestroyAcceleratorTable(g_np.mainAcc);
    if(g_np.slipUpAcc) DestroyAcceleratorTable(g_np.slipUpAcc);
    CoUninitialize();
    return exitCode;
}

/* Register window classes, accelerator tables, find-replace message id,
 * persisted settings. No telemetry providers, no WinEvent hooks. */
static int np_startup_init(void)
{
    INITCOMMONCONTROLSEX icc;
    icc.dwSize = sizeof(icc);
    icc.dwICC = ICC_BAR_CLASSES | ICC_LINK_CLASS;
    InitCommonControlsEx(&icc);
    np_load_settings();
    if(!g_np.haveFont) np_default_font();

    HINSTANCE inst = GetModuleHandleW(NULL);
    WNDCLASSEXW wc;
    memset(&wc, 0, sizeof(wc));
    wc.cbSize = sizeof(wc);
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = np_wnd_proc;
    wc.hInstance = inst;
    wc.hCursor = LoadCursorW(NULL, IDC_ARROW);
    wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.lpszClassName = NP_CLASS_MAIN;
    wc.hIcon = LoadIconW(inst, MAKEINTRESOURCEW(2));
    if(!wc.hIcon) wc.hIcon = LoadIconW(NULL, IDI_APPLICATION);
    wc.hIconSm = wc.hIcon;
    if(!RegisterClassExW(&wc)){
        MessageBoxW(NULL, S_NOMEM, S_APP, MB_OK | MB_ICONSTOP);
        return 0;
    }

    memset(&wc, 0, sizeof(wc));
    wc.cbSize = sizeof(wc);
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = np_overview_proc;
    wc.hInstance = inst;
    wc.hCursor = LoadCursorW(NULL, IDC_ARROW);
    wc.lpszClassName = L"NPOverview";
    if(!RegisterClassExW(&wc)){
        MessageBoxW(NULL, S_NOMEM, S_APP, MB_OK | MB_ICONSTOP);
        return 0;
    }
    np_build_accelerators();
    return 1;
}

static void np_build_accelerators(void)
{
    g_np.globalAcc = CreateAcceleratorTableW((LPACCEL)g_globalAccTable,
        (int)(sizeof(g_globalAccTable) / sizeof(g_globalAccTable[0])));
    /* POWER extras ride on the main table (genuine table stays first). */
    static const ACCEL extras[] = {
        { FVIRTKEY | FCONTROL, 'T', CMD_FILE_NEWTAB },
        { FVIRTKEY | FCONTROL | FSHIFT, 'W', CMD_FILE_CLOSETAB },
        { FVIRTKEY | FCONTROL, VK_TAB, CMD_FILE_NEWTAB + 1000 },
        { FVIRTKEY | FCONTROL | FSHIFT, VK_TAB, CMD_FILE_NEWTAB + 1001 },
        { FVIRTKEY | FCONTROL, 'P', CMD_TOOLS_PALETTE },
        { FVIRTKEY | FCONTROL, 'M', CMD_EDIT_MATH },
    { FVIRTKEY | FCONTROL, 'D', CMD_EDIT_DUP },
    { FVIRTKEY | FCONTROL, 'L', CMD_EDIT_DELLINE },
    { FVIRTKEY | FCONTROL, 'Y', CMD_EDIT_UNDO },
    { FVIRTKEY | FCONTROL | FSHIFT, 'T', CMD_EDIT_REOPEN },
        { FVIRTKEY | FCONTROL, VK_SPACE, CMD_EDIT_MATH + 1000 },
        { FVIRTKEY | FCONTROL | FSHIFT, 'R', CMD_MACRO_RECORD },
        { FVIRTKEY | FCONTROL | FSHIFT, 'E', CMD_MACRO_PLAY },
        { FVIRTKEY, VK_F11, CMD_VIEW_DISTRACT },
    };
    ACCEL merged[64];
    int n = (int)(sizeof(g_mainAccTable) / sizeof(g_mainAccTable[0]));
    memcpy(merged, g_mainAccTable, sizeof(g_mainAccTable));
    memcpy(merged + n, extras, sizeof(extras));
    n += (int)(sizeof(extras) / sizeof(extras[0]));
    g_np.mainAcc = CreateAcceleratorTableW(merged, n);
    g_np.slipUpAcc = CreateAcceleratorTableW((LPACCEL)g_slipUpAccTable,
        (int)(sizeof(g_slipUpAccTable) / sizeof(g_slipUpAccTable[0])));
}

/* Extra accelerator pseudo-commands (handled in np_on_command). */
enum { CMD_TAB_NEXT = CMD_FILE_NEWTAB + 1000, CMD_TAB_PREV, CMD_AC_FORCE = CMD_EDIT_MATH + 1000 };

/* ==========================================================================
 *  Modernization: DPI awareness + light/dark theme + Mica
 * ========================================================================== */

/* PerMonitorV2 without touching the manifest: set it before any window
 * exists. Dynamically loaded so old systems simply stay unaware. */
static void np_enable_dpi_awareness(void)
{
    HMODULE user32 = GetModuleHandleW(L"user32.dll");
    if(user32){
        typedef BOOL (WINAPI *SETDPI_FN)(int);
        SETDPI_FN setCtx = (SETDPI_FN)GetProcAddress(user32,
            "SetProcessDpiAwarenessContext");
        if(setCtx && setCtx(-4 /* DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 */))
            return;
        {
            typedef BOOL (WINAPI *SETDPIOLD_FN)(void);
            SETDPIOLD_FN setOld = (SETDPIOLD_FN)GetProcAddress(user32,
                "SetProcessDPIAware");
            if(setOld) setOld();
        }
    }
}

/* System dark mode: AppsUseLightTheme == 0 (missing key = light). */
static int np_system_uses_dark(void)
{
    HKEY key;
    DWORD v = 1, type = 0, size = sizeof(v);
    if(RegOpenKeyExW(HKEY_CURRENT_USER,
            L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
            0, KEY_READ, &key) != ERROR_SUCCESS)
        return 0;
    if(RegQueryValueExW(key, L"AppsUseLightTheme", NULL, &type,
                        (BYTE *)&v, &size) != ERROR_SUCCESS || type != REG_DWORD)
        v = 1;
    RegCloseKey(key);
    return v == 0;
}

/* Effective editor colors: a custom .theme wins; otherwise the built-in
 * light/dark pair follows g_np.darkActive. */
static void np_effective_colors(COLORREF *bg, COLORREF *fg, int *dark)
{
    if(g_np.themeName[0]){
        *bg = g_np.themeBg;
        *fg = g_np.themeFg;
        double lum = (0.299 * GetRValue(*bg) + 0.587 * GetGValue(*bg) +
                      0.114 * GetBValue(*bg)) / 255.0;
        *dark = (lum < 0.5);
        return;
    }
    *dark = g_np.darkActive;
    if(g_np.darkActive){
        *bg = RGB(LOOS_BRAND_SURFACE_R, LOOS_BRAND_SURFACE_G, LOOS_BRAND_SURFACE_B);
        *fg = RGB(LOOS_BRAND_TEXT_R, LOOS_BRAND_TEXT_G, LOOS_BRAND_TEXT_B);
    }else{
        *bg = RGB(255, 255, 255);
        *fg = RGB(0, 0, 0);
    }
}

/* Resolve and apply the effective theme to frame, controls and brushes. */
static void np_apply_theme(void)
{
    int dark = (g_np.theme == 2) || (g_np.theme == 0 && np_system_uses_dark());
    g_np.darkActive = dark;
    if(g_np.themeName[0]){
        /* A custom theme decides the chrome by background luminance. */
        COLORREF bg = 0, fg = 0;
        int effDark = 0;
        np_effective_colors(&bg, &fg, &effDark);
        g_np.darkActive = dark = effDark;
    }

    if(!g_dwmSetAttr){
        HMODULE dwm = LoadLibraryW(L"dwmapi.dll");
        if(dwm)
            g_dwmSetAttr = (DWM_SET_WINDOW_ATTRIBUTE_FN)GetProcAddress(dwm,
                "DwmSetWindowAttribute");
    }
    if(g_dwmSetAttr && g_np.mainWnd){
        BOOL useDark = dark ? TRUE : FALSE;
        g_dwmSetAttr(g_np.mainWnd, DWMWA_USE_IMMERSIVE_DARK_MODE,
                     &useDark, sizeof(useDark));
        int mica = DWMSBT_MAINWINDOW;
        g_dwmSetAttr(g_np.mainWnd, DWMWA_SYSTEMBACKDROP_TYPE,
                     &mica, sizeof(mica));
    }

    /* DarkMode_Explorer gives dark scrollbars/combos/buttons on Win10 1809+. */
    if(g_np.editBox)
        SetWindowTheme(g_np.editBox,
            dark ? L"DarkMode_Explorer" : L"Explorer", NULL);
    if(g_np.statusBar)
        SetWindowTheme(g_np.statusBar,
            dark ? L"DarkMode_Explorer" : L"Explorer", NULL);

    if(g_np.themeBrush){ DeleteObject(g_np.themeBrush); g_np.themeBrush = NULL; }
    {
        COLORREF bg = 0, fg = 0;
        int effDark = 0;
        np_effective_colors(&bg, &fg, &effDark);
        g_np.themeBrush = CreateSolidBrush(bg);
    }

    if(g_np.editBox){
        COLORREF bg = 0, fg = 0;
        int effDark = 0;
        np_effective_colors(&bg, &fg, &effDark);
        SendMessageW(g_np.editBox, EM_SETBKGNDCOLOR, 0, bg);
        InvalidateRect(g_np.editBox, NULL, TRUE);
    }
    if(g_np.mdView){
        COLORREF bg = 0, fg = 0;
        int effDark = 0;
        np_effective_colors(&bg, &fg, &effDark);
        SendMessageW(g_np.mdView, EM_SETBKGNDCOLOR, 0, bg);
        SetWindowTheme(g_np.mdView,
            dark ? L"DarkMode_Explorer" : L"Explorer", NULL);
        np_md_update();
    }
    if(g_np.statusBar) InvalidateRect(g_np.statusBar, NULL, TRUE);
    if(g_np.toolBar) InvalidateRect(g_np.toolBar, NULL, TRUE);
    if(g_np.tabBar) InvalidateRect(g_np.tabBar, NULL, TRUE);
    np_tip_recolor();
    /* Dark/custom themes get owner-drawn menus built from birth; light
     * stays 100% native. Rebuild only when the mode actually changes. */
    {
        int wantDark = (dark || g_np.themeName[0] != 0) ? 1 : 0;
        if(wantDark != g_np.menuDark && g_np.mainWnd && GetMenu(g_np.mainWnd))
            np_rebuild_menu();
    }
    if(g_np.mainWnd){
        np_refresh_title();
        np_update_status();
    }
    np_debug(L"theme applied: %s", dark ? L"dark" : L"light");
}

static void np_set_theme(int theme)
{
    if(theme < 0 || theme > 2) return;
    g_np.theme = theme;
    g_np.themeName[0] = L'\0';
    np_apply_theme();
    np_save_settings();
}

/* Fallback editor font: Lucida Console 10pt (the classic Notepad default). */
static void np_default_font(void)
{
    HDC screen = GetDC(NULL);
    int dpi = screen ? GetDeviceCaps(screen, LOGPIXELSY) : 96;
    if(screen) ReleaseDC(NULL, screen);
    memset(&g_np.font, 0, sizeof(g_np.font));
    g_np.font.lfHeight = -MulDiv(10, dpi, 72);
    g_np.font.lfWeight = FW_NORMAL;
    g_np.font.lfCharSet = DEFAULT_CHARSET;
    g_np.font.lfOutPrecision = OUT_DEFAULT_PRECIS;
    g_np.font.lfClipPrecision = CLIP_DEFAULT_PRECIS;
    g_np.font.lfQuality = DEFAULT_QUALITY;
    g_np.font.lfPitchAndFamily = FIXED_PITCH | FF_MODERN;
    wcsncpy_s(g_np.font.lfFaceName, LF_FACESIZE, L"Lucida Console", _TRUNCATE);
    g_np.haveFont = 1;
}

static HWND np_create_main_window(const wchar_t *fileToOpen, int showCmd)
{
    HINSTANCE inst = GetModuleHandleW(NULL);
    int x = CW_USEDEFAULT, y = CW_USEDEFAULT, dx = 800, dy = 600;
    if(g_np.saveWinPos && g_np.winDX > 200 && g_np.winDY > 100){
        x = g_np.winX; y = g_np.winY; dx = g_np.winDX; dy = g_np.winDY;
    }
    g_np.mainWnd = CreateWindowExW(0, NP_CLASS_MAIN, S_UNTITLED,
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        x, y, dx, dy, NULL, NULL, inst, NULL);
    if(!g_np.mainWnd) return NULL;

    DWORD editStyle = WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE |
                      ES_AUTOVSCROLL | ES_WANTRETURN;
    if(!g_np.wrap) editStyle |= WS_HSCROLL | ES_AUTOHSCROLL;
    g_np.editBox = CreateWindowExW(WS_EX_CLIENTEDGE, L"Edit", L"", editStyle,
        0, 0, 100, 100, g_np.mainWnd, (HMENU)(INT_PTR)IDC_EDITBOX, inst, NULL);

    g_np.statusBar = CreateWindowExW(0, STATUSCLASSNAMEW, NULL,
        WS_CHILD | SBARS_SIZEGRIP, 0, 0, 0, 0,
        g_np.mainWnd, (HMENU)(INT_PTR)IDC_STATUSBAR, inst, NULL);


    HFONT hf = CreateFontIndirectW(&g_np.font);
    if(hf) SendMessageW(g_np.editBox, WM_SETFONT, (WPARAM)hf, TRUE);
    np_apply_zoom();
    np_edit_wire(g_np.editBox);
    HFONT hui = np_get_menu_font();
    if(hui){
        SendMessageW(g_np.statusBar, WM_SETFONT, (WPARAM)hui, TRUE);
        SendMessageW(g_np.tabBar, WM_SETFONT, (WPARAM)hui, TRUE);
    }
    DragAcceptFiles(g_np.mainWnd, TRUE);
    HMENU bar = np_build_menu();
    SetMenu(g_np.mainWnd, bar);
    g_np.editMenu = GetSubMenu(bar, 1);
    g_np.toolsMenu = GetSubMenu(bar, 4);
    ShowWindow(g_np.statusBar, (!g_np.wrap && g_np.showBar) ? SW_SHOW : SW_HIDE);
    if(!np_tabs_init()) return NULL;
    /* Sticky markdown header bar (hidden until enabled + heading found). */
    g_np.stickyBar = CreateWindowExW(0, L"Static", L"",
        WS_CHILD | SS_LEFTNOWORDWRAP, 0, 0, 100, NP_STICKY_H,
        g_np.mainWnd, NULL, inst, NULL);
    {
        HFONT hbuif = CreateFontW(-11, 0, 0, 0, FW_BOLD, FALSE, FALSE, FALSE,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY, DEFAULT_PITCH | FF_DONTCARE, L"Segoe UI");
        if(hbuif) SendMessageW(g_np.stickyBar, WM_SETFONT, (WPARAM)hbuif, TRUE);
    }
    if(g_np.showOverview){
        g_np.overviewBar = CreateWindowExW(0, L"NPOverview", NULL,
            WS_CHILD, 0, 0, 60, 100,
            g_np.mainWnd, NULL, inst, NULL);
        if(!g_np.stickyOn && g_np.overviewBar)
            SetTimer(g_np.mainWnd, IDT_STICKY, 500, NULL);
    }
    np_plugins_load();
    np_theme_menu(GetSubMenu(bar, 3)); /* View popup: append custom themes */
    np_macro_load();
    np_menu_init_states(bar);
    np_toolbar_create();
    if(g_np.showToolbar && g_np.toolBar) ShowWindow(g_np.toolBar, SW_SHOW);
    else if(g_np.toolBar) ShowWindow(g_np.toolBar, SW_HIDE);

    /* /.SETUP chromeless mode (Windows Setup): open the named file with the
     * SlipUp accelerator table instead of the main one. */
    if(fileToOpen && wcsncmp(fileToOpen, L"/.SETUP", 7) == 0){
        const wchar_t *p = fileToOpen + 7;
        while(*p == L' ' || *p == L'\t') p++;
        if(*p){
            g_np.setupMode = 1;
            SetWindowLongW(g_np.mainWnd, GWL_STYLE, 0xFD0000);
            SetWindowPos(g_np.mainWnd, NULL, 0, 0, 0, 0,
                         SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER |
                         SWP_FRAMECHANGED);
            np_setup_open(p);
        }
    }else if(fileToOpen && *fileToOpen){
        size_t n = wcslen(fileToOpen);
        if(n >= 2 && fileToOpen[0] == L'"' && fileToOpen[n - 1] == L'"'){
            wchar_t clean[NP_MAX_PATH];
            wcsncpy_s(clean, NP_MAX_PATH, fileToOpen + 1, n - 2);
            np_load_file(clean, -1, 0, 1);
        }else{
            np_load_file(fileToOpen, -1, 0, 1);
        }
    }else{
        /* Crash snapshots win over the last clean session; a clean start
         * restores the session, else offers any legacy snapshot. */
        if(!np_autosave_offer()){
            np_session_restore();
            if(g_np.tabCount == 1 && !g_np.path[0] && !g_np.dirty &&
               GetWindowTextLengthW(g_np.editBox) == 0)
                np_autosave_offer();
        }
    }
    np_refresh_title();
    np_on_size();
    SetTimer(g_np.mainWnd, IDT_AUTOSAVE, NP_AUTOSAVE_MS, NULL);
    ShowWindow(g_np.mainWnd, showCmd);
    UpdateWindow(g_np.mainWnd);
    if(g_np.themeName[0]) np_theme_apply_custom(g_np.themeName);
    else np_apply_theme();
    np_fire_event(NP_EVENT_STARTUP);
    return g_np.mainWnd;
}

/* The genuine menu, item for item: same labels, same order, same command
 * IDs. Top-level labels are stored as (id-char + text) in the resource, so
 * File/Edit/View/Zoom/Help get '&' prepended and Format becomes "F&ormat".
 * Cut/Copy/Paste/Delete start grayed (resource state 0x1). Help has no
 * Send Feedback item: feedback-hub collection is stripped (see header). */
static HMENU np_build_menu(void)
{
    HMENU bar = CreateMenu();
    HMENU file = CreatePopupMenu();
    HMENU edit = CreatePopupMenu();
    HMENU format = CreatePopupMenu();
    HMENU view = CreatePopupMenu();
    HMENU zoom = CreatePopupMenu();
    HMENU theme = CreatePopupMenu();
    HMENU sort = CreatePopupMenu();
    HMENU macro = CreatePopupMenu();
    HMENU column = CreatePopupMenu();
    HMENU split = CreatePopupMenu();
    HMENU tools = CreatePopupMenu();
    HMENU help = CreatePopupMenu();

    np_menu_append(file, MF_STRING, CMD_FILE_NEW, L"&New\tCtrl+N");
    np_menu_append(file, MF_STRING, CMD_FILE_NEW_WINDOW, L"New &Window\tCtrl+Shift+N");
    np_menu_append(file, MF_STRING, CMD_FILE_NEWTAB, L"New &Tab\tCtrl+T");
    np_menu_append(file, MF_STRING, CMD_FILE_CLOSETAB, L"&Close Tab\tCtrl+Shift+W");
    np_menu_append(file, MF_STRING, CMD_FILE_OPEN, L"&Open...\tCtrl+O");
    np_menu_append(file, MF_STRING, CMD_FILE_SAVE, L"&Save\tCtrl+S");
    np_menu_append(file, MF_STRING, CMD_FILE_SAVEAS, L"Save &As...\tCtrl+Shift+S");
    np_menu_append(file, MF_STRING, CMD_FILE_HISTORY, L"Version Histor&y...");
    AppendMenuW(file, MF_SEPARATOR, 0, NULL);
    np_menu_append(file, MF_STRING, CMD_FILE_PAGESETUP, L"Page Set&up...");
    np_menu_append(file, MF_STRING, CMD_FILE_PRINT, L"&Print...\tCtrl+P");
    AppendMenuW(file, MF_SEPARATOR, 0, NULL);
    np_menu_append(file, MF_STRING, CMD_FILE_EXIT, L"E&xit");

    np_menu_append(edit, MF_STRING, CMD_EDIT_UNDO, L"&Undo\tCtrl+Z");
    AppendMenuW(edit, MF_SEPARATOR, 0, NULL);
    np_menu_append(edit, MF_STRING, CMD_EDIT_CUT, L"Cu&t\tCtrl+X");
    np_menu_append(edit, MF_STRING, CMD_EDIT_COPY, L"&Copy\tCtrl+C");
    np_menu_append(edit, MF_STRING, CMD_EDIT_PASTE, L"&Paste\tCtrl+V");
    np_menu_append(edit, MF_STRING, CMD_EDIT_DELETE, L"De&lete\tDel");
    AppendMenuW(edit, MF_SEPARATOR, 0, NULL);
    np_menu_append(edit, MF_STRING, CMD_EDIT_FIND, L"&Find...\tCtrl+F");
    np_menu_append(edit, MF_STRING, CMD_EDIT_FINDNEXT, L"Find &Next\tF3");
    np_menu_append(edit, MF_STRING, CMD_EDIT_FINDPREV, L"Find Pre&vious\tShift+F3");
    np_menu_append(edit, MF_STRING, CMD_EDIT_REPLACE, L"&Replace...\tCtrl+H");
    np_menu_append(edit, MF_STRING, CMD_EDIT_GOTO, L"&Go To...\tCtrl+G");
    AppendMenuW(edit, MF_SEPARATOR, 0, NULL);
    np_menu_append(edit, MF_STRING, CMD_EDIT_SELECTALL, L"Select &All\tCtrl+A");
    np_menu_append(edit, MF_STRING, CMD_EDIT_TIMEDATE, L"Time/&Date\tF5");
    AppendMenuW(edit, MF_SEPARATOR, 0, NULL);
    /* POWER additions below (genuine menu ends at Time/Date). */
    np_menu_append(sort, MF_STRING, CMD_SORT_ALPHA, L"Alphabetical (A-Z)");
    np_menu_append(sort, MF_STRING, CMD_SORT_DESC, L"Alphabetical (Z-A)");
    np_menu_append(sort, MF_STRING, CMD_SORT_NUMERIC, L"Numeric");
    np_menu_append(sort, MF_STRING, CMD_SORT_LENGTH, L"By length");
    np_menu_append(sort, MF_STRING, CMD_SORT_UNIQUE, L"Remove duplicate lines");
    np_menu_append(sort, MF_STRING, CMD_SORT_TRIM, L"Trim trailing whitespace");
    np_menu_append(edit, MF_POPUP, (UINT_PTR)sort, L"Sort &Lines");
    np_menu_append(edit, MF_STRING, CMD_EDIT_MATH, L"Evaluate &Math\tCtrl+M");
    np_menu_append(macro, MF_STRING, CMD_MACRO_RECORD, L"&Record\tCtrl+Shift+R");
    np_menu_append(macro, MF_STRING, CMD_MACRO_PLAY, L"&Play\tCtrl+Shift+E");
    np_menu_append(edit, MF_POPUP, (UINT_PTR)macro, L"&Macro");
    np_menu_append(column, MF_STRING, CMD_COL_INSERT, L"&Insert text at column...");
    np_menu_append(column, MF_STRING, CMD_COL_DELETE, L"&Delete columns...");
    np_menu_append(column, MF_STRING, CMD_COL_COPY, L"&Copy columns...");
    np_menu_append(edit, MF_POPUP, (UINT_PTR)column, L"&Columns");

    np_menu_append(format, MF_STRING, CMD_FORMAT_WORDWRAP, L"&Word Wrap");
    np_menu_append(format, MF_STRING, CMD_FORMAT_FONT, L"&Font...");
    np_menu_append(format, MF_STRING, CMD_FORMAT_PAIRS, L"Auto-Close &Pairs");

    np_menu_append(zoom, MF_STRING, CMD_VIEW_ZOOMIN, L"Zoom &In\tCtrl+Plus");
    np_menu_append(zoom, MF_STRING, CMD_VIEW_ZOOMOUT, L"Zoom &Out\tCtrl+Minus");
    np_menu_append(zoom, MF_STRING, CMD_VIEW_ZOOMRESTORE, L"&Restore Default Zoom\tCtrl+0");
    np_menu_append(view, MF_POPUP, (UINT_PTR)zoom, L"&Zoom");
    np_menu_append(view, MF_STRING, CMD_VIEW_STATUSBAR, L"&Status Bar");
    np_menu_append(view, MF_STRING, CMD_VIEW_TOOLBAR, L"&Toolbar");
    np_menu_append(view, MF_STRING, CMD_VIEW_OVERVIEW, L"&Overview Strip");
    AppendMenuW(view, MF_SEPARATOR, 0, NULL);
    /* Modern addition (radio-checked in np_refresh_title). */
    np_menu_append(theme, MF_STRING, CMD_THEME_SYSTEM, L"Use &system setting");
    np_menu_append(theme, MF_STRING, CMD_THEME_LIGHT, L"&Light");
    np_menu_append(theme, MF_STRING, CMD_THEME_DARK, L"&Dark");
    np_menu_append(view, MF_POPUP, (UINT_PTR)theme, L"&Theme");
    /* POWER additions below. */
    AppendMenuW(view, MF_SEPARATOR, 0, NULL);
    np_menu_append(view, MF_STRING, CMD_VIEW_MD, L"&Markdown Preview");
    np_menu_append(view, MF_STRING, CMD_VIEW_STICKYSCROLL, L"Stic&ky Scroll Header");
    np_menu_append(split, MF_STRING, CMD_VIEW_SPLITV, L"&Vertical");
    np_menu_append(split, MF_STRING, CMD_VIEW_SPLITH, L"&Horizontal");
    np_menu_append(split, MF_STRING, CMD_VIEW_SPLITSYNC, L"&Sync Scrolling");
    np_menu_append(view, MF_POPUP, (UINT_PTR)split, L"S&plit View");
    np_menu_append(view, MF_STRING, CMD_VIEW_DISTRACT, L"&Distraction Free\tF11");
    np_menu_append(view, MF_STRING, CMD_VIEW_TOPMOST, L"&Always on Top");
    np_menu_append(view, MF_STRING, CMD_VIEW_STICKYNOTE, L"Sticky &Note Mode");

    np_menu_append(tools, MF_STRING, CMD_TOOLS_PALETTE, L"Command &Palette...\tCtrl+P");
    AppendMenuW(tools, MF_SEPARATOR, 0, NULL);
    g_np.toolsMenu = tools;

    np_menu_append(help, MF_STRING, CMD_HELP_TOPIC, L"View &Help");
    AppendMenuW(help, MF_SEPARATOR, 0, NULL);
    np_menu_append(help, MF_STRING, CMD_HELP_ABOUT, L"&About LoOS Notepad");

    /* Top-level bar: native in light mode, owner-drawn in dark mode
     * (drawn flat without arrows by np_menu_draw). */
    np_menu_append_bar(bar, (UINT_PTR)file, L"&File");
    np_menu_append_bar(bar, (UINT_PTR)edit, L"&Edit");
    np_menu_append_bar(bar, (UINT_PTR)format, L"F&ormat");
    np_menu_append_bar(bar, (UINT_PTR)view, L"&View");
    np_menu_append_bar(bar, (UINT_PTR)tools, L"&Tools");
    np_menu_append_bar(bar, (UINT_PTR)help, L"&Help");
    return bar;
}

/* Same loop shape as the original: GLOBALACC, then the modeless Find dialog
 * gets its keys, then MAINACC (SlipUpAcc instead in /.SETUP mode). */
static int np_message_loop(void)
{
    MSG msg;
    HACCEL second = g_np.setupMode ? g_np.slipUpAcc : g_np.mainAcc;
    while(GetMessageW(&msg, NULL, 0, 0)){
        if(TranslateAcceleratorW(g_np.mainWnd, g_np.globalAcc, &msg)) continue;
        if(g_np.findDlg && IsDialogMessageW(g_np.findDlg, &msg)) continue;
        if(TranslateAcceleratorW(g_np.mainWnd, second, &msg)) continue;
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    return (int)msg.wParam;
}

/* ==========================================================================
 *  Window procedures
 * ========================================================================== */
static LRESULT CALLBACK np_wnd_proc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam)
{
    switch(msg){
    case WM_SIZE:
        np_on_size();
        return 0;
    case WM_SETFOCUS:
        if(g_np.editBox) SetFocus(g_np.editBox);
        return 0;
    case WM_CTLCOLOREDIT:
        if((HWND)lParam == g_np.editBox){
            COLORREF bg = 0, fg = 0;
            int effDark = 0;
            np_effective_colors(&bg, &fg, &effDark);
            if(effDark || g_np.themeName[0]){
                HDC hdc = (HDC)wParam;
                SetTextColor(hdc, fg);
                SetBkColor(hdc, bg);
                return (LRESULT)g_np.themeBrush;
            }
        }
        break;
    case WM_CTLCOLORLISTBOX:
        if((HWND)lParam == g_np.acPopup){
            LRESULT br = np_dark_ctl((HDC)wParam, (HWND)lParam);
            if(br) return br;
        }
        break;
    case WM_CTLCOLORSTATIC: {
        if((HWND)lParam == g_np.stickyBar){
            COLORREF bg = 0, fg = 0;
            int effDark = 0;
            np_effective_colors(&bg, &fg, &effDark);
            if(!effDark && !g_np.themeName[0]) break;
            HDC hdc = (HDC)wParam;
            SetTextColor(hdc, fg);
            SetBkColor(hdc, bg);
            if(!g_np.themeBrush){
                COLORREF b2 = 0, f2 = 0;
                int d2 = 0;
                np_effective_colors(&b2, &f2, &d2);
                g_np.themeBrush = CreateSolidBrush(b2);
            }
            return (LRESULT)g_np.themeBrush;
        }
        break;
    }
    case WM_SETTINGCHANGE:
        /* System theme toggled and we follow it: re-apply live. */
        if(g_np.theme == 0) np_apply_theme();
        return 0;
    case WM_DPICHANGED:
        /* DefWindowProc applies the suggested rect; children reflow below. */
        np_on_size();
        np_update_status();
        break;
    case WM_COMMAND:
        if((HWND)lParam == g_np.editBox){
            if(HIWORD(wParam) == EN_CHANGE){
                if(!g_np.editGuard && g_np.caretCount > 1)
                    np_multicaret_clear();
                np_set_dirty(1);
                np_update_status();
                np_fire_event(NP_EVENT_CHANGE);
                if(g_np.mdPreview && !g_np.mdTimerOn){
                    g_np.mdTimerOn = 1;
                    SetTimer(g_np.mainWnd, IDT_MDUPDATE, 400, NULL);
                }
                if(g_np.stickyOn) np_sticky_update();
                if(g_np.showOverview && g_np.overviewBar)
                    InvalidateRect(g_np.overviewBar, NULL, FALSE);
                return 0;
            }
            if(HIWORD(wParam) == EN_ERRSPACE){
                MessageBoxW(g_np.mainWnd, S_NOMEM, S_APP, MB_OK | MB_ICONSTOP);
                return 0;
            }
        }
        if((HWND)lParam == g_np.splitEdit && HIWORD(wParam) == EN_CHANGE &&
           !g_np.editGuard && g_np.splitTab >= 0 &&
           g_np.splitTab < g_np.tabCount){
            /* The split pane edits its own tab live. */
            g_np.tabs[g_np.splitTab].dirty = 1;
            np_tab_label(g_np.splitTab);
            return 0;
        }
        np_on_command(LOWORD(wParam));
        return 0;
    case WM_INITMENUPOPUP:
        if((HMENU)wParam == g_np.editMenu) np_init_edit_popup((HMENU)wParam);
        return 0;
    case WM_TIMER:
        if(wParam == IDT_AUTOSAVE) np_autosave_write();
        else if(wParam == IDT_STATUSMSG){
            KillTimer(hwnd, IDT_STATUSMSG);
            g_np.statusMsgActive = 0;
            np_update_status();
        }else if(wParam == IDT_MDUPDATE){
            KillTimer(hwnd, IDT_MDUPDATE);
            g_np.mdTimerOn = 0;
            np_md_update();
        }else if(wParam == IDT_STICKY){
            if(g_np.stickyOn) np_sticky_update();
            if(g_np.showOverview && g_np.overviewBar &&
               IsWindowVisible(g_np.overviewBar))
                InvalidateRect(g_np.overviewBar, NULL, FALSE);
        }else if(wParam == IDT_CARET){
            np_caret_tick();
        }else if(wParam == IDT_PLUGINIDLE){
            np_idle_tick();
        }
        return 0;
    case WM_NOTIFY: {
        LPNMHDR hdr = (LPNMHDR)lParam;
        if(hdr->hwndFrom == g_np.tabBar && hdr->code == TCN_SELCHANGE){
            int sel = (int)SendMessageW(g_np.tabBar, TCM_GETCURSEL, 0, 0);
            np_tab_show(sel);
            return 0;
        }
        if(hdr->hwndFrom == g_np.editBox && hdr->code == EN_LINK){
            np_link_notify(hdr, lParam);
            return 0;
        }
        if(hdr->hwndFrom == g_np.toolBar && hdr->code == NM_CUSTOMDRAW)
            return np_toolbar_customdraw((LPNMTBCUSTOMDRAW)lParam);
        break;
    }
    case WM_MEASUREITEM:
        np_menu_measure((MEASUREITEMSTRUCT *)lParam);
        return TRUE;
    case WM_DRAWITEM: {
        DRAWITEMSTRUCT *dis = (DRAWITEMSTRUCT *)lParam;
        if(dis->CtlType == ODT_MENU){
            np_menu_draw(dis);
            return TRUE;
        }
        if(dis->CtlType == ODT_TAB && dis->hwndItem == g_np.tabBar){
            np_tab_draw(dis);
            return TRUE;
        }
        if(dis->CtlType == ODT_STATIC && dis->hwndItem == g_np.statusBar){
            np_status_draw(dis);
            return TRUE;
        }
        break;
    }
    case WM_VSCROLL:
    case WM_HSCROLL:
        if((HWND)lParam == g_np.editBox || (HWND)lParam == g_np.splitEdit)
            np_split_sync_from((HWND)lParam, msg, wParam);
        if(g_np.stickyOn) np_sticky_update();
        if(g_np.showOverview && g_np.overviewBar)
            InvalidateRect(g_np.overviewBar, NULL, FALSE);
        break;
    case WM_QUERYENDSESSION:
        if(g_np.saveDlgOpen){
            MessageBoxW(hwnd, S_SHUTDOWN, S_APP, MB_OK | MB_ICONWARNING);
            return FALSE;
        }
        return TRUE;
    case WM_CLOSE: {
        /* Every dirty tab gets its own save prompt (active tab switches
         * along so titles, encodings and contents all match). */
        np_tab_stash();
        for(int i = 0; i < g_np.tabCount; i++){
            if(g_np.tabs[i].dirty){
                np_tab_show(i);
                if(np_ask_save() == IDCANCEL) return 0;
            }
        }
        KillTimer(hwnd, IDT_AUTOSAVE);
        KillTimer(hwnd, IDT_STATUSMSG);
        KillTimer(hwnd, IDT_MDUPDATE);
        KillTimer(hwnd, IDT_STICKY);
        KillTimer(hwnd, IDT_CARET);
        DestroyWindow(hwnd);
        return 0;
    }
    case WM_DESTROY:
        np_fire_event(NP_EVENT_SHUTDOWN);
        KillTimer(hwnd, IDT_PLUGINIDLE);
        np_autosave_clear();
        np_session_save();
        np_macro_save();
        np_macro_free();
        np_plugins_free();
        if(g_np.menuFont){ DeleteObject(g_np.menuFont); g_np.menuFont = NULL; }
        np_save_settings();
        if(g_np.themeBrush){ DeleteObject(g_np.themeBrush); g_np.themeBrush = NULL; }
        for(int ti = 0; ti < g_np.tabCount; ti++) free(g_np.tabs[ti].text);
        free(g_np.tabs);
        for(int ci = 0; ci < g_closedCount; ci++) free(g_closed[ci].text);
        if(g_np.richLib) FreeLibrary(g_np.richLib);
        PostQuitMessage(0);
        return 0;
    case WM_DROPFILES: {
        HDROP drop = (HDROP)wParam;
        UINT count = DragQueryFileW(drop, 0xFFFFFFFF, NULL, 0);
        for(UINT i = 0; i < count; i++){
            wchar_t path[NP_MAX_PATH];
            if(!DragQueryFileW(drop, i, path, NP_MAX_PATH)) continue;
            if(i > 0 || g_np.path[0] || g_np.dirty ||
               GetWindowTextLengthW(g_np.editBox) != 0)
                np_tab_new();
            np_load_file(path, -1, 0, 1);
        }
        DragFinish(drop);
        return 0;
    }
    case WM_MOUSEWHEEL:
        if(GetKeyState(VK_CONTROL) & 0x8000){
            np_zoom_by(GET_WHEEL_DELTA_WPARAM(wParam) > 0 ? NP_ZOOM_STEP : -NP_ZOOM_STEP);
            return 0;
        }
        break;
    case WM_SYSCOMMAND:
        /* Minimized = invisible: hand our working set back to the OS.
         * (Private bytes stay tiny anyway; this drops the resident set.) */
        if((wParam & 0xFFF0) == SC_MINIMIZE)
            SetProcessWorkingSetSize(GetCurrentProcess(),
                                     (SIZE_T)-1, (SIZE_T)-1);
        break;
    case WM_NCHITTEST:
        /* Borderless sticky notes drag with Alt held anywhere. */
        if(g_np.stickyNote && (GetKeyState(VK_MENU) & 0x8000)){
            LRESULT hit = DefWindowProcW(hwnd, msg, wParam, lParam);
            if(hit == HTCLIENT) return HTCAPTION;
            return hit;
        }
        break;
    }
    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

/* Banner on top, edit fills the rest, status bar takes the bottom strip. */
static void np_on_size(void)
{
    if(!g_np.mainWnd || !g_np.editBox) return;
    RECT rc;
    GetClientRect(g_np.mainWnd, &rc);
    int top = 0;
    if(g_np.tabBar && !g_np.distractOn && !g_np.stickyNote){
        MoveWindow(g_np.tabBar, 0, top, rc.right, NP_TABSTRIP_H, TRUE);
        top += NP_TABSTRIP_H;
    }
    if(g_np.toolBar && g_np.showToolbar &&
       !g_np.distractOn && !g_np.stickyNote){
        MoveWindow(g_np.toolBar, 0, top, rc.right, 30, TRUE);
        top += 30;
    }
    if(g_np.stickyBar && g_np.stickyOn && IsWindowVisible(g_np.stickyBar)){
        MoveWindow(g_np.stickyBar, 0, top, rc.right, NP_STICKY_H, TRUE);
        top += NP_STICKY_H;
    }
    int barH = 0;
    if(g_np.statusBar && IsWindowVisible(g_np.statusBar) &&
       !g_np.distractOn && !g_np.stickyNote){
        SendMessageW(g_np.statusBar, WM_SIZE, 0, 0);
        RECT br;
        GetWindowRect(g_np.statusBar, &br);
        barH = br.bottom - br.top;
    }
    int editTop = top;
    int editH = rc.bottom - top - barH;
    int editW = rc.right;
    int editX = 0;
    if(g_np.splitEdit && IsWindowVisible(g_np.splitEdit) && g_np.splitMode){
        if(g_np.splitMode == 1){
            editW = rc.right * 3 / 5;
            MoveWindow(g_np.splitEdit, editW, top,
                       rc.right - editW, editH, TRUE);
        }else{
            editH = editH * 3 / 5;
            MoveWindow(g_np.splitEdit, 0, top + editH,
                       rc.right, rc.bottom - top - editH - barH, TRUE);
        }
    }
    int showOverview = (g_np.overviewBar && g_np.showOverview &&
                        !g_np.distractOn && !g_np.stickyNote) ? 1 : 0;
    if(showOverview) editW -= 60;
    if(g_np.mdView && IsWindowVisible(g_np.mdView) && g_np.mdPreview){
        int half = editW / 2;
        MoveWindow(g_np.editBox, editX, editTop, half, editH, TRUE);
        MoveWindow(g_np.mdView, editX + half, editTop,
                   editW - half, editH, TRUE);
    }else{
        MoveWindow(g_np.editBox, editX, editTop, editW, editH, TRUE);
    }
    if(g_np.overviewBar){
        if(showOverview){
            MoveWindow(g_np.overviewBar, editX + editW, editTop,
                       60, editH, TRUE);
            ShowWindow(g_np.overviewBar, SW_SHOW);
            InvalidateRect(g_np.overviewBar, NULL, FALSE);
        }else{
            ShowWindow(g_np.overviewBar, SW_HIDE);
        }
    }
    np_update_status();
}

static void np_on_command(int id)
{
    switch(id){
    case CMD_FILE_NEW: np_file_new(); break;
    case CMD_FILE_NEW_WINDOW: {
        wchar_t exe[NP_MAX_PATH];
        GetModuleFileNameW(NULL, exe, NP_MAX_PATH);
        ShellExecuteW(NULL, L"open", exe, NULL, NULL, SW_SHOWNORMAL);
        break;
    }
    case CMD_FILE_OPEN: np_file_open(); break;
    case CMD_FILE_SAVE: np_file_save(); break;
    case CMD_FILE_SAVEAS: np_file_save_as(); break;
    case CMD_FILE_HISTORY: np_history_open(); break;
    case CMD_FILE_NEWTAB: np_tab_new(); break;
    case CMD_FILE_CLOSETAB: np_tab_close(g_np.tabActive); break;
    case CMD_TAB_NEXT:
        if(g_np.tabCount > 1)
            np_tab_show((g_np.tabActive + 1) % g_np.tabCount);
        break;
    case CMD_TAB_PREV:
        if(g_np.tabCount > 1)
            np_tab_show((g_np.tabActive + g_np.tabCount - 1) % g_np.tabCount);
        break;
    case CMD_FILE_PAGESETUP: np_page_setup(); break;
    case CMD_FILE_PRINT: np_print(); break;
    case CMD_FILE_EXIT:
        SendMessageW(g_np.mainWnd, WM_CLOSE, 0, 0);
        break;
    case CMD_EDIT_UNDO: SendMessageW(g_np.editBox, EM_UNDO, 0, 0); break;
    case CMD_EDIT_CUT: SendMessageW(g_np.editBox, WM_CUT, 0, 0); break;
    case CMD_EDIT_COPY: SendMessageW(g_np.editBox, WM_COPY, 0, 0); break;
    case CMD_EDIT_PASTE: SendMessageW(g_np.editBox, WM_PASTE, 0, 0); break;
    case CMD_EDIT_DELETE: SendMessageW(g_np.editBox, WM_CLEAR, 0, 0); break;
    case CMD_EDIT_FIND: np_find_or_replace(0); break;
    case CMD_EDIT_REPLACE: np_find_or_replace(1); break;
    case CMD_EDIT_FINDNEXT: np_do_find(0); break;
    case CMD_EDIT_FINDPREV: {
        int down = g_np.findDown;
        g_np.findDown = 0;
        np_do_find(0);
        g_np.findDown = down;
        break;
    }
    case CMD_EDIT_GOTO: np_goto_line(); break;
    case CMD_EDIT_SELECTALL:
        SendMessageW(g_np.editBox, EM_SETSEL, 0, (LPARAM)-1);
        break;
    case CMD_EDIT_TIMEDATE: np_insert_time_date(); break;
    case CMD_SORT_ALPHA: np_sort_lines(0); break;
    case CMD_SORT_DESC: np_sort_lines(1); break;
    case CMD_SORT_NUMERIC: np_sort_lines(2); break;
    case CMD_SORT_LENGTH: np_sort_lines(3); break;
    case CMD_SORT_UNIQUE: np_sort_lines(4); break;
    case CMD_SORT_TRIM: np_sort_lines(5); break;
    case CMD_EDIT_MATH: np_eval_math(); break;
    case CMD_AC_FORCE: np_auto_complete(1); break;
    case CMD_MACRO_RECORD: np_macro_toggle(); break;
    case CMD_MACRO_PLAY: np_macro_play(); break;
    case CMD_COL_INSERT: np_column_op(0); break;
    case CMD_COL_DELETE: np_column_op(1); break;
    case CMD_COL_COPY: np_column_op(2); break;
    case CMD_EDIT_DUP: np_edit_dupline(); break;
    case CMD_EDIT_DELLINE: np_edit_delline(); break;
    case CMD_EDIT_REOPEN: np_closed_reopen(); break;
    case CMD_FORMAT_WORDWRAP: np_toggle_wrap(); break;
    case CMD_FORMAT_FONT: np_pick_font(); break;
    case CMD_FORMAT_PAIRS:
        g_np.autoPairs = !g_np.autoPairs;
        np_refresh_title();
        np_save_settings();
        break;
    case CMD_VIEW_STATUSBAR: np_toggle_statusbar(); break;
    case CMD_VIEW_TOOLBAR:
        g_np.showToolbar = !g_np.showToolbar;
        if(g_np.toolBar)
            ShowWindow(g_np.toolBar, g_np.showToolbar ? SW_SHOW : SW_HIDE);
        np_on_size();
        np_refresh_title();
        np_save_settings();
        break;
    case CMD_VIEW_OVERVIEW: np_overview_toggle(); break;
    case CMD_VIEW_ZOOMIN: np_zoom_by(NP_ZOOM_STEP); break;
    case CMD_VIEW_ZOOMOUT: np_zoom_by(-NP_ZOOM_STEP); break;
    case CMD_VIEW_ZOOMRESTORE: np_zoom_reset(); break;
    case CMD_THEME_SYSTEM: np_set_theme(0); break;
    case CMD_THEME_LIGHT: np_set_theme(1); break;
    case CMD_THEME_DARK: np_set_theme(2); break;
    case CMD_VIEW_MD: np_md_toggle(); break;
    case CMD_VIEW_STICKYSCROLL: np_sticky_toggle(); break;
    case CMD_VIEW_SPLITV: np_split_set(1); break;
    case CMD_VIEW_SPLITH: np_split_set(2); break;
    case CMD_VIEW_SPLITSYNC:
        g_np.splitSync = !g_np.splitSync;
        np_refresh_title();
        break;
    case CMD_VIEW_DISTRACT: np_distract_toggle(); break;
    case CMD_VIEW_TOPMOST: np_topmost_toggle(); break;
    case CMD_VIEW_STICKYNOTE: np_stickynote_toggle(); break;
    case CMD_TOOLS_PALETTE: np_palette_open(); break;
    case CMD_HELP_TOPIC:
        ShellExecuteW(NULL, L"open", NP_HELP_URL, NULL, NULL, SW_SHOWNORMAL);
        break;
    case CMD_HELP_ABOUT: np_show_about(); break;
    default:
        if(id >= CMD_PLUGIN_BASE && id < CMD_PLUGIN_END){
            for(int i = 0; i < g_np.pluginCount; i++){
                if(id >= g_np.plugins[i].baseId &&
                   id < g_np.plugins[i].baseId + g_np.plugins[i].count){
                    NP_PLUGIN *pl = &g_np.plugins[i];
                    int cmd = id - pl->baseId;
                    int saved = s_activePlugin;
                    s_activePlugin = i;
                    if(pl->abiVersion >= 3 && pl->exec3)
                        pl->exec3(cmd, &g_host3);
                    else if(pl->abiVersion >= 2 && pl->exec2)
                        pl->exec2(cmd, (const NP_HOST *)&g_host3);
                    else if(pl->exec)
                        pl->exec(cmd, g_np.editBox, g_np.mainWnd);
                    s_activePlugin = saved;
                    break;
                }
            }
        }else if(id >= CMD_THEME_CUSTOM && id < CMD_THEME_CUSTOM_END){
            int idx = id - CMD_THEME_CUSTOM;
            if(idx >= 0 && idx < g_np.customThemeCount)
                np_theme_apply_custom(g_np.customThemes[idx]);
        }
        break;
    }
}

/* Cut/Copy/Delete need a selection, Paste needs clipboard text, Undo
 * needs an undoable edit -- exactly like the genuine menu states. */
static void np_init_edit_popup(HMENU editMenu)
{
    DWORD selS = 0, selE = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&selS, (LPARAM)&selE);
    int haveSel = (selS != selE);
    int canUndo = (int)SendMessageW(g_np.editBox, EM_CANUNDO, 0, 0);
    int canPaste = IsClipboardFormatAvailable(CF_UNICODETEXT) ||
                   IsClipboardFormatAvailable(CF_TEXT);
    EnableMenuItem(editMenu, CMD_EDIT_UNDO,
        MF_BYCOMMAND | (canUndo ? MF_ENABLED : MF_GRAYED));
    EnableMenuItem(editMenu, CMD_EDIT_CUT,
        MF_BYCOMMAND | (haveSel ? MF_ENABLED : MF_GRAYED));
    EnableMenuItem(editMenu, CMD_EDIT_COPY,
        MF_BYCOMMAND | (haveSel ? MF_ENABLED : MF_GRAYED));
    EnableMenuItem(editMenu, CMD_EDIT_PASTE,
        MF_BYCOMMAND | (canPaste ? MF_ENABLED : MF_GRAYED));
    EnableMenuItem(editMenu, CMD_EDIT_DELETE,
        MF_BYCOMMAND | (haveSel ? MF_ENABLED : MF_GRAYED));
}

/* ==========================================================================
 *  Title + dirty flag + save prompt
 * ========================================================================== */
static const wchar_t *np_base_name(void)
{
    if(!g_np.path[0]) return S_UNTITLED;
    const wchar_t *slash = wcsrchr(g_np.path, L'\\');
    return slash ? slash + 1 : g_np.path;
}

/* Exact title format: "%1%2 - Notepad" where %1 is "*" when dirty. */
static void np_refresh_title(void)
{
    wchar_t title[NP_TITLE_MAX];
    swprintf_s(title, NP_TITLE_MAX, S_TITLE_FMT,
               g_np.dirty ? L"*" : L"", np_base_name());
    SetWindowTextW(g_np.mainWnd, title);
    HMENU menu = GetMenu(g_np.mainWnd);
    if(menu){
        CheckMenuItem(menu, CMD_FORMAT_WORDWRAP,
            MF_BYCOMMAND | (g_np.wrap ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_FORMAT_PAIRS,
            MF_BYCOMMAND | (g_np.autoPairs ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_VIEW_STATUSBAR,
            MF_BYCOMMAND | (g_np.showBar ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_VIEW_TOOLBAR,
            MF_BYCOMMAND | (g_np.showToolbar ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_VIEW_OVERVIEW,
            MF_BYCOMMAND | (g_np.showOverview ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuRadioItem(menu, CMD_THEME_SYSTEM, CMD_THEME_DARK,
            CMD_THEME_SYSTEM + g_np.theme, MF_BYCOMMAND);
        if(g_np.themeName[0]){
            /* A custom theme owns the check: clear built-ins, check it. */
            CheckMenuRadioItem(menu, CMD_THEME_SYSTEM, CMD_THEME_DARK,
                               0, MF_BYCOMMAND);
            for(int ti = 0; ti < g_np.customThemeCount; ti++)
                CheckMenuItem(menu, CMD_THEME_CUSTOM + ti, MF_BYCOMMAND |
                    (_wcsicmp(g_np.customThemes[ti], g_np.themeName) == 0
                     ? MF_CHECKED : MF_UNCHECKED));
        }
        CheckMenuItem(menu, CMD_VIEW_MD,
            MF_BYCOMMAND | (g_np.mdPreview ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_VIEW_STICKYSCROLL,
            MF_BYCOMMAND | (g_np.stickyOn ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_VIEW_SPLITSYNC,
            MF_BYCOMMAND | (g_np.splitSync ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_VIEW_DISTRACT,
            MF_BYCOMMAND | (g_np.distractOn ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_VIEW_TOPMOST,
            MF_BYCOMMAND | (g_np.topmost ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_VIEW_STICKYNOTE,
            MF_BYCOMMAND | (g_np.stickyNote ? MF_CHECKED : MF_UNCHECKED));
        CheckMenuItem(menu, CMD_MACRO_RECORD,
            MF_BYCOMMAND | (g_np.recording ? MF_CHECKED : MF_UNCHECKED));
        EnableMenuItem(menu, CMD_FORMAT_WORDWRAP, MF_BYCOMMAND |
            (g_np.tabs && g_np.tabActive >= 0 &&
             g_np.tabActive < g_np.tabCount &&
             g_np.tabs[g_np.tabActive].largeFile ? MF_DISABLED : MF_ENABLED));
    }
}

static void np_set_dirty(int dirty)
{
    if(g_np.dirty != dirty){
        g_np.dirty = dirty;
        if(g_np.tabs && g_np.tabActive >= 0 && g_np.tabActive < g_np.tabCount){
            g_np.tabs[g_np.tabActive].dirty = dirty;
            np_tab_label(g_np.tabActive);
        }
        np_refresh_title();
    }
}

/* File > New means a fresh tab; the current tab is left alone. */
static int np_file_new(void)
{
    return np_tab_new();
}

/* Save prompt: TaskDialogIndirect with &Save / Do&n't Save / Cancel, title
 * "Notepad" (strings 3000-3003); classic MessageBox YESNOCANCEL|QUESTION
 * (0x23) fallback if TaskDialog is unavailable. Returns IDYES/IDNO/IDCANCEL. */
static int np_ask_save(void)
{
    if(!g_np.dirty) return IDNO;
    WCHAR *text = np_format1(S_SAVE_TASK, np_base_name());
    if(!text) return IDCANCEL;

    TASKDIALOG_BUTTON buttons[2];
    buttons[0].nButtonID = IDYES;
    buttons[0].pszButtonText = S_SAVE_BTN;
    buttons[1].nButtonID = IDNO;
    buttons[1].pszButtonText = S_DONTSAVE_BTN;

    TASKDIALOGCONFIG cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.cbSize = sizeof(cfg);
    cfg.hwndParent = g_np.mainWnd;
    cfg.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION;
    cfg.dwCommonButtons = TDCBF_CANCEL_BUTTON;
    cfg.pszWindowTitle = S_APP;
    cfg.pszMainInstruction = text;
    cfg.cButtons = 2;
    cfg.pButtons = buttons;
    cfg.nDefaultButton = IDYES;

    int pressed = 0;
    HRESULT hr = TaskDialogIndirect(&cfg, &pressed, NULL, NULL);
    if(FAILED(hr) || pressed == 0){
        /* Fallback: "The text in the %s file has changed..." (string 3). */
        WCHAR *classic = np_format1(S_SAVE_CLASSIC, np_base_name());
        if(classic){
            pressed = MessageBoxW(g_np.mainWnd, classic, S_APP,
                                  MB_YESNOCANCEL | MB_ICONQUESTION);
            LocalFree(classic);
        }else{
            pressed = IDCANCEL;
        }
    }
    LocalFree(text);
    if(pressed == IDYES && !np_file_save()) return IDCANCEL;
    return pressed;
}

/* ==========================================================================
 *  File
 * ========================================================================== */
/* (np_file_new lives above with the dirty-flag helpers: New = fresh tab.) */

static int np_file_open(void)
{
    wchar_t path[NP_MAX_PATH] = {0};
    OPENFILENAMEW ofn;
    memset(&ofn, 0, sizeof(ofn));
    ofn.lStructSize = sizeof(ofn);
    ofn.hwndOwner = g_np.mainWnd;
    ofn.hInstance = GetModuleHandleW(NULL);
    ofn.lpstrFile = path;
    ofn.nMaxFile = NP_MAX_PATH;
    /* Display names are strings 20/21 (+44): "Text Documents (*.txt)",
     * "All Files (*.*)". Written as one literal (adjacent concat needs
     * literals, so the S_FILTER_* constants document the pieces). */
    ofn.lpstrFilter = L"Text Documents (*.txt)\0*.txt\0All Files (*.*)\0*.*\0";
    ofn.nFilterIndex = 1;
    ofn.lpstrTitle = S_OPEN_TITLE;
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_HIDEREADONLY | OFN_ENABLEHOOK |
                OFN_ENABLETEMPLATE;
    ofn.lpfnHook = np_ofn_hook;
    ofn.lpTemplateName = MAKEINTRESOURCEW(DLG_ENCHOOK);
    g_np.ofnMode = 1;
    g_np.forceEnc = -1;
    g_np.saveDlgOpen = 1;
    int ok = GetOpenFileNameW(&ofn);
    g_np.saveDlgOpen = 0;
    g_np.ofnMode = 0;
    if(!ok) return 0;
    /* Open into a fresh tab (a pristine blank slot is reused in place). */
    if(g_np.path[0] || g_np.dirty ||
       GetWindowTextLengthW(g_np.editBox) != 0)
        np_tab_new();
    return np_load_file(path, g_np.forceEnc, g_np.forceBom, 0);
}

static int np_file_save(void)
{
    if(g_np.path[0]) return np_save_file(g_np.path);
    return np_file_save_as();
}

static int np_file_save_as(void)
{
    wchar_t path[NP_MAX_PATH] = {0};
    if(g_np.path[0]) wcsncpy_s(path, NP_MAX_PATH, g_np.path, _TRUNCATE);
    OPENFILENAMEW ofn;
    memset(&ofn, 0, sizeof(ofn));
    ofn.lStructSize = sizeof(ofn);
    ofn.hwndOwner = g_np.mainWnd;
    ofn.hInstance = GetModuleHandleW(NULL);
    ofn.lpstrFile = path;
    ofn.nMaxFile = NP_MAX_PATH;
    ofn.lpstrFilter = L"Text Documents (*.txt)\0*.txt\0All Files (*.*)\0*.*\0";
    ofn.lpstrDefExt = L"txt";
    ofn.lpstrTitle = S_SAVEAS_TITLE;
    ofn.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_ENABLEHOOK |
                OFN_ENABLETEMPLATE;
    ofn.lpfnHook = np_ofn_hook;
    ofn.lpTemplateName = MAKEINTRESOURCEW(DLG_ENCHOOK);
    g_np.ofnMode = 2;
    g_np.saveEnc = (int)g_np.encoding;
    g_np.saveBom = g_np.bomPresent;
    g_np.saveDlgOpen = 1;
    int ok = GetSaveFileNameW(&ofn);
    g_np.saveDlgOpen = 0;
    g_np.ofnMode = 0;
    if(!ok) return 0;
    g_np.useSaveChoice = 1;
    int done = np_save_file(path);
    g_np.useSaveChoice = 0;
    return done;
}

/* Generic file error in plain words (covers the bad-name/disk cases). */
static void np_file_error(const wchar_t *path)
{
    wchar_t reason[256];
    DWORD err = GetLastError();
    if(!FormatMessageW(FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
                       NULL, err, 0, reason, 256, NULL))
        wcscpy_s(reason, 256, S_BAD_NAME);
    wchar_t *nl = wcschr(reason, L'\r');
    if(nl) *nl = L'\0';
    nl = wcschr(reason, L'\n');
    if(nl) *nl = L'\0';
    wchar_t msg[NP_MSG_MAX];
    WCHAR *full = np_format1(S_CANT_OPEN, path);
    if(full){
        swprintf_s(msg, NP_MSG_MAX, L"%s\n%s", full, reason);
        LocalFree(full);
    }else{
        swprintf_s(msg, NP_MSG_MAX, L"%s\n%s", path, reason);
    }
    MessageBoxW(g_np.mainWnd, msg, S_APP, MB_OKCANCEL | MB_ICONWARNING);
}

static const wchar_t *np_encoding_name(NP_ENCODING enc)
{
    switch(enc){
    case ENC_UTF8: return L"UTF-8";        /* 35 */
    case ENC_UTF16LE: return L"UTF-16 LE"; /* 33 */
    case ENC_UTF16BE: return L"UTF-16 BE"; /* 34 */
    default: return L"ANSI";               /* 32 */
    }
}

static const wchar_t *np_eol_name(NP_EOL eol)
{
    switch(eol){
    case EOL_LF: return L" Unix (LF)";         /* 52 */
    case EOL_CR: return L" Macintosh (CR)";    /* 53 */
    default: return L" Windows (CRLF)";        /* 51 */
    }
}

/* Detect EOL style and normalize everything to CRLF for the EDIT control.
 * Returns a fresh buffer the caller must free. */
static WCHAR *np_normalize_eol(const WCHAR *text, NP_EOL *detected)
{
    size_t len = wcslen(text);
    size_t crlf = 0, lf = 0, cr = 0;
    for(size_t i = 0; i < len; i++){
        if(text[i] == L'\r'){
            if(i + 1 < len && text[i + 1] == L'\n'){ crlf++; i++; }
            else cr++;
        }else if(text[i] == L'\n'){
            lf++;
        }
    }
    if(detected){
        if(lf > 0 && crlf == 0 && cr == 0) *detected = EOL_LF;
        else if(cr > 0 && crlf == 0 && lf == 0) *detected = EOL_CR;
        else *detected = EOL_CRLF;
    }
    WCHAR *out = malloc((len * 2 + 2) * sizeof(WCHAR));
    if(!out) return NULL;
    size_t o = 0;
    for(size_t i = 0; i < len; i++){
        if(text[i] == L'\r'){
            if(i + 1 < len && text[i + 1] == L'\n') i++;
            out[o++] = L'\r'; out[o++] = L'\n';
        }else if(text[i] == L'\n'){
            out[o++] = L'\r'; out[o++] = L'\n';
        }else{
            out[o++] = text[i];
        }
    }
    out[o] = L'\0';
    return out;
}

/* Convert CRLF back to the document's EOL style before writing to disk. */
static WCHAR *np_denormalize_eol(const WCHAR *text, NP_EOL mode)
{
    if(mode == EOL_CRLF){
        size_t len = wcslen(text) + 1;
        WCHAR *out = malloc(len * sizeof(WCHAR));
        if(out) memcpy(out, text, len * sizeof(WCHAR));
        return out;
    }
    WCHAR lone = (mode == EOL_LF) ? L'\n' : L'\r';
    size_t len = wcslen(text);
    WCHAR *out = malloc((len + 1) * sizeof(WCHAR));
    if(!out) return NULL;
    size_t o = 0;
    for(size_t i = 0; i < len; i++){
        if(text[i] == L'\r' && i + 1 < len && text[i + 1] == L'\n'){ out[o++] = lone; i++; }
        else out[o++] = text[i];
    }
    out[o] = L'\0';
    return out;
}

/* Encoding strip hooked into the Open/Save dialogs (genuine dialog 15).
 * Save offers the five encodings (default: current document encoding);
 * Open adds Auto-Detect on top. The choice lands in g_np.saveEnc/saveBom
 * (Save As) or g_np.forceEnc/forceBom (Open) on CDN_FILEOK. */
static void np_ofn_fill_encodings(HWND dlg, int forSave)
{
    HWND combo = GetDlgItem(dlg, CTL_ENC_COMBO);
    if(!combo) return;
    SendMessageW(combo, CB_RESETCONTENT, 0, 0);
    int def = 0;
    if(!forSave){
        SendMessageW(combo, CB_ADDSTRING, 0, (LPARAM)S_ENC_AUTO);
        def = 0;
    }
    SendMessageW(combo, CB_ADDSTRING, 0, (LPARAM)L"ANSI");
    SendMessageW(combo, CB_ADDSTRING, 0, (LPARAM)L"UTF-16 LE");
    SendMessageW(combo, CB_ADDSTRING, 0, (LPARAM)L"UTF-16 BE");
    SendMessageW(combo, CB_ADDSTRING, 0, (LPARAM)L"UTF-8");
    SendMessageW(combo, CB_ADDSTRING, 0, (LPARAM)S_ENC_UTF8BOM);
    if(forSave){
        switch(g_np.encoding){
        case ENC_UTF16LE: def = 1; break;
        case ENC_UTF16BE: def = 2; break;
        case ENC_UTF8: def = g_np.bomPresent ? 4 : 3; break;
        default: def = 0; break;
        }
    }
    SendMessageW(combo, CB_SETCURSEL, def, 0);
}

static UINT_PTR CALLBACK np_ofn_hook(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam)
{
    (void)wParam;
    if(msg == WM_INITDIALOG){
        np_dark_dlg_init(dlg);
        np_ofn_fill_encodings(dlg, g_np.ofnMode == 2);
        return TRUE;
    }
    if(msg == WM_CTLCOLOREDIT || msg == WM_CTLCOLORSTATIC ||
       msg == WM_CTLCOLORLISTBOX || msg == WM_CTLCOLORBTN){
        LRESULT br = np_dark_ctl((HDC)wParam, (HWND)lParam);
        if(br){
            SetWindowLongPtrW(dlg, DWLP_MSGRESULT, br);
            return 1;
        }
        return 0;
    }
    if(msg == WM_NOTIFY){
        OFNOTIFYW *notify = (OFNOTIFYW *)lParam;
        if(notify->hdr.code == CDN_FILEOK){
            HWND combo = GetDlgItem(dlg, CTL_ENC_COMBO);
            int sel = combo ? (int)SendMessageW(combo, CB_GETCURSEL, 0, 0) : 0;
            if(g_np.ofnMode == 2){
                static const NP_ENCODING map[5] =
                    { ENC_ANSI, ENC_UTF16LE, ENC_UTF16BE, ENC_UTF8, ENC_UTF8 };
                static const int bom[5] = { 0, 1, 1, 0, 1 };
                if(sel < 0 || sel > 4) sel = 0;
                g_np.saveEnc = (int)map[sel];
                g_np.saveBom = bom[sel];
            }else{
                if(sel <= 0){ g_np.forceEnc = -1; g_np.forceBom = 0; }
                else{
                    static const NP_ENCODING map[5] =
                        { ENC_ANSI, ENC_UTF16LE, ENC_UTF16BE, ENC_UTF8, ENC_UTF8 };
                    static const int bom[5] = { 0, 1, 1, 0, 1 };
                    if(sel - 1 < 0 || sel - 1 > 4){ g_np.forceEnc = -1; g_np.forceBom = 0; }
                    else{
                        g_np.forceEnc = (int)map[sel - 1];
                        g_np.forceBom = bom[sel - 1];
                    }
                }
            }
            SetWindowLongPtrW(dlg, DWLP_MSGRESULT, 0);
            return 1;
        }
        if(notify->hdr.code == CDN_HELP){
            ShellExecuteW(NULL, L"open", NP_HELP_URL, NULL, NULL, SW_SHOWNORMAL);
            return 1;
        }
    }
    return 0;
}

/* Unicode-loss guard (genuine string 27): saving Unicode text as ANSI. */
static int np_check_unicode_loss(const WCHAR *text)
{
    int len = (int)wcslen(text);
    if(len == 0) return 1;
    BOOL usedDefault = FALSE;
    WideCharToMultiByte(CP_ACP, 0, text, len, NULL, 0, NULL, &usedDefault);
    if(!usedDefault) return 1;
    WCHAR *msg = np_format1(S_UNICODE_LOSS,
                            g_np.path[0] ? g_np.path : S_UNTITLED);
    int answer = IDYES;
    if(msg){
        answer = MessageBoxW(g_np.mainWnd, msg, S_APP, MB_YESNO | MB_ICONWARNING);
        LocalFree(msg);
    }
    return answer == IDYES;
}

/* Read a whole file: BOM detection (UTF-16 LE/BE, UTF-8) first, forced
 * encoding from the Open dialog second, then UTF-8 validation, then ANSI.
 * Missing command-line/drop files offer creation (genuine string 2). */
static int np_load_file(const wchar_t *path, int forceEnc, int forceBom, int askCreate)
{
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL,
                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if(h == INVALID_HANDLE_VALUE){
        if(askCreate && GetLastError() == ERROR_FILE_NOT_FOUND){
            WCHAR *msg = np_format1(S_NOTFOUND_NEW, path);
            int answer = IDNO;
            if(msg){
                answer = MessageBoxW(g_np.mainWnd, msg, S_APP,
                                     MB_YESNOCANCEL | MB_ICONWARNING);
                LocalFree(msg);
            }
            if(answer == IDYES){
                wcsncpy_s(g_np.path, NP_MAX_PATH, path, _TRUNCATE);
                SetWindowTextW(g_np.editBox, L"");
                g_np.encoding = ENC_ANSI;
                g_np.bomPresent = 0;
                g_np.eolMode = EOL_CRLF;
                np_set_dirty(0);
                np_refresh_title();
                np_update_status();
                return 1;
            }
            return 0;
        }
        np_file_error(path);
        return 0;
    }
    DWORD size = GetFileSize(h, NULL);
    /* Absurdly large files: refuse with the genuine message (string 8). */
    MEMORYSTATUSEX mem;
    mem.dwLength = sizeof(mem);
    if(GlobalMemoryStatusEx(&mem) &&
       ((unsigned long long)size > mem.ullAvailPhys / 2 ||
        (unsigned long long)size > 1024ULL * 1024ULL * 1024ULL)){
        CloseHandle(h);
        WCHAR *msg = np_format1(S_TOO_LARGE, path);
        if(msg){
            MessageBoxW(g_np.mainWnd, msg, S_APP, MB_OK | MB_ICONSTOP);
            LocalFree(msg);
        }
        return 0;
    }
    BYTE *raw = malloc(size + 4);
    if(!raw){ CloseHandle(h); return 0; }
    DWORD read = 0;
    int ok = ReadFile(h, raw, size, &read, NULL);
    CloseHandle(h);
    if(!ok){ free(raw); np_file_error(path); return 0; }

    WCHAR *text = NULL;
    DWORD nChars = 0;
    g_np.bomPresent = 0;
    if(forceEnc == ENC_UTF16LE || forceEnc == ENC_UTF16BE){
        g_np.encoding = (NP_ENCODING)forceEnc;
        g_np.bomPresent = forceBom;
        DWORD skip = (read >= 2 && ((raw[0] == 0xFF && raw[1] == 0xFE) ||
                     (raw[0] == 0xFE && raw[1] == 0xFF))) ? 2 : 0;
        nChars = (read - skip) / 2;
        text = malloc(((size_t)nChars + 1) * sizeof(WCHAR));
        if(text){
            if(forceEnc == ENC_UTF16LE) memcpy(text, raw + skip, nChars * sizeof(WCHAR));
            else for(DWORD i = 0; i < nChars; i++)
                text[i] = (WCHAR)((raw[skip + i * 2] << 8) | raw[skip + i * 2 + 1]);
        }
    }else if(forceEnc == ENC_UTF8){
        g_np.encoding = ENC_UTF8;
        g_np.bomPresent = forceBom;
        DWORD skip = (read >= 3 && raw[0] == 0xEF && raw[1] == 0xBB && raw[2] == 0xBF) ? 3 : 0;
        int need = MultiByteToWideChar(CP_UTF8, 0, (char *)raw + skip, read - skip, NULL, 0);
        text = malloc(((size_t)need + 1) * sizeof(WCHAR));
        if(text) nChars = MultiByteToWideChar(CP_UTF8, 0, (char *)raw + skip,
                                              read - skip, text, need);
    }else if(forceEnc == ENC_ANSI){
        g_np.encoding = ENC_ANSI;
        g_np.bomPresent = 0;
        int need = MultiByteToWideChar(CP_ACP, 0, (char *)raw, read, NULL, 0);
        text = malloc(((size_t)need + 1) * sizeof(WCHAR));
        if(text) nChars = MultiByteToWideChar(CP_ACP, 0, (char *)raw, read, text, need);
    }else if(read >= 2 && raw[0] == 0xFF && raw[1] == 0xFE){
        g_np.encoding = ENC_UTF16LE; g_np.bomPresent = 1;
        nChars = (read - 2) / 2;
        text = malloc(((size_t)nChars + 1) * sizeof(WCHAR));
        if(text) memcpy(text, raw + 2, nChars * sizeof(WCHAR));
    }else if(read >= 2 && raw[0] == 0xFE && raw[1] == 0xFF){
        g_np.encoding = ENC_UTF16BE; g_np.bomPresent = 1;
        nChars = (read - 2) / 2;
        text = malloc(((size_t)nChars + 1) * sizeof(WCHAR));
        for(DWORD i = 0; text && i < nChars; i++)
            text[i] = (WCHAR)((raw[2 + i * 2] << 8) | raw[2 + i * 2 + 1]);
    }else if(read >= 3 && raw[0] == 0xEF && raw[1] == 0xBB && raw[2] == 0xBF){
        g_np.encoding = ENC_UTF8; g_np.bomPresent = 1;
        int need = MultiByteToWideChar(CP_UTF8, 0, (char *)raw + 3, read - 3, NULL, 0);
        text = malloc(((size_t)need + 1) * sizeof(WCHAR));
        if(text) nChars = MultiByteToWideChar(CP_UTF8, 0, (char *)raw + 3,
                                              read - 3, text, need);
    }else{
        int need = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS,
                                       (char *)raw, read, NULL, 0);
        UINT cp = (need > 0) ? CP_UTF8 : CP_ACP;
        g_np.encoding = (cp == CP_UTF8) ? ENC_UTF8 : ENC_ANSI;
        g_np.bomPresent = 0;
        need = MultiByteToWideChar(cp, 0, (char *)raw, read, NULL, 0);
        text = malloc(((size_t)need + 1) * sizeof(WCHAR));
        if(text) nChars = MultiByteToWideChar(cp, 0, (char *)raw, read, text, need);
    }
    free(raw);
    if(!text) return 0;
    text[nChars] = L'\0';

    WCHAR *shown = np_normalize_eol(text, &g_np.eolMode);
    free(text);
    if(!shown) return 0;
    SetWindowTextW(g_np.editBox, shown);
    free(shown);

    wcsncpy_s(g_np.path, NP_MAX_PATH, path, _TRUNCATE);
    np_set_dirty(0);
    np_refresh_title();
    np_update_status();
    np_recent_add(path);
    SHAddToRecentDocs(SHARD_PATHW, path);
    np_debug(L"loaded %s as %s/%s", path,
             np_encoding_name(g_np.encoding), np_eol_name(g_np.eolMode));
    np_largefile_check(size);
    np_fire_event(NP_EVENT_OPEN);
    return 1;
}

/* Write back in the Save As encoding choice (or the document's own for
 * plain Save), with the document's EOL style and Unicode-loss guard. */
static int np_save_file(const wchar_t *path)
{
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *text = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!text) return 0;
    GetWindowTextW(g_np.editBox, text, len + 1);

    NP_ENCODING enc = g_np.encoding;
    int useBom = g_np.bomPresent;
    if(g_np.useSaveChoice){
        /* A Save As choice overrides the document encoding. */
        enc = (NP_ENCODING)g_np.saveEnc; useBom = g_np.saveBom;
    }else if(!g_np.path[0]){
        int asciiOnly = 1;
        for(int i = 0; i < len; i++) if(text[i] > 127){ asciiOnly = 0; break; }
        if(asciiOnly){ enc = ENC_ANSI; useBom = 0; }
    }
    if(enc == ENC_ANSI && !np_check_unicode_loss(text)){ free(text); return 0; }

    WCHAR *toWrite = np_denormalize_eol(text, g_np.eolMode);
    free(text);
    if(!toWrite) return 0;
    len = (int)wcslen(toWrite);

    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                           FILE_ATTRIBUTE_NORMAL, NULL);
    if(h == INVALID_HANDLE_VALUE){ free(toWrite); np_file_error(path); return 0; }
    DWORD written = 0;
    int ok = 1;
    if(enc == ENC_UTF16LE || enc == ENC_UTF16BE){
        BYTE bom[2];
        bom[0] = (enc == ENC_UTF16LE) ? 0xFF : 0xFE;
        bom[1] = (enc == ENC_UTF16LE) ? 0xFE : 0xFF;
        ok = WriteFile(h, bom, 2, &written, NULL);
        if(ok && enc == ENC_UTF16LE){
            ok = WriteFile(h, toWrite, (DWORD)len * 2, &written, NULL);
        }else if(ok){
            for(int i = 0; ok && i < len; i++){
                BYTE pair[2];
                pair[0] = (BYTE)(toWrite[i] >> 8);
                pair[1] = (BYTE)(toWrite[i] & 0xFF);
                ok = WriteFile(h, pair, 2, &written, NULL);
            }
        }
    }else if(enc == ENC_UTF8){
        if(useBom){
            static const BYTE utf8bom[3] = { 0xEF, 0xBB, 0xBF };
            ok = WriteFile(h, utf8bom, 3, &written, NULL);
        }
        if(ok){
            int need = WideCharToMultiByte(CP_UTF8, 0, toWrite, len, NULL, 0, NULL, NULL);
            char *mb = malloc((size_t)need + 1);
            if(!mb) ok = 0;
            else{
                WideCharToMultiByte(CP_UTF8, 0, toWrite, len, mb, need, NULL, NULL);
                ok = WriteFile(h, mb, need, &written, NULL);
                free(mb);
            }
        }
    }else{
        int need = WideCharToMultiByte(CP_ACP, 0, toWrite, len, NULL, 0, NULL, NULL);
        char *mb = malloc((size_t)need + 1);
        if(!mb) ok = 0;
        else{
            WideCharToMultiByte(CP_ACP, 0, toWrite, len, mb, need, NULL, NULL);
            ok = WriteFile(h, mb, need, &written, NULL);
            free(mb);
        }
    }
    CloseHandle(h);
    free(toWrite);
    if(!ok){ np_file_error(path); return 0; }

    g_np.encoding = enc;
    g_np.bomPresent = useBom;
    wcsncpy_s(g_np.path, NP_MAX_PATH, path, _TRUNCATE);
    np_set_dirty(0);
    np_refresh_title();
    np_update_status();
    np_autosave_clear_tab(g_np.tabActive);
    np_recent_add(path);
    np_history_write();
    np_fire_event(NP_EVENT_SAVE);
    SHAddToRecentDocs(SHARD_PATHW, path);
    return 1;
}

/* ==========================================================================
 *  Edit features
 * ========================================================================== */

/* F5 -- insert "2:41 PM 9/5/2026" at the caret, locale formatted.
 * EM_REPLACESEL keeps Undo working, exactly like the original. */
static void np_insert_time_date(void)
{
    SYSTEMTIME st;
    GetLocalTime(&st);
    wchar_t date[80], time[80], out[200];
    GetDateFormatW(LOCALE_USER_DEFAULT, 0, &st, NULL, date, 80);
    GetTimeFormatW(LOCALE_USER_DEFAULT, 0, &st, NULL, time, 80);
    swprintf_s(out, 200, L"%s %s", time, date);
    SendMessageW(g_np.editBox, EM_REPLACESEL, (WPARAM)TRUE, (LPARAM)out);
    SetFocus(g_np.editBox);
}

/* Open the genuine Find (1540) / Replace (1541) modeless dialogs. */
static void np_find_or_replace(int replace)
{
    if(g_np.findDlg){ DestroyWindow(g_np.findDlg); g_np.findDlg = NULL; }
    g_np.findIsReplace = replace;
    g_np.findDlg = CreateDialogParamW(GetModuleHandleW(NULL),
        MAKEINTRESOURCEW(replace ? DLG_REPLACE : DLG_FIND),
        g_np.mainWnd, replace ? np_replace_proc : np_find_proc, 0);
    if(g_np.findDlg) ShowWindow(g_np.findDlg, SW_SHOW);
}

/* Sync dialog controls from state (and seed the boxes with live text). */
static void np_find_dlg_init(HWND dlg, int replace)
{
    (void)replace;
    if(g_findWhat[0]) SetDlgItemTextW(dlg, CTL_FIND_EDIT, g_findWhat);
    if(g_replaceWith[0]) SetDlgItemTextW(dlg, CTL_REPLACE_EDIT, g_replaceWith);
    CheckDlgButton(dlg, CTL_WHOLEWORD, g_np.wholeWord ? BST_CHECKED : BST_UNCHECKED);
    CheckDlgButton(dlg, CTL_MATCHCASE, g_np.matchCase ? BST_CHECKED : BST_UNCHECKED);
    CheckDlgButton(dlg, CTL_WRAPAROUND, g_np.wrapAround ? BST_CHECKED : BST_UNCHECKED);
    CheckDlgButton(dlg, CTL_REGEX, g_np.findRegex ? BST_CHECKED : BST_UNCHECKED);
    CheckRadioButton(dlg, CTL_DIR_UP, CTL_DIR_DOWN,
                     g_np.findDown ? CTL_DIR_DOWN : CTL_DIR_UP);
    /* Pre-fill "Find what" with the current selection, like the original. */
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    if(s != e && e - s < NP_FIND_MAX){
        int len = GetWindowTextLengthW(g_np.editBox);
        WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
        if(buf){
            GetWindowTextW(g_np.editBox, buf, len + 1);
            WCHAR sel[NP_FIND_MAX];
            memcpy(sel, buf + s, (e - s) * sizeof(WCHAR));
            sel[e - s] = L'\0';
            free(buf);
            if(!wcschr(sel, L'\r') && !wcschr(sel, L'\n')){
                SetDlgItemTextW(dlg, CTL_FIND_EDIT, sel);
                GetDlgItemTextW(dlg, CTL_FIND_EDIT, g_findWhat, NP_FIND_MAX);
            }
        }
    }
    SendDlgItemMessageW(dlg, CTL_FIND_EDIT, EM_SETSEL, 0, (LPARAM)-1);
    np_find_count_show(dlg);
}

/* Read live control state back into globals. */
static void np_find_dlg_read(HWND dlg)
{
    GetDlgItemTextW(dlg, CTL_FIND_EDIT, g_findWhat, NP_FIND_MAX);
    if(g_np.findIsReplace)
        GetDlgItemTextW(dlg, CTL_REPLACE_EDIT, g_replaceWith, NP_FIND_MAX);
    g_np.wholeWord = IsDlgButtonChecked(dlg, CTL_WHOLEWORD) == BST_CHECKED;
    g_np.matchCase = IsDlgButtonChecked(dlg, CTL_MATCHCASE) == BST_CHECKED;
    g_np.wrapAround = IsDlgButtonChecked(dlg, CTL_WRAPAROUND) == BST_CHECKED;
    g_np.findRegex = IsDlgButtonChecked(dlg, CTL_REGEX) == BST_CHECKED;
    g_np.findDown = IsDlgButtonChecked(dlg, CTL_DIR_DOWN) == BST_CHECKED;
}

/* Keep the right-hand buttons glued to the edge and stretch the text boxes
 * when the user resizes the Find/Replace windows. */
static void np_find_layout(HWND dlg, int replace)
{
    RECT rc;
    GetClientRect(dlg, &rc);
    int right = rc.right;
    int btnIds[] = { IDOK, CTL_REPLACE_BTN, CTL_REPLACE_ALL,
                     IDCANCEL, CTL_FIND_HELP };
    int nBtns = replace ? 5 : 3;
    for(int i = 0; i < nBtns; i++){
        HWND b = GetDlgItem(dlg, btnIds[i]);
        if(!b) continue;
        RECT br;
        GetWindowRect(b, &br);
        MapWindowPoints(NULL, dlg, (POINT *)&br, 2);
        int w = br.right - br.left, h = br.bottom - br.top;
        MoveWindow(b, right - 4 - w, br.top, w, h, TRUE);
    }
    HWND fe = GetDlgItem(dlg, CTL_FIND_EDIT);
    if(fe){
        RECT er;
        GetWindowRect(fe, &er);
        MapWindowPoints(NULL, dlg, (POINT *)&er, 2);
        MoveWindow(fe, er.left, er.top,
                   right - 4 - 54 - er.left, er.bottom - er.top, TRUE);
    }
    if(replace){
        HWND re = GetDlgItem(dlg, CTL_REPLACE_EDIT);
        if(re){
            RECT er;
            GetWindowRect(re, &er);
            MapWindowPoints(NULL, dlg, (POINT *)&er, 2);
            MoveWindow(re, er.left, er.top,
                       right - 4 - 54 - er.left, er.bottom - er.top, TRUE);
        }
    }
}

static INT_PTR CALLBACK np_find_proc(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam)
{
    (void)lParam;
    switch(msg){
    case WM_INITDIALOG:
        np_find_dlg_init(dlg, 0);
        np_dark_dlg_init(dlg);
        return TRUE;
    case WM_SIZE:
        np_find_layout(dlg, 0);
        return TRUE;
    case WM_GETMINMAXINFO: {
        LPMINMAXINFO mmi = (LPMINMAXINFO)lParam;
        mmi->ptMinTrackSize.x = 250;
        mmi->ptMinTrackSize.y = 130;
        return TRUE;
    }
    case WM_CTLCOLORDLG: {
        LRESULT br = np_dark_ctl((HDC)wParam, dlg);
        if(br) return br;
        break;
    }
    case WM_COMMAND:
        switch(LOWORD(wParam)){
        case IDOK:
            np_find_dlg_read(dlg);
            np_find_count_show(dlg);
            np_do_find(1);
            return TRUE;
        case IDCANCEL:
            DestroyWindow(dlg);
            g_np.findDlg = NULL;
            return TRUE;
        case CTL_FIND_HELP:
            ShellExecuteW(NULL, L"open", NP_HELP_URL, NULL, NULL, SW_SHOWNORMAL);
            return TRUE;
        }
        if(HIWORD(wParam) == EN_CHANGE && LOWORD(wParam) == CTL_FIND_EDIT){
            GetDlgItemTextW(dlg, CTL_FIND_EDIT, g_findWhat, NP_FIND_MAX);
            np_find_count_show(dlg);
        }
        if(HIWORD(wParam) == BN_CLICKED &&
           (LOWORD(wParam) == CTL_REGEX || LOWORD(wParam) == CTL_WHOLEWORD ||
            LOWORD(wParam) == CTL_MATCHCASE)){
            np_find_dlg_read(dlg);
            np_find_count_show(dlg);
        }
        break;
    case WM_CLOSE:
        DestroyWindow(dlg);
        g_np.findDlg = NULL;
        return TRUE;
    }
    return FALSE;
}

static INT_PTR CALLBACK np_replace_proc(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam)
{
    (void)lParam;
    switch(msg){
    case WM_CTLCOLORDLG: {
        LRESULT br = np_dark_ctl((HDC)wParam, dlg);
        if(br) return br;
        break;
    }
    case WM_INITDIALOG:
        np_find_dlg_init(dlg, 1);
        np_dark_dlg_init(dlg);
        return TRUE;
    case WM_SIZE:
        np_find_layout(dlg, 1);
        return TRUE;
    case WM_GETMINMAXINFO: {
        LPMINMAXINFO mmi = (LPMINMAXINFO)lParam;
        mmi->ptMinTrackSize.x = 244;
        mmi->ptMinTrackSize.y = 150;
        return TRUE;
    }
    case WM_COMMAND:
        switch(LOWORD(wParam)){
        case IDOK:
            np_find_dlg_read(dlg);
            np_find_count_show(dlg);
            np_do_find(1);
            return TRUE;
        case CTL_REPLACE_BTN:
            np_find_dlg_read(dlg);
            np_find_count_show(dlg);
            np_do_replace();
            return TRUE;
        case CTL_REPLACE_ALL:
            np_find_dlg_read(dlg);
            np_find_count_show(dlg);
            np_do_replace_all();
            return TRUE;
        case IDCANCEL:
            DestroyWindow(dlg);
            g_np.findDlg = NULL;
            return TRUE;
        case CTL_FIND_HELP:
            ShellExecuteW(NULL, L"open", NP_HELP_URL, NULL, NULL, SW_SHOWNORMAL);
            return TRUE;
        }
        if(HIWORD(wParam) == EN_CHANGE &&
           (LOWORD(wParam) == CTL_FIND_EDIT || LOWORD(wParam) == CTL_REPLACE_EDIT)){
            np_find_dlg_read(dlg);
            np_find_count_show(dlg);
        }
        if(HIWORD(wParam) == BN_CLICKED &&
           (LOWORD(wParam) == CTL_REGEX || LOWORD(wParam) == CTL_WHOLEWORD ||
            LOWORD(wParam) == CTL_MATCHCASE)){
            np_find_dlg_read(dlg);
            np_find_count_show(dlg);
        }
        break;
    case WM_CLOSE:
        DestroyWindow(dlg);
        g_np.findDlg = NULL;
        return TRUE;
    }
    return FALSE;
}

/* Does the needle match at text[pos]? Honors case and whole-word flags. */
static int np_match_at(const WCHAR *text, int pos)
{
    size_t m = wcslen(g_findWhat);
    int same = g_np.matchCase ? (wcsncmp(text + pos, g_findWhat, m) == 0)
                              : (_wcsnicmp(text + pos, g_findWhat, m) == 0);
    if(!same) return 0;
    if(g_np.wholeWord){
        if(pos > 0){
            WCHAR before = text[pos - 1];
            if(iswalnum(before) || before == L'_') return 0;
        }
        WCHAR after = text[pos + m];
        if(after && (iswalnum(after) || after == L'_')) return 0;
    }
    return 1;
}

/* Search for the needle; down=1 forward from `from`, else backward.
 * Returns the match offset or -1. */
static int np_find_pos(const WCHAR *text, int textLen, int from, int down)
{
    size_t m = wcslen(g_findWhat);
    if(m == 0 || textLen < (int)m) return -1;
    if(down){
        if(from < 0) from = 0;
        for(int i = from; i + (int)m <= textLen; i++)
            if(np_match_at(text, i)) return i;
    }else{
        int start = from - (int)m;
        if(start > textLen - (int)m) start = textLen - (int)m;
        for(int i = start; i >= 0; i--)
            if(np_match_at(text, i)) return i;
    }
    return -1;
}

/* Select a match and scroll it into view. Focus change is the caller's job. */
static void np_select_match(int pos, int len, int focus)
{
    SendMessageW(g_np.editBox, EM_SETSEL, pos, pos + len);
    SendMessageW(g_np.editBox, EM_SCROLLCARET, 0, 0);
    if(focus) SetFocus(g_np.editBox);
}

/* Screen-reader announcement plus a transient status-bar message. */
static void np_announce(const wchar_t *msg)
{
    wcsncpy_s(g_np.statusMsg, 256, msg, _TRUNCATE);
    g_np.statusMsgActive = 1;
    np_update_status();
    if(g_np.statusBar)
        NotifyWinEvent(EVENT_OBJECT_LIVEREGIONCHANGED, g_np.statusBar,
                       OBJID_CLIENT, 0);
    SetTimer(g_np.mainWnd, IDT_STATUSMSG, NP_STATUS_MSG_MS, NULL);
    np_debug(L"announce: %s", msg);
}

/* F3 / Find Next: search from the caret with direction, case, whole-word
 * and wrap-around. Announces results with the genuine strings. */
static int np_do_find(int fromDlg)
{
    if(!g_findWhat[0]){ np_find_or_replace(0); return 0; }
    DWORD selS = 0, selE = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&selS, (LPARAM)&selE);
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return 0;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    int from = g_np.findDown ? (int)selE : (int)selS;
    int found = -1, matchLen = 0, wrapped = 0;
    int groups[NP_RX_MAXGROUPS][2];
    if(g_np.findRegex){
        found = np_regex_find_pos(buf, len, from, g_np.findDown,
                                  &matchLen, groups);
        if(found == -2){
            free(buf);
            MessageBoxW(fromDlg && g_np.findDlg ? g_np.findDlg : g_np.mainWnd,
                        L"Regular expression too complex.", S_APP, MB_OK);
            return 0;
        }
        if(found < 0 && g_np.wrapAround){
            found = np_regex_find_pos(buf, len, g_np.findDown ? 0 : len,
                                      g_np.findDown, &matchLen, groups);
            wrapped = (found >= 0);
        }
        if(found >= 0 && g_np.wholeWord){
            WCHAR before = (found > 0) ? buf[found - 1] : 0;
            WCHAR after = buf[found + matchLen];
            if((before && (iswalnum(before) || before == L'_')) ||
               (after && (iswalnum(after) || after == L'_')))
                found = -1;
        }
    }else{
        found = np_find_pos(buf, len, from, g_np.findDown);
        matchLen = (int)wcslen(g_findWhat);
        if(found < 0 && g_np.wrapAround){
            found = np_find_pos(buf, len, g_np.findDown ? 0 : len, g_np.findDown);
            wrapped = (found >= 0);
        }
    }
    free(buf);
    if(found < 0){
        WCHAR *msg = np_format1(S_CANT_FIND, g_findWhat);
        if(msg){
            MessageBoxW(fromDlg && g_np.findDlg ? g_np.findDlg : g_np.mainWnd,
                        msg, S_APP, MB_OK);
            LocalFree(msg);
        }
        np_announce(S_NOT_FOUND);
        return 0;
    }
    np_select_match(found, matchLen, !fromDlg);
    if(wrapped) np_announce(g_np.findDown ? S_WRAP_TOP : S_WRAP_BOTTOM);
    else np_announce(S_FOUND_NEXT);
    return 1;
}

/* Replace button: only if the selection really is a match, then find next.
 * Regex mode expands $1..$9 in the replacement. */
static int np_do_replace(void)
{
    if(!g_findWhat[0]){ np_find_or_replace(1); return 0; }
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int replaced = 0;
    if(s != e){
        int len = GetWindowTextLengthW(g_np.editBox);
        WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
        if(buf){
            GetWindowTextW(g_np.editBox, buf, len + 1);
            if(g_np.findRegex){
                int groups[NP_RX_MAXGROUPS][2];
                int steps = 0;
                int end = np_regex_attempt(g_findWhat, (int)s, buf, len,
                                           g_np.matchCase, groups,
                                           NP_RX_MAXGROUPS, &steps);
                if(end == (int)e && end > (int)s){
                    WCHAR exp[NP_FIND_MAX * 4];
                    np_regex_expand(g_replaceWith, buf, groups,
                                    NP_RX_MAXGROUPS, exp, NP_FIND_MAX * 4);
                    SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE,
                                 (LPARAM)exp);
                    replaced = 1;
                }
            }else{
                size_t m = wcslen(g_findWhat);
                if((int)(e - s) == (int)m){
                    WCHAR *sel = malloc((m + 1) * sizeof(WCHAR));
                    if(sel){
                        memcpy(sel, buf + s, m * sizeof(WCHAR));
                        sel[m] = L'\0';
                        int same = g_np.matchCase
                            ? (wcscmp(sel, g_findWhat) == 0)
                            : (_wcsicmp(sel, g_findWhat) == 0);
                        free(sel);
                        if(same){
                            SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE,
                                         (LPARAM)g_replaceWith);
                            replaced = 1;
                        }
                    }
                }
            }
            free(buf);
        }
    }
    if(replaced){
        np_announce(S_REPLACED);
        np_do_find(1);
        return 1;
    }
    np_announce(S_NOT_REPLACED);
    return np_do_find(1);
}

/* Replace All: one buffer pass over the whole document, with a count.
 * Regex mode expands $1..$9 per match and steps over zero-length hits. */
static int np_do_replace_all(void)
{
    if(!g_findWhat[0]){ np_find_or_replace(1); return 0; }
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return 0;
    GetWindowTextW(g_np.editBox, buf, len + 1);

    if(g_np.findRegex){
        int steps = 0;
        size_t cap = (size_t)len + 256;
        WCHAR *out = malloc(cap * sizeof(WCHAR));
        if(!out){ free(buf); return 0; }
        size_t o = 0;
        int count = 0, firstPos = -1, i = 0;
        int groups[NP_RX_MAXGROUPS][2];
        WCHAR exp[NP_FIND_MAX * 4];
        while(i <= len){
            int e = np_regex_attempt(g_findWhat, i, buf, len, g_np.matchCase,
                                     groups, NP_RX_MAXGROUPS, &steps);
            if(e == -2 || steps > NP_RX_STEP_LIMIT){
                free(out); free(buf);
                MessageBoxW(g_np.findDlg ? g_np.findDlg : g_np.mainWnd,
                            L"Regular expression too complex.", S_APP, MB_OK);
                return 0;
            }
            if(e < 0) break;
            if(g_np.wholeWord && e > i){
                WCHAR before = (i > 0) ? buf[i - 1] : 0;
                WCHAR after = buf[e];
                if((before && (iswalnum(before) || before == L'_')) ||
                   (after && (iswalnum(after) || after == L'_'))){
                    if(o + 1 >= cap){
                        cap *= 2;
                        WCHAR *no = realloc(out, cap * sizeof(WCHAR));
                        if(!no){ free(out); free(buf); return 0; }
                        out = no;
                    }
                    out[o++] = buf[i++];
                    continue;
                }
            }
            np_regex_expand(g_replaceWith, buf, groups, NP_RX_MAXGROUPS,
                            exp, NP_FIND_MAX * 4);
            size_t el = wcslen(exp);
            while(o + el + 2 >= cap){
                cap *= 2;
                WCHAR *no = realloc(out, cap * sizeof(WCHAR));
                if(!no){ free(out); free(buf); return 0; }
                out = no;
            }
            if(firstPos < 0) firstPos = (int)o;
            memcpy(out + o, exp, el * sizeof(WCHAR));
            o += el;
            count++;
            if(e == i){
                if(o + 1 >= cap){
                    cap *= 2;
                    WCHAR *no = realloc(out, cap * sizeof(WCHAR));
                    if(!no){ free(out); free(buf); return 0; }
                    out = no;
                }
                if(i < len) out[o++] = buf[i];
                i++;
            }else{
                i = e;
            }
            if(count > 1000000) break;
        }
        out[o] = L'\0';
        free(buf);
        if(count == 0){
            free(out);
            WCHAR *msg = np_format1(S_CANT_FIND, g_findWhat);
            if(msg){
                MessageBoxW(g_np.findDlg ? g_np.findDlg : g_np.mainWnd,
                            msg, S_APP, MB_OK);
                LocalFree(msg);
            }
            np_announce(S_NOT_FOUND);
            return 0;
        }
        SetWindowTextW(g_np.editBox, out);
        free(out);
        if(firstPos >= 0) np_select_match(firstPos, 0, 0);
        np_announce(S_REPLACED_ALL);
        return count;
    }
    size_t m = wcslen(g_findWhat);
    size_t r = wcslen(g_replaceWith);
    int count = 0;
    for(int i = 0; i + (int)m <= len; i++)
        if(np_match_at(buf, i)){ count++; i += (int)m - 1; }
    if(count == 0){
        free(buf);
        WCHAR *msg = np_format1(S_CANT_FIND, g_findWhat);
        if(msg){
            MessageBoxW(g_np.findDlg ? g_np.findDlg : g_np.mainWnd,
                        msg, S_APP, MB_OK);
            LocalFree(msg);
        }
        np_announce(S_NOT_FOUND);
        return 0;
    }
    size_t newLen = (size_t)len + (size_t)count * r - (size_t)count * m;
    WCHAR *out = malloc((newLen + 1) * sizeof(WCHAR));
    if(!out){ free(buf); return 0; }
    size_t o = 0;
    int firstPos = -1;
    for(int i = 0; i < len;){
        if(i + (int)m <= len && np_match_at(buf, i)){
            if(firstPos < 0) firstPos = (int)o;
            memcpy(out + o, g_replaceWith, r * sizeof(WCHAR));
            o += r; i += (int)m;
        }else{
            out[o++] = buf[i++];
        }
    }
    out[o] = L'\0';
    free(buf);
    SetWindowTextW(g_np.editBox, out);
    free(out);
    if(firstPos >= 0) np_select_match(firstPos, (int)r, 0);
    np_announce(S_REPLACED_ALL);
    return count;
}

/* Go To Line: the genuine dialog 14. Out-of-range shows string 31. */
static void np_goto_line(void)
{
    DialogBoxParamW(GetModuleHandleW(NULL), MAKEINTRESOURCEW(DLG_GOTO),
                    g_np.mainWnd, np_goto_proc, 0);
}

static INT_PTR CALLBACK np_goto_proc(HWND dlg, UINT msg, WPARAM wParam, LPARAM lParam)
{
    (void)lParam;
    switch(msg){
    case WM_CTLCOLORDLG: {
        LRESULT br = np_dark_ctl((HDC)wParam, dlg);
        if(br) return br;
        break;
    }
    case WM_INITDIALOG: {
        int cur = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR,
                                    (WPARAM)-1, 0) + 1;
        HWND edit = GetDlgItem(dlg, CTL_GOTO_EDIT);
        wchar_t buf[32];
        swprintf_s(buf, 32, L"%d", cur);
        SetWindowTextW(edit, buf);
        SendMessageW(edit, EM_SETSEL, 0, (LPARAM)-1);
        SetFocus(edit);
        np_dark_dlg_init(dlg);
        return FALSE;
    }
    case WM_COMMAND:
        if(LOWORD(wParam) == IDOK){
            wchar_t buf[32];
            GetDlgItemTextW(dlg, CTL_GOTO_EDIT, buf, 32);
            int want = _wtoi(buf);
            int total = (int)SendMessageW(g_np.editBox, EM_GETLINECOUNT, 0, 0);
            if(want < 1) want = 1;
            if(want > total){
                MessageBoxW(dlg, S_GOTO_RANGE, S_GOTO_CAP,
                            MB_OK | MB_ICONWARNING);
                return TRUE;
            }
            int index = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, want - 1, 0);
            SendMessageW(g_np.editBox, EM_SETSEL, index, index);
            SendMessageW(g_np.editBox, EM_SCROLLCARET, 0, 0);
            SetFocus(g_np.editBox);
            EndDialog(dlg, IDOK);
            return TRUE;
        }
        if(LOWORD(wParam) == IDCANCEL){ EndDialog(dlg, IDCANCEL); return TRUE; }
        break;
    }
    return FALSE;
}

/* ==========================================================================
 *  Format / View / Help
 * ========================================================================== */

/* Word wrap toggle: the EDIT control's wrap style is fixed at creation, so
 * (like the original) we recreate it, preserving text, font and selection. */
static void np_toggle_wrap(void)
{
    g_np.wrap = !g_np.wrap;
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *text = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(text) GetWindowTextW(g_np.editBox, text, len + 1);
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    HFONT hf = (HFONT)SendMessageW(g_np.editBox, WM_GETFONT, 0, 0);
    DestroyWindow(g_np.editBox);

    DWORD style = WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE |
                  ES_AUTOVSCROLL | ES_WANTRETURN;
    if(!g_np.wrap) style |= WS_HSCROLL | ES_AUTOHSCROLL;
    g_np.editBox = CreateWindowExW(WS_EX_CLIENTEDGE, L"Edit", L"", style,
        0, 0, 100, 100, g_np.mainWnd, (HMENU)(INT_PTR)IDC_EDITBOX,
        GetModuleHandleW(NULL), NULL);
    np_edit_wire(g_np.editBox);
    if(text){ SetWindowTextW(g_np.editBox, text); free(text); }
    if(hf) SendMessageW(g_np.editBox, WM_SETFONT, (WPARAM)hf, TRUE);
    np_apply_zoom();
    SendMessageW(g_np.editBox, EM_SETSEL, s, e);
    SetFocus(g_np.editBox);
    ShowWindow(g_np.statusBar, (!g_np.wrap && g_np.showBar) ? SW_SHOW : SW_HIDE);
    np_on_size();
    np_refresh_title();
}

static void np_pick_font(void)
{
    CHOOSEFONTW cf;
    memset(&cf, 0, sizeof(cf));
    cf.lStructSize = sizeof(cf);
    cf.hwndOwner = g_np.mainWnd;
    cf.lpLogFont = &g_np.font;
    cf.Flags = CF_SCREENFONTS | CF_EFFECTS | CF_ENABLEHOOK;
    cf.lpfnHook = np_cf_hook;
    if(g_np.haveFont) cf.Flags |= CF_INITTOLOGFONTSTRUCT;
    if(ChooseFontW(&cf)){
        g_np.font = *cf.lpLogFont;
        g_np.haveFont = 1;
        HFONT hf = CreateFontIndirectW(&g_np.font);
        if(hf) SendMessageW(g_np.editBox, WM_SETFONT, (WPARAM)hf, TRUE);
    }
}

/* Darken the system Font dialog (best effort; listviews stay native). */
static UINT_PTR CALLBACK np_cf_hook(HWND dlg, UINT msg, WPARAM wParam,
                                    LPARAM lParam)
{
    if(msg == WM_INITDIALOG){
        np_dark_dlg_init(dlg);
        return TRUE;
    }
    if(msg == WM_CTLCOLOREDIT || msg == WM_CTLCOLORSTATIC ||
       msg == WM_CTLCOLORLISTBOX || msg == WM_CTLCOLORBTN){
        LRESULT br = np_dark_ctl((HDC)wParam, (HWND)lParam);
        if(br){
            SetWindowLongPtrW(dlg, DWLP_MSGRESULT, br);
            return 1;
        }
    }
    return 0;
}

static void np_toggle_statusbar(void)
{
    if(g_np.wrap){
        MessageBoxW(g_np.mainWnd,
            L"The status bar is only available when Word Wrap is off.",
            S_APP, MB_OK);
        return;
    }
    g_np.showBar = !g_np.showBar;
    ShowWindow(g_np.statusBar, g_np.showBar ? SW_SHOW : SW_HIDE);
    np_on_size();
    np_refresh_title();
}

static void np_zoom_by(int delta)
{
    g_np.zoom += delta;
    if(g_np.zoom < NP_ZOOM_MIN) g_np.zoom = NP_ZOOM_MIN;
    if(g_np.zoom > NP_ZOOM_MAX) g_np.zoom = NP_ZOOM_MAX;
    np_apply_zoom();
    np_update_status();
}

static void np_zoom_reset(void)
{
    g_np.zoom = NP_ZOOM_DEFAULT;
    np_apply_zoom();
    np_update_status();
}

/* EM_SETZOOM takes numerator/denominator -- i.e. a percentage. */
static void np_apply_zoom(void)
{
    if(g_np.editBox)
        SendMessageW(g_np.editBox, EM_SETZOOM, (WPARAM)g_np.zoom, (LPARAM)100);
}

/* " Ln 3, Col 12 (7 selected) | 100% | Windows (CRLF) | UTF-8" -- exact
 * classic formats, plus a modern selection character count. A live
 * announcement overrides the last part briefly. */
static void np_update_status(void)
{
    np_toolbar_update();
    if(!g_np.statusBar || !IsWindowVisible(g_np.statusBar)) return;
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int line = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, e, 0);
    int lineStart = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, line, 0);
    wchar_t col0[96], col1[64], col2[64], col3[64];
    if(s != e)
        swprintf_s(col0, 96, L" Ln %d, Col %d (%d selected)",
                   line + 1, (int)(e - lineStart) + 1, (int)(e - s));
    else if(g_np.caretCount > 1)
        swprintf_s(col0, 96, L" Ln %d, Col %d (%d carets)",
                   line + 1, (int)(e - lineStart) + 1, g_np.caretCount);
    else
        swprintf_s(col0, 96, L" Ln %d, Col %d",
                   line + 1, (int)(e - lineStart) + 1);
    swprintf_s(col1, 64, L" %d%%", g_np.zoom);
    swprintf_s(col2, 64, L"%s", np_eol_name(g_np.eolMode));
    if(g_np.statusMsgActive) wcsncpy_s(col3, 64, g_np.statusMsg, _TRUNCATE);
    else if(g_np.tabs && g_np.tabActive >= 0 &&
            g_np.tabActive < g_np.tabCount &&
            g_np.tabs[g_np.tabActive].largeFile)
        swprintf_s(col3, 64, L"%s (large file)",
                   np_encoding_name(g_np.encoding));
    else swprintf_s(col3, 64, L"%s", np_encoding_name(g_np.encoding));
    wcsncpy_s(g_np.sbText[0], 96, col0, _TRUNCATE);
    wcsncpy_s(g_np.sbText[1], 96, col1, _TRUNCATE);
    wcsncpy_s(g_np.sbText[2], 96, col2, _TRUNCATE);
    wcsncpy_s(g_np.sbText[3], 96, col3, _TRUNCATE);
    int widths[4];
    widths[0] = 230; widths[1] = 290; widths[2] = 420; widths[3] = -1;
    SendMessageW(g_np.statusBar, SB_SETPARTS, 4, (LPARAM)widths);
    /* Owner-drawn parts trade the sunken 1990s boxes for theme colors. */
    SendMessageW(g_np.statusBar, SB_SETTEXTW, 0 | SBT_OWNERDRAW, 0);
    SendMessageW(g_np.statusBar, SB_SETTEXTW, 1 | SBT_OWNERDRAW, 0);
    SendMessageW(g_np.statusBar, SB_SETTEXTW, 2 | SBT_OWNERDRAW, 0);
    SendMessageW(g_np.statusBar, SB_SETTEXTW, 3 | SBT_OWNERDRAW, 0);
}

/* (np_show_about lives in the dark-everywhere section: custom dialog.) */

/* ==========================================================================
 *  Page Setup + Printing
 * ========================================================================== */

/* (PAGESETUP_CTX + g_fallbackPapers live above, next to Global State.) */

/* Current printer display name: from the stored DEVNAMES, else default. */
static int np_printer_name(WCHAR *out, size_t outChars)
{
    if(g_np.hDevNames){
        DEVNAMES *dn = GlobalLock(g_np.hDevNames);
        if(dn){
            const WCHAR *dev = (const WCHAR *)((const BYTE *)dn + dn->wDeviceOffset);
            wcsncpy_s(out, outChars, dev, _TRUNCATE);
            GlobalUnlock(g_np.hDevNames);
            return 1;
        }
    }
    DWORD size = 0;
    GetDefaultPrinterW(NULL, &size);
    if(size == 0 || size > outChars) return 0;
    return GetDefaultPrinterW(out, &size) != 0;
}

/* Make sure a DEVMODE exists (default printer), so Page Setup and printing
 * have paper/orientation state even before any Printer button press. */
static int np_ensure_devmode(void)
{
    if(g_np.hDevMode) return 1;
    wchar_t printer[NP_MAX_PATH];
    if(!np_printer_name(printer, NP_MAX_PATH)) return 0;
    HANDLE hPrinter = NULL;
    if(!OpenPrinterW(printer, &hPrinter, NULL)) return 0;
    LONG size = DocumentPropertiesW(NULL, hPrinter, printer, NULL, NULL, 0);
    if(size <= 0){ ClosePrinter(hPrinter); return 0; }
    HGLOBAL h = GlobalAlloc(GHND, (SIZE_T)size);
    if(!h){ ClosePrinter(hPrinter); return 0; }
    DEVMODEW *dm = GlobalLock(h);
    if(!dm){ GlobalFree(h); ClosePrinter(hPrinter); return 0; }
    LONG rc = DocumentPropertiesW(NULL, hPrinter, printer, dm, NULL,
                                  DM_OUT_BUFFER);
    GlobalUnlock(h);
    ClosePrinter(hPrinter);
    if(rc != IDOK){ GlobalFree(h); return 0; }
    /* Re-apply persisted paper/orientation on top of printer defaults. */
    dm = GlobalLock(h);
    if(dm){
        if(g_np.savedPaper){
            dm->dmFields |= DM_PAPERSIZE;
            dm->dmPaperSize = (short)g_np.savedPaper;
        }
        if(g_np.savedOrient == DMORIENT_PORTRAIT ||
           g_np.savedOrient == DMORIENT_LANDSCAPE){
            dm->dmFields |= DM_ORIENTATION;
            dm->dmOrientation = (short)g_np.savedOrient;
        }
        GlobalUnlock(h);
    }
    g_np.hDevMode = h;
    return 1;
}

static void np_free_devmode(void)
{
    if(g_np.hDevMode){ GlobalFree(g_np.hDevMode); g_np.hDevMode = NULL; }
    if(g_np.hDevNames){ GlobalFree(g_np.hDevNames); g_np.hDevNames = NULL; }
}

/* The genuine Page Setup dialog (resource 12). Papers/sources enumerate the
 * real printer; margins/header/footer validate and persist. */

/* (PAGESETUP_CTX + g_fallbackPapers live above, next to Global State.) */
static void np_page_setup(void)
{
    DialogBoxParamW(GetModuleHandleW(NULL), MAKEINTRESOURCEW(DLG_PAGESETUP),
                    g_np.mainWnd, np_pagesetup_proc, 0);
}

static void np_ps_free_lists(PAGESETUP_CTX *ctx)
{
    /* Only the enumeration lists; working DEVMODE/margins/text survive
     * (Cancel still discards via the dialog ending without commit). */
    free(ctx->papers); ctx->papers = NULL;
    free(ctx->paperNames); ctx->paperNames = NULL;
    free(ctx->paperSizes); ctx->paperSizes = NULL;
    free(ctx->bins); ctx->bins = NULL;
    free(ctx->binNames); ctx->binNames = NULL;
    ctx->paperCount = 0;
    ctx->binCount = 0;
}

/* Fill paper/source combos from the working DEVMODE's printer. Margins,
 * header/footer/DEVMODE are caller-owned and left untouched. */
static void np_ps_fill_lists(HWND dlg, PAGESETUP_CTX *ctx)
{
    HWND paperCombo = GetDlgItem(dlg, CTL_PS_PAPER);
    HWND srcCombo = GetDlgItem(dlg, CTL_PS_SOURCE);
    SendMessageW(paperCombo, CB_RESETCONTENT, 0, 0);
    SendMessageW(srcCombo, CB_RESETCONTENT, 0, 0);
    np_ps_free_lists(ctx);

    wchar_t printer[NP_MAX_PATH];
    int havePrinter = np_printer_name(printer, NP_MAX_PATH);
    if(ctx->hasDm){
        ctx->device[0] = L'\0';
        if(havePrinter) wcsncpy_s(ctx->device, NP_MAX_PATH, printer, _TRUNCATE);
    }
    if(havePrinter){
        int n = DeviceCapabilitiesW(printer, NULL, DC_PAPERS, NULL,
                                    ctx->hasDm ? &ctx->dm : NULL);
        int nb = DeviceCapabilitiesW(printer, NULL, DC_BINS, NULL, NULL);
        if(n > 0 && n < 1000){
            ctx->papers = malloc((size_t)n * sizeof(WORD));
            ctx->paperNames = malloc((size_t)n * 64 * sizeof(wchar_t));
            ctx->paperSizes = malloc((size_t)n * sizeof(POINT));
            if(ctx->papers && ctx->paperNames && ctx->paperSizes){
                DeviceCapabilitiesW(printer, NULL, DC_PAPERS,
                                    (LPWSTR)ctx->papers,
                                    ctx->hasDm ? &ctx->dm : NULL);
                DeviceCapabilitiesW(printer, NULL, DC_PAPERNAMES,
                                    (LPWSTR)ctx->paperNames,
                                    ctx->hasDm ? &ctx->dm : NULL);
                DeviceCapabilitiesW(printer, NULL, DC_PAPERSIZE,
                                    (LPWSTR)ctx->paperSizes,
                                    ctx->hasDm ? &ctx->dm : NULL);
                ctx->paperCount = n;
                for(int i = 0; i < n; i++)
                    SendMessageW(paperCombo, CB_ADDSTRING, 0,
                                 (LPARAM)ctx->paperNames[i]);
            }
        }
        if(nb > 0 && nb < 1000){
            ctx->bins = malloc((size_t)nb * sizeof(WORD));
            ctx->binNames = malloc((size_t)nb * 64 * sizeof(wchar_t));
            if(ctx->bins && ctx->binNames){
                DeviceCapabilitiesW(printer, NULL, DC_BINS,
                                    (LPWSTR)ctx->bins, NULL);
                DeviceCapabilitiesW(printer, NULL, DC_BINNAMES,
                                    (LPWSTR)ctx->binNames, NULL);
                ctx->binCount = nb;
                for(int i = 0; i < nb; i++)
                    SendMessageW(srcCombo, CB_ADDSTRING, 0,
                                 (LPARAM)ctx->binNames[i]);
            }
        }
    }
    if(ctx->paperCount == 0){
        ctx->paperCount = (int)(sizeof(g_fallbackPapers) / sizeof(g_fallbackPapers[0]));
        for(int i = 0; i < ctx->paperCount; i++)
            SendMessageW(paperCombo, CB_ADDSTRING, 0,
                         (LPARAM)g_fallbackPapers[i].name);
    }
    if(ctx->binCount == 0){
        SendMessageW(srcCombo, CB_ADDSTRING, 0, (LPARAM)L"Auto");
        ctx->binCount = -1; /* sentinel: fallback list */
    }
    /* Select current paper/source. */
    int paperSel = 0;
    if(ctx->hasDm && (ctx->dm.dmFields & DM_PAPERSIZE)){
        for(int i = 0; i < ctx->paperCount; i++){
            WORD id = (ctx->papers) ? ctx->papers[i]
                                    : g_fallbackPapers[i].id;
            if(id == ctx->dm.dmPaperSize){ paperSel = i; break; }
        }
    }
    SendMessageW(paperCombo, CB_SETCURSEL, paperSel, 0);
    int srcSel = 0;
    if(ctx->hasDm && (ctx->dm.dmFields & DM_DEFAULTSOURCE) && ctx->bins){
        for(int i = 0; i < ctx->binCount; i++)
            if(ctx->bins[i] == ctx->dm.dmDefaultSource){ srcSel = i; break; }
    }
    SendMessageW(srcCombo, CB_SETCURSEL, srcSel, 0);
}

/* Paper size in thousandths of an inch for the selected paper. */
static void np_ps_paper_thou(PAGESETUP_CTX *ctx, int paperSel,
                             DWORD *w, DWORD *h)
{
    if(ctx->papers && paperSel >= 0 && paperSel < ctx->paperCount){
        *w = (DWORD)((long long)ctx->paperSizes[paperSel].x * 1000 / 254);
        *h = (DWORD)((long long)ctx->paperSizes[paperSel].y * 1000 / 254);
    }else{
        if(paperSel < 0 || paperSel >= ctx->paperCount) paperSel = 0;
        *w = (DWORD)(g_fallbackPapers[paperSel].w * 1000.0);
        *h = (DWORD)(g_fallbackPapers[paperSel].h * 1000.0);
    }
    if(ctx->hasDm && (ctx->dm.dmFields & DM_ORIENTATION) &&
       ctx->dm.dmOrientation == DMORIENT_LANDSCAPE){
        DWORD t = *w; *w = *h; *h = t;
    }
}

static void np_ps_show_margins(HWND dlg, PAGESETUP_CTX *ctx)
{
    static const int ids[4] = { CTL_PS_LEFT, CTL_PS_TOP,
                                CTL_PS_RIGHT, CTL_PS_BOTTOM };
    for(int i = 0; i < 4; i++){
        wchar_t buf[32];
        swprintf_s(buf, 32, L"%.2f", ctx->margins[i] / 1000.0);
        SetDlgItemTextW(dlg, ids[i], buf);
    }
}

/* Read the four margin edits; invalid input beeps and keeps focus. */
static int np_ps_read_margins(HWND dlg, PAGESETUP_CTX *ctx, int complain)
{
    static const int ids[4] = { CTL_PS_LEFT, CTL_PS_TOP,
                                CTL_PS_RIGHT, CTL_PS_BOTTOM };
    DWORD vals[4];
    HWND paperCombo = GetDlgItem(dlg, CTL_PS_PAPER);
    int paperSel = (int)SendMessageW(paperCombo, CB_GETCURSEL, 0, 0);
    DWORD pw = 8500, ph = 11000;
    np_ps_paper_thou(ctx, paperSel, &pw, &ph);
    for(int i = 0; i < 4; i++){
        wchar_t buf[32];
        GetDlgItemTextW(dlg, ids[i], buf, 32);
        wchar_t *end = NULL;
        double inches = wcstod(buf, &end);
        if(end == buf || inches < 0 || inches * 1000.0 > 100000.0){
            if(complain){
                MessageBeep(MB_ICONWARNING);
                SetFocus(GetDlgItem(dlg, ids[i]));
                SendMessageW(GetDlgItem(dlg, ids[i]), EM_SETSEL, 0, (LPARAM)-1);
            }
            return 0;
        }
        vals[i] = (DWORD)(inches * 1000.0 + 0.5);
    }
    if(vals[0] + vals[2] >= pw || vals[1] + vals[3] >= ph){
        if(complain){
            MessageBeep(MB_ICONWARNING);
            SetFocus(GetDlgItem(dlg, ids[0]));
        }
        return 0;
    }
    for(int i = 0; i < 4; i++) ctx->margins[i] = vals[i];
    return 1;
}

/* Move the gray margin guides inside the white page preview. */
static void np_ps_update_preview(HWND dlg, PAGESETUP_CTX *ctx)
{
    HWND paperCombo = GetDlgItem(dlg, CTL_PS_PAPER);
    int paperSel = (int)SendMessageW(paperCombo, CB_GETCURSEL, 0, 0);
    DWORD pw = 8500, ph = 11000;
    np_ps_paper_thou(ctx, paperSel, &pw, &ph);
    RECT white;
    GetWindowRect(GetDlgItem(dlg, CTL_PS_PREVIEW_PAGE), &white);
    MapWindowPoints(NULL, dlg, (POINT *)&white, 2);
    int ww = white.right - white.left, wh = white.bottom - white.top;
    if(pw == 0) pw = 1;
    if(ph == 0) ph = 1;
    int rw = ww * (int)ctx->margins[2] / (int)pw;
    int bh = wh * (int)ctx->margins[3] / (int)ph;
    if(rw < 2) rw = 2;
    if(bh < 2) bh = 2;
    if(rw > ww) rw = ww;
    if(bh > wh) bh = wh;
    MoveWindow(GetDlgItem(dlg, CTL_PS_PREVIEW_R),
               white.right - rw, white.top, rw, wh, TRUE);
    MoveWindow(GetDlgItem(dlg, CTL_PS_PREVIEW_B),
               white.left, white.bottom - bh, ww, bh, TRUE);
}

static INT_PTR CALLBACK np_pagesetup_proc(HWND dlg, UINT msg,
                                          WPARAM wParam, LPARAM lParam)
{
    static PAGESETUP_CTX ctx;
    switch(msg){
    case WM_INITDIALOG:
        memset(&ctx, 0, sizeof(ctx));
        if(g_np.hDevMode){
            DEVMODEW *dm = GlobalLock(g_np.hDevMode);
            if(dm){ ctx.dm = *dm; ctx.hasDm = 1; GlobalUnlock(g_np.hDevMode); }
        }
        wcsncpy_s(ctx.header, 256, g_np.header, _TRUNCATE);
        wcsncpy_s(ctx.footer, 256, g_np.footer, _TRUNCATE);
        ctx.margins[0] = g_np.marginLeft; ctx.margins[1] = g_np.marginTop;
        ctx.margins[2] = g_np.marginRight; ctx.margins[3] = g_np.marginBottom;
        np_ps_fill_lists(dlg, &ctx);
        CheckRadioButton(dlg, CTL_PS_PORTRAIT, CTL_PS_LANDSCAPE,
            (ctx.hasDm && (ctx.dm.dmFields & DM_ORIENTATION) &&
             ctx.dm.dmOrientation == DMORIENT_LANDSCAPE)
            ? CTL_PS_LANDSCAPE : CTL_PS_PORTRAIT);
        np_ps_show_margins(dlg, &ctx);
        SetDlgItemTextW(dlg, CTL_PS_HEADER, ctx.header);
        SetDlgItemTextW(dlg, CTL_PS_FOOTER, ctx.footer);
        np_ps_update_preview(dlg, &ctx);
        np_dark_dlg_init(dlg);
        return TRUE;
    case WM_CTLCOLORDLG: {
        LRESULT br = np_dark_ctl((HDC)wParam, dlg);
        if(br) return br;
        break;
    }
    case WM_CTLCOLOREDIT:
    case WM_CTLCOLORSTATIC:
    case WM_CTLCOLORLISTBOX:
    case WM_CTLCOLORBTN: {
        LRESULT br = np_dark_ctl((HDC)wParam, (HWND)lParam);
        if(br) return br;
        break;
    }
    case WM_COMMAND: {
        int id = LOWORD(wParam);
        int code = HIWORD(wParam);
        if(id == IDOK){
            HWND paperCombo = GetDlgItem(dlg, CTL_PS_PAPER);
            HWND srcCombo = GetDlgItem(dlg, CTL_PS_SOURCE);
            int paperSel = (int)SendMessageW(paperCombo, CB_GETCURSEL, 0, 0);
            int srcSel = (int)SendMessageW(srcCombo, CB_GETCURSEL, 0, 0);
            if(!np_ps_read_margins(dlg, &ctx, 1)) return TRUE;
            int landscape = IsDlgButtonChecked(dlg, CTL_PS_LANDSCAPE) == BST_CHECKED;
            if(!ctx.hasDm){
                memset(&ctx.dm, 0, sizeof(ctx.dm));
                ctx.dm.dmSize = sizeof(ctx.dm);
                ctx.dm.dmSpecVersion = DM_SPECVERSION;
                ctx.hasDm = 1;
            }
            ctx.dm.dmFields |= DM_ORIENTATION | DM_PAPERSIZE | DM_DEFAULTSOURCE;
            ctx.dm.dmOrientation = landscape ? DMORIENT_LANDSCAPE : DMORIENT_PORTRAIT;
            if(ctx.papers && paperSel >= 0 && paperSel < ctx.paperCount)
                ctx.dm.dmPaperSize = ctx.papers[paperSel];
            else if(paperSel >= 0 && paperSel < ctx.paperCount)
                ctx.dm.dmPaperSize = g_fallbackPapers[paperSel].id;
            if(ctx.bins && srcSel >= 0 && srcSel < ctx.binCount)
                ctx.dm.dmDefaultSource = ctx.bins[srcSel];
            GetDlgItemTextW(dlg, CTL_PS_HEADER, ctx.header, 256);
            GetDlgItemTextW(dlg, CTL_PS_FOOTER, ctx.footer, 256);
            g_np.marginLeft = ctx.margins[0]; g_np.marginTop = ctx.margins[1];
            g_np.marginRight = ctx.margins[2]; g_np.marginBottom = ctx.margins[3];
            g_np.savedPaper = (int)ctx.dm.dmPaperSize;
            g_np.savedOrient = (int)ctx.dm.dmOrientation;
            wcsncpy_s(g_np.header, 256, ctx.header, _TRUNCATE);
            wcsncpy_s(g_np.footer, 256, ctx.footer, _TRUNCATE);
            /* Commit the working DEVMODE to the globals (printer
             * selection survives; only the mode block is replaced). */
            if(g_np.hDevMode) GlobalFree(g_np.hDevMode);
            g_np.hDevMode = NULL;
            HGLOBAL h = GlobalAlloc(GHND, ctx.dm.dmSize + ctx.dm.dmDriverExtra);
            if(h){
                DEVMODEW *dm = GlobalLock(h);
                if(dm){ memcpy(dm, &ctx.dm, ctx.dm.dmSize + ctx.dm.dmDriverExtra); GlobalUnlock(h); g_np.hDevMode = h; }
                else GlobalFree(h);
            }
            np_ps_free_lists(&ctx);
            EndDialog(dlg, IDOK);
            return TRUE;
        }
        if(id == IDCANCEL){ np_ps_free_lists(&ctx); EndDialog(dlg, IDCANCEL); return TRUE; }
        if(id == CTL_PS_PRINTER){
            PRINTDLGW pd;
            memset(&pd, 0, sizeof(pd));
            pd.lStructSize = sizeof(pd);
            pd.hwndOwner = dlg;
            pd.hDevMode = g_np.hDevMode;
            pd.hDevNames = g_np.hDevNames;
            pd.Flags = PD_PRINTSETUP | PD_RETURNDC | PD_ENABLEPRINTHOOK;
            pd.lpfnPrintHook = np_print_hook;
            HGLOBAL oldMode = g_np.hDevMode, oldNames = g_np.hDevNames;
            g_np.hDevMode = NULL;
            g_np.hDevNames = NULL;
            if(PrintDlgW(&pd)){
                if(pd.hDC) DeleteDC(pd.hDC);
                if(oldMode) GlobalFree(oldMode);
                if(oldNames) GlobalFree(oldNames);
                g_np.hDevMode = pd.hDevMode;
                g_np.hDevNames = pd.hDevNames;
                if(g_np.hDevMode){
                    DEVMODEW *dm = GlobalLock(g_np.hDevMode);
                    if(dm){ ctx.dm = *dm; ctx.hasDm = 1; GlobalUnlock(g_np.hDevMode); }
                }
                np_ps_fill_lists(dlg, &ctx);
                np_ps_read_margins(dlg, &ctx, 0);
                np_ps_update_preview(dlg, &ctx);
            }else{
                g_np.hDevMode = pd.hDevMode;
                g_np.hDevNames = pd.hDevNames;
                if(CommDlgExtendedError() != 0)
                    MessageBeep(MB_ICONWARNING);
            }
            return TRUE;
        }
        if((id == CTL_PS_PORTRAIT || id == CTL_PS_LANDSCAPE) && code == BN_CLICKED){
            if(!ctx.hasDm){ memset(&ctx.dm, 0, sizeof(ctx.dm)); ctx.dm.dmSize = sizeof(ctx.dm); ctx.hasDm = 1; }
            ctx.dm.dmFields |= DM_ORIENTATION;
            ctx.dm.dmOrientation = (id == CTL_PS_LANDSCAPE) ? DMORIENT_LANDSCAPE : DMORIENT_PORTRAIT;
            np_ps_update_preview(dlg, &ctx);
            return TRUE;
        }
        if(id == CTL_PS_PAPER && code == CBN_SELCHANGE){
            int paperSel = (int)SendMessageW(GetDlgItem(dlg, CTL_PS_PAPER), CB_GETCURSEL, 0, 0);
            if(!ctx.hasDm){ memset(&ctx.dm, 0, sizeof(ctx.dm)); ctx.dm.dmSize = sizeof(ctx.dm); ctx.hasDm = 1; }
            ctx.dm.dmFields |= DM_PAPERSIZE;
            if(ctx.papers && paperSel >= 0 && paperSel < ctx.paperCount)
                ctx.dm.dmPaperSize = ctx.papers[paperSel];
            else if(paperSel >= 0)
                ctx.dm.dmPaperSize = g_fallbackPapers[paperSel % ctx.paperCount].id;
            np_ps_update_preview(dlg, &ctx);
            return TRUE;
        }
        if(id == CTL_PS_SOURCE && code == CBN_SELCHANGE){
            int srcSel = (int)SendMessageW(GetDlgItem(dlg, CTL_PS_SOURCE), CB_GETCURSEL, 0, 0);
            if(ctx.bins && srcSel >= 0 && srcSel < ctx.binCount){
                if(!ctx.hasDm){ memset(&ctx.dm, 0, sizeof(ctx.dm)); ctx.dm.dmSize = sizeof(ctx.dm); ctx.hasDm = 1; }
                ctx.dm.dmFields |= DM_DEFAULTSOURCE;
                ctx.dm.dmDefaultSource = ctx.bins[srcSel];
            }
            return TRUE;
        }
        if((id == CTL_PS_LEFT || id == CTL_PS_TOP ||
            id == CTL_PS_RIGHT || id == CTL_PS_BOTTOM) && code == EN_CHANGE){
            if(np_ps_read_margins(dlg, &ctx, 0)) np_ps_update_preview(dlg, &ctx);
            return TRUE;
        }
        break;
    }
    case WM_NOTIFY: {
        LPNMHDR hdr = (LPNMHDR)lParam;
        if(hdr->idFrom == CTL_PS_VALUESLINK &&
           (hdr->code == NM_CLICK || hdr->code == NM_RETURN)){
            ShellExecuteW(NULL, L"open", NP_HELP_URL, NULL, NULL, SW_SHOWNORMAL);
            return TRUE;
        }
        break;
    }
    }
    return FALSE;
}

/* Expand header/footer codes: &l/&c/&r alignment sections, &d date,
 * &t time, &f file path, &p/&P page number, && literal. Unknown codes stay
 * literal. Each of left/center/right is drawn in its third of the line. */
static void np_expand_codes(const WCHAR *templ, int page, WCHAR *out, size_t outChars,
                            WCHAR *left, size_t leftChars,
                            WCHAR *center, size_t centerChars,
                            WCHAR *right, size_t rightChars)
{
    (void)out; (void)outChars;
    WCHAR *dst = left;
    size_t cap = leftChars;
    size_t o = 0;
    SYSTEMTIME st;
    GetLocalTime(&st);
    wchar_t date[64] = {0}, time[64] = {0}, num[32] = {0};
    GetDateFormatW(LOCALE_USER_DEFAULT, DATE_SHORTDATE, &st, NULL, date, 64);
    GetTimeFormatW(LOCALE_USER_DEFAULT, 0, &st, NULL, time, 64);
    swprintf_s(num, 32, L"%d", page);
    for(size_t i = 0; templ[i] && o + 1 < cap;){
        if(templ[i] == L'&'){
            wchar_t c = templ[i + 1];
            if(c == L'l'){ dst = left; cap = leftChars; o = 0; i += 2; continue; }
            if(c == L'c'){ dst = center; cap = centerChars; o = 0; i += 2; continue; }
            if(c == L'r'){ dst = right; cap = rightChars; o = 0; i += 2; continue; }
            const wchar_t *ins = NULL;
            if(c == L'd') ins = date;
            else if(c == L't') ins = time;
            else if(c == L'f') ins = g_np.path[0] ? g_np.path : S_UNTITLED;
            else if(c == L'p' || c == L'P') ins = num;
            else if(c == L'&') ins = L"&";
            if(ins){
                size_t n = wcslen(ins);
                if(o + n >= cap) n = cap - o - 1;
                memcpy(dst + o, ins, n * sizeof(WCHAR));
                o += n;
                i += 2;
                continue;
            }
            if(o + 2 < cap){ dst[o++] = L'&'; dst[o++] = c; }
            i += 2;
            continue;
        }
        dst[o++] = templ[i++];
    }
    dst[o] = L'\0';
}

/* Darken the system Print dialog (best effort; listviews stay native). */
static UINT_PTR CALLBACK np_print_hook(HWND dlg, UINT msg, WPARAM wParam,
                                       LPARAM lParam)
{
    if(msg == WM_INITDIALOG){
        np_dark_dlg_init(dlg);
        return TRUE;
    }
    if(msg == WM_CTLCOLOREDIT || msg == WM_CTLCOLORSTATIC ||
       msg == WM_CTLCOLORLISTBOX || msg == WM_CTLCOLORBTN){
        LRESULT br = np_dark_ctl((HDC)wParam, (HWND)lParam);
        if(br){
            SetWindowLongPtrW(dlg, DWLP_MSGRESULT, br);
            return 1;
        }
    }
    return 0;
}

static void np_print(void)
{
    PRINTDLGW pd;
    memset(&pd, 0, sizeof(pd));
    pd.lStructSize = sizeof(pd);
    pd.hwndOwner = g_np.mainWnd;
    pd.hDevMode = g_np.hDevMode;
    pd.hDevNames = g_np.hDevNames;
    pd.Flags = PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION |
               PD_ENABLEPRINTHOOK;
    pd.lpfnPrintHook = np_print_hook;
    HGLOBAL oldMode = g_np.hDevMode, oldNames = g_np.hDevNames;
    g_np.hDevMode = NULL;
    g_np.hDevNames = NULL;
    if(!PrintDlgW(&pd)){
        g_np.hDevMode = pd.hDevMode;
        g_np.hDevNames = pd.hDevNames;
        if(CommDlgExtendedError() == PDERR_NODEFAULTPRN)
            MessageBoxW(g_np.mainWnd, S_NO_PRINTER, S_APP, MB_OK | MB_ICONSTOP);
        return;
    }
    if(oldMode) GlobalFree(oldMode);
    if(oldNames) GlobalFree(oldNames);
    g_np.hDevMode = pd.hDevMode;
    g_np.hDevNames = pd.hDevNames;

    HDC hdc = pd.hDC;
    DOCINFOW di;
    memset(&di, 0, sizeof(di));
    di.cbSize = sizeof(di);
    di.lpszDocName = S_APP;
    if(StartDocW(hdc, &di) <= 0){ DeleteDC(hdc); return; }

    /* "Now Printing" abort dialog (genuine dialog 11). */
    g_np.abortPrint = 0;
    EnableWindow(g_np.mainWnd, FALSE);
    g_np.abortDlg = CreateDialogParamW(GetModuleHandleW(NULL),
        MAKEINTRESOURCEW(DLG_PRINTING), g_np.mainWnd, np_abort_dlg_proc, 0);
    if(g_np.abortDlg){
        SetDlgItemTextW(g_np.abortDlg, CTL_PRINT_NAME, np_base_name());
        SetDlgItemTextW(g_np.abortDlg, CTL_PRINT_PAGE, L"");
        ShowWindow(g_np.abortDlg, SW_SHOW);
    }
    SetAbortProc(hdc, np_abort_proc);
    int ok = np_print_document(hdc);
    if(ok) EndDoc(hdc);
    else AbortDoc(hdc);
    if(g_np.abortDlg){ DestroyWindow(g_np.abortDlg); g_np.abortDlg = NULL; }
    EnableWindow(g_np.mainWnd, TRUE);
    SetFocus(g_np.editBox);
    DeleteDC(hdc);
}

/* Abort callback: pump dialog messages so Cancel stays responsive. */
static BOOL CALLBACK np_abort_proc(HDC hdc, int code)
{
    (void)hdc; (void)code;
    MSG msg;
    while(!g_np.abortPrint && PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE)){
        if(g_np.abortDlg && IsDialogMessageW(g_np.abortDlg, &msg)) continue;
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    return !g_np.abortPrint;
}

static INT_PTR CALLBACK np_abort_dlg_proc(HWND dlg, UINT msg,
                                          WPARAM wParam, LPARAM lParam)
{
    (void)dlg; (void)lParam;
    if(msg == WM_COMMAND && LOWORD(wParam) == IDCANCEL){
        g_np.abortPrint = 1;
        return TRUE;
    }
    if(msg == WM_CLOSE){ g_np.abortPrint = 1; return TRUE; }
    return FALSE;
}

/* Draw the whole document: header (codes expanded), word-wrapped body
 * paginated against the Page Setup margins, footer with page number. */
static int np_print_document(HDC hdc)
{
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *text = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!text) return 0;
    GetWindowTextW(g_np.editBox, text, len + 1);

    int dpiX = GetDeviceCaps(hdc, LOGPIXELSX);
    int dpiY = GetDeviceCaps(hdc, LOGPIXELSY);
    int offX = GetDeviceCaps(hdc, PHYSICALOFFSETX);
    int offY = GetDeviceCaps(hdc, PHYSICALOFFSETY);
    int pageW = GetDeviceCaps(hdc, HORZRES);
    int pageH = GetDeviceCaps(hdc, VERTRES);
    int physW = GetDeviceCaps(hdc, PHYSICALWIDTH);
    int physH = GetDeviceCaps(hdc, PHYSICALHEIGHT);

    int mL = (int)((long long)g_np.marginLeft * dpiX / 1000) - offX;
    int mT = (int)((long long)g_np.marginTop * dpiY / 1000) - offY;
    int mR = (int)((long long)g_np.marginRight * dpiX / 1000) - (physW - offX - pageW);
    int mB = (int)((long long)g_np.marginBottom * dpiY / 1000) - (physH - offY - pageH);
    if(mL < 0) mL = 0;
    if(mT < 0) mT = 0;
    if(mR < 0) mR = 0;
    if(mB < 0) mB = 0;

    HDC screen = GetDC(NULL);
    int scrDPI = screen ? GetDeviceCaps(screen, LOGPIXELSY) : 96;
    if(screen) ReleaseDC(NULL, screen);
    LOGFONTW pf = g_np.font;
    pf.lfHeight = MulDiv(g_np.font.lfHeight, dpiY, scrDPI);
    HFONT hFont = CreateFontIndirectW(&pf);
    HFONT hOld = hFont ? SelectObject(hdc, hFont) : NULL;

    SIZE sz;
    GetTextExtentPoint32W(hdc, L"Ay", 2, &sz);
    int lineH = sz.cy + 2;
    if(pageH - mT - mB < lineH * 3){
        MessageBoxW(g_np.mainWnd, S_PAGE_SMALL, S_APP, MB_OK | MB_ICONSTOP);
        if(hOld) SelectObject(hdc, hOld);
        if(hFont) DeleteObject(hFont);
        free(text);
        return 0;
    }

    wchar_t hl[256], hc[256], hr[256], fl[256], fc[256], fr[256];
    int page = 1;
    int y = 0;
    int rc = 1;

    StartPage(hdc);
    np_expand_codes(g_np.header, page, NULL, 0, hl, 256, hc, 256, hr, 256);
    {
        RECT r;
        r.left = mL; r.right = pageW - mR; r.top = mT; r.bottom = mT + lineH;
        DrawTextW(hdc, hl, -1, &r, DT_LEFT | DT_NOPREFIX | DT_SINGLELINE);
        DrawTextW(hdc, hc, -1, &r, DT_CENTER | DT_NOPREFIX | DT_SINGLELINE);
        DrawTextW(hdc, hr, -1, &r, DT_RIGHT | DT_NOPREFIX | DT_SINGLELINE);
    }
    y = mT + lineH * 2;
    if(g_np.abortDlg){
        wchar_t pg[64];
        swprintf_s(pg, 64, L"Page %d", page);
        SetDlgItemTextW(g_np.abortDlg, CTL_PRINT_PAGE, pg);
    }

    WCHAR *para = text;
    for(;;){
        if(g_np.abortPrint){ rc = 0; break; }
        WCHAR *eol = wcsstr(para, L"\r\n");
        WCHAR saved = 0;
        if(eol){ saved = *eol; *eol = L'\0'; }

        RECT measure;
        measure.left = mL; measure.right = pageW - mR;
        measure.top = 0; measure.bottom = 0;
        int h = DrawTextW(hdc, para, -1, &measure,
                          DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_EXPANDTABS);
        if(h <= 0) h = lineH;
        if(y + h > pageH - mB - lineH * 2){
            np_expand_codes(g_np.footer, page, NULL, 0, fl, 256, fc, 256, fr, 256);
            RECT r;
            r.left = mL; r.right = pageW - mR;
            r.top = pageH - mB - lineH; r.bottom = pageH - mB;
            DrawTextW(hdc, fl, -1, &r, DT_LEFT | DT_NOPREFIX | DT_SINGLELINE);
            DrawTextW(hdc, fc, -1, &r, DT_CENTER | DT_NOPREFIX | DT_SINGLELINE);
            DrawTextW(hdc, fr, -1, &r, DT_RIGHT | DT_NOPREFIX | DT_SINGLELINE);
            if(EndPage(hdc) <= 0 || g_np.abortPrint){
                if(eol) *eol = saved;
                rc = 0;
                break;
            }
            page++;
            StartPage(hdc);
            np_expand_codes(g_np.header, page, NULL, 0, hl, 256, hc, 256, hr, 256);
            r.top = mT; r.bottom = mT + lineH;
            DrawTextW(hdc, hl, -1, &r, DT_LEFT | DT_NOPREFIX | DT_SINGLELINE);
            DrawTextW(hdc, hc, -1, &r, DT_CENTER | DT_NOPREFIX | DT_SINGLELINE);
            DrawTextW(hdc, hr, -1, &r, DT_RIGHT | DT_NOPREFIX | DT_SINGLELINE);
            y = mT + lineH * 2;
            if(g_np.abortDlg){
                wchar_t pg[64];
                swprintf_s(pg, 64, L"Page %d", page);
                SetDlgItemTextW(g_np.abortDlg, CTL_PRINT_PAGE, pg);
            }
        }
        RECT body;
        body.left = mL; body.right = pageW - mR;
        body.top = y; body.bottom = pageH - mB;
        DrawTextW(hdc, para, -1, &body,
                  DT_WORDBREAK | DT_NOPREFIX | DT_EXPANDTABS);
        y += h;

        if(eol){ *eol = saved; para = eol + 2; if(*para == L'\0') break; }
        else break;
    }
    if(rc && !g_np.abortPrint){
        np_expand_codes(g_np.footer, page, NULL, 0, fl, 256, fc, 256, fr, 256);
        RECT r;
        r.left = mL; r.right = pageW - mR;
        r.top = pageH - mB - lineH; r.bottom = pageH - mB;
        DrawTextW(hdc, fl, -1, &r, DT_LEFT | DT_NOPREFIX | DT_SINGLELINE);
        DrawTextW(hdc, fc, -1, &r, DT_CENTER | DT_NOPREFIX | DT_SINGLELINE);
        DrawTextW(hdc, fr, -1, &r, DT_RIGHT | DT_NOPREFIX | DT_SINGLELINE);
        rc = EndPage(hdc) > 0;
    }
    if(hOld) SelectObject(hdc, hOld);
    if(hFont) DeleteObject(hFont);
    free(text);
    return rc && !g_np.abortPrint;
}

/* ==========================================================================
 *  Crash autosave (%LOCALAPPDATA%\LoOS\Notepad\autosave, UTF-16LE)
 * ========================================================================== */
static int np_autosave_dir(wchar_t *out, size_t outChars)
{
    wchar_t appdata[NP_MAX_PATH];
    if(FAILED(SHGetFolderPathW(NULL, CSIDL_LOCAL_APPDATA, NULL, 0, appdata))) return 0;
    swprintf_s(out, outChars, L"%s\\LoOS\\Notepad\\autosave", appdata);
    wchar_t tmp[NP_MAX_PATH];
    wcsncpy_s(tmp, NP_MAX_PATH, out, _TRUNCATE);
    for(wchar_t *p = tmp; *p; p++){
        if(*p == L'\\'){
            wchar_t save = *p; *p = L'\0';
            if(wcslen(tmp) > 3) CreateDirectoryW(tmp, NULL);
            *p = save;
        }
    }
    CreateDirectoryW(tmp, NULL);
    return 1;
}

/* Every 30s: snapshot each dirty tab (large-file tabs are skipped: their
 * snapshots would be huge and slow). Active tab snapshots live text. */
static void np_autosave_write(void)
{
    np_tab_stash();
    wchar_t dir[NP_MAX_PATH];
    if(!np_autosave_dir(dir, NP_MAX_PATH)) return;
    for(int i = 0; i < g_np.tabCount; i++){
        NP_TAB *t = &g_np.tabs[i];
        if(!t->dirty || t->largeFile) continue;
        const WCHAR *txt = t->text;
        WCHAR *live = NULL;
        if(i == g_np.tabActive){
            int len = GetWindowTextLengthW(g_np.editBox);
            if(len == 0) continue;
            live = malloc(((size_t)len + 1) * sizeof(WCHAR));
            if(!live) continue;
            GetWindowTextW(g_np.editBox, live, len + 1);
            txt = live;
        }else if(!txt || !txt[0]){
            continue;
        }
        wchar_t bak[NP_MAX_PATH], meta[NP_MAX_PATH];
        swprintf_s(bak, NP_MAX_PATH, L"%s\\recovery_%d.txt", dir, i);
        swprintf_s(meta, NP_MAX_PATH, L"%s\\recovery_%d.meta", dir, i);
        HANDLE h = CreateFileW(bak, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                               FILE_ATTRIBUTE_NORMAL, NULL);
        if(h != INVALID_HANDLE_VALUE){
            static const BYTE bom[2] = { 0xFF, 0xFE };
            DWORD written = 0;
            if(WriteFile(h, bom, 2, &written, NULL))
                WriteFile(h, txt, (DWORD)wcslen(txt) * 2, &written, NULL);
            CloseHandle(h);
            HANDLE m = CreateFileW(meta, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                                   FILE_ATTRIBUTE_NORMAL, NULL);
            if(m != INVALID_HANDLE_VALUE){
                DWORD ml = (DWORD)(wcslen(t->path) * sizeof(WCHAR));
                if(ml > 0) WriteFile(m, t->path, ml, &written, NULL);
                CloseHandle(m);
            }
            np_debug(L"autosaved tab %d (%d chars)", i, (int)wcslen(txt));
        }
        free(live);
    }
}

/* A clean save or a clean exit removes every snapshot (plus legacy names). */
static void np_autosave_clear_tab(int idx)
{
    wchar_t dir[NP_MAX_PATH];
    if(!np_autosave_dir(dir, NP_MAX_PATH)) return;
    wchar_t bak[NP_MAX_PATH], meta[NP_MAX_PATH];
    swprintf_s(bak, NP_MAX_PATH, L"%s\\recovery_%d.txt", dir, idx);
    swprintf_s(meta, NP_MAX_PATH, L"%s\\recovery_%d.meta", dir, idx);
    DeleteFileW(bak);
    DeleteFileW(meta);
}

static void np_autosave_clear(void)
{
    for(int i = 0; i < NP_MAX_TABS; i++) np_autosave_clear_tab(i);
    wchar_t dir[NP_MAX_PATH];
    if(!np_autosave_dir(dir, NP_MAX_PATH)) return;
    wchar_t bak[NP_MAX_PATH], meta[NP_MAX_PATH];
    for(int i = 0; i < NP_MAX_TABS; i++) np_autosave_clear_tab(i);
    swprintf_s(bak, NP_MAX_PATH, L"%s\\recovery.txt", dir);
    swprintf_s(meta, NP_MAX_PATH, L"%s\\recovery.meta", dir);
    DeleteFileW(bak);
    DeleteFileW(meta);
}

/* At startup: if crash snapshots exist, offer a full session restore from
 * them. Returns 1 when anything was restored (caller then skips session). */
static int np_autosave_offer(void)
{
    wchar_t dir[NP_MAX_PATH];
    if(!np_autosave_dir(dir, NP_MAX_PATH)) return 0;
    wchar_t probe[NP_MAX_PATH];
    swprintf_s(probe, NP_MAX_PATH, L"%s\\recovery_0.txt", dir);
    if(GetFileAttributesW(probe) == INVALID_FILE_ATTRIBUTES){
        /* Legacy single-file snapshot from older builds. */
        swprintf_s(probe, NP_MAX_PATH, L"%s\\recovery.txt", dir);
        if(GetFileAttributesW(probe) == INVALID_FILE_ATTRIBUTES) return 0;
        wchar_t meta[NP_MAX_PATH], orig[NP_MAX_PATH] = {0};
        swprintf_s(meta, NP_MAX_PATH, L"%s\\recovery.meta", dir);
        HANDLE m = CreateFileW(meta, GENERIC_READ, FILE_SHARE_READ, NULL,
                               OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if(m != INVALID_HANDLE_VALUE){
            DWORD read = 0, size = GetFileSize(m, NULL);
            if(size > 0 && size < (NP_MAX_PATH - 1) * 2)
                ReadFile(m, orig, size, &read, NULL);
            orig[read / 2] = L'\0';
            CloseHandle(m);
        }
        wchar_t msg[NP_MSG_MAX];
        swprintf_s(msg, NP_MSG_MAX,
                   L"Unsaved work from %s was found.\nDo you want to recover it?",
                   orig[0] ? orig : S_UNTITLED);
        if(MessageBoxW(g_np.mainWnd, msg, S_APP,
                       MB_YESNO | MB_ICONQUESTION) == IDYES){
            if(np_load_file(probe, ENC_UTF16LE, 1, 0)){
                if(orig[0]) wcsncpy_s(g_np.path, NP_MAX_PATH, orig, _TRUNCATE);
                else g_np.path[0] = L'\0';
                np_set_dirty(1);
                np_refresh_title();
                np_update_status();
            }
        }
        np_autosave_clear();
        return 1;
    }
    if(MessageBoxW(g_np.mainWnd,
            L"Unsaved work from a previous session was found.\n"
            L"Do you want to recover all open tabs?",
            S_APP, MB_YESNO | MB_ICONQUESTION) != IDYES){
        np_autosave_clear();
        return 0;
    }
    /* Drop the pristine starter tab, then rebuild from snapshots. */
    if(g_np.tabCount == 1 && !g_np.path[0] && !g_np.dirty &&
       GetWindowTextLengthW(g_np.editBox) == 0){
        g_np.tabCount = 0;
        SendMessageW(g_np.tabBar, TCM_DELETEITEM, 0, 0);
    }
    for(int i = 0; i < NP_MAX_TABS; i++){
        wchar_t bak[NP_MAX_PATH], meta[NP_MAX_PATH], orig[NP_MAX_PATH] = {0};
        swprintf_s(bak, NP_MAX_PATH, L"%s\\recovery_%d.txt", dir, i);
        if(GetFileAttributesW(bak) == INVALID_FILE_ATTRIBUTES) continue;
        swprintf_s(meta, NP_MAX_PATH, L"%s\\recovery_%d.meta", dir, i);
        HANDLE m = CreateFileW(meta, GENERIC_READ, FILE_SHARE_READ, NULL,
                               OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if(m != INVALID_HANDLE_VALUE){
            DWORD read = 0, size = GetFileSize(m, NULL);
            if(size > 0 && size < (NP_MAX_PATH - 1) * 2)
                ReadFile(m, orig, size, &read, NULL);
            orig[read / 2] = L'\0';
            CloseHandle(m);
        }
        if(g_np.tabCount >= NP_MAX_TABS) break;
        if(g_np.tabCount >= g_np.tabCap){
            int cap = g_np.tabCap * 2;
            NP_TAB *nt = realloc(g_np.tabs, (size_t)cap * sizeof(NP_TAB));
            if(!nt) break;
            g_np.tabs = nt;
            g_np.tabCap = cap;
        }
        NP_TAB *t = &g_np.tabs[g_np.tabCount];
        memset(t, 0, sizeof(*t));
        HANDLE h = CreateFileW(bak, GENERIC_READ, FILE_SHARE_READ, NULL,
                               OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if(h == INVALID_HANDLE_VALUE) continue;
        DWORD size = GetFileSize(h, NULL);
        BYTE *raw = malloc(size + 2);
        if(raw){
            DWORD r = 0;
            ReadFile(h, raw, size, &r, NULL);
            DWORD skip = (r >= 2 && raw[0] == 0xFF && raw[1] == 0xFE) ? 2 : 0;
            DWORD nch = (r - skip) / 2;
            t->text = malloc(((size_t)nch + 1) * sizeof(WCHAR));
            if(t->text){
                memcpy(t->text, raw + skip, nch * sizeof(WCHAR));
                t->text[nch] = L'\0';
                t->textLen = (int)nch;
                t->dirty = 1;
                if(orig[0]) wcsncpy_s(t->path, NP_MAX_PATH, orig, _TRUNCATE);
                g_np.tabCount++;
                np_tab_label(g_np.tabCount - 1);
            }
            free(raw);
        }
        CloseHandle(h);
    }
    np_autosave_clear();
    if(g_np.tabCount > 0){
        int want = g_np.tabCount - 1;
        g_np.tabActive = -1;
        np_tab_show(want);
        return 1;
    }
    return 0;
}

/* ==========================================================================
 *  Settings (HKCU\Software\Microsoft\Notepad, original value names)
 * ========================================================================== */
static void np_reg_dword(HKEY key, const wchar_t *name, DWORD value)
{
    RegSetValueExW(key, name, 0, REG_DWORD, (BYTE *)&value, sizeof(value));
}

static int np_reg_get_dword(HKEY key, const wchar_t *name, DWORD *out)
{
    DWORD type = 0, v = 0, size = sizeof(v);
    if(RegQueryValueExW(key, name, NULL, &type, (BYTE *)&v, &size) != ERROR_SUCCESS) return 0;
    if(type != REG_DWORD) return 0;
    *out = v;
    return 1;
}

static void np_reg_get_str(HKEY key, const wchar_t *name,
                           wchar_t *out, size_t outChars)
{
    DWORD type = 0, size = (DWORD)(outChars * sizeof(WCHAR));
    if(RegQueryValueExW(key, name, NULL, &type, (BYTE *)out, &size) != ERROR_SUCCESS)
        out[0] = L'\0';
    else out[outChars - 1] = L'\0';
}

static void np_load_settings(void)
{
    HKEY key;
    if(RegOpenKeyExW(HKEY_CURRENT_USER, NP_REG_KEY, 0, KEY_READ, &key) != ERROR_SUCCESS)
        return;
    DWORD v = 0;
    if(np_reg_get_dword(key, L"fWrap", &v)) g_np.wrap = (v != 0);
    if(np_reg_get_dword(key, L"fStatusBar", &v)) g_np.showBar = (v != 0);
    if(np_reg_get_dword(key, L"fToolbar", &v)) g_np.showToolbar = (v != 0);
    if(np_reg_get_dword(key, L"fAutoPairs", &v)) g_np.autoPairs = (v != 0);
    if(np_reg_get_dword(key, L"fOverview", &v)) g_np.showOverview = (v != 0);
    if(np_reg_get_dword(key, L"iZoom", &v) &&
       (int)v >= NP_ZOOM_MIN && (int)v <= NP_ZOOM_MAX)
        g_np.zoom = (int)v;
    if(np_reg_get_dword(key, L"fWrapAround", &v)) g_np.wrapAround = (v != 0);
    if(np_reg_get_dword(key, L"fSaveWindowPositions", &v)) g_np.saveWinPos = (v != 0);
    if(np_reg_get_dword(key, L"fMatchCase", &v)) g_np.matchCase = (v != 0);
    if(np_reg_get_dword(key, L"fWholeWord", &v)) g_np.wholeWord = (v != 0);
    if(np_reg_get_dword(key, L"fRegex", &v)) g_np.findRegex = (v != 0);
    if(np_reg_get_dword(key, L"fReverse", &v)) g_np.findDown = (v == 0);
    if(np_reg_get_dword(key, L"iTheme", &v) && (int)v >= 0 && (int)v <= 2)
        g_np.theme = (int)v;
    np_reg_get_str(key, L"sTheme", g_np.themeName, 64);
    if(np_reg_get_dword(key, L"iWindowPosX", &v)) g_np.winX = (int)v;
    if(np_reg_get_dword(key, L"iWindowPosY", &v)) g_np.winY = (int)v;
    if(np_reg_get_dword(key, L"iWindowPosDX", &v)) g_np.winDX = (int)v;
    if(np_reg_get_dword(key, L"iWindowPosDY", &v)) g_np.winDY = (int)v;
    if(np_reg_get_dword(key, L"iMarginLeft", &v)) g_np.marginLeft = v;
    if(np_reg_get_dword(key, L"iMarginTop", &v)) g_np.marginTop = v;
    if(np_reg_get_dword(key, L"iMarginRight", &v)) g_np.marginRight = v;
    if(np_reg_get_dword(key, L"iMarginBottom", &v)) g_np.marginBottom = v;
    if(np_reg_get_dword(key, L"iPaperSize", &v)) g_np.savedPaper = (int)v;
    if(np_reg_get_dword(key, L"iOrientation", &v)) g_np.savedOrient = (int)v;
    np_reg_get_str(key, L"szHeader", g_np.header, 256);
    np_reg_get_str(key, L"szFooter", g_np.footer, 256);
    if(!g_np.header[0]) wcsncpy_s(g_np.header, 256, L"&f", _TRUNCATE);
    if(!g_np.footer[0]) wcsncpy_s(g_np.footer, 256, L"Page &p", _TRUNCATE);
    DWORD type = 0, size = sizeof(g_np.font);
    if(RegQueryValueExW(key, L"lfFaceName", NULL, &type,
                        (BYTE *)&g_np.font, &size) == ERROR_SUCCESS)
        g_np.haveFont = 1;
    size = sizeof(g_findWhat);
    RegQueryValueExW(key, L"searchString", NULL, &type, (BYTE *)g_findWhat, &size);
    size = sizeof(g_replaceWith);
    RegQueryValueExW(key, L"replaceString", NULL, &type, (BYTE *)g_replaceWith, &size);
    RegCloseKey(key);
}

static void np_save_settings(void)
{
    HKEY key;
    if(RegCreateKeyW(HKEY_CURRENT_USER, NP_REG_KEY, &key) != ERROR_SUCCESS)
        return;
    np_reg_dword(key, L"fWrap", g_np.wrap ? 1 : 0);
    np_reg_dword(key, L"fStatusBar", g_np.showBar ? 1 : 0);
    np_reg_dword(key, L"fToolbar", g_np.showToolbar ? 1 : 0);
    np_reg_dword(key, L"fAutoPairs", g_np.autoPairs ? 1 : 0);
    np_reg_dword(key, L"fOverview", g_np.showOverview ? 1 : 0);
    np_reg_dword(key, L"iZoom", (DWORD)g_np.zoom);
    np_reg_dword(key, L"fWrapAround", g_np.wrapAround ? 1 : 0);
    np_reg_dword(key, L"fSaveWindowPositions", g_np.saveWinPos ? 1 : 0);
    np_reg_dword(key, L"fMatchCase", g_np.matchCase ? 1 : 0);
    np_reg_dword(key, L"fWholeWord", g_np.wholeWord ? 1 : 0);
    np_reg_dword(key, L"fRegex", g_np.findRegex ? 1 : 0);
    np_reg_dword(key, L"fReverse", g_np.findDown ? 0 : 1);
    np_reg_dword(key, L"iTheme", (DWORD)g_np.theme);
    RegSetValueExW(key, L"sTheme", 0, REG_SZ, (BYTE *)g_np.themeName,
                   (DWORD)((wcslen(g_np.themeName) + 1) * sizeof(WCHAR)));
    np_reg_dword(key, L"iMarginLeft", g_np.marginLeft);
    np_reg_dword(key, L"iMarginTop", g_np.marginTop);
    np_reg_dword(key, L"iMarginRight", g_np.marginRight);
    np_reg_dword(key, L"iMarginBottom", g_np.marginBottom);
    np_reg_dword(key, L"iPaperSize", (DWORD)g_np.savedPaper);
    np_reg_dword(key, L"iOrientation", (DWORD)g_np.savedOrient);
    if(g_np.mainWnd){
        WINDOWPLACEMENT wp;
        wp.length = sizeof(wp);
        if(GetWindowPlacement(g_np.mainWnd, &wp) &&
           wp.showCmd != SW_SHOWMINIMIZED && wp.showCmd != SW_SHOWMAXIMIZED){
            np_reg_dword(key, L"iWindowPosX", (DWORD)wp.rcNormalPosition.left);
            np_reg_dword(key, L"iWindowPosY", (DWORD)wp.rcNormalPosition.top);
            np_reg_dword(key, L"iWindowPosDX",
                         (DWORD)(wp.rcNormalPosition.right - wp.rcNormalPosition.left));
            np_reg_dword(key, L"iWindowPosDY",
                         (DWORD)(wp.rcNormalPosition.bottom - wp.rcNormalPosition.top));
        }
    }
    if(g_np.haveFont)
        RegSetValueExW(key, L"lfFaceName", 0, REG_BINARY,
                       (BYTE *)&g_np.font, sizeof(g_np.font));
    RegSetValueExW(key, L"szHeader", 0, REG_SZ, (BYTE *)g_np.header,
                   (DWORD)((wcslen(g_np.header) + 1) * sizeof(WCHAR)));
    RegSetValueExW(key, L"szFooter", 0, REG_SZ, (BYTE *)g_np.footer,
                   (DWORD)((wcslen(g_np.footer) + 1) * sizeof(WCHAR)));
    RegSetValueExW(key, L"searchString", 0, REG_SZ, (BYTE *)g_findWhat,
                   (DWORD)((wcslen(g_findWhat) + 1) * sizeof(WCHAR)));
    RegSetValueExW(key, L"replaceString", 0, REG_SZ, (BYTE *)g_replaceWith,
                   (DWORD)((wcslen(g_replaceWith) + 1) * sizeof(WCHAR)));
    RegCloseKey(key);
}

/* ==========================================================================
 *  /.SETUP mode (Windows Setup): open-or-create the named file, chromeless.
 * ========================================================================== */
static int np_setup_open(const wchar_t *path)
{
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL,
                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if(h == INVALID_HANDLE_VALUE){
        if(GetLastError() == ERROR_FILE_NOT_FOUND){
            WCHAR *msg = np_format1(S_NOTFOUND_NEW, path);
            int answer = IDNO;
            if(msg){
                answer = MessageBoxW(g_np.mainWnd, msg, S_APP,
                                     MB_YESNOCANCEL | MB_ICONWARNING);
                LocalFree(msg);
            }
            if(answer == IDYES){
                wcsncpy_s(g_np.path, NP_MAX_PATH, path, _TRUNCATE);
                SetWindowTextW(g_np.editBox, L"");
                np_set_dirty(0);
                np_refresh_title();
                return 1;
            }
            return 0;
        }
        np_file_error(path);
        return 0;
    }
    CloseHandle(h);
    return np_load_file(path, -1, 0, 0);
}

/* ==========================================================================
 *  Tabs + session
 * ========================================================================== */

/* Tab strip (SysTabControl32) above the editor. One EDIT serves all tabs;
 * np_tab_stash parks the live text into the outgoing tab, np_tab_show
 * loads the incoming one. Selection, scroll, encoding, EOL and dirty flag
 * travel with each tab; undo history does not (EDIT control limit). */
static int np_tabs_init(void)
{
    HINSTANCE inst = GetModuleHandleW(NULL);
    g_np.tabBar = CreateWindowExW(0, WC_TABCONTROLW, NULL,
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | TCS_SINGLELINE |
        TCS_FOCUSNEVER | TCS_OWNERDRAWFIXED,
        0, 0, 100, NP_TABSTRIP_H, g_np.mainWnd, NULL, inst, NULL);
    if(!g_np.tabBar) return 0;
    SetWindowTheme(g_np.tabBar, L"Explorer", NULL);
    SetWindowSubclass(g_np.tabBar, np_tab_subproc, 3, 0);
    /* Room for the close box: widen the default padding. */
    SendMessageW(g_np.tabBar, TCM_SETPADDING, 0, MAKELPARAM(26, 4));
    SendMessageW(g_np.tabBar, WM_SETFONT, (WPARAM)np_get_menu_font(), TRUE);
    g_np.tabHover = -1;
    g_np.tabCap = 4;
    g_np.tabs = calloc((size_t)g_np.tabCap, sizeof(NP_TAB));
    if(!g_np.tabs) return 0;
    g_np.tabCount = 1;
    g_np.tabActive = 0;
    memset(&g_np.tabs[0], 0, sizeof(NP_TAB));
    np_tab_label(0);
    return 1;
}

static void np_tab_label(int idx)
{
    if(idx < 0 || idx >= g_np.tabCount || !g_np.tabBar) return;
    NP_TAB *t = &g_np.tabs[idx];
    const wchar_t *name = L"Untitled";
    if(t->path[0]){
        const wchar_t *slash = wcsrchr(t->path, L'\\');
        name = slash ? slash + 1 : t->path;
    }else if(idx == g_np.tabActive && !g_np.path[0]){
        name = np_base_name();
    }
    wchar_t label[NP_MAX_PATH + 8];
    if((idx == g_np.tabActive ? g_np.dirty : t->dirty))
        swprintf_s(label, NP_MAX_PATH + 8, L"*%s", name);
    else
        wcsncpy_s(label, NP_MAX_PATH + 8, name, _TRUNCATE);
    TCITEMW item;
    memset(&item, 0, sizeof(item));
    item.mask = TCIF_TEXT;
    item.pszText = label;
    if((int)SendMessageW(g_np.tabBar, TCM_GETITEMCOUNT, 0, 0) <= idx)
        SendMessageW(g_np.tabBar, TCM_INSERTITEMW, idx, (LPARAM)&item);
    else
        SendMessageW(g_np.tabBar, TCM_SETITEMW, idx, (LPARAM)&item);
    SendMessageW(g_np.tabBar, TCM_SETCURSEL, g_np.tabActive, 0);
}

/* Park live EDIT text/selection/scroll/flags into the active tab slot. */
static int np_tab_stash(void)
{
    if(g_np.tabActive < 0 || g_np.tabActive >= g_np.tabCount) return 0;
    NP_TAB *t = &g_np.tabs[g_np.tabActive];
    wcsncpy_s(t->path, NP_MAX_PATH, g_np.path, _TRUNCATE);
    t->enc = g_np.encoding;
    t->bom = g_np.bomPresent;
    t->eol = g_np.eolMode;
    t->dirty = g_np.dirty;
    /* largeFile persists per tab (set at load, cleared on new/close). */
    {
        DWORD s = 0, e = 0;
        SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
        t->selS = (int)s; t->selE = (int)e;
        t->firstLine = (int)SendMessageW(g_np.editBox, EM_GETFIRSTVISIBLELINE, 0, 0);
    }
    int len = GetWindowTextLengthW(g_np.editBox);
    free(t->text);
    t->text = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!t->text){ t->textLen = 0; return 0; }
    GetWindowTextW(g_np.editBox, t->text, len + 1);
    t->textLen = len;
    return 1;
}

/* Load tab idx into the live editor. Stashes the current tab first. */
static int np_tab_show(int idx)
{
    if(idx < 0 || idx >= g_np.tabCount || idx == g_np.tabActive) return 0;
    np_tab_stash();
    /* Sync the split pane if it was showing the tab we leave. */
    if(g_np.splitEdit && g_np.splitTab == g_np.tabActive){
        int len = GetWindowTextLengthW(g_np.splitEdit);
        WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
        if(buf){
            GetWindowTextW(g_np.splitEdit, buf, len + 1);
            free(g_np.tabs[g_np.tabActive].text);
            g_np.tabs[g_np.tabActive].text = buf;
            g_np.tabs[g_np.tabActive].textLen = len;
        }
    }
    g_np.tabActive = idx;
    NP_TAB *t = &g_np.tabs[idx];
    wcsncpy_s(g_np.path, NP_MAX_PATH, t->path, _TRUNCATE);
    g_np.encoding = t->enc;
    g_np.bomPresent = t->bom;
    g_np.eolMode = t->eol;
    if(t->largeFile && g_np.wrap){
        /* Large-file tabs keep word wrap locked off (see np_load_file). */
        g_np.wrap = 0;
        ShowWindow(g_np.statusBar, g_np.showBar ? SW_SHOW : SW_HIDE);
    }
    g_np.editGuard = 1;
    SetWindowTextW(g_np.editBox, t->text ? t->text : L"");
    g_np.editGuard = 0;
    SendMessageW(g_np.editBox, EM_SETSEL, t->selS, t->selE);
    {
        int cur = (int)SendMessageW(g_np.editBox, EM_GETFIRSTVISIBLELINE, 0, 0);
        SendMessageW(g_np.editBox, EM_LINESCROLL, 0, t->firstLine - cur);
    }
    np_apply_zoom();
    g_np.dirty = 0;
    np_set_dirty(t->dirty);
    np_refresh_title();
    np_update_status();
    np_tab_label(idx);
    if(g_np.splitEdit) np_split_refresh();
    np_sticky_update();
    if(g_np.mdPreview) np_md_update();
    np_fire_event(NP_EVENT_TAB);
    return 1;
}

static int np_tab_new(void)
{
    np_tab_stash();
    if(g_np.tabCount >= NP_MAX_TABS) return 0;
    if(g_np.tabCount >= g_np.tabCap){
        int cap = g_np.tabCap * 2;
        NP_TAB *nt = realloc(g_np.tabs, (size_t)cap * sizeof(NP_TAB));
        if(!nt) return 0;
        g_np.tabs = nt;
        g_np.tabCap = cap;
    }
    NP_TAB *t = &g_np.tabs[g_np.tabCount];
    memset(t, 0, sizeof(*t));
    g_np.tabCount++;
    for(int i = 0; i < g_np.tabCount; i++) np_tab_label(i);
    return np_tab_show(g_np.tabCount - 1) + 1;
}

/* Close a tab (with save prompt when dirty). The last tab becomes a fresh
 * blank instead of exiting; File > Exit still quits. */
static int np_tab_close(int idx)
{
    if(idx < 0 || idx >= g_np.tabCount) return 0;
    int wasActive = (idx == g_np.tabActive);
    np_tab_stash(); /* refresh the live tab's slot before indices shift */
    np_closed_push(&g_np.tabs[idx]);
    if(g_np.tabs[idx].dirty){
        int showing = g_np.tabActive;
        if(!wasActive) np_tab_show(idx);
        int answer = np_ask_save();
        if(!wasActive) np_tab_show(showing > idx ? showing - 1 : showing);
        if(answer == IDCANCEL) return 0;
        if(wasActive) np_tab_stash();
    }
    free(g_np.tabs[idx].text);
    for(int i = idx; i + 1 < g_np.tabCount; i++)
        g_np.tabs[i] = g_np.tabs[i + 1];
    g_np.tabCount--;
    SendMessageW(g_np.tabBar, TCM_DELETEITEM, idx, 0);
    if(g_np.tabCount == 0){
        g_np.tabCount = 1;
        memset(&g_np.tabs[0], 0, sizeof(NP_TAB));
        g_np.tabActive = 0;
        np_tab_label(0);
        np_tab_show(0);
        return 1;
    }
    if(g_np.tabActive >= g_np.tabCount) g_np.tabActive = g_np.tabCount - 1;
    else if(g_np.tabActive > idx) g_np.tabActive--;
    for(int i = 0; i < g_np.tabCount; i++) np_tab_label(i);
    /* Reload through the normal path (stash is a no-op at index -1). */
    {
        int want = g_np.tabActive;
        g_np.tabActive = -1;
        np_tab_show(want);
    }
    if(g_np.splitEdit) np_split_refresh();
    HeapCompact(GetProcessHeap(), 0); /* give big transient blocks back */
    return 1;
}

/* Session + recent files live in %APPDATA%\LoOS\Notepad\session.ini
 * (plain INI via the profile APIs). Untitled tab contents stash beside it.
 * Crash recovery reuses the same files plus per-tab recovery snapshots. */
static void np_session_dir(wchar_t *out, size_t outChars)
{
    wchar_t appdata[NP_MAX_PATH];
    if(FAILED(SHGetFolderPathW(NULL, CSIDL_APPDATA, NULL, 0, appdata))){
        out[0] = L'\0';
        return;
    }
    swprintf_s(out, outChars, L"%s\\LoOS\\Notepad", appdata);
    CreateDirectoryW(out, NULL);
    {
        wchar_t sub[NP_MAX_PATH];
        swprintf_s(sub, NP_MAX_PATH, L"%s\\LoOS", appdata);
        CreateDirectoryW(sub, NULL);
        CreateDirectoryW(out, NULL);
    }
}

static void np_session_save(void)
{
    wchar_t dir[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    if(!dir[0]) return;
    np_tab_stash();
    wchar_t ini[NP_MAX_PATH];
    swprintf_s(ini, NP_MAX_PATH, L"%s\\session.ini", dir);
    wchar_t buf[32];
    swprintf_s(buf, 32, L"%d", g_np.tabCount);
    WritePrivateProfileStringW(L"session", L"tabs", buf, ini);
    swprintf_s(buf, 32, L"%d", g_np.tabActive);
    WritePrivateProfileStringW(L"session", L"active", buf, ini);
    for(int i = 0; i < g_np.tabCount && i < NP_MAX_TABS; i++){
        wchar_t key[32], val[NP_MAX_PATH + 32];
        swprintf_s(key, 32, L"path%d", i);
        WritePrivateProfileStringW(L"tabs", key, g_np.tabs[i].path, ini);
        swprintf_s(key, 32, L"caret%d", i);
        swprintf_s(val, NP_MAX_PATH + 32, L"%d,%d", g_np.tabs[i].selS,
                   g_np.tabs[i].selE);
        WritePrivateProfileStringW(L"tabs", key, val, ini);
        if(!g_np.tabs[i].path[0] &&
           ((i == g_np.tabActive) ||
            (g_np.tabs[i].text && g_np.tabs[i].text[0]))){
            /* Untitled content must survive: stash to a sidecar file. */
            wchar_t side[NP_MAX_PATH];
            swprintf_s(side, NP_MAX_PATH, L"%s\\untitled_%d.txt", dir, i);
            const WCHAR *txt = (i == g_np.tabActive) ? NULL : g_np.tabs[i].text;
            WCHAR *live = NULL;
            if(i == g_np.tabActive){
                int len = GetWindowTextLengthW(g_np.editBox);
                live = malloc(((size_t)len + 1) * sizeof(WCHAR));
                if(live) GetWindowTextW(g_np.editBox, live, len + 1);
                txt = live;
            }
            if(txt && txt[0]){
                HANDLE h = CreateFileW(side, GENERIC_WRITE, 0, NULL,
                                       CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL,
                                       NULL);
                if(h != INVALID_HANDLE_VALUE){
                    static const BYTE bom[2] = { 0xFF, 0xFE };
                    DWORD w = 0;
                    WriteFile(h, bom, 2, &w, NULL);
                    WriteFile(h, txt,
                              (DWORD)wcslen(txt) * 2, &w, NULL);
                    CloseHandle(h);
                    swprintf_s(key, 32, L"untitled%d", i);
                    WritePrivateProfileStringW(L"tabs", key, side, ini);
                }
            }
            free(live);
        }
    }
    np_debug(L"session saved (%d tabs)", g_np.tabCount);
}

static void np_session_restore(void)
{
    wchar_t dir[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    if(!dir[0]) return;
    wchar_t ini[NP_MAX_PATH];
    swprintf_s(ini, NP_MAX_PATH, L"%s\\session.ini", dir);
    if(GetFileAttributesW(ini) == INVALID_FILE_ATTRIBUTES) return;
    int n = GetPrivateProfileIntW(L"session", L"tabs", 0, ini);
    int active = GetPrivateProfileIntW(L"session", L"active", 0, ini);
    if(n <= 0) return;
    if(n > NP_MAX_TABS) n = NP_MAX_TABS;
    /* Drop the default blank tab if it is still pristine. */
    int pristine = (g_np.tabCount == 1 && !g_np.path[0] && !g_np.dirty &&
                   GetWindowTextLengthW(g_np.editBox) == 0);
    if(pristine){
        free(g_np.tabs[0].text);
        g_np.tabCount = 0;
        SendMessageW(g_np.tabBar, TCM_DELETEITEM, 0, 0);
    }
    for(int i = 0; i < n; i++){
        wchar_t key[32], path[NP_MAX_PATH], side[NP_MAX_PATH], caret[64];
        swprintf_s(key, 32, L"path%d", i);
        GetPrivateProfileStringW(L"tabs", key, L"", path, NP_MAX_PATH, ini);
        swprintf_s(key, 32, L"caret%d", i);
        GetPrivateProfileStringW(L"tabs", key, L"0,0", caret, 64, ini);
        swprintf_s(key, 32, L"untitled%d", i);
        GetPrivateProfileStringW(L"tabs", key, L"", side, NP_MAX_PATH, ini);
        if(!path[0] && !side[0]) continue;
        if(g_np.tabCount >= NP_MAX_TABS) break;
        if(g_np.tabCount >= g_np.tabCap){
            int cap = g_np.tabCap * 2;
            NP_TAB *nt = realloc(g_np.tabs, (size_t)cap * sizeof(NP_TAB));
            if(!nt) break;
            g_np.tabs = nt;
            g_np.tabCap = cap;
        }
        NP_TAB *t = &g_np.tabs[g_np.tabCount];
        memset(t, 0, sizeof(*t));
        int s = 0, e = 0;
        swscanf_s(caret, L"%d,%d", &s, &e);
        t->selS = s; t->selE = e;
        if(path[0] && GetFileAttributesW(path) != INVALID_FILE_ATTRIBUTES){
            /* Load via the normal loader with live state parked aside. */
            WCHAR savedPath[NP_MAX_PATH];
            NP_ENCODING savedEnc = g_np.encoding;
            int savedBom = g_np.bomPresent, savedDirty = g_np.dirty;
            NP_EOL savedEol = g_np.eolMode;
            int savedActive = g_np.tabActive;
            int liveLen = GetWindowTextLengthW(g_np.editBox);
            WCHAR *liveText = malloc(((size_t)liveLen + 1) * sizeof(WCHAR));
            if(liveText) GetWindowTextW(g_np.editBox, liveText, liveLen + 1);
            wcsncpy_s(savedPath, NP_MAX_PATH, g_np.path, _TRUNCATE);
            g_np.tabCount++;
            g_np.tabActive = g_np.tabCount - 1;
            g_np.path[0] = L'\0';
            g_np.encoding = ENC_ANSI; g_np.bomPresent = 0;
            g_np.eolMode = EOL_CRLF; g_np.dirty = 0;
            if(np_load_file(path, -1, 0, 0)){
                np_tab_stash();
                np_tab_label(g_np.tabCount - 1);
            }else{
                g_np.tabCount--;
            }
            /* Restore the live view. */
            wcsncpy_s(g_np.path, NP_MAX_PATH, savedPath, _TRUNCATE);
            g_np.encoding = savedEnc; g_np.bomPresent = savedBom;
            g_np.eolMode = savedEol; g_np.dirty = savedDirty;
            g_np.editGuard = 1;
            SetWindowTextW(g_np.editBox, liveText ? liveText : L"");
            g_np.editGuard = 0;
            free(liveText);
            g_np.tabActive = savedActive;
            np_refresh_title();
            np_update_status();
        }else if(side[0]){
            wcsncpy_s(t->path, NP_MAX_PATH, L"", NP_MAX_PATH);
            HANDLE h = CreateFileW(side, GENERIC_READ, FILE_SHARE_READ, NULL,
                                   OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
            if(h != INVALID_HANDLE_VALUE){
                DWORD size = GetFileSize(h, NULL);
                BYTE *raw = malloc(size + 2);
                if(raw){
                    DWORD r = 0;
                    ReadFile(h, raw, size, &r, NULL);
                    DWORD skip = (r >= 2 && raw[0] == 0xFF && raw[1] == 0xFE) ? 2 : 0;
                    DWORD nch = (r - skip) / 2;
                    t->text = malloc(((size_t)nch + 1) * sizeof(WCHAR));
                    if(t->text){
                        memcpy(t->text, raw + skip, nch * sizeof(WCHAR));
                        t->text[nch] = L'\0';
                        t->textLen = (int)nch;
                        t->dirty = 1;
                    }
                    free(raw);
                }
                CloseHandle(h);
            }
            g_np.tabCount++;
            np_tab_label(g_np.tabCount - 1);
        }
    }
    if(g_np.tabCount == 0){
        g_np.tabCount = 1;
        memset(&g_np.tabs[0], 0, sizeof(NP_TAB));
        g_np.tabActive = 0;
        np_tab_label(0);
        return;
    }
    if(active < 0 || active >= g_np.tabCount) active = g_np.tabCount - 1;
    {
        int want = active;
        g_np.tabActive = -1;
        np_tab_show(want);
    }
}

/* MRU list for the palette (session.ini [recent], newest first). */
static void np_recent_add(const wchar_t *path)
{
    if(!path || !path[0]) return;
    wchar_t dir[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    if(!dir[0]) return;
    wchar_t ini[NP_MAX_PATH];
    swprintf_s(ini, NP_MAX_PATH, L"%s\\session.ini", dir);
    wchar_t items[20][1024];
    int n = 0;
    for(int i = 0; i < 20; i++){
        wchar_t key[32];
        swprintf_s(key, 32, L"file%d", i);
        GetPrivateProfileStringW(L"recent", key, L"", items[n], 1024, ini);
        if(!items[n][0]) break;
        if(_wcsicmp(items[n], path) == 0) continue; /* de-dup, re-add on top */
        n++;
    }
    /* Shift down and insert at top (cap 20). */
    for(int i = (n < 19 ? n : 19); i > 0; i--)
        wcsncpy_s(items[i], 1024, items[i - 1], _TRUNCATE);
    wcsncpy_s(items[0], 1024, path, _TRUNCATE);
    if(n < 20) n++;
    for(int i = 0; i < n && i < 20; i++){
        wchar_t key[32];
        swprintf_s(key, 32, L"file%d", i);
        WritePrivateProfileStringW(L"recent", key, items[i], ini);
    }
}

static int np_recent_list(wchar_t out[][1024], int max)
{
    wchar_t dir[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    if(!dir[0]) return 0;
    wchar_t ini[NP_MAX_PATH];
    swprintf_s(ini, NP_MAX_PATH, L"%s\\session.ini", dir);
    int n = 0;
    for(int i = 0; i < max; i++){
        wchar_t key[32];
        swprintf_s(key, 32, L"file%d", i);
        GetPrivateProfileStringW(L"recent", key, L"", out[n], 1024, ini);
        if(!out[n][0]) break;
        n++;
    }
    return n;
}

/* ==========================================================================
 *  Regex engine (own implementation, memory-light streaming matcher)
 * ==========================================================================
 *  Supported: ^ $ . * + ? {m,n} {m,} {m} | ( ) [ranges] [^neg] \d\D\w\W\s\S\b
 *  plus escaped literals. No backreferences in patterns; $1..$9 work in
 *  REPLACEMENTS. Matching is a backtracking VM over caller-owned buffers
 *  (no document copies beyond what find already holds). A step budget aborts
 *  pathological patterns with -2 instead of hanging on huge files.
 *  (NP_RX_MAXGROUPS / NP_RX_STEP_LIMIT live up in Constants.) */

typedef struct {
    const WCHAR *pat;
    int plen;
    const WCHAR *text;
    int tlen;
    int matchCase;
    int *steps;
    int groups[NP_RX_MAXGROUPS][2];
    int ngroups;
} NP_RX_CTX;

static int np_rx_bump(NP_RX_CTX *rx)
{
    (*rx->steps)++;
    return (*rx->steps > NP_RX_STEP_LIMIT) ? -2 : 0;
}

static int np_rx_eq(NP_RX_CTX *rx, WCHAR a, WCHAR b)
{
    if(a == b) return 1;
    if(!rx->matchCase && towlower(a) == towlower(b)) return 1;
    return 0;
}

static int np_rx_is_word(WCHAR c)
{
    return iswalnum(c) || c == L'_';
}

static int np_rx_expr(NP_RX_CTX *rx, int pi, int ti, int end);

/* Match one class body (pi points past '['). Returns new ti or -1/-2. */
static int np_rx_class(NP_RX_CTX *rx, int pi, int ti, int *nextPi)
{
    int neg = 0, matched = 0;
    if(ti >= rx->tlen) return -1;
    if(pi < rx->plen && rx->pat[pi] == L'^'){ neg = 1; pi++; }
    WCHAR c = rx->text[ti];
    while(pi < rx->plen && rx->pat[pi] != L']'){
        WCHAR lo = rx->pat[pi], hi = lo;
        int isEsc = 0;
        if(lo == L'\\' && pi + 1 < rx->plen){
            pi++;
            lo = hi = rx->pat[pi];
            isEsc = 1;
            if(lo == L'd' || lo == L'D' || lo == L'w' || lo == L'W' ||
               lo == L's' || lo == L'S'){
                int hit = 0;
                if(lo == L'd') hit = iswdigit(c);
                else if(lo == L'D') hit = !iswdigit(c);
                else if(lo == L'w') hit = np_rx_is_word(c);
                else if(lo == L'W') hit = !np_rx_is_word(c);
                else if(lo == L's') hit = iswspace(c);
                else hit = !iswspace(c);
                if(hit) matched = 1;
                pi++;
                continue;
            }
            if(lo == L'n') lo = hi = L'\n';
            else if(lo == L't') lo = hi = L'\t';
            else if(lo == L'r') lo = hi = L'\r';
        }
        if(!isEsc && pi + 2 < rx->plen && rx->pat[pi + 1] == L'-' &&
           rx->pat[pi + 2] != L']'){
            hi = rx->pat[pi + 2];
            pi += 3;
        }else{
            pi++;
        }
        if(rx->matchCase){
            if(lo <= c && c <= hi) matched = 1;
        }else{
            WCHAR cl = towlower(c);
            if(towlower(lo) <= cl && cl <= towlower(hi)) matched = 1;
        }
    }
    if(pi >= rx->plen) return -1;
    *nextPi = pi + 1;
    if(np_rx_bump(rx)) return -2;
    return (matched != neg) ? ti + 1 : -1;
}

/* Match one atom at pi/ti. Sets *nextPi (past atom) and returns end ti,
 * ti for zero-width, -1 no match, -2 abort. */
static int np_rx_atom(NP_RX_CTX *rx, int pi, int ti, int *nextPi)
{
    WCHAR c = rx->pat[pi];
    if(c == L'('){
        int depth = 1, q = pi + 1, g = -1;
        if(rx->ngroups < NP_RX_MAXGROUPS) g = rx->ngroups++;
        while(q < rx->plen && depth > 0){
            if(rx->pat[q] == L'\\'){ q += 2; continue; }
            if(rx->pat[q] == L'['){
                q++;
                while(q < rx->plen && rx->pat[q] != L']'){
                    if(rx->pat[q] == L'\\') q++;
                    q++;
                }
                q++;
                continue;
            }
            if(rx->pat[q] == L'(') depth++;
            else if(rx->pat[q] == L')') depth--;
            q++;
        }
        if(depth != 0) return -1;
        int saved[NP_RX_MAXGROUPS][2];
        memcpy(saved, rx->groups, sizeof(saved));
        int e = np_rx_expr(rx, pi + 1, ti, q - 1);
        if(e < 0){ memcpy(rx->groups, saved, sizeof(saved)); return e; }
        if(g >= 0){ rx->groups[g][0] = ti; rx->groups[g][1] = e; }
        *nextPi = q;
        return e;
    }
    if(c == L'[') return np_rx_class(rx, pi + 1, ti, nextPi);
    if(c == L'.'){
        if(ti >= rx->tlen) return -1;
        if(rx->text[ti] == L'\r' || rx->text[ti] == L'\n') return -1;
        if(np_rx_bump(rx)) return -2;
        *nextPi = pi + 1;
        return ti + 1;
    }
    if(c == L'^'){
        *nextPi = pi + 1;
        if(ti == 0 || rx->text[ti - 1] == L'\n') return ti;
        return -1;
    }
    if(c == L'$'){
        *nextPi = pi + 1;
        if(ti >= rx->tlen || rx->text[ti] == L'\r' || rx->text[ti] == L'\n')
            return ti;
        return -1;
    }
    if(c == L'\\' && pi + 1 < rx->plen){
        WCHAR e = rx->pat[pi + 1];
        *nextPi = pi + 2;
        if(e == L'b'){
            int left = ti > 0 && np_rx_is_word(rx->text[ti - 1]);
            int right = ti < rx->tlen && np_rx_is_word(rx->text[ti]);
            return (left != right) ? ti : -1;
        }
        if(ti >= rx->tlen) return -1;
        WCHAR t = rx->text[ti];
        int hit = 0;
        if(e == L'd') hit = iswdigit(t);
        else if(e == L'D') hit = !iswdigit(t);
        else if(e == L'w') hit = np_rx_is_word(t);
        else if(e == L'W') hit = !np_rx_is_word(t);
        else if(e == L's') hit = iswspace(t);
        else if(e == L'S') hit = !iswspace(t);
        else if(e == L'n') hit = (t == L'\n');
        else if(e == L't') hit = (t == L'\t');
        else if(e == L'r') hit = (t == L'\r');
        else hit = np_rx_eq(rx, t, e);
        if(np_rx_bump(rx)) return -2;
        return hit ? ti + 1 : -1;
    }
    if(ti >= rx->tlen) return -1;
    if(np_rx_bump(rx)) return -2;
    *nextPi = pi + 1;
    return np_rx_eq(rx, rx->text[ti], c) ? ti + 1 : -1;
}

/* Parse {m,n}/{m,}/{m} at pi. Returns 1 if a quantifier was present. */
static int np_rx_quant(const WCHAR *pat, int plen, int pi,
                       int *minOut, int *maxOut, int *nextPi)
{
    if(pi >= plen) return 0;
    WCHAR c = pat[pi];
    if(c == L'*'){ *minOut = 0; *maxOut = -1; *nextPi = pi + 1; return 1; }
    if(c == L'+'){ *minOut = 1; *maxOut = -1; *nextPi = pi + 1; return 1; }
    if(c == L'?'){ *minOut = 0; *maxOut = 1; *nextPi = pi + 1; return 1; }
    if(c != L'{') return 0;
    int q = pi + 1, m = 0, n = -1, digits = 0;
    while(q < plen && pat[q] >= L'0' && pat[q] <= L'9'){
        m = m * 10 + (pat[q] - L'0'); q++; digits++;
    }
    if(!digits || m > 1000) return 0;
    if(q < plen && pat[q] == L','){
        q++;
        if(q < plen && pat[q] == L'}'){ n = -1; q++; }
        else{
            n = 0; digits = 0;
            while(q < plen && pat[q] >= L'0' && pat[q] <= L'9'){
                n = n * 10 + (pat[q] - L'0'); q++; digits++;
            }
            if(!digits || n > 1000 || n < m) return 0;
            if(q >= plen || pat[q] != L'}') return 0;
            q++;
        }
    }else if(q < plen && pat[q] == L'}'){
        n = m; q++;
    }else{
        return 0;
    }
    *minOut = m; *maxOut = n; *nextPi = q;
    return 1;
}

/* Concatenation with greedy quantified atoms and backtracking. */
static int np_rx_concat(NP_RX_CTX *rx, int pi, int ti, int end)
{
    if(pi >= end) return ti;
    int mn = 1, mx = 1, qp = pi, hasQ = 0;
    /* Find the atom extent, then a trailing quantifier. */
    {
        WCHAR cc = rx->pat[pi];
        if(cc == L'('){
            int depth = 1;
            qp = pi + 1;
            while(qp < end && depth > 0){
                if(rx->pat[qp] == L'\\'){ qp += 2; continue; }
                if(rx->pat[qp] == L'(') depth++;
                else if(rx->pat[qp] == L')') depth--;
                qp++;
            }
        }else if(cc == L'['){
            qp = pi + 1;
            while(qp < end && rx->pat[qp] != L']'){
                if(rx->pat[qp] == L'\\') qp++;
                qp++;
            }
            qp++;
        }else if(cc == L'\\'){
            qp = pi + 2;
        }else{
            qp = pi + 1;
        }
        if(qp <= end) hasQ = np_rx_quant(rx->pat, end, qp, &mn, &mx, &qp);
        if(qp > end){ hasQ = 0; mn = 1; mx = 1; qp = pi; }
    }
    if(!hasQ){
        int np = 0, e = np_rx_atom(rx, pi, ti, &np);
        if(e < 0) return e;
        return np_rx_concat(rx, np, e, end);
    }
    {
        WCHAR ac = rx->pat[pi];
        if(ac == L'^' || ac == L'$' ||
           (ac == L'\\' && pi + 1 < end && rx->pat[pi + 1] == L'b')){
            int np = 0, e = np_rx_atom(rx, pi, ti, &np);
            if(e < 0) return e;
            return np_rx_concat(rx, qp, e, end);
        }
    }
    /* Greedy counted match with backtracking. */
    int pos[1024];
    int cnt = 0;
    pos[0] = ti;
    int saved[NP_RX_MAXGROUPS][2];
    memcpy(saved, rx->groups, sizeof(saved));
    while(mx < 0 || cnt < mx){
        int np = 0;
        int e = np_rx_atom(rx, pi, pos[cnt], &np);
        if(e == -2){ memcpy(rx->groups, saved, sizeof(saved)); return -2; }
        if(e < 0) break;
        cnt++;
        if(cnt >= 1023) break;
        pos[cnt] = e;
        if(e == pos[cnt - 1]) break; /* zero-width: one step only */
        (void)np;
    }
    for(int k = cnt; k >= mn; k--){
        int e = np_rx_concat(rx, qp, pos[k], end);
        if(e >= 0) return e;
        if(e == -2){ memcpy(rx->groups, saved, sizeof(saved)); return -2; }
    }
    memcpy(rx->groups, saved, sizeof(saved));
    return -1;
}

static int np_rx_expr(NP_RX_CTX *rx, int pi, int ti, int end)
{
    int depth = 0, start = pi;
    for(int q = pi; ; q++){
        int isEnd = (q >= end);
        WCHAR c = isEnd ? L'\0' : rx->pat[q];
        if(!isEnd){
            if(c == L'\\'){ q++; continue; }
            if(c == L'['){
                q++;
                while(q < end && rx->pat[q] != L']'){
                    if(rx->pat[q] == L'\\') q++;
                    q++;
                }
                continue;
            }
            if(c == L'('){ depth++; continue; }
            if(c == L')'){ depth--; continue; }
            if(c == L'|' && depth == 0){
                int saved[NP_RX_MAXGROUPS][2];
                memcpy(saved, rx->groups, sizeof(saved));
                int e = np_rx_concat(rx, start, ti, q);
                if(e >= 0) return e;
                if(e == -2) return -2;
                memcpy(rx->groups, saved, sizeof(saved));
                start = q + 1;
                continue;
            }
        }else{
            return np_rx_concat(rx, start, ti, end);
        }
    }
}

/* Try a match starting exactly at `from`. Returns end offset, -1, or -2. */
static int np_regex_attempt(const WCHAR *pat, int from,
                            const WCHAR *text, int tlen, int matchCase,
                            int groups[][2], int maxGroups, int *steps)
{
    NP_RX_CTX rx;
    rx.pat = pat;
    rx.plen = (int)wcslen(pat);
    rx.text = text;
    rx.tlen = tlen;
    rx.matchCase = matchCase;
    rx.steps = steps;
    rx.ngroups = 0;
    for(int i = 0; i < NP_RX_MAXGROUPS; i++)
        rx.groups[i][0] = rx.groups[i][1] = -1;
    int e = np_rx_expr(&rx, 0, from, rx.plen);
    if(e >= 0){
        for(int i = 0; i < maxGroups && i < NP_RX_MAXGROUPS; i++){
            groups[i][0] = rx.groups[i][0];
            groups[i][1] = rx.groups[i][1];
        }
    }
    return e;
}

/* Full search entry: start offset, match length, groups; -1 none, -2 abort. */
static int np_regex_match(const WCHAR *pat, const WCHAR *text, int matchCase,
                          int *matchLen, int groups[][2], int maxGroups,
                          int *steps)
{
    if(!pat || !pat[0]) return -1;
    int tlen = (int)wcslen(text);
    int localGroups[NP_RX_MAXGROUPS][2];
    int (*g)[2] = groups ? groups : localGroups;
    int mg = groups ? maxGroups : NP_RX_MAXGROUPS;
    for(int i = 0; i <= tlen; i++){
        int e = np_regex_attempt(pat, i, text, tlen, matchCase, g, mg, steps);
        if(e == -2) return -2;
        if(e >= 0){
            if(matchLen) *matchLen = e - i;
            return i;
        }
        if(*steps > NP_RX_STEP_LIMIT) return -2;
    }
    return -1;
}

/* Directional regex search over a counted buffer. */
static int np_regex_find_pos(const WCHAR *text, int textLen, int from,
                             int down, int *matchLen, int groups[][2])
{
    int steps = 0;
    if(down){
        if(from < 0) from = 0;
        for(int i = from; i <= textLen; i++){
            int e = np_regex_attempt(g_findWhat, i, text, textLen,
                                     g_np.matchCase, groups,
                                     NP_RX_MAXGROUPS, &steps);
            if(e == -2) return -2;
            if(e >= 0){ *matchLen = e - i; return i; }
            if(steps > NP_RX_STEP_LIMIT) return -2;
        }
    }else{
        if(from > textLen) from = textLen;
        for(int i = from; i >= 0; i--){
            int e = np_regex_attempt(g_findWhat, i, text, textLen,
                                     g_np.matchCase, groups,
                                     NP_RX_MAXGROUPS, &steps);
            if(e == -2) return -2;
            if(e >= 0){ *matchLen = e - i; return i; }
            if(steps > NP_RX_STEP_LIMIT) return -2;
        }
    }
    return -1;
}

/* Expand $1..$9 ($$ = literal $) against absolute group spans in src. */
static void np_regex_expand(const WCHAR *repl, const WCHAR *src,
                            int groups[][2], int ngroups,
                            WCHAR *out, size_t outChars)
{
    size_t o = 0;
    for(size_t i = 0; repl[i] && o + 1 < outChars; i++){
        if(repl[i] == L'$' && repl[i + 1] >= L'1' && repl[i + 1] <= L'9'){
            int g = repl[i + 1] - L'1';
            i++;
            if(g < ngroups && groups[g][0] >= 0 && groups[g][1] > groups[g][0]){
                for(int k = groups[g][0]; k < groups[g][1] && o + 1 < outChars; k++)
                    out[o++] = src[k];
            }
        }else if(repl[i] == L'$' && repl[i + 1] == L'$'){
            out[o++] = L'$';
            i++;
        }else{
            out[o++] = repl[i];
        }
    }
    out[o] = L'\0';
}

/* Count matches in the live document (regex or literal). -1 = too complex. */
static int np_find_count(void)
{
    if(!g_findWhat[0]) return 0;
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return 0;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    int count = 0;
    if(g_np.findRegex){
        int steps = 0, groups[NP_RX_MAXGROUPS][2];
        int i = 0;
        while(i <= len){
            int e = np_regex_attempt(g_findWhat, i, buf, len, g_np.matchCase,
                                     groups, NP_RX_MAXGROUPS, &steps);
            if(e == -2 || steps > NP_RX_STEP_LIMIT){ count = -1; break; }
            if(e < 0) break;
            count++;
            i = (e == i) ? i + 1 : e;
            if(count > 1000000) break;
        }
    }else{
        size_t m = wcslen(g_findWhat);
        for(int i = 0; i + (int)m <= len; i++)
            if(np_match_at(buf, i)){ count++; i += (int)m - 1; }
    }
    free(buf);
    return count;
}

static void np_find_count_show(HWND dlg)
{
    HWND cnt = GetDlgItem(dlg, CTL_MATCHCOUNT);
    if(!cnt) return;
    if(!g_findWhat[0]){ SetWindowTextW(cnt, L""); return; }
    int n = np_find_count();
    wchar_t buf[64];
    if(n < 0) swprintf_s(buf, 64, L"too complex");
    else if(n == 1) swprintf_s(buf, 64, L"1 match");
    else swprintf_s(buf, 64, L"%d matches", n);
    SetWindowTextW(cnt, buf);
}

/* ==========================================================================
 *  Sort Lines + Math evaluator
 * ==========================================================================
 *  Sort works on the selected lines (expanded to full lines) or the whole
 *  document. Modes: alpha A-Z / Z-A, numeric by leading number, by length,
 *  unique (first occurrence wins, order kept), trim trailing whitespace. */

static int np_sort_alpha_cmp(const void *a, const void *b)
{
    const WCHAR * const *pa = a, * const *pb = b;
    return _wcsicmp(*pa, *pb);
}

static int np_sort_state;

static int np_sort_numeric_cmp(const void *a, const void *b)
{
    const WCHAR * const *pa = a, * const *pb = b;
    double va = wcstod(*pa, NULL), vb = wcstod(*pb, NULL);
    if(va < vb) return -1;
    if(va > vb) return 1;
    int r = _wcsicmp(*pa, *pb);
    return np_sort_state ? -r : r;
}

static int np_sort_length_cmp(const void *a, const void *b)
{
    const WCHAR * const *pa = a, * const *pb = b;
    size_t la = wcslen(*pa), lb = wcslen(*pb);
    if(la < lb) return -1;
    if(la > lb) return 1;
    return _wcsicmp(*pa, *pb);
}

static void np_sort_lines(int mode)
{
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return;
    GetWindowTextW(g_np.editBox, buf, len + 1);

    /* Expand to full lines. */
    int l0 = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, s, 0);
    int l1 = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, e, 0);
    int total = (int)SendMessageW(g_np.editBox, EM_GETLINECOUNT, 0, 0);
    if(l1 >= total) l1 = total - 1;
    int start = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, l0, 0);
    int endNext = (l1 + 1 < total)
        ? (int)SendMessageW(g_np.editBox, EM_LINEINDEX, l1 + 1, 0) : len;
    if(start < 0) start = 0;
    if(endNext < start) endNext = start;

    /* Split the range into lines (without terminators). */
    int cap = 64, n = 0;
    WCHAR **lines = malloc((size_t)cap * sizeof(WCHAR *));
    if(!lines){ free(buf); return; }
    int p = start;
    while(p <= endNext){
        int q = p;
        while(q < endNext && buf[q] != L'\r' && buf[q] != L'\n') q++;
        if(n >= cap){
            cap *= 2;
            WCHAR **nl = realloc(lines, (size_t)cap * sizeof(WCHAR *));
            if(!nl) break;
            lines = nl;
        }
        int ll = q - p;
        lines[n] = malloc(((size_t)ll + 1) * sizeof(WCHAR));
        if(!lines[n]) break;
        memcpy(lines[n], buf + p, (size_t)ll * sizeof(WCHAR));
        lines[n][ll] = L'\0';
        n++;
        if(q >= endNext) break;
        p = q + ((q + 1 < endNext && buf[q] == L'\r' && buf[q + 1] == L'\n') ? 2 : 1);
    }

    if(mode == 5){
        /* Trim trailing whitespace in place. */
        for(int i = 0; i < n; i++){
            int ll = (int)wcslen(lines[i]);
            while(ll > 0 && (lines[i][ll - 1] == L' ' || lines[i][ll - 1] == L'\t'))
                lines[i][--ll] = L'\0';
        }
    }else if(mode == 4){
        /* Unique: first occurrence wins, order kept (case-insensitive). */
        int w = 0;
        for(int i = 0; i < n; i++){
            int dup = 0;
            for(int j = 0; j < w; j++)
                if(_wcsicmp(lines[i], lines[j]) == 0){ dup = 1; break; }
            if(dup) free(lines[i]);
            else lines[w++] = lines[i];
        }
        n = w;
    }else{
        np_sort_state = (mode == 1);
        if(mode == 0) qsort(lines, (size_t)n, sizeof(WCHAR *), np_sort_alpha_cmp);
        else if(mode == 2) qsort(lines, (size_t)n, sizeof(WCHAR *), np_sort_numeric_cmp);
        else if(mode == 3) qsort(lines, (size_t)n, sizeof(WCHAR *), np_sort_length_cmp);
        else if(mode == 1){
            qsort(lines, (size_t)n, sizeof(WCHAR *), np_sort_alpha_cmp);
            for(int i = 0; i < n / 2; i++){
                WCHAR *t = lines[i];
                lines[i] = lines[n - 1 - i];
                lines[n - 1 - i] = t;
            }
        }
    }

    /* Rebuild: lines joined with CRLF. */
    size_t need = 1;
    for(int i = 0; i < n; i++) need += wcslen(lines[i]) + 2;
    WCHAR *out = malloc(need * sizeof(WCHAR));
    if(out){
        size_t o = 0;
        for(int i = 0; i < n; i++){
            size_t ll = wcslen(lines[i]);
            memcpy(out + o, lines[i], ll * sizeof(WCHAR));
            o += ll;
            if(i + 1 < n){ out[o++] = L'\r'; out[o++] = L'\n'; }
        }
        out[o] = L'\0';
        SendMessageW(g_np.editBox, EM_SETSEL, start, endNext);
        SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)out);
        free(out);
    }
    for(int i = 0; i < n; i++) free(lines[i]);
    free(lines);
    free(buf);
}

/* -- Math evaluator (recursive descent: + - * / % ^ parens, integers) -- */
static void np_math_skip(const WCHAR **pp)
{
    while(**pp == L' ' || **pp == L'\t') (*pp)++;
}

static long long np_math_factor(const WCHAR **pp, int *ok);

static long long np_math_power(const WCHAR **pp, int *ok)
{
    long long base = np_math_factor(pp, ok);
    if(!*ok) return 0;
    np_math_skip(pp);
    if(**pp == L'^'){
        (*pp)++;
        long long exp = np_math_power(pp, ok);
        if(!*ok) return 0;
        if(exp < 0 || exp > 30){ *ok = 0; return 0; }
        long long r = 1;
        for(long long i = 0; i < exp; i++) r *= base;
        return r;
    }
    return base;
}

static long long np_math_term(const WCHAR **pp, int *ok)
{
    long long left = np_math_power(pp, ok);
    for(;;){
        if(!*ok) return 0;
        np_math_skip(pp);
        WCHAR op = **pp;
        if(op != L'*' && op != L'/' && op != L'%') return left;
        (*pp)++;
        long long right = np_math_power(pp, ok);
        if(!*ok) return 0;
        if(op == L'*') left = left * right;
        else if(right == 0){ *ok = 0; return 0; }
        else if(op == L'/') left = left / right;
        else left = left % right;
    }
}

static long long np_math_expr(const WCHAR **pp, int *ok)
{
    long long left = np_math_term(pp, ok);
    for(;;){
        if(!*ok) return 0;
        np_math_skip(pp);
        WCHAR op = **pp;
        if(op != L'+' && op != L'-') return left;
        (*pp)++;
        long long right = np_math_term(pp, ok);
        if(!*ok) return 0;
        left = (op == L'+') ? left + right : left - right;
    }
}

static long long np_math_factor(const WCHAR **pp, int *ok)
{
    np_math_skip(pp);
    if(**pp == L'('){
        (*pp)++;
        long long v = np_math_expr(pp, ok);
        np_math_skip(pp);
        if(**pp != L')'){ *ok = 0; return 0; }
        (*pp)++;
        return v;
    }
    if(**pp == L'-'){ (*pp)++; return -np_math_factor(pp, ok); }
    if(**pp == L'+'){ (*pp)++; return np_math_factor(pp, ok); }
    if(!iswdigit(**pp)){ *ok = 0; return 0; }
    long long v = 0;
    while(iswdigit(**pp)){ v = v * 10 + (**pp - L'0'); (*pp)++; }
    return v;
}

/* Evaluate the selection (or the expression around the caret) and replace
 * it with the result. */
static void np_eval_math(void)
{
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    int a = (int)s, b = (int)e;
    if(a == b){
        while(a > 0 && (iswdigit(buf[a - 1]) || wcschr(L"+-*/%^(). \t", buf[a - 1])))
            a--;
        while(b < len && (iswdigit(buf[b]) || wcschr(L"+-*/%^(). \t", buf[b])))
            b++;
        while(a < b && (buf[a] == L' ' || buf[a] == L'\t')) a++;
        while(b > a && (buf[b - 1] == L' ' || buf[b - 1] == L'\t')) b--;
        if(a == b){ free(buf); return; }
    }
    int el = b - a;
    WCHAR *expr = malloc(((size_t)el + 1) * sizeof(WCHAR));
    if(!expr){ free(buf); return; }
    memcpy(expr, buf + a, (size_t)el * sizeof(WCHAR));
    expr[el] = L'\0';
    free(buf);
    const WCHAR *p = expr;
    int ok = 1;
    long long v = np_math_expr(&p, &ok);
    np_math_skip(&p);
    if(!ok || *p){
        free(expr);
        MessageBoxW(g_np.mainWnd, L"Not a valid expression.",
                    S_APP, MB_OK | MB_ICONWARNING);
        return;
    }
    wchar_t out[64];
    swprintf_s(out, 64, L"%lld", v);
    free(expr);
    SendMessageW(g_np.editBox, EM_SETSEL, a, b);
    SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)out);
}

/* ==========================================================================
 *  Edit subclass: pairs, smart backspace, macro hooks, carets, hover
 * ==========================================================================
 *  One subclass proc on the EDIT control (re-attached whenever word-wrap
 *  recreates it). All power typing behavior funnels through here. */

static void np_edit_wire(HWND edit)
{
    SetWindowSubclass(edit, np_edit_subproc, 1, 0);
    LRESULT mask = SendMessageW(edit, EM_GETEVENTMASK, 0, 0);
    SendMessageW(edit, EM_SETEVENTMASK, 0, mask | ENM_LINK | ENM_CHANGE);
    /* Breathing room + a 2px modern caret. */
    SendMessageW(edit, EM_SETMARGINS, EC_LEFTMARGIN | EC_RIGHTMARGIN,
                 MAKELPARAM(6, 6));
    SendMessageW(edit, EM_SETCARETWIDTH, 0, 2);
    {
        int large = (g_np.tabs && g_np.tabActive >= 0 &&
                     g_np.tabActive < g_np.tabCount &&
                     g_np.tabs[g_np.tabActive].largeFile);
        SendMessageW(edit, EM_AUTOURLDETECT, large ? 0 : 1, 0);
    }
}

static int np_is_word_char(WCHAR c)
{
    return iswalnum(c) || c == L'_';
}

static LRESULT CALLBACK np_edit_subproc(HWND h, UINT m, WPARAM w, LPARAM l,
                                        UINT_PTR id, DWORD_PTR ref)
{
    (void)id; (void)ref;
    if(h != g_np.editBox && h != g_np.splitEdit)
        return DefSubclassProc(h, m, w, l);

    if(m == WM_SETFOCUS && h == g_np.editBox){
        COLORREF bg = 0, fg = 0;
        int dark = 0;
        np_effective_colors(&bg, &fg, &dark);
        g_np.caretFocus = dark ? 1 : 0;
        g_np.caretPhase = 0;
        if(dark) SetTimer(g_np.mainWnd, IDT_CARET, 500, NULL);
        else KillTimer(g_np.mainWnd, IDT_CARET);
        return DefSubclassProc(h, m, w, l);
    }
    if(m == WM_KILLFOCUS && h == g_np.editBox){
        g_np.caretFocus = 0;
        KillTimer(g_np.mainWnd, IDT_CARET);
        if(g_np.caretPhase){
            g_np.caretPhase = 0;
            RECT r;
            np_caret_pos(&r);
            InvalidateRect(h, &r, FALSE);
        }
        return DefSubclassProc(h, m, w, l);
    }
    if(m == WM_LBUTTONDOWN){
        int ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
        int alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
        if(alt && h == g_np.editBox){
            POINTL ptl;
            ptl.x = (LONG)(short)LOWORD(l);
            ptl.y = (LONG)(short)HIWORD(l);
            int idx = (int)SendMessageW(h, EM_CHARFROMPOS, 0, (LPARAM)&ptl);
            np_color_at(idx, 1);
            return 0;
        }
        if(ctrl && h == g_np.editBox){
            POINTL ptl;
            ptl.x = (LONG)(short)LOWORD(l);
            ptl.y = (LONG)(short)HIWORD(l);
            int idx = (int)SendMessageW(h, EM_CHARFROMPOS, 0, (LPARAM)&ptl);
            if(g_np.caretCount < NP_MAX_CARETS){
                g_np.carets[g_np.caretCount++] = idx;
                np_update_status();
            }
            return 0;
        }
        if(g_np.caretCount > 1) np_multicaret_clear();
        np_ac_cancel();
        return DefSubclassProc(h, m, w, l);
    }
    if(m == WM_MOUSEMOVE && h == g_np.editBox){
        DWORD now = GetTickCount();
        if(now - g_np.tipTick > 300){
            g_np.tipTick = now;
            POINTL ptl;
            ptl.x = (LONG)(short)LOWORD(l);
            ptl.y = (LONG)(short)HIWORD(l);
            int idx = (int)SendMessageW(h, EM_CHARFROMPOS, 0, (LPARAM)&ptl);
            np_color_hover(idx);
        }
        return DefSubclassProc(h, m, w, l);
    }
    if(m == WM_KEYDOWN){
        if(g_np.acActive){
            if(w == VK_UP || w == VK_DOWN){
                HWND list = g_np.acList;
                int n = (int)SendMessageW(list, LB_GETCOUNT, 0, 0);
                int cur = (int)SendMessageW(list, LB_GETCURSEL, 0, 0);
                if(w == VK_DOWN) cur++;
                else cur--;
                if(cur < 0) cur = 0;
                if(cur >= n) cur = n - 1;
                SendMessageW(list, LB_SETCURSEL, cur, 0);
                return 0;
            }
            if(w == VK_RETURN || w == VK_TAB){ np_ac_accept(); return 0; }
            if(w == VK_ESCAPE){ np_ac_cancel(); return 0; }
        }
        if(w == VK_ESCAPE){
            if(g_np.caretCount > 1){ np_multicaret_clear(); return 0; }
            if(g_np.tipWnd) np_tip_hide();
            return DefSubclassProc(h, m, w, l);
        }
        if(w == VK_BACK && g_np.autoPairs && h == g_np.editBox &&
           !g_np.editGuard){
            DWORD s = 0, e = 0;
            SendMessageW(h, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
            if(s == e && s > 0){
                int len = GetWindowTextLengthW(h);
                WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
                if(buf){
                    GetWindowTextW(h, buf, len + 1);
                    WCHAR prev = buf[s - 1];
                    WCHAR next = (s < (DWORD)len) ? buf[s] : 0;
                    int isPair = (prev == L'(' && next == L')') ||
                                 (prev == L'[' && next == L']') ||
                                 (prev == L'{' && next == L'}') ||
                                 (prev == L'"' && next == L'"') ||
                                 (prev == L'\'' && next == L'\'');
                    if(isPair){
                        /* Auto-delete the pair together. */
                        SendMessageW(h, EM_SETSEL, s - 1, s + 1);
                        SendMessageW(h, EM_REPLACESEL, TRUE, (LPARAM)L"");
                        if(g_np.recording) np_macro_record_key(VK_BACK);
                        free(buf);
                        return 0;
                    }
                    /* Smart backspace: spaces back to the 4-tab stop. */
                    if(prev == L' '){
                        int line = (int)SendMessageW(h, EM_LINEFROMCHAR, s, 0);
                        int ls = (int)SendMessageW(h, EM_LINEINDEX, line, 0);
                        int col = (int)s - ls;
                        int back = col % 4;
                        if(back == 0) back = 4;
                        int k = 0;
                        while(k < back && (int)s - 1 - k >= ls &&
                              buf[s - 1 - k] == L' ') k++;
                        if(k > 1){
                            SendMessageW(h, EM_SETSEL, s - k, s);
                            SendMessageW(h, EM_REPLACESEL, TRUE, (LPARAM)L"");
                            if(g_np.recording) np_macro_record_key(VK_BACK);
                            free(buf);
                            return 0;
                        }
                    }
                    free(buf);
                }
            }
        }
        if(g_np.recording && h == g_np.editBox &&
           (w == VK_BACK || w == VK_DELETE || w == VK_RETURN ||
            w == VK_TAB || w == VK_LEFT || w == VK_RIGHT ||
            w == VK_UP || w == VK_DOWN || w == VK_HOME || w == VK_END))
            np_macro_record_key((int)w);
        if(g_np.caretCount > 1 && h == g_np.editBox &&
           (w == VK_BACK || w == VK_DELETE || w == VK_LEFT ||
            w == VK_RIGHT || w == VK_UP || w == VK_DOWN ||
            w == VK_HOME || w == VK_END)){
            np_multicaret_apply((int)w, 0);
            return 0;
        }
        return DefSubclassProc(h, m, w, l);
    }
    if(m == WM_CHAR){
        if(w == VK_TAB && g_np.acActive){ np_ac_accept(); return 0; }
        if(g_np.recording && h == g_np.editBox && w >= 32 && w != 127)
            np_macro_record_char((WCHAR)w);
        if(g_np.caretCount > 1 && h == g_np.editBox && w >= 32){
            np_multicaret_apply(0, (WCHAR)w);
            return 0;
        }
        if(g_np.autoPairs && h == g_np.editBox && !g_np.editGuard){
            if(w == L'(' || w == L'[' || w == L'{'){
                WCHAR pair[3];
                pair[0] = (WCHAR)w;
                pair[1] = (w == L'(') ? L')' : (w == L'[') ? L']' : L'}';
                pair[2] = L'\0';
                DWORD s = 0, e = 0;
                SendMessageW(h, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
                SendMessageW(h, EM_REPLACESEL, TRUE, (LPARAM)pair);
                SendMessageW(h, EM_SETSEL, s + 1, s + 1);
                return 0;
            }
            if(w == L'"' || w == L'\''){
                DWORD s = 0, e = 0;
                SendMessageW(h, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
                int len = GetWindowTextLengthW(h);
                WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
                if(buf){
                    GetWindowTextW(h, buf, len + 1);
                    WCHAR next = (s < (DWORD)len) ? buf[s] : 0;
                    WCHAR prev = (s > 0) ? buf[s - 1] : 0;
                    free(buf);
                    if(next == w){
                        SendMessageW(h, EM_SETSEL, s + 1, s + 1);
                        return 0;
                    }
                    if(s == e && (s == (DWORD)len || next == L' ' ||
                       next == L'\t' || next == L'\r' || next == L'\n') &&
                       (s == 0 || (!np_is_word_char(prev) && prev != L'\\'))){
                        WCHAR pair[3];
                        pair[0] = pair[1] = (WCHAR)w;
                        pair[2] = L'\0';
                        SendMessageW(h, EM_REPLACESEL, TRUE, (LPARAM)pair);
                        SendMessageW(h, EM_SETSEL, s + 1, s + 1);
                        return 0;
                    }
                }
            }
            if((w == L')' || w == L']' || w == L'}')){
                DWORD s = 0, e = 0;
                SendMessageW(h, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
                int len = GetWindowTextLengthW(h);
                WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
                if(buf){
                    GetWindowTextW(h, buf, len + 1);
                    WCHAR next = (s < (DWORD)len) ? buf[s] : 0;
                    free(buf);
                    if(s == e && next == w){
                        SendMessageW(h, EM_SETSEL, s + 1, s + 1);
                        return 0;
                    }
                }
            }
        }
        LRESULT r = DefSubclassProc(h, m, w, l);
        if(h == g_np.editBox && !g_np.editGuard &&
           (np_is_word_char((WCHAR)w) || w == VK_BACK))
            np_auto_complete(0);
        return r;
    }
    return DefSubclassProc(h, m, w, l);
}

/* ==========================================================================
 *  Autocomplete popup + persistent macro recorder
 * ========================================================================== */

/* Collect up to 400 unique document words with the given prefix (case-
 * insensitive). Returns the count; exact-prefix word sorts first. */
static int np_ac_collect(const WCHAR *prefix, wchar_t words[][64], int max)
{
    int len = GetWindowTextLengthW(g_np.editBox);
    if(len > 8 * 1024 * 1024) len = 8 * 1024 * 1024;
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return 0;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    size_t pl = wcslen(prefix);
    int n = 0;
    int i = 0;
    while(i < len && n < max){
        while(i < len && !np_is_word_char(buf[i])) i++;
        int a = i;
        while(i < len && np_is_word_char(buf[i])) i++;
        int wl = i - a;
        if(wl >= 3 && wl < 64){
            wchar_t w[64];
            memcpy(w, buf + a, (size_t)wl * sizeof(WCHAR));
            w[wl] = L'\0';
            if(pl == 0 || _wcsnicmp(w, prefix, pl) == 0){
                int dup = 0;
                for(int k = 0; k < n; k++)
                    if(_wcsicmp(words[k], w) == 0){ dup = 1; break; }
                if(!dup){
                    wcsncpy_s(words[n], 64, w, _TRUNCATE);
                    n++;
                }
            }
        }
    }
    free(buf);
    for(int a = 0; a < n; a++)
        for(int b = a + 1; b < n; b++){
            int sa = (_wcsicmp(words[a], prefix) == 0) ? 0 : 1;
            int sb = (_wcsicmp(words[b], prefix) == 0) ? 0 : 1;
            if(sa > sb || (sa == sb && _wcsicmp(words[a], words[b]) > 0)){
                wchar_t t[64];
                wcsncpy_s(t, 64, words[a], _TRUNCATE);
                wcsncpy_s(words[a], 64, words[b], _TRUNCATE);
                wcsncpy_s(words[b], 64, t, _TRUNCATE);
            }
        }
    return n;
}

static void np_ac_destroy(void)
{
    if(g_np.acPopup){ DestroyWindow(g_np.acPopup); g_np.acPopup = NULL; }
    g_np.acList = NULL;
    g_np.acActive = 0;
}

static void np_ac_cancel(void)
{
    np_ac_destroy();
}

/* Show (or refresh) the completion list for the caret word. force=1 shows
 * even short/empty prefixes (Ctrl+Space). */
static void np_auto_complete(int force)
{
    if(g_np.editGuard || !g_np.editBox) return;
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    if(s != e){ np_ac_cancel(); return; }
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    int b = (int)s, a = b;
    while(a > 0 && np_is_word_char(buf[a - 1])) a--;
    int wlen = b - a;
    wchar_t prefix[64];
    if(wlen >= 64) wlen = 63;
    memcpy(prefix, buf + a, (size_t)wlen * sizeof(WCHAR));
    prefix[wlen] = L'\0';
    free(buf);
    if(wlen < 3 && !force){ np_ac_cancel(); return; }

    static wchar_t words[400][64];
    int n = np_ac_collect(prefix, words, 400);
    if(n == 0){ np_ac_cancel(); return; }
    if(!force && n == 1 && _wcsicmp(words[0], prefix) == 0){
        np_ac_cancel();
        return;
    }
    if(!g_np.acPopup){
        g_np.acPopup = CreateWindowExW(WS_EX_TOPMOST, L"ListBox", NULL,
            WS_POPUP | WS_BORDER | WS_VSCROLL | LBS_NOTIFY,
            0, 0, 220, 130, g_np.mainWnd, NULL,
            GetModuleHandleW(NULL), NULL);
        if(!g_np.acPopup) return;
        g_np.acList = g_np.acPopup;
        HFONT hf = (HFONT)SendMessageW(g_np.editBox, WM_GETFONT, 0, 0);
        if(hf) SendMessageW(g_np.acPopup, WM_SETFONT, (WPARAM)hf, TRUE);
    }
    SendMessageW(g_np.acPopup, LB_RESETCONTENT, 0, 0);
    for(int i = 0; i < n; i++)
        SendMessageW(g_np.acPopup, LB_ADDSTRING, 0, (LPARAM)words[i]);
    SendMessageW(g_np.acPopup, LB_SETCURSEL, 0, 0);
    DWORD pos = (DWORD)SendMessageW(g_np.editBox, EM_POSFROMCHAR, s, 0);
    POINT pt;
    pt.x = (int)(short)LOWORD(pos);
    pt.y = (int)(short)HIWORD(pos);
    {
        SIZE sz;
        HDC dc = GetDC(g_np.editBox);
        HFONT hf = (HFONT)SendMessageW(g_np.editBox, WM_GETFONT, 0, 0);
        HFONT old = SelectObject(dc, hf);
        GetTextExtentPoint32W(dc, L"Ay", 2, &sz);
        SelectObject(dc, old);
        ReleaseDC(g_np.editBox, dc);
        pt.y += sz.cy + 2;
    }
    MapWindowPoints(g_np.editBox, g_np.mainWnd, &pt, 1);
    int rows = n > 8 ? 8 : n;
    SetWindowPos(g_np.acPopup, HWND_TOPMOST, pt.x, pt.y, 220,
                 rows * 16 + 4, SWP_SHOWWINDOW);
    g_np.acActive = 1;
}

static void np_ac_accept(void)
{
    if(!g_np.acActive || !g_np.acPopup) return;
    int cur = (int)SendMessageW(g_np.acPopup, LB_GETCURSEL, 0, 0);
    wchar_t word[64] = {0};
    if(cur >= 0)
        SendMessageW(g_np.acPopup, LB_GETTEXT, cur, (LPARAM)word);
    np_ac_destroy();
    if(!word[0]) return;
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    int a = (int)s;
    while(a > 0 && np_is_word_char(buf[a - 1])) a--;
    free(buf);
    SendMessageW(g_np.editBox, EM_SETSEL, a, s);
    SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)word);
    SetFocus(g_np.editBox);
}

/* -- Macro recorder: keystrokes persist across restarts -- */
static void np_macro_op(int kind, int key)
{
    if(g_np.macroCount >= NP_MAX_MACRO_OPS) return;
    if(g_np.macroCount >= g_np.macroCap){
        int cap = g_np.macroCap ? g_np.macroCap * 2 : 256;
        NP_MACRO_OP *no = realloc(g_np.macro, (size_t)cap * sizeof(NP_MACRO_OP));
        if(!no) return;
        g_np.macro = no;
        g_np.macroCap = cap;
    }
    g_np.macro[g_np.macroCount].kind = kind;
    g_np.macro[g_np.macroCount].key = key;
    g_np.macro[g_np.macroCount].text = NULL;
    g_np.macroCount++;
}

static void np_macro_record_char(WCHAR ch)
{
    np_macro_op(0, (int)ch);
}

static void np_macro_record_key(int vk)
{
    switch(vk){
    case VK_BACK: np_macro_op(1, vk); break;
    case VK_DELETE: np_macro_op(2, vk); break;
    case VK_RETURN: np_macro_op(3, vk); break;
    case VK_TAB: np_macro_op(4, vk); break;
    case VK_LEFT: np_macro_op(5, vk); break;
    case VK_RIGHT: np_macro_op(6, vk); break;
    case VK_UP: np_macro_op(7, vk); break;
    case VK_DOWN: np_macro_op(8, vk); break;
    case VK_HOME: np_macro_op(9, vk); break;
    case VK_END: np_macro_op(10, vk); break;
    default: break;
    }
}

static void np_macro_toggle(void)
{
    if(g_np.recording){
        g_np.recording = 0;
        np_macro_save();
        np_announce(L"Macro recording stopped");
    }else{
        np_macro_free();
        g_np.recording = 1;
        np_announce(L"Macro recording started");
    }
    np_refresh_title();
}

static void np_macro_play(void)
{
    if(g_np.recording){
        g_np.recording = 0;
        np_macro_save();
        np_refresh_title();
    }
    if(g_np.macroCount == 0){
        MessageBoxW(g_np.mainWnd, L"No macro recorded.",
                    S_APP, MB_OK | MB_ICONINFORMATION);
        return;
    }
    np_multicaret_clear();
    SetFocus(g_np.editBox);
    for(int i = 0; i < g_np.macroCount; i++){
        NP_MACRO_OP *op = &g_np.macro[i];
        if(op->kind == 0){
            wchar_t ch[2];
            ch[0] = (WCHAR)op->key; ch[1] = L'\0';
            SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)ch);
        }else if(op->kind == 3){
            SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)L"\r\n");
        }else if(op->kind == 4){
            SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)L"\t");
        }else if(op->kind == 1){
            SendMessageW(g_np.editBox, WM_KEYDOWN, VK_BACK, 0);
        }else if(op->kind == 2){
            SendMessageW(g_np.editBox, WM_KEYDOWN, VK_DELETE, 0);
        }else if(op->kind >= 5 && op->kind <= 10){
            static const int vks[] = { 0, 0, 0, 0, 0,
                VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_HOME, VK_END };
            SendMessageW(g_np.editBox, WM_KEYDOWN, vks[op->kind], 0);
        }
    }
}

static void np_macro_path(wchar_t *out, size_t outChars)
{
    wchar_t dir[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    swprintf_s(out, outChars, L"%s\\macro.bin", dir[0] ? dir : L".");
}

static void np_macro_save(void)
{
    wchar_t path[NP_MAX_PATH];
    np_macro_path(path, NP_MAX_PATH);
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                           FILE_ATTRIBUTE_NORMAL, NULL);
    if(h == INVALID_HANDLE_VALUE) return;
    DWORD w = 0, n = (DWORD)g_np.macroCount;
    WriteFile(h, &n, sizeof(n), &w, NULL);
    for(int i = 0; i < g_np.macroCount; i++){
        int kind = g_np.macro[i].kind, key = g_np.macro[i].key;
        WriteFile(h, &kind, sizeof(kind), &w, NULL);
        WriteFile(h, &key, sizeof(key), &w, NULL);
    }
    CloseHandle(h);
}

static void np_macro_load(void)
{
    np_macro_free();
    wchar_t path[NP_MAX_PATH];
    np_macro_path(path, NP_MAX_PATH);
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL,
                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if(h == INVALID_HANDLE_VALUE) return;
    DWORD r = 0, n = 0;
    if(!ReadFile(h, &n, sizeof(n), &r, NULL) || r != sizeof(n) ||
       n > NP_MAX_MACRO_OPS){
        CloseHandle(h);
        return;
    }
    for(DWORD i = 0; i < n; i++){
        int kind = 0, key = 0;
        if(!ReadFile(h, &kind, sizeof(kind), &r, NULL) || r != sizeof(kind)) break;
        if(!ReadFile(h, &key, sizeof(key), &r, NULL) || r != sizeof(key)) break;
        if(kind == 0) np_macro_op(0, key);
        else if(kind >= 1 && kind <= 10) np_macro_op(kind, key);
    }
    CloseHandle(h);
}

static void np_macro_free(void)
{
    free(g_np.macro);
    g_np.macro = NULL;
    g_np.macroCount = 0;
    g_np.macroCap = 0;
}

/* ==========================================================================
 *  Links, hover tooltip, color picker, multi-caret, column ops
 * ========================================================================== */

/* Single tracking tooltip shared by URL hover and hex-color hover. */
static void np_tip_ensure(void)
{
    if(g_np.tipWnd) return;
    g_np.tipWnd = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, NULL,
        WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
        CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
        g_np.mainWnd, NULL, GetModuleHandleW(NULL), NULL);
    if(!g_np.tipWnd) return;
    TOOLINFOW ti;
    memset(&ti, 0, sizeof(ti));
    ti.cbSize = sizeof(ti);
    ti.uFlags = TTF_TRACK | TTF_ABSOLUTE;
    ti.hwnd = g_np.mainWnd;
    ti.lpszText = L"";
    SendMessageW(g_np.tipWnd, TTM_ADDTOOLW, 0, (LPARAM)&ti);
    np_tip_recolor();
}

static void np_tip_recolor(void)
{
    COLORREF bg = 0, fg = 0;
    int dark = 0;
    if(!g_np.tipWnd) return;
    np_effective_colors(&bg, &fg, &dark);
    if(dark || g_np.themeName[0]){
        SendMessageW(g_np.tipWnd, TTM_SETTIPBKCOLOR, bg, 0);
        SendMessageW(g_np.tipWnd, TTM_SETTIPTEXTCOLOR, fg, 0);
    }else{
        SendMessageW(g_np.tipWnd, TTM_SETTIPBKCOLOR,
                     GetSysColor(COLOR_INFOBK), 0);
        SendMessageW(g_np.tipWnd, TTM_SETTIPTEXTCOLOR,
                     GetSysColor(COLOR_INFOTEXT), 0);
    }
}

static void np_tip_show(const WCHAR *text, int sx, int sy)
{
    np_tip_ensure();
    if(!g_np.tipWnd) return;
    TOOLINFOW ti;
    memset(&ti, 0, sizeof(ti));
    ti.cbSize = sizeof(ti);
    ti.hwnd = g_np.mainWnd;
    SendMessageW(g_np.tipWnd, TTM_UPDATETIPTEXTW, 0, (LPARAM)&ti);
    {
        /* Update text via the AddTool slot (id 0). */
        TOOLINFOW set;
        memset(&set, 0, sizeof(set));
        set.cbSize = sizeof(set);
        set.hwnd = g_np.mainWnd;
        set.uId = 0;
        set.lpszText = (LPWSTR)text;
        SendMessageW(g_np.tipWnd, TTM_SETTOOLINFOW, 0, (LPARAM)&set);
    }
    SendMessageW(g_np.tipWnd, TTM_TRACKPOSITION, 0, MAKELPARAM(sx, sy + 18));
    SendMessageW(g_np.tipWnd, TTM_TRACKACTIVATE, TRUE, (LPARAM)&ti);
    g_np.tipArmed = 1;
}

static void np_tip_hide(void)
{
    if(!g_np.tipWnd) return;
    TOOLINFOW ti;
    memset(&ti, 0, sizeof(ti));
    ti.cbSize = sizeof(ti);
    ti.hwnd = g_np.mainWnd;
    SendMessageW(g_np.tipWnd, TTM_TRACKACTIVATE, FALSE, (LPARAM)&ti);
    g_np.tipArmed = 0;
}

/* EN_LINK notifications from the edit control: click opens, hover previews. */
static void np_link_notify(NMHDR *hdr, LPARAM lParam)
{
    ENLINK *link = (ENLINK *)lParam;
    (void)hdr;
    if(link->msg == WM_LBUTTONDOWN){
        TEXTRANGEW tr;
        int n = link->chrg.cpMax - link->chrg.cpMin;
        if(n <= 0 || n > 2048) return;
        WCHAR *url = malloc(((size_t)n + 1) * sizeof(WCHAR));
        if(!url) return;
        tr.chrg = link->chrg;
        tr.lpstrText = url;
        SendMessageW(g_np.editBox, EM_GETTEXTRANGE, 0, (LPARAM)&tr);
        ShellExecuteW(NULL, L"open", url, NULL, NULL, SW_SHOWNORMAL);
        free(url);
    }else if(link->msg == WM_MOUSEMOVE){
        TEXTRANGEW tr;
        int n = link->chrg.cpMax - link->chrg.cpMin;
        if(n <= 0 || n > 2048){ np_tip_hide(); return; }
        WCHAR *url = malloc(((size_t)n + 1) * sizeof(WCHAR));
        if(!url) return;
        tr.chrg = link->chrg;
        tr.lpstrText = url;
        SendMessageW(g_np.editBox, EM_GETTEXTRANGE, 0, (LPARAM)&tr);
        POINT pt;
        GetCursorPos(&pt);
        np_tip_show(url, pt.x, pt.y);
        free(url);
    }
}

/* Read one line (no terminator) into buf. Returns the line length. */
static int np_get_line_text(HWND edit, int line, WCHAR *buf, int bufChars)
{
    int start = (int)SendMessageW(edit, EM_LINEINDEX, line, 0);
    if(start < 0) return 0;
    int len = (int)SendMessageW(edit, EM_LINELENGTH, start, 0);
    if(len >= bufChars) len = bufChars - 1;
    *(WORD *)buf = (WORD)bufChars;
    int got = (int)SendMessageW(edit, EM_GETLINE, line, (LPARAM)buf);
    if(got < 0) got = 0;
    buf[got] = L'\0';
    return got;
}

/* Is there a #RRGGBB hex code covering doc offset idx? Fills rgb on match. */
static int np_hex_at(const WCHAR *lineText, int colInLine, COLORREF *rgb)
{
    int len = (int)wcslen(lineText);
    for(int s = (colInLine > 6 ? colInLine - 6 : 0);
        s <= colInLine && s + 7 <= len; s++){
        if(lineText[s] != L'#') continue;
        int ok = 1, v = 0;
        for(int k = 1; k <= 6; k++){
            WCHAR c = lineText[s + k];
            v <<= 4;
            if(c >= L'0' && c <= L'9') v |= c - L'0';
            else if(c >= L'a' && c <= L'f') v |= c - L'a' + 10;
            else if(c >= L'A' && c <= L'F') v |= c - L'A' + 10;
            else{ ok = 0; break; }
        }
        if(!ok) continue;
        if(s + 7 < len && np_is_word_char(lineText[s + 7])) continue;
        if(colInLine < s || colInLine > s + 6) continue;
        if(rgb) *rgb = RGB((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
        return s;
    }
    return -1;
}

/* Hover: show "RGB(...) - Alt+Click to edit" for hex codes under the mouse. */
static void np_color_hover(int idx)
{
    int line = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, idx, 0);
    int start = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, line, 0);
    WCHAR buf[512];
    if(!np_get_line_text(g_np.editBox, line, buf, 512)){ np_tip_hide(); return; }
    COLORREF rgb = 0;
    if(np_hex_at(buf, idx - start, &rgb) < 0){ np_tip_hide(); return; }
    wchar_t tip[128];
    swprintf_s(tip, 128, L"RGB(%d, %d, %d) - Alt+Click to edit",
               GetRValue(rgb), GetGValue(rgb), GetBValue(rgb));
    POINT pt;
    GetCursorPos(&pt);
    np_tip_show(tip, pt.x, pt.y);
}

/* Alt+Click: open the color chooser on the hex code and replace it. */
static void np_color_at(int idx, int edit)
{
    (void)edit;
    int line = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, idx, 0);
    int start = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, line, 0);
    WCHAR buf[512];
    if(!np_get_line_text(g_np.editBox, line, buf, 512)) return;
    COLORREF rgb = 0;
    int s = np_hex_at(buf, idx - start, &rgb);
    if(s < 0) return;
    static DWORD custom[16] = {0};
    CHOOSECOLORW cc;
    memset(&cc, 0, sizeof(cc));
    cc.lStructSize = sizeof(cc);
    cc.hwndOwner = g_np.mainWnd;
    cc.rgbResult = rgb;
    cc.lpCustColors = custom;
    cc.Flags = CC_FULLOPEN | CC_RGBINIT;
    if(!ChooseColorW(&cc)) return;
    wchar_t hex[16];
    swprintf_s(hex, 16, L"#%02X%02X%02X",
               GetRValue(cc.rgbResult), GetGValue(cc.rgbResult),
               GetBValue(cc.rgbResult));
    SendMessageW(g_np.editBox, EM_SETSEL, start + s, start + s + 7);
    SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)hex);
}

/* -- Multi-caret: Ctrl+click adds carets; typing/deleting hits them all.
 * Only the main caret is drawn (EDIT control limit); the status bar shows
 * the live caret count, and every text outcome is exact. -- */
static void np_multicaret_clear(void)
{
    if(g_np.caretCount > 1){
        g_np.caretCount = 0;
        np_update_status();
    }else{
        g_np.caretCount = 0;
    }
}

/* carets[] holds EXTRA positions; the live caret is the main one. vk=0 for
 * character input (ch valid), else a navigation/edit virtual key. */
static void np_multicaret_apply(int vk, WCHAR ch)
{
    if(g_np.caretCount == 0) return;
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    if(s != e){ np_multicaret_clear(); return; }
    int main = (int)s;
    /* Gather all positions, highest first so offsets stay valid. */
    int pos[NP_MAX_CARETS + 1];
    int n = 0;
    pos[n++] = main;
    for(int i = 0; i < g_np.caretCount && n < NP_MAX_CARETS + 1; i++){
        int dup = 0;
        for(int k = 0; k < n; k++) if(pos[k] == g_np.carets[i]){ dup = 1; break; }
        if(!dup) pos[n++] = g_np.carets[i];
    }
    for(int i = 0; i < n; i++)
        for(int j = i + 1; j < n; j++)
            if(pos[j] > pos[i]){ int t = pos[i]; pos[i] = pos[j]; pos[j] = t; }
    int len = GetWindowTextLengthW(g_np.editBox);
    if(vk == 0){
        wchar_t ins[2];
        ins[0] = ch; ins[1] = L'\0';
        g_np.editGuard = 1;
        for(int i = 0; i < n; i++){
            SendMessageW(g_np.editBox, EM_SETSEL, pos[i], pos[i]);
            SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)ins);
        }
        g_np.editGuard = 0;
        /* Everything shifted +1; the live caret ends after the main char. */
        for(int i = 0; i < g_np.caretCount; i++) g_np.carets[i]++;
        SendMessageW(g_np.editBox, EM_SETSEL, main + 1, main + 1);
    }else if(vk == VK_BACK){
        g_np.editGuard = 1;
        for(int i = 0; i < n; i++){
            if(pos[i] <= 0) continue;
            SendMessageW(g_np.editBox, EM_SETSEL, pos[i] - 1, pos[i]);
            SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)L"");
            for(int k = 0; k < n; k++)
                if(pos[k] > pos[i] - 1) pos[k]--;
        }
        g_np.editGuard = 0;
        for(int i = 0, k = 0; i < g_np.caretCount; i++){
            /* positions were mutated in pos[]; recompute simply: -1 each */
            (void)k;
            if(g_np.carets[i] > 0) g_np.carets[i]--;
        }
        SendMessageW(g_np.editBox, EM_SETSEL, main > 0 ? main - 1 : 0,
                     main > 0 ? main - 1 : 0);
    }else if(vk == VK_DELETE){
        g_np.editGuard = 1;
        for(int i = 0; i < n; i++){
            if(pos[i] >= len) continue;
            SendMessageW(g_np.editBox, EM_SETSEL, pos[i], pos[i] + 1);
            SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)L"");
            for(int k = 0; k < n; k++)
                if(pos[k] > pos[i]) pos[k]--;
        }
        g_np.editGuard = 0;
        SendMessageW(g_np.editBox, EM_SETSEL, main, main);
    }else{
        /* Arrows/home/end collapse back to one caret. */
        np_multicaret_clear();
    }
    (void)len;
}

/* -- Small two-field input dialog (column ops), built in memory -- */
typedef struct {
    const WCHAR *title;
    const WCHAR *label1;
    const WCHAR *def1;
    const WCHAR *label2;
    const WCHAR *def2;
    WCHAR out1[64];
    WCHAR out2[64];
} NP_INPUT;

static BYTE *np_in_dlg_align(BYTE *p)
{
    return (BYTE *)(((uintptr_t)p + 3u) & ~(uintptr_t)3u);
}

static INT_PTR CALLBACK np_input_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    NP_INPUT *in;
    switch(msg){
    case WM_INITDIALOG:
        in = (NP_INPUT *)l;
        SetWindowLongPtrW(dlg, DWLP_USER, (LONG_PTR)in);
        SetWindowTextW(dlg, in->title);
        SetDlgItemTextW(dlg, 101, in->label1);
        SetDlgItemTextW(dlg, 102, in->def1);
        if(in->label2 && in->label2[0]){
            SetDlgItemTextW(dlg, 103, in->label2);
            SetDlgItemTextW(dlg, 104, in->def2);
        }else{
            ShowWindow(GetDlgItem(dlg, 103), SW_HIDE);
            ShowWindow(GetDlgItem(dlg, 104), SW_HIDE);
        }
        SetFocus(GetDlgItem(dlg, 102));
        SendDlgItemMessageW(dlg, 102, EM_SETSEL, 0, (LPARAM)-1);
        np_dark_dlg_init(dlg);
        return FALSE;
    case WM_CTLCOLORDLG: {
        LRESULT br = np_dark_ctl((HDC)w, dlg);
        if(br) return br;
        break;
    }
    case WM_CTLCOLOREDIT:
    case WM_CTLCOLORSTATIC:
    case WM_CTLCOLORLISTBOX:
    case WM_CTLCOLORBTN: {
        LRESULT br = np_dark_ctl((HDC)w, (HWND)l);
        if(br) return br;
        break;
    }
    case WM_COMMAND:
        if(LOWORD(w) == IDOK){
            in = (NP_INPUT *)GetWindowLongPtrW(dlg, DWLP_USER);
            GetDlgItemTextW(dlg, 102, in->out1, 64);
            GetDlgItemTextW(dlg, 104, in->out2, 64);
            EndDialog(dlg, IDOK);
            return TRUE;
        }
        if(LOWORD(w) == IDCANCEL){ EndDialog(dlg, IDCANCEL); return TRUE; }
        break;
    }
    return FALSE;
}

/* Returns IDOK/IDCANCEL; out1/out2 always filled (def2 may be ""). */
static int np_input_box(const WCHAR *title, const WCHAR *label1,
                        const WCHAR *def1, const WCHAR *label2,
                        const WCHAR *def2, WCHAR *out1, WCHAR *out2)
{
    BYTE buf[1024];
    BYTE *p = buf;
    DLGTEMPLATE *dlg = (DLGTEMPLATE *)p;
    dlg->style = WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_SETFONT;
    dlg->dwExtendedStyle = 0;
    dlg->cdit = 6;
    dlg->x = 0; dlg->y = 0; dlg->cx = 210; dlg->cy = 92;
    p += sizeof(DLGTEMPLATE);
    *(WORD *)p = 0; p += 2;
    *(WORD *)p = 0; p += 2;
    *(WORD *)p = 0; p += 2; /* caption set by proc */
    *(WORD *)p = 8; p += 2;
    wcscpy((WCHAR *)p, L"MS Shell Dlg");
    p += (wcslen(L"MS Shell Dlg") + 1) * sizeof(WCHAR);
    struct { DWORD st; int x, y, cx, cy; WORD id; WORD cls; const WCHAR *t; } items[] = {
        { WS_CHILD | WS_VISIBLE | SS_LEFT, 8, 8, 194, 8, 101, 0x82, L"" },
        { WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL, 8, 18, 194, 13, 102, 0x81, L"" },
        { WS_CHILD | WS_VISIBLE | SS_LEFT, 8, 36, 194, 8, 103, 0x82, L"" },
        { WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL, 8, 46, 194, 13, 104, 0x81, L"" },
        { WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON, 55, 70, 50, 14, IDOK, 0x80, L"OK" },
        { WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON, 110, 70, 50, 14, IDCANCEL, 0x80, L"Cancel" },
    };
    for(int i = 0; i < 6; i++){
        p = np_in_dlg_align(p);
        DLGITEMTEMPLATE *it = (DLGITEMTEMPLATE *)p;
        it->style = items[i].st;
        it->dwExtendedStyle = 0;
        it->x = (short)items[i].x; it->y = (short)items[i].y;
        it->cx = (short)items[i].cx; it->cy = (short)items[i].cy;
        it->id = items[i].id;
        p += sizeof(DLGITEMTEMPLATE);
        *(WORD *)p = 0xFFFF; p += 2;
        *(WORD *)p = items[i].cls; p += 2;
        wcscpy((WCHAR *)p, items[i].t);
        p += (wcslen(items[i].t) + 1) * sizeof(WCHAR);
        *(WORD *)p = 0; p += 2;
    }
    NP_INPUT in;
    memset(&in, 0, sizeof(in));
    in.title = title; in.label1 = label1; in.def1 = def1;
    in.label2 = label2; in.def2 = def2;
    INT_PTR r = DialogBoxIndirectParamW(GetModuleHandleW(NULL),
        (DLGTEMPLATE *)buf, g_np.mainWnd, np_input_proc, (LPARAM)&in);
    if(r == IDOK){
        wcsncpy_s(out1, 64, in.out1, _TRUNCATE);
        wcsncpy_s(out2, 64, in.out2, _TRUNCATE);
        return IDOK;
    }
    return IDCANCEL;
}

/* Column ops over the selected lines (0 = insert text, 1 = delete range,
 * 2 = copy range to clipboard). Columns are 1-based character columns. */
static void np_column_op(int op)
{
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    if(s == e){
        MessageBoxW(g_np.mainWnd, L"Select some lines first.",
                    S_APP, MB_OK | MB_ICONINFORMATION);
        return;
    }
    wchar_t f1[64] = {0}, f2[64] = {0};
    const WCHAR *title = L"Insert at Column";
    const WCHAR *cap1 = L"Column (1-based):";
    const WCHAR *cap2 = L"Text:";
    if(op == 1){ title = L"Delete Columns"; cap2 = L"To column:"; }
    if(op == 2){ title = L"Copy Columns"; cap2 = L"To column:"; }
    if(np_input_box(title, cap1, L"1", cap2, L"",
                    f1, f2) != IDOK)
        return;
    int c1 = _wtoi(f1);
    int c2 = (op == 0) ? c1 : _wtoi(f2);
    if(c1 < 1) c1 = 1;
    if(op != 0 && c2 < c1) c2 = c1;

    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    int l0 = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, s, 0);
    int l1 = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, e, 0);
    int total = (int)SendMessageW(g_np.editBox, EM_GETLINECOUNT, 0, 0);
    if(l1 >= total) l1 = total - 1;
    int start = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, l0, 0);
    int endNext = (l1 + 1 < total)
        ? (int)SendMessageW(g_np.editBox, EM_LINEINDEX, l1 + 1, 0) : len;

    /* Split selected range into lines. */
    int cap = 64, n = 0;
    WCHAR **lines = malloc((size_t)cap * sizeof(WCHAR *));
    if(!lines){ free(buf); return; }
    int p = start;
    while(p <= endNext){
        int q = p;
        while(q < endNext && buf[q] != L'\r' && buf[q] != L'\n') q++;
        if(n >= cap){
            cap *= 2;
            WCHAR **nl = realloc(lines, (size_t)cap * sizeof(WCHAR *));
            if(!nl) break;
            lines = nl;
        }
        int ll = q - p;
        lines[n] = malloc(((size_t)ll + 1) * sizeof(WCHAR));
        if(!lines[n]) break;
        memcpy(lines[n], buf + p, (size_t)ll * sizeof(WCHAR));
        lines[n][ll] = L'\0';
        n++;
        if(q >= endNext) break;
        p = q + ((q + 1 < endNext && buf[q] == L'\r' && buf[q + 1] == L'\n') ? 2 : 1);
    }

    if(op == 2){
        /* Copy the column slice of every line to the clipboard. */
        size_t need = 1;
        for(int i = 0; i < n; i++){
            int ll = (int)wcslen(lines[i]);
            int a = c1 - 1, b = c2;
            if(a > ll) a = ll;
            if(b > ll) b = ll;
            if(b < a) b = a;
            need += (size_t)(b - a) + 2;
        }
        WCHAR *out = malloc(need * sizeof(WCHAR));
        if(out){
            size_t o = 0;
            for(int i = 0; i < n; i++){
                int ll = (int)wcslen(lines[i]);
                int a = c1 - 1, b = c2;
                if(a > ll) a = ll;
                if(b > ll) b = ll;
                if(b < a) b = a;
                memcpy(out + o, lines[i] + a, (size_t)(b - a) * sizeof(WCHAR));
                o += (size_t)(b - a);
                out[o++] = L'\r'; out[o++] = L'\n';
            }
            out[o] = L'\0';
            if(OpenClipboard(g_np.mainWnd)){
                EmptyClipboard();
                size_t bytes = (o + 1) * sizeof(WCHAR);
                HGLOBAL h = GlobalAlloc(GMEM_MOVEABLE, bytes);
                if(h){
                    WCHAR *dst = GlobalLock(h);
                    if(dst){ memcpy(dst, out, bytes); GlobalUnlock(h); }
                    else GlobalFree(h);
                    if(dst) SetClipboardData(CF_UNICODETEXT, h);
                }
                CloseClipboard();
            }
            free(out);
        }
    }else{
        size_t add = (op == 0) ? wcslen(f2) : 0;
        size_t need = 1;
        for(int i = 0; i < n; i++)
            need += wcslen(lines[i]) + add + (size_t)(c1 > (int)wcslen(lines[i]) ? c1 : 0) + 2;
        WCHAR *out = malloc(need * sizeof(WCHAR));
        if(out){
            size_t o = 0;
            for(int i = 0; i < n; i++){
                int ll = (int)wcslen(lines[i]);
                if(op == 0){
                    int at = c1 - 1;
                    while(ll < at) lines[i][ll++] = L' ', lines[i][ll] = L'\0';
                    memcpy(out + o, lines[i], (size_t)at * sizeof(WCHAR));
                    o += (size_t)at;
                    size_t tl = wcslen(f2);
                    memcpy(out + o, f2, tl * sizeof(WCHAR));
                    o += tl;
                    size_t rest = wcslen(lines[i] + at);
                    memcpy(out + o, lines[i] + at, rest * sizeof(WCHAR));
                    o += rest;
                }else{
                    int a = c1 - 1, b = c2;
                    if(a > ll) a = ll;
                    if(b > ll) b = ll;
                    if(b < a) b = a;
                    memcpy(out + o, lines[i], (size_t)a * sizeof(WCHAR));
                    o += (size_t)a;
                    size_t rest = wcslen(lines[i] + b);
                    memcpy(out + o, lines[i] + b, rest * sizeof(WCHAR));
                    o += rest;
                }
                if(i + 1 < n){ out[o++] = L'\r'; out[o++] = L'\n'; }
            }
            out[o] = L'\0';
            SendMessageW(g_np.editBox, EM_SETSEL, start, endNext);
            SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)out);
            free(out);
        }
    }
    for(int i = 0; i < n; i++) free(lines[i]);
    free(lines);
    free(buf);
}

/* ==========================================================================
 *  Markdown preview, version history, sticky scroll header
 * ========================================================================== */

/* --- Markdown preview (RichEdit RTF renderer, debounced live update) --- */

static DWORD CALLBACK np_md_stream_cb(DWORD_PTR cookie, LPBYTE buf, LONG cb,
                                      LONG *pcb)
{
    const char **pp = (const char **)cookie;
    size_t left = strlen(*pp);
    size_t n = (size_t)cb < left ? (size_t)cb : left;
    memcpy(buf, *pp, n);
    *pp += n;
    *pcb = (LONG)n;
    return 0;
}

/* Append escaped RTF text with *bold*, _italic_, `code`, [text](url). */
static void np_md_inline(const WCHAR *line, int dark, char *out, size_t *o,
                         size_t cap)
{
#define MD_PUTC(c) do{ if(*o + 1 < cap) out[(*o)++] = (c); }while(0)
#define MD_PUTS(s) do{ const char *_s = (s); while(*_s) MD_PUTC(*_s++); }while(0)
#define MD_LINK MD_PUTS(dark ? "\\cf4\\ul " : "\\cf1\\ul ")
    /* Emit one WCHAR as RTF: ASCII direct, else \uN?. */
    for(size_t i = 0; line[i];){
        if(line[i] == L'*' && line[i + 1] == L'*'){
            size_t j = i + 2;
            while(line[j] && !(line[j] == L'*' && line[j + 1] == L'*')) j++;
            if(line[j]){
                MD_PUTS("\\b ");
                for(size_t k = i + 2; k < j; k++){
                    if(line[k] < 128 && line[k] >= 32 &&
                       line[k] != L'\\' && line[k] != L'{' && line[k] != L'}')
                        MD_PUTC((char)line[k]);
                    else{ char e[16]; sprintf_s(e, 16, "\\u%d?", (int)line[k]); MD_PUTS(e); }
                }
                MD_PUTS("\\b0 ");
                i = j + 2;
                continue;
            }
        }
        if((line[i] == L'*' || line[i] == L'_') && line[i + 1] &&
           line[i + 1] != L' '){
            WCHAR mark = line[i];
            size_t j = i + 1;
            while(line[j] && line[j] != mark) j++;
            if(line[j] && j > i + 1){
                MD_PUTS("\\i ");
                for(size_t k = i + 1; k < j; k++){
                    if(line[k] < 128 && line[k] >= 32 &&
                       line[k] != L'\\' && line[k] != L'{' && line[k] != L'}')
                        MD_PUTC((char)line[k]);
                    else{ char e[16]; sprintf_s(e, 16, "\\u%d?", (int)line[k]); MD_PUTS(e); }
                }
                MD_PUTS("\\i0 ");
                i = j + 1;
                continue;
            }
        }
        if(line[i] == L'`'){
            size_t j = i + 1;
            while(line[j] && line[j] != L'`') j++;
            if(line[j]){
                MD_PUTS("\\f1 ");
                for(size_t k = i + 1; k < j; k++){
                    if(line[k] < 128 && line[k] >= 32 &&
                       line[k] != L'\\' && line[k] != L'{' && line[k] != L'}')
                        MD_PUTC((char)line[k]);
                    else{ char e[16]; sprintf_s(e, 16, "\\u%d?", (int)line[k]); MD_PUTS(e); }
                }
                MD_PUTS("\\f0 ");
                i = j + 1;
                continue;
            }
        }
        if(line[i] == L'['){
            size_t t = i + 1;
            while(line[t] && line[t] != L']') t++;
            if(line[t] == L']' && line[t + 1] == L'('){
                MD_LINK;
                for(size_t k = i + 1; k < t; k++){
                    if(line[k] < 128 && line[k] >= 32 &&
                       line[k] != L'\\' && line[k] != L'{' && line[k] != L'}')
                        MD_PUTC((char)line[k]);
                    else{ char e[16]; sprintf_s(e, 16, "\\u%d?", (int)line[k]); MD_PUTS(e); }
                }
                MD_PUTS("\\ul0\\cf0 ");
                size_t u = t + 2;
                while(line[u] && line[u] != L')') u++;
                i = line[u] ? u + 1 : u;
                continue;
            }
        }
        WCHAR c = line[i++];
        if(c == L'\\' || c == L'{' || c == L'}') MD_PUTC('\\');
        if(c < 128 && c >= 32) MD_PUTC((char)c);
        else if(c == L'\t') MD_PUTS("\\tab ");
        else{ char e[16]; sprintf_s(e, 16, "\\u%d?", (int)c); MD_PUTS(e); }
    }
#undef MD_PUTC
#undef MD_PUTS
#undef MD_LINK
}

/* Render whole-document Markdown to an RTF blob (caller frees *rtfOut). */
static void np_md_render(const WCHAR *text, char **rtfOut)
{
#define MD_EMIT(t_) do{ const char *_e = (t_); while(*_e && o + 1 < cap) out[o++] = *_e++; }while(0)
    size_t cap = wcslen(text) * 4 + 1024;
    if(cap < 4096) cap = 4096;
    char *out = malloc(cap);
    if(!out){ *rtfOut = NULL; return; }
    size_t o = 0;
    int dark = g_np.darkActive;
    const char *head =
        "{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Segoe UI;}{\\f1 Consolas;}}"
        "{\\colortbl ;\\red0\\green0\\blue255;\\red128\\green128\\blue128;"
        "\\red255\\green255\\blue255;\\red140\\green180\\blue255;}"
        "\\f0\\fs20 ";
    while(*head && o + 1 < cap) out[o++] = *head++;
    if(dark)MD_EMIT("\\cf3 ");
    /* Split into lines. */
    const WCHAR *p = text;
    int inCode = 0;
    while(*p){
        const WCHAR *eol = p;
        while(*eol && *eol != L'\r' && *eol != L'\n') eol++;
        size_t ll = (size_t)(eol - p);
        WCHAR *line = malloc((ll + 1) * sizeof(WCHAR));
        if(!line) break;
        memcpy(line, p, ll * sizeof(WCHAR));
        line[ll] = L'\0';
        if(wcsncmp(line, L"```", 3) == 0){
            inCode = !inCode;
            MD_EMIT("\\par\r\n");
        }else if(inCode){
            MD_EMIT("\\f1 ");
            for(size_t k = 0; k < ll; k++){
                if(line[k] < 128 && line[k] >= 32 &&
                   line[k] != L'\\' && line[k] != L'{' && line[k] != L'}'){
                    if(o + 1 < cap) out[o++] = (char)line[k];
                }else{ char e[16]; sprintf_s(e, 16, "\\u%d?", (int)line[k]);
                    const char *se = e; while(*se && o + 1 < cap) out[o++] = *se++; }
            }
            MD_EMIT("\\f0\\par\r\n");
        }else if(line[0] == L'#'){
            int level = 0;
            while(line[level] == L'#') level++;
            if(level > 6) level = 6;
            const char *sz = (level == 1) ? "\\fs32" : (level == 2) ? "\\fs26" : "\\fs22";
            while(*sz && o + 1 < cap) out[o++] = *sz++;
            MD_EMIT("\\b ");
            size_t s = (size_t)level;
            while(line[s] == L' ') s++;
            np_md_inline(line + s, dark, out, &o, cap);
            MD_EMIT("\\b0\\par\r\n");
        }else if(line[0] == L'>' && (line[1] == L' ' || line[1] == L'\0')){
            const char *c = dark ? "\\cf3\\i " : "\\cf2\\i ";
            while(*c && o + 1 < cap) out[o++] = *c++;
            np_md_inline(line[1] == L' ' ? line + 2 : line + 1, dark, out, &o, cap);
            { const char *cc = "\\i0"; if(dark){ cc = "\\i0\\cf3 "; } else { cc = "\\i0\\cf0 "; }
              while(*cc && o + 1 < cap) out[o++] = *cc++;
              const char *pp = "\\par\r\n"; while(*pp && o + 1 < cap) out[o++] = *pp++; }
        }else if((line[0] == L'-' || line[0] == L'*') && line[1] == L' '){
            MD_EMIT("\\bullet\\tab ");
            np_md_inline(line + 2, dark, out, &o, cap);
            MD_EMIT("\\par\r\n");
        }else if(ll == 0)MD_EMIT("\\par\r\n");else{
            np_md_inline(line, dark, out, &o, cap);
            MD_EMIT("\\par\r\n");
        }
        free(line);
        p = eol;
        if(*p == L'\r') p++;
        if(*p == L'\n') p++;
        if(o + 256 >= cap){
            cap *= 2;
            char *no = realloc(out, cap);
            if(!no) break;
            out = no;
        }
    }
    if(o + 1 < cap) out[o++] = '}';
    out[o < cap ? o : cap - 1] = '\0';
    *rtfOut = out;
#undef MD_EMIT
}

static void np_md_set_rtf(const char *rtf)
{
    EDITSTREAM es;
    const char *p = rtf;
    es.dwCookie = (DWORD_PTR)&p;
    es.dwError = 0;
    es.pfnCallback = np_md_stream_cb;
    SendMessageW(g_np.mdView, EM_STREAMIN, SF_RTF, (LPARAM)&es);
}

static void np_md_update(void)
{
    if(!g_np.mdPreview || !g_np.mdView) return;
    if(g_np.tabs && g_np.tabActive >= 0 && g_np.tabActive < g_np.tabCount &&
       g_np.tabs[g_np.tabActive].largeFile){
        np_md_set_rtf("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Segoe UI;}}\\f0\\fs20 Preview unavailable for large files.}");
        return;
    }
    int len = GetWindowTextLengthW(g_np.editBox);
    if(len > 2 * 1024 * 1024) len = 2 * 1024 * 1024;
    WCHAR *text = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!text) return;
    GetWindowTextW(g_np.editBox, text, len + 1);
    char *rtf = NULL;
    np_md_render(text, &rtf);
    free(text);
    if(rtf){
        np_md_set_rtf(rtf);
        free(rtf);
    }
}

static void np_md_toggle(void)
{
    g_np.mdPreview = !g_np.mdPreview;
    if(g_np.mdPreview && !g_np.mdView){
        if(!g_np.richLib)
            g_np.richLib = LoadLibraryW(L"Msftedit.dll");
        if(g_np.richLib){
            g_np.mdView = CreateWindowExW(0, MSFTEDIT_CLASS, NULL,
                WS_CHILD | WS_VSCROLL | ES_MULTILINE | ES_READONLY | ES_WANTRETURN,
                0, 0, 100, 100, g_np.mainWnd, NULL,
                GetModuleHandleW(NULL), NULL);
            if(g_np.mdView){
                SendMessageW(g_np.mdView, EM_SETBKGNDCOLOR, 0,
                    g_np.darkActive ? RGB(32, 32, 32) : RGB(255, 255, 255));
                SetWindowTheme(g_np.mdView,
                    g_np.darkActive ? L"DarkMode_Explorer" : L"Explorer", NULL);
            }
        }
        if(!g_np.mdView) g_np.mdPreview = 0;
    }
    if(g_np.mdView){
        ShowWindow(g_np.mdView, g_np.mdPreview ? SW_SHOW : SW_HIDE);
        if(g_np.mdPreview) np_md_update();
    }
    np_on_size();
    np_refresh_title();
}

/* --- Version history: timestamped snapshots with a restore dialog --- */

static void np_history_dir(wchar_t *out, size_t outChars, const wchar_t *path)
{
    wchar_t dir[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    wchar_t safe[NP_MAX_PATH];
    size_t o = 0;
    for(size_t i = 0; path[i] && o + 1 < NP_MAX_PATH; i++){
        WCHAR c = path[i];
        if(c == L'\\' || c == L'/' || c == L':' || c == L'*' || c == L'?' ||
           c == L'"' || c == L'<' || c == L'>' || c == L'|')
            c = L'_';
        safe[o++] = c;
    }
    safe[o] = L'\0';
    swprintf_s(out, outChars, L"%s\\history\\%s", dir, safe);
}

static void np_history_write(void)
{
    if(!g_np.path[0]) return;
    wchar_t dir[NP_MAX_PATH];
    np_history_dir(dir, NP_MAX_PATH, g_np.path);
    /* Create parents: session\history may not exist yet. */
    {
        wchar_t tmp[NP_MAX_PATH];
        wcsncpy_s(tmp, NP_MAX_PATH, dir, _TRUNCATE);
        for(wchar_t *p = tmp; *p; p++){
            if(*p == L'\\'){
                wchar_t save = *p; *p = L'\0';
                if(wcslen(tmp) > 3) CreateDirectoryW(tmp, NULL);
                *p = save;
            }
        }
        CreateDirectoryW(tmp, NULL);
    }
    SYSTEMTIME st;
    GetLocalTime(&st);
    wchar_t snap[NP_MAX_PATH];
    swprintf_s(snap, NP_MAX_PATH,
               L"%s\\%04d%02d%02d-%02d%02d%02d.txt", dir,
               st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond);
    if(!CopyFileW(g_np.path, snap, TRUE)) return;
    /* Prune to the newest 50. */
    {
        wchar_t pat[NP_MAX_PATH];
        swprintf_s(pat, NP_MAX_PATH, L"%s\\*.txt", dir);
        WIN32_FIND_DATAW fd;
        HANDLE h = FindFirstFileW(pat, &fd);
        wchar_t names[128][64];
        int n = 0;
        if(h != INVALID_HANDLE_VALUE){
            do{
                if(n < 128){
                    wcsncpy_s(names[n], 64, fd.cFileName, _TRUNCATE);
                    n++;
                }
            }while(FindNextFileW(h, &fd));
            FindClose(h);
        }
        for(int i = 0; i < n; i++)
            for(int j = i + 1; j < n; j++)
                if(_wcsicmp(names[i], names[j]) > 0){
                    wchar_t t[64];
                    wcsncpy_s(t, 64, names[i], _TRUNCATE);
                    wcsncpy_s(names[i], 64, names[j], _TRUNCATE);
                    wcsncpy_s(names[j], 64, t, _TRUNCATE);
                }
        for(int i = 0; i + 50 < n; i++){
            wchar_t old[NP_MAX_PATH];
            swprintf_s(old, NP_MAX_PATH, L"%s\\%s", dir, names[i]);
            DeleteFileW(old);
        }
    }
}

typedef struct { wchar_t dir[NP_MAX_PATH]; wchar_t sel[NP_MAX_PATH]; } NP_HISTDLG;

static INT_PTR CALLBACK np_history_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    NP_HISTDLG *ctx = NULL;
    (void)ctx;
    switch(msg){
    case WM_INITDIALOG:
        ctx = (NP_HISTDLG *)l;
        SetWindowLongPtrW(dlg, DWLP_USER, (LONG_PTR)ctx);
        SetWindowTextW(dlg, L"Version History");
        {
            wchar_t pat[NP_MAX_PATH];
            swprintf_s(pat, NP_MAX_PATH, L"%s\\*.txt", ctx->dir);
            WIN32_FIND_DATAW fd;
            HANDLE h = FindFirstFileW(pat, &fd);
            wchar_t names[128][64];
            int n = 0;
            if(h != INVALID_HANDLE_VALUE){
                do{
                    if(n < 128){
                        wcsncpy_s(names[n], 64, fd.cFileName, _TRUNCATE);
                        n++;
                    }
                }while(FindNextFileW(h, &fd));
                FindClose(h);
            }
            for(int i = 0; i < n; i++)
                for(int j = i + 1; j < n; j++)
                    if(_wcsicmp(names[i], names[j]) < 0){
                        wchar_t t[64];
                        wcsncpy_s(t, 64, names[i], _TRUNCATE);
                        wcsncpy_s(names[i], 64, names[j], _TRUNCATE);
                        wcsncpy_s(names[j], 64, t, _TRUNCATE);
                    }
            HWND list = GetDlgItem(dlg, 1001);
            for(int i = 0; i < n; i++){
                /* "20260905-221830.txt" -> "2026-09-05 22:18:30". */
                wchar_t pretty[64];
                if(wcslen(names[i]) == 19)
                    swprintf_s(pretty, 64, L"%.4s-%.2s-%.2s %.2s:%.2s:%.2s",
                               names[i], names[i] + 4, names[i] + 6,
                               names[i] + 9, names[i] + 11, names[i] + 13);
                else
                    wcsncpy_s(pretty, 64, names[i], _TRUNCATE);
                SendMessageW(list, LB_ADDSTRING, 0, (LPARAM)pretty);
            }
            if(n > 0) SendMessageW(list, LB_SETCURSEL, 0, 0);
            else SetWindowTextW(dlg, L"Version History - no versions yet");
        }
        np_dark_dlg_init(dlg);
        return TRUE;
    case WM_CTLCOLORDLG: {
        LRESULT br = np_dark_ctl((HDC)w, dlg);
        if(br) return br;
        break;
    }
    case WM_CTLCOLOREDIT:
    case WM_CTLCOLORSTATIC:
    case WM_CTLCOLORLISTBOX:
    case WM_CTLCOLORBTN: {
        LRESULT br = np_dark_ctl((HDC)w, (HWND)l);
        if(br) return br;
        break;
    }
    case WM_COMMAND:
        if(LOWORD(w) == 1002){
            ctx = (NP_HISTDLG *)GetWindowLongPtrW(dlg, DWLP_USER);
            HWND list = GetDlgItem(dlg, 1001);
            int cur = (int)SendMessageW(list, LB_GETCURSEL, 0, 0);
            if(cur < 0){ MessageBeep(MB_ICONWARNING); return TRUE; }
            wchar_t pretty[64];
            SendMessageW(list, LB_GETTEXT, cur, (LPARAM)pretty);
            /* Map back: strip separators to the file name. */
            wchar_t name[64];
            int o = 0;
            for(int i = 0; pretty[i] && o < 18; i++)
                if(pretty[i] >= L'0' && pretty[i] <= L'9')
                    name[o++] = pretty[i];
            name[o] = L'\0';
            if(o == 14)
                swprintf_s(ctx->sel, NP_MAX_PATH, L"%s\\%.4s%.2s%.2s-%.2s%.2s%.2s.txt",
                           ctx->dir, name, name + 4, name + 6,
                           name + 8, name + 10, name + 12);
            else
                swprintf_s(ctx->sel, NP_MAX_PATH, L"%s\\%s", ctx->dir, pretty);
            EndDialog(dlg, 1002);
            return TRUE;
        }
        if(LOWORD(w) == IDCANCEL){ EndDialog(dlg, IDCANCEL); return TRUE; }
        if(LOWORD(w) == 1001 && HIWORD(w) == LBN_DBLCLK){
            SendMessageW(dlg, WM_COMMAND, 1002, 0);
            return TRUE;
        }
        break;
    }
    return FALSE;
}

/* In-memory version-history dialog: list + Restore + Close. */
static void np_history_open(void)
{
    if(!g_np.path[0]){
        MessageBoxW(g_np.mainWnd, L"Save the file first to track versions.",
                    S_APP, MB_OK | MB_ICONINFORMATION);
        return;
    }
    wchar_t dir[NP_MAX_PATH];
    np_history_dir(dir, NP_MAX_PATH, g_np.path);
    BYTE buf[768];
    BYTE *p = buf;
    DLGTEMPLATE *dlg = (DLGTEMPLATE *)p;
    dlg->style = WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_SETFONT;
    dlg->dwExtendedStyle = 0;
    dlg->cdit = 3;
    dlg->x = 0; dlg->y = 0; dlg->cx = 300; dlg->cy = 195;
    p += sizeof(DLGTEMPLATE);
    *(WORD *)p = 0; p += 2;
    *(WORD *)p = 0; p += 2;
    *(WORD *)p = 0; p += 2;
    *(WORD *)p = 8; p += 2;
    wcscpy((WCHAR *)p, L"MS Shell Dlg");
    p += (wcslen(L"MS Shell Dlg") + 1) * sizeof(WCHAR);
    struct { DWORD st; int x, y, cx, cy; WORD id; WORD cls; const WCHAR *t; } items[] = {
        { WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | LBS_NOTIFY, 8, 8, 284, 150, 1001, 0x83, L"" },
        { WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON, 60, 166, 80, 14, 1002, 0x80, L"&Restore" },
        { WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON, 160, 166, 80, 14, IDCANCEL, 0x80, L"Close" },
    };
    for(int i = 0; i < 3; i++){
        while(((uintptr_t)p & 3)) p++;
        DLGITEMTEMPLATE *it = (DLGITEMTEMPLATE *)p;
        it->style = items[i].st;
        it->dwExtendedStyle = 0;
        it->x = (short)items[i].x; it->y = (short)items[i].y;
        it->cx = (short)items[i].cx; it->cy = (short)items[i].cy;
        it->id = items[i].id;
        p += sizeof(DLGITEMTEMPLATE);
        *(WORD *)p = 0xFFFF; p += 2;
        *(WORD *)p = items[i].cls; p += 2;
        wcscpy((WCHAR *)p, items[i].t);
        p += (wcslen(items[i].t) + 1) * sizeof(WCHAR);
        *(WORD *)p = 0; p += 2;
    }
    NP_HISTDLG ctx;
    memset(&ctx, 0, sizeof(ctx));
    wcsncpy_s(ctx.dir, NP_MAX_PATH, dir, _TRUNCATE);
    if(DialogBoxIndirectParamW(GetModuleHandleW(NULL), (DLGTEMPLATE *)buf,
            g_np.mainWnd, np_history_proc, (LPARAM)&ctx) != 1002)
        return;
    if(!ctx.sel[0]) return;
    if(MessageBoxW(g_np.mainWnd,
            L"Restore this version? Current unsaved changes will be lost.",
            S_APP, MB_YESNO | MB_ICONQUESTION) != IDYES)
        return;
    HANDLE h = CreateFileW(ctx.sel, GENERIC_READ, FILE_SHARE_READ, NULL,
                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if(h == INVALID_HANDLE_VALUE){ np_file_error(ctx.sel); return; }
    DWORD size = GetFileSize(h, NULL);
    BYTE *raw = malloc(size + 4);
    if(!raw){ CloseHandle(h); return; }
    DWORD r = 0;
    if(!ReadFile(h, raw, size, &r, NULL)){ free(raw); CloseHandle(h); return; }
    CloseHandle(h);
    int need = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS,
                                   (char *)raw, r, NULL, 0);
    UINT cp = (need > 0) ? CP_UTF8 : CP_ACP;
    need = MultiByteToWideChar(cp, 0, (char *)raw, r, NULL, 0);
    WCHAR *text = malloc(((size_t)need + 1) * sizeof(WCHAR));
    if(text){
        MultiByteToWideChar(cp, 0, (char *)raw, r, text, need);
        text[need] = L'\0';
        WCHAR *shown = np_normalize_eol(text, NULL);
        free(text);
        if(shown){
            SetWindowTextW(g_np.editBox, shown);
            free(shown);
            np_set_dirty(1);
        }
    }
    free(raw);
}

/* --- Sticky scroll: pin the current Markdown heading above the edit --- */

static void np_sticky_toggle(void)
{
    g_np.stickyOn = !g_np.stickyOn;
    if(g_np.stickyOn){
        SetTimer(g_np.mainWnd, IDT_STICKY, 500, NULL);
        np_sticky_update();
    }else{
        if(!(g_np.showOverview && g_np.overviewBar))
            KillTimer(g_np.mainWnd, IDT_STICKY);
        if(g_np.stickyBar) ShowWindow(g_np.stickyBar, SW_HIDE);
        np_on_size();
    }
    np_refresh_title();
}

static void np_sticky_update(void)
{
    if(!g_np.stickyOn || !g_np.stickyBar) return;
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int line = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, e, 0);
    wchar_t found[256] = {0};
    for(int l = line; l >= 0 && l > line - 20000; l--){
        WCHAR buf[512];
        if(!np_get_line_text(g_np.editBox, l, buf, 512)) break;
        wchar_t *t = buf;
        while(*t == L' ' || *t == L'\t') t++;
        if(*t == L'#'){
            while(*t == L'#') t++;
            while(*t == L' ' || *t == L'\t') t++;
            wcsncpy_s(found, 256, t, _TRUNCATE);
            break;
        }
    }
    int show = (found[0] != L'\0');
    int was = IsWindowVisible(g_np.stickyBar);
    if(show) SetWindowTextW(g_np.stickyBar, found);
    if((show && !was) || (!show && was)){
        ShowWindow(g_np.stickyBar, show ? SW_SHOW : SW_HIDE);
        np_on_size();
    }
}

/* ==========================================================================
 *  Palette, split view, display modes, theme files, plugins, large files
 * ========================================================================== */

/* -- Fuzzy command palette (Ctrl+P): open tabs + recent files -- */

/* Palette entries use short display buffers (paths truncate for display;
 * opening uses the tab slot or the full recent path, never this copy). */
#define NP_PAL_LABEL 512

typedef struct {
    wchar_t label[NP_PAL_LABEL];
    int tabIdx;
    wchar_t path[NP_PAL_LABEL];
    int score;
} NP_PALITEM;

static HWND palWin, palEdit, palList;
static NP_PALITEM *palItems;
static int palCount, palCap;

static void np_pal_ensure(void)
{
    if(palItems) return;
    palCap = NP_MAX_TABS + 20;
    palItems = calloc((size_t)palCap, sizeof(NP_PALITEM));
}

static void np_pal_free(void)
{
    free(palItems);
    palItems = NULL;
    palCount = 0;
    palCap = 0;
}

static int np_fuzzy_score(const WCHAR *pat, const WCHAR *text)
{
    if(!pat[0]) return 1;
    int score = 0, ti = 0, consec = 0, first = -1;
    for(int pi = 0; pat[pi]; pi++){
        WCHAR pc = towlower(pat[pi]);
        int found = -1;
        for(int k = ti; text[k]; k++){
            if(towlower(text[k]) == pc){ found = k; break; }
        }
        if(found < 0) return 0;
        if(first < 0) first = found;
        if(found == 0) score += 10;
        else if(text[found - 1] == L'\\' || text[found - 1] == L'/' ||
                text[found - 1] == L' ' || text[found - 1] == L'_' ||
                text[found - 1] == L'-' || text[found - 1] == L'.')
            score += 8;
        if(found == ti){ consec++; score += 5 + consec * 2; }
        else consec = 0;
        score += 2;
        ti = found + 1;
    }
    score -= first;
    return score + 1;
}

static void np_palette_refill(void)
{
    wchar_t pat[NP_MAX_PATH];
    GetWindowTextW(palEdit, pat, NP_MAX_PATH);
    struct { int idx; int score; } order[NP_MAX_TABS + 20];
    int n = 0;
    for(int i = 0; i < palCount; i++){
        int s = np_fuzzy_score(pat, palItems[i].label);
        if(s > 0){ order[n].idx = i; order[n].score = s; n++; }
    }
    for(int i = 0; i < n; i++)
        for(int j = i + 1; j < n; j++)
            if(order[j].score > order[i].score){
                int t = order[i].idx; order[i].idx = order[j].idx; order[j].idx = t;
                int s = order[i].score; order[i].score = order[j].score; order[j].score = s;
            }
    SendMessageW(palList, LB_RESETCONTENT, 0, 0);
    for(int i = 0; i < n && i < 100; i++)
        SendMessageW(palList, LB_ADDSTRING, 0,
                     (LPARAM)palItems[order[i].idx].label);
    /* Stash the visible order in item scores for activation. */
    for(int i = 0; i < n && i < 100; i++)
        palItems[order[i].idx].score = 1000 + i;
    if(n > 0) SendMessageW(palList, LB_SETCURSEL, 0, 0);
}

static void np_palette_go(void)
{
    int cur = (int)SendMessageW(palList, LB_GETCURSEL, 0, 0);
    if(cur < 0){ DestroyWindow(palWin); return; }
    wchar_t label[NP_PAL_LABEL];
    SendMessageW(palList, LB_GETTEXT, cur, (LPARAM)label);
    int pick = -1;
    for(int i = 0; i < palCount; i++){
        if(palItems[i].score < 1000) continue;
        if(palItems[i].score - 1000 == cur &&
           _wcsicmp(palItems[i].label, label) == 0){ pick = i; break; }
    }
    if(pick < 0){
        /* Fallback: match by label text. */
        for(int i = 0; i < palCount; i++)
            if(_wcsicmp(palItems[i].label, label) == 0){ pick = i; break; }
    }
    /* Copy out before DestroyWindow (WM_DESTROY frees the table). */
    int tabIdx = -1;
    wchar_t path[NP_PAL_LABEL] = {0};
    if(pick >= 0){
        tabIdx = palItems[pick].tabIdx;
        wcsncpy_s(path, NP_PAL_LABEL, palItems[pick].path, _TRUNCATE);
    }
    DestroyWindow(palWin);
    palWin = NULL;
    if(pick < 0){ np_pal_free(); return; }
    if(tabIdx >= 0){
        np_tab_show(tabIdx);
    }else if(path[0]){
        if(g_np.path[0] || g_np.dirty ||
           GetWindowTextLengthW(g_np.editBox) != 0)
            np_tab_new();
        np_load_file(path, -1, 0, 0);
    }
    SetFocus(g_np.editBox);
}

static LRESULT CALLBACK np_palette_proc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    (void)l;
    switch(m){
    case WM_ERASEBKGND: {
        COLORREF bg = 0, fg = 0;
        int dark = 0;
        np_effective_colors(&bg, &fg, &dark);
        if(dark || g_np.themeName[0]){
            RECT rc;
            GetClientRect(h, &rc);
            HBRUSH br = CreateSolidBrush(bg);
            FillRect((HDC)w, &rc, br);
            DeleteObject(br);
            return 1;
        }
        break;
    }
    case WM_CTLCOLOREDIT:
    case WM_CTLCOLORSTATIC:
    case WM_CTLCOLORLISTBOX:
    case WM_CTLCOLORBTN: {
        LRESULT br = np_dark_ctl((HDC)w, (HWND)l);
        if(br) return br;
        break;
    }
    case WM_COMMAND:
        if((HWND)l == palEdit && HIWORD(w) == EN_CHANGE){
            np_palette_refill();
            return 0;
        }
        if((HWND)l == palList && HIWORD(w) == LBN_DBLCLK){
            np_palette_go();
            return 0;
        }
        break;
    case WM_KEYDOWN:
        if(w == VK_ESCAPE){ DestroyWindow(h); return 0; }
        break;
    case WM_DESTROY:
        if(h == palWin){
            palWin = NULL;
            palEdit = NULL;
            palList = NULL;
            np_pal_free();
        }
        break;
    }
    return DefWindowProcW(h, m, w, l);
}

static void np_palette_open(void)
{
    if(palWin){ SetFocus(palEdit); return; }
    np_pal_ensure();
    if(!palItems) return;
    WNDCLASSW wc;
    memset(&wc, 0, sizeof(wc));
    wc.lpfnWndProc = np_palette_proc;
    wc.hInstance = GetModuleHandleW(NULL);
    wc.hCursor = LoadCursorW(NULL, IDC_ARROW);
    wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.lpszClassName = L"NPPalette";
    RegisterClassW(&wc);
    palCount = 0;
    for(int i = 0; i < g_np.tabCount && palCount < palCap; i++){
        NP_PALITEM *it = &palItems[palCount];
        const wchar_t *name = L"Untitled";
        if(g_np.tabs[i].path[0]){
            const wchar_t *s = wcsrchr(g_np.tabs[i].path, L'\\');
            name = s ? s + 1 : g_np.tabs[i].path;
        }
        swprintf_s(it->label, NP_PAL_LABEL, L"Tab: %s%s",
                   g_np.tabs[i].dirty ? L"*" : L"", name);
        it->tabIdx = i;
        it->path[0] = L'\0';
        it->score = 0;
        palCount++;
    }
    {
        wchar_t recent[20][1024];
        int n = np_recent_list(recent, 20);
        for(int i = 0; i < n && palCount < palCap; i++){
            int dup = 0;
            for(int k = 0; k < g_np.tabCount; k++)
                if(_wcsicmp(g_np.tabs[k].path, recent[i]) == 0){ dup = 1; break; }
            if(dup) continue;
            NP_PALITEM *it = &palItems[palCount];
            const wchar_t *s = wcsrchr(recent[i], L'\\');
            swprintf_s(it->label, NP_PAL_LABEL, L"File: %s",
                       s ? s + 1 : recent[i]);
            it->tabIdx = -1;
            wcsncpy_s(it->path, NP_PAL_LABEL, recent[i], _TRUNCATE);
            it->score = 0;
            palCount++;
        }
    }
    RECT mr;
    GetWindowRect(g_np.mainWnd, &mr);
    int w = 480, hH = 320;
    int x = mr.left + ((mr.right - mr.left) - w) / 2;
    int y = mr.top + 80;
    palWin = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, L"NPPalette",
        L"Command Palette", WS_POPUPWINDOW | WS_CAPTION,
        x, y, w, hH, g_np.mainWnd, NULL, GetModuleHandleW(NULL), NULL);
    if(!palWin) return;
    palEdit = CreateWindowExW(WS_EX_CLIENTEDGE, L"Edit", L"",
        WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL,
        8, 8, w - 16, 24, palWin, NULL, GetModuleHandleW(NULL), NULL);
    palList = CreateWindowExW(WS_EX_CLIENTEDGE, L"ListBox", NULL,
        WS_CHILD | WS_VISIBLE | WS_VSCROLL | LBS_NOTIFY,
        8, 38, w - 16, hH - 46, palWin, NULL, GetModuleHandleW(NULL), NULL);
    {
        HFONT hf = (HFONT)SendMessageW(g_np.editBox, WM_GETFONT, 0, 0);
        if(hf){
            SendMessageW(palEdit, WM_SETFONT, (WPARAM)hf, TRUE);
            SendMessageW(palList, WM_SETFONT, (WPARAM)hf, TRUE);
        }
    }
    /* Route palette edit keys (arrows/enter/escape) locally. */
    SetWindowSubclass(palEdit, np_palette_edit_subproc, 2, 0);
    np_dark_dlg_init(palWin);
    np_palette_refill();
    ShowWindow(palWin, SW_SHOW);
    SetFocus(palEdit);
}

static LRESULT CALLBACK np_palette_edit_subproc(HWND h, UINT m, WPARAM w,
                                                LPARAM l, UINT_PTR id,
                                                DWORD_PTR ref)
{
    (void)id; (void)ref;
    if(m == WM_KEYDOWN){
        if(w == VK_DOWN || w == VK_UP){
            int n = (int)SendMessageW(palList, LB_GETCOUNT, 0, 0);
            int cur = (int)SendMessageW(palList, LB_GETCURSEL, 0, 0);
            cur += (w == VK_DOWN) ? 1 : -1;
            if(cur < 0) cur = 0;
            if(cur >= n) cur = n - 1;
            SendMessageW(palList, LB_SETCURSEL, cur, 0);
            return 0;
        }
        if(w == VK_RETURN){ np_palette_go(); return 0; }
        if(w == VK_ESCAPE){ DestroyWindow(palWin); palWin = NULL; return 0; }
        if(w == VK_TAB && palWin){
            /* Tab switches focus to the list; Escape comes back. */
            SetFocus(palList);
            return 0;
        }
    }
    if(m == WM_DESTROY && h == palEdit){
        palWin = NULL;
        palEdit = NULL;
        palList = NULL;
    }
    return DefSubclassProc(h, m, w, l);
}

/* -- Split view: second EDIT bound to another tab, optional scroll sync -- */

static void np_split_stash(void)
{
    if(!g_np.splitEdit || g_np.splitTab < 0 ||
       g_np.splitTab >= g_np.tabCount)
        return;
    int len = GetWindowTextLengthW(g_np.splitEdit);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return;
    GetWindowTextW(g_np.splitEdit, buf, len + 1);
    free(g_np.tabs[g_np.splitTab].text);
    g_np.tabs[g_np.splitTab].text = buf;
    g_np.tabs[g_np.splitTab].textLen = len;
}

static void np_split_refresh(void)
{
    if(!g_np.splitEdit) return;
    if(g_np.splitTab < 0 || g_np.splitTab >= g_np.tabCount ||
       g_np.splitTab == g_np.tabActive){
        int other = -1;
        for(int i = 0; i < g_np.tabCount; i++)
            if(i != g_np.tabActive){ other = i; break; }
        if(other < 0){
            np_split_set(0);
            return;
        }
        g_np.splitTab = other;
    }
    NP_TAB *t = &g_np.tabs[g_np.splitTab];
    g_np.editGuard = 1;
    SetWindowTextW(g_np.splitEdit, t->text ? t->text : L"");
    g_np.editGuard = 0;
}

static void np_split_set(int mode)
{
    if(mode == g_np.splitMode) mode = 0;
    if(mode && g_np.tabCount < 2){
        MessageBoxW(g_np.mainWnd, L"Open at least two tabs to split.",
                    S_APP, MB_OK | MB_ICONINFORMATION);
        return;
    }
    if(mode){
        if(!g_np.splitEdit){
            DWORD st = WS_CHILD | WS_VSCROLL | ES_MULTILINE |
                       ES_AUTOVSCROLL | ES_WANTRETURN;
            if(!g_np.wrap) st |= WS_HSCROLL | ES_AUTOHSCROLL;
            g_np.splitEdit = CreateWindowExW(WS_EX_CLIENTEDGE, L"Edit", L"",
                st, 0, 0, 100, 100, g_np.mainWnd, NULL,
                GetModuleHandleW(NULL), NULL);
            if(!g_np.splitEdit) return;
            HFONT hf = (HFONT)SendMessageW(g_np.editBox, WM_GETFONT, 0, 0);
            if(hf) SendMessageW(g_np.splitEdit, WM_SETFONT, (WPARAM)hf, TRUE);
            SendMessageW(g_np.splitEdit, EM_SETZOOM, (WPARAM)g_np.zoom,
                         (LPARAM)100);
        }
        g_np.splitMode = mode;
        g_np.splitTab = -1;
        np_split_refresh();
        ShowWindow(g_np.splitEdit, SW_SHOW);
        SetFocus(g_np.splitEdit);
    }else{
        np_split_stash();
        if(g_np.splitEdit) ShowWindow(g_np.splitEdit, SW_HIDE);
        g_np.splitMode = 0;
        SetFocus(g_np.editBox);
    }
    np_on_size();
    np_refresh_title();
}

/* Mirror a scroll message into the other pane (echo-guarded). */
static void np_split_sync_from(HWND src, UINT msg, WPARAM wParam)
{
    if(!g_np.splitSync || g_np.splitGuard || !g_np.splitMode) return;
    HWND other = (src == g_np.editBox) ? g_np.splitEdit : g_np.editBox;
    if(!other || !IsWindowVisible(other)) return;
    g_np.splitGuard = 1;
    SendMessageW(other, msg, wParam, 0);
    g_np.splitGuard = 0;
}

/* -- Distraction-free, always-on-top, sticky-note modes -- */

static void np_distract_toggle(void)
{
    if(!g_np.distractOn){
        g_np.savedMenu = GetMenu(g_np.mainWnd);
        SetMenu(g_np.mainWnd, NULL);
        g_np.savedPlace.length = sizeof(g_np.savedPlace);
        GetWindowPlacement(g_np.mainWnd, &g_np.savedPlace);
        g_np.savedStyle = (DWORD)GetWindowLongW(g_np.mainWnd, GWL_STYLE);
        g_np.savedExStyle = (DWORD)GetWindowLongW(g_np.mainWnd, GWL_EXSTYLE);
        ShowWindow(g_np.tabBar, SW_HIDE);
        ShowWindow(g_np.toolBar, SW_HIDE);
        ShowWindow(g_np.statusBar, SW_HIDE);
        if(g_np.stickyBar) ShowWindow(g_np.stickyBar, SW_HIDE);
        SetWindowLongW(g_np.mainWnd, GWL_STYLE, WS_POPUP | WS_VISIBLE);
        HMONITOR mon = MonitorFromWindow(g_np.mainWnd, MONITOR_DEFAULTTONEAREST);
        MONITORINFO mi;
        mi.cbSize = sizeof(mi);
        GetMonitorInfoW(mon, &mi);
        SetWindowPos(g_np.mainWnd, NULL,
                     mi.rcMonitor.left, mi.rcMonitor.top,
                     mi.rcMonitor.right - mi.rcMonitor.left,
                     mi.rcMonitor.bottom - mi.rcMonitor.top,
                     SWP_FRAMECHANGED | SWP_SHOWWINDOW);
        g_np.distractOn = 1;
    }else{
        SetWindowLongW(g_np.mainWnd, GWL_STYLE, (LONG)g_np.savedStyle);
        SetWindowLongW(g_np.mainWnd, GWL_EXSTYLE, (LONG)g_np.savedExStyle);
        SetMenu(g_np.mainWnd, g_np.savedMenu);
        ShowWindow(g_np.tabBar, SW_SHOW);
        ShowWindow(g_np.toolBar, g_np.showToolbar ? SW_SHOW : SW_HIDE);
        ShowWindow(g_np.statusBar,
                   (!g_np.wrap && g_np.showBar) ? SW_SHOW : SW_HIDE);
        if(g_np.stickyOn) np_sticky_update();
        SetWindowPlacement(g_np.mainWnd, &g_np.savedPlace);
        g_np.distractOn = 0;
    }
    np_on_size();
    np_refresh_title();
}

static void np_topmost_toggle(void)
{
    g_np.topmost = !g_np.topmost;
    SetWindowPos(g_np.mainWnd, g_np.topmost ? HWND_TOPMOST : HWND_NOTOPMOST,
                 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    np_refresh_title();
}

static void np_stickynote_toggle(void)
{
    if(!g_np.stickyNote){
        g_np.savedMenu = GetMenu(g_np.mainWnd);
        SetMenu(g_np.mainWnd, NULL);
        g_np.savedPlace.length = sizeof(g_np.savedPlace);
        GetWindowPlacement(g_np.mainWnd, &g_np.savedPlace);
        g_np.savedStyle = (DWORD)GetWindowLongW(g_np.mainWnd, GWL_STYLE);
        g_np.savedExStyle = (DWORD)GetWindowLongW(g_np.mainWnd, GWL_EXSTYLE);
        SetWindowLongW(g_np.mainWnd, GWL_STYLE, WS_POPUP | WS_VISIBLE);
        SetWindowLongW(g_np.mainWnd, GWL_EXSTYLE,
                       g_np.savedExStyle | WS_EX_TOPMOST | WS_EX_TOOLWINDOW);
        RECT rc;
        GetWindowRect(g_np.mainWnd, &rc);
        int w = rc.right - rc.left, h = rc.bottom - rc.top;
        if(w > 520 || h > 420){ w = 480; h = 360; }
        SetWindowPos(g_np.mainWnd, HWND_TOPMOST, rc.left, rc.top, w, h,
                     SWP_FRAMECHANGED | SWP_SHOWWINDOW);
        g_np.stickyNote = 1;
        g_np.topmost = 1;
    }else{
        SetWindowLongW(g_np.mainWnd, GWL_STYLE, (LONG)g_np.savedStyle);
        SetWindowLongW(g_np.mainWnd, GWL_EXSTYLE, (LONG)g_np.savedExStyle);
        SetMenu(g_np.mainWnd, g_np.savedMenu);
        SetWindowPlacement(g_np.mainWnd, &g_np.savedPlace);
        g_np.stickyNote = 0;
        g_np.topmost = 0;
    }
    np_on_size();
    np_refresh_title();
}

/* -- Custom .theme files (%APPDATA%\LoOS\Notepad\themes\*.theme) -- */

static void np_theme_file_dir(wchar_t *out, size_t outChars)
{
    wchar_t dir[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    swprintf_s(out, outChars, L"%s\\themes", dir);
    CreateDirectoryW(out, NULL);
}

static int np_theme_parse_color(const wchar_t *s, COLORREF *out)
{
    while(*s == L' ' || *s == L'\t' || *s == L'#') s++;
    if(wcslen(s) < 6) return 0;
    int v = 0;
    for(int i = 0; i < 6; i++){
        WCHAR c = s[i];
        v <<= 4;
        if(c >= L'0' && c <= L'9') v |= c - L'0';
        else if(c >= L'a' && c <= L'f') v |= c - L'a' + 10;
        else if(c >= L'A' && c <= L'F') v |= c - L'A' + 10;
        else return 0;
    }
    *out = RGB((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
    return 1;
}

static void np_theme_load(const wchar_t *name, COLORREF *bg, COLORREF *fg)
{
    *bg = RGB(255, 255, 255);
    *fg = RGB(0, 0, 0);
    wchar_t dir[NP_MAX_PATH], path[NP_MAX_PATH];
    np_theme_file_dir(dir, NP_MAX_PATH);
    swprintf_s(path, NP_MAX_PATH, L"%s\\%s.theme", dir, name);
    wchar_t fileBg[32] = {0}, fileFg[32] = {0};
    GetPrivateProfileStringW(L"theme", L"bg", L"", fileBg, 32, path);
    GetPrivateProfileStringW(L"theme", L"fg", L"", fileFg, 32, path);
    if(fileBg[0]) np_theme_parse_color(fileBg, bg);
    if(fileFg[0]) np_theme_parse_color(fileFg, fg);
}

/* Append installed themes to the View menu (called once at startup). */
static void np_theme_menu(HMENU viewMenu)
{
    wchar_t dir[NP_MAX_PATH], pat[NP_MAX_PATH];
    np_theme_file_dir(dir, NP_MAX_PATH);
    swprintf_s(pat, NP_MAX_PATH, L"%s\\*.theme", dir);
    WIN32_FIND_DATAW fd;
    HANDLE h = FindFirstFileW(pat, &fd);
    if(h == INVALID_HANDLE_VALUE) return;
    AppendMenuW(viewMenu, MF_SEPARATOR, 0, NULL);
    int idx = 0;
    do{
        if(idx >= 32) break;
        wchar_t name[64];
        wcsncpy_s(name, 64, fd.cFileName, _TRUNCATE);
        wchar_t *dot = wcsrchr(name, L'.');
        if(dot) *dot = L'\0';
        wcsncpy_s(g_np.customThemes[idx], 64, name, _TRUNCATE);
        np_menu_append(viewMenu, MF_STRING, CMD_THEME_CUSTOM + idx, name);
        if(_wcsicmp(g_np.themeName, name) == 0)
            CheckMenuItem(viewMenu, CMD_THEME_CUSTOM + idx,
                          MF_BYCOMMAND | MF_CHECKED);
        idx++;
    }while(FindNextFileW(h, &fd));
    FindClose(h);
    g_np.customThemeCount = idx;
}

static void np_theme_apply_custom(const wchar_t *name)
{
    COLORREF bg = 0, fg = 0;
    np_theme_load(name, &bg, &fg);
    wcsncpy_s(g_np.themeName, 64, name, _TRUNCATE);
    g_np.themeBg = bg;
    g_np.themeFg = fg;
    /* Luminance decides the chrome (title bar, scrollbars). */
    double lum = (0.299 * GetRValue(bg) + 0.587 * GetGValue(bg) +
                  0.114 * GetBValue(bg)) / 255.0;
    g_np.theme = (lum < 0.5) ? 2 : 1;
    np_apply_theme();
    np_save_settings();
}

/* -- Minimal plugin host (Tools menu, documented ABI in the header) -- */

typedef int (WINAPI *NPQ_FN)(const NP_PLUGIN_CMD **out);
typedef void (WINAPI *NPEXEC_FN)(int id, HWND editBox, HWND mainWnd);
typedef void (WINAPI *NPEXEC2_FN)(int id, const NP_HOST *host);
typedef void (WINAPI *NPEXEC3_FN)(int id, const NP_HOST3 *host);
typedef void (WINAPI *NPEVENT_FN)(int event, const NP_HOST *host);
typedef void (WINAPI *NPIDLE_FN)(const NP_HOST *host);
typedef int (WINAPI *NPVER_FN)(void);

/* v2 host services: safe wrappers that keep dirty flag and status fresh. */
static void np_host_getSel(int *s, int *e)
{
    DWORD a = 0, b = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&a, (LPARAM)&b);
    if(s) *s = (int)a;
    if(e) *e = (int)b;
}

static void np_host_setSel(int s, int e)
{
    int len = GetWindowTextLengthW(g_np.editBox);
    if(s < 0) s = 0;
    if(e < 0) e = 0;
    if(s > len) s = len;
    if(e > len) e = len;
    SendMessageW(g_np.editBox, EM_SETSEL, s, e);
    SendMessageW(g_np.editBox, EM_SCROLLCARET, 0, 0);
    SetFocus(g_np.editBox);
}

static int np_host_getTextLen(void)
{
    return GetWindowTextLengthW(g_np.editBox);
}

static void np_host_getText(WCHAR *out, int outChars)
{
    if(!out || outChars <= 0) return;
    GetWindowTextW(g_np.editBox, out, outChars);
}

static void np_host_replaceSel(const WCHAR *text)
{
    if(!text) return;
    SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)text);
    np_set_dirty(1);
    np_update_status();
    SetFocus(g_np.editBox);
}

static void np_host_replaceAll(const WCHAR *text)
{
    if(!text) return;
    g_np.editGuard = 1;
    SetWindowTextW(g_np.editBox, text);
    g_np.editGuard = 0;
    np_set_dirty(1);
    np_update_status();
}

static void np_host_getPath(WCHAR *out, int outChars)
{
    if(!out || outChars <= 0) return;
    wcsncpy_s(out, outChars, g_np.path, _TRUNCATE);
}

static void np_host_setStatus(const WCHAR *text)
{
    if(text) np_announce(text);
}

static void np_host_getConfigDir(WCHAR *out, int outChars)
{
    wchar_t dir[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    if(out && outChars > 0)
        wcsncpy_s(out, outChars, dir, _TRUNCATE);
}

static int np_host_confirm(const WCHAR *text)
{
    return MessageBoxW(g_np.mainWnd, text ? text : L"",
                       S_APP, MB_YESNO | MB_ICONQUESTION);
}

/* (g_npHost lives up with the closed-tab stack; filled by np_host_init.) */

/* (np_host_init lives in the ABI v3 section at the end of the file.) */

static void np_plugins_load(void)
{
    np_host_init();
    wchar_t dir[NP_MAX_PATH], pat[NP_MAX_PATH];
    np_session_dir(dir, NP_MAX_PATH);
    swprintf_s(pat, NP_MAX_PATH, L"%s\\plugins", dir);
    CreateDirectoryW(pat, NULL);
    swprintf_s(pat, NP_MAX_PATH, L"%s\\plugins\\*.dll", dir);
    WIN32_FIND_DATAW fd;
    HANDLE h = FindFirstFileW(pat, &fd);
    if(h == INVALID_HANDLE_VALUE) return;
    int nextId = CMD_PLUGIN_BASE;
    do{
        if(g_np.pluginCount >= NP_MAX_PLUGINS) break;
        wchar_t full[NP_MAX_PATH];
        swprintf_s(full, NP_MAX_PATH, L"%s\\plugins\\%s", dir, fd.cFileName);
        HMODULE lib = LoadLibraryW(full);
        if(!lib) continue;
        NPVER_FN ver = (NPVER_FN)GetProcAddress(lib, "NpPluginVersion");
        NPQ_FN query = (NPQ_FN)GetProcAddress(lib, "NpPluginCommands");
        NPEXEC_FN exec = (NPEXEC_FN)GetProcAddress(lib, "NpPluginExec");
        NPEXEC2_FN exec2 = (NPEXEC2_FN)GetProcAddress(lib, "NpPluginExec2");
        NPEXEC3_FN exec3 = (NPEXEC3_FN)GetProcAddress(lib, "NpPluginExec3");
        NPEVENT_FN onEvent = (NPEVENT_FN)GetProcAddress(lib, "NpPluginEvent");
        NPIDLE_FN onIdle = (NPIDLE_FN)GetProcAddress(lib, "NpPluginIdle");
        if(!ver || !query){
            FreeLibrary(lib);
            continue;
        }
        int abi = ver();
        if(abi < 1 || abi > 3 || (!exec && !exec2 && !exec3)){
            FreeLibrary(lib);
            continue;
        }
        const NP_PLUGIN_CMD *cmds = NULL;
        int count = query(&cmds);
        if(count <= 0 || count > NP_PLUGIN_MAX_COMMANDS || !cmds ||
           nextId + count > CMD_PLUGIN_END){
            FreeLibrary(lib);
            continue;
        }
        NP_PLUGIN *p = &g_np.plugins[g_np.pluginCount];
        p->lib = lib;
        p->baseId = nextId;
        p->count = count;
        p->abiVersion = abi;
        p->exec = exec;
        p->exec2 = exec2;
        p->exec3 = (abi >= 3) ? exec3 : NULL;
        p->onEvent = onEvent;
        p->onIdle = onIdle;
        wcsncpy_s(p->fileName, 64, fd.cFileName, _TRUNCATE);
        for(int i = 0; i < count; i++)
            np_menu_append(g_np.toolsMenu, MF_STRING, nextId + i, cmds[i].name);
        np_debug(L"plugin %s: abi %d, %d commands%s%s", fd.cFileName, abi,
                 count, onEvent ? L", events" : L"",
                 onIdle ? L", idle" : L"");
        nextId += count;
        g_np.pluginCount++;
    }while(FindNextFileW(h, &fd));
    FindClose(h);
    if(np_any_idle())
        SetTimer(g_np.mainWnd, IDT_PLUGINIDLE, 2000, NULL);
}

static void np_plugins_free(void)
{
    for(int i = 0; i < g_np.pluginCount; i++)
        if(g_np.plugins[i].lib) FreeLibrary(g_np.plugins[i].lib);
    g_np.pluginCount = 0;
}

/* -- Large-file mode: >32 MB locks wrap off and skips heavy extras -- */

static void np_largefile_check(DWORD size)
{
    int large = (size >= NP_LARGE_FILE);
    if(g_np.tabActive >= 0 && g_np.tabActive < g_np.tabCount)
        g_np.tabs[g_np.tabActive].largeFile = large;
    if(!large) return;
    if(g_np.wrap){
        g_np.wrap = 0;
        ShowWindow(g_np.statusBar, g_np.showBar ? SW_SHOW : SW_HIDE);
    }
    SendMessageW(g_np.editBox, EM_EMPTYUNDOBUFFER, 0, 0);
    np_announce(L"Large file mode: word wrap off, undo cleared");
    np_refresh_title();
}

/* ==========================================================================
 *  Polish: owner-drawn menus, UI fonts
 * ==========================================================================
 *  Submenu items are created MFT_OWNERDRAW from birth in dark/custom
 *  themes (the label literal becomes the item data); light mode builds
 *  plain MFT_STRING so it stays 100% native. The top bar is always
 *  native. */

static HFONT np_get_menu_font(void)
{
    if(g_np.menuFont) return g_np.menuFont;
    HDC screen = GetDC(NULL);
    int dpi = screen ? GetDeviceCaps(screen, LOGPIXELSY) : 96;
    if(screen) ReleaseDC(NULL, screen);
    int h = -MulDiv(9, dpi, 72);
    /* Segoe UI Variable first (Win11), Segoe UI, then system default. */
    g_np.menuFont = CreateFontW(h, 0, 0, 0, FW_NORMAL,
        FALSE, FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_DONTCARE,
        L"Segoe UI Variable Text");
    {
        wchar_t face[64] = {0};
        HDC dc = GetDC(NULL);
        HFONT old = SelectObject(dc, g_np.menuFont);
        GetTextFaceW(dc, 64, face);
        SelectObject(dc, old);
        ReleaseDC(NULL, dc);
        if(_wcsicmp(face, L"Segoe UI Variable Text") != 0){
            DeleteObject(g_np.menuFont);
            g_np.menuFont = CreateFontW(h, 0, 0, 0, FW_NORMAL,
                FALSE, FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
                DEFAULT_PITCH | FF_DONTCARE, L"Segoe UI");
        }
    }
    if(!g_np.menuFont)
        g_np.menuFont = CreateFontW(h, 0, 0, 0, FW_NORMAL,
            FALSE, FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, DEFAULT_PITCH, NULL);
    return g_np.menuFont;
}

/* Append one submenu item. In dark mode every real item is created
 * MFT_OWNERDRAW from birth (label literal becomes the item data, with a
 * high tag bit for popups so arrows draw); separators and the top bar
 * always stay native. */
#define NP_MENU_POPUP_BIT ((ULONG_PTR)1 << 63)
#define NP_MENU_BAR_BIT ((ULONG_PTR)1 << 62)

static void np_menu_append(HMENU pop, UINT flags, UINT_PTR id,
                           const wchar_t *text)
{
    if(g_np.menuDark && !(flags & MF_SEPARATOR)){
        ULONG_PTR data = (ULONG_PTR)text;
        if(flags & MF_POPUP) data |= NP_MENU_POPUP_BIT;
        flags = (flags & ~MF_STRING) | MF_OWNERDRAW;
        AppendMenuW(pop, flags, id, (LPCWSTR)data);
        return;
    }
    AppendMenuW(pop, flags, id, text);
}

/* Top-level bar items: owner-drawn flat in dark mode (BAR tag = no check
 * area, no arrow, centered text); native otherwise. */
static void np_menu_append_bar(HMENU bar, UINT_PTR id, const wchar_t *text)
{
    if(g_np.menuDark){
        ULONG_PTR data = (ULONG_PTR)text | NP_MENU_POPUP_BIT | NP_MENU_BAR_BIT;
        AppendMenuW(bar, MF_POPUP | MF_OWNERDRAW, id, (LPCWSTR)data);
        return;
    }
    AppendMenuW(bar, MF_POPUP, id, text);
}

/* Re-append loaded plugin commands (used after a menu rebuild). */
static void np_plugins_reappend(void)
{
    if(!g_np.toolsMenu) return;
    for(int i = 0; i < g_np.pluginCount; i++){
        NP_PLUGIN *p = &g_np.plugins[i];
        NPQ_FN query = (NPQ_FN)GetProcAddress(p->lib, "NpPluginCommands");
        if(!query) continue;
        const NP_PLUGIN_CMD *cmds = NULL;
        int count = query(&cmds);
        if(count != p->count || !cmds) continue;
        for(int k = 0; k < count; k++)
            np_menu_append(g_np.toolsMenu, MF_STRING,
                           p->baseId + k, cmds[k].name);
    }
}

/* Initial states the resource carried (grayed until text/selection):
 * applied after every menu build; live updates continue via INITMENUPOPUP. */
static void np_menu_init_states(HMENU bar)
{
    HMENU edit = GetSubMenu(bar, 1);
    if(!edit) return;
    static const int grayed[] = { CMD_EDIT_CUT, CMD_EDIT_COPY,
                                  CMD_EDIT_PASTE, CMD_EDIT_DELETE };
    for(size_t i = 0; i < sizeof(grayed) / sizeof(grayed[0]); i++)
        EnableMenuItem(edit, grayed[i], MF_BYCOMMAND | MF_GRAYED);
}

/* Rebuild the whole menu bar (used on theme switches so dark mode gets
 * true owner-drawn items and light mode stays 100% native). Re-attaches
 * custom themes and plugin commands, then refreshes all check states. */
static void np_rebuild_menu(void)
{
    HMENU old = GetMenu(g_np.mainWnd);
    int dark = g_np.darkActive || g_np.themeName[0] != 0;
    g_np.menuDark = dark ? 1 : 0;
    HMENU bar = np_build_menu();
    if(!bar) return;
    SetMenu(g_np.mainWnd, bar);
    g_np.editMenu = GetSubMenu(bar, 1);
    g_np.toolsMenu = GetSubMenu(bar, 4);
    np_theme_menu(GetSubMenu(bar, 3));
    np_plugins_reappend();
    np_menu_init_states(bar);
    DrawMenuBar(g_np.mainWnd);
    if(old) DestroyMenu(old);
    np_refresh_title();
}


static void np_menu_split(const wchar_t *text, const wchar_t **left,
                          const wchar_t **right)
{
    static wchar_t lbuf[128], rbuf[64];
    const wchar_t *tab = wcschr(text, L'\t');
    if(!tab){
        wcsncpy_s(lbuf, 128, text, _TRUNCATE);
        rbuf[0] = L'\0';
    }else{
        size_t n = (size_t)(tab - text);
        if(n >= 128) n = 127;
        memcpy(lbuf, text, n * sizeof(WCHAR));
        lbuf[n] = L'\0';
        wcsncpy_s(rbuf, 64, tab + 1, _TRUNCATE);
    }
    *left = lbuf;
    *right = rbuf;
}

static void np_menu_measure(MEASUREITEMSTRUCT *mis)
{
    ULONG_PTR raw = (ULONG_PTR)mis->itemData;
    int isPopup = (raw & NP_MENU_POPUP_BIT) != 0;
    int isBar = (raw & NP_MENU_BAR_BIT) != 0;
    const wchar_t *text = (const wchar_t *)
        (raw & ~(NP_MENU_POPUP_BIT | NP_MENU_BAR_BIT));
    if(!text || !text[0]){
        mis->itemWidth = 0;
        mis->itemHeight = 7;
        return;
    }
    HDC dc = GetDC(g_np.mainWnd);
    HFONT old = SelectObject(dc, np_get_menu_font());
    const wchar_t *left, *right;
    np_menu_split(text, &left, &right);
    SIZE a = {0, 0}, b = {0, 0};
    GetTextExtentPoint32W(dc, left, (int)wcslen(left), &a);
    if(right[0] && !isBar)
        GetTextExtentPoint32W(dc, right, (int)wcslen(right), &b);
    SelectObject(dc, old);
    ReleaseDC(g_np.mainWnd, dc);
    if(isBar){
        mis->itemWidth = (UINT)(a.cx + 20);
        mis->itemHeight = (UINT)(a.cy + 10);
        return;
    }
    mis->itemWidth = (UINT)(34 + a.cx + (b.cx ? b.cx + 24 : 0) +
                            (isPopup ? 22 : 0));
    mis->itemHeight = (UINT)(a.cy + 9);
}

static void np_menu_draw(const DRAWITEMSTRUCT *dis)
{
    ULONG_PTR raw = (ULONG_PTR)dis->itemData;
    int isPopup = (raw & NP_MENU_POPUP_BIT) != 0;
    int isBar = (raw & NP_MENU_BAR_BIT) != 0;
    const wchar_t *text =
        (const wchar_t *)(raw & ~(NP_MENU_POPUP_BIT | NP_MENU_BAR_BIT));
    int dark = g_np.darkActive || g_np.themeName[0] != 0;
    int selected = (dis->itemState & ODS_SELECTED) != 0;
    int disabled = (dis->itemState & (ODS_GRAYED | ODS_DISABLED)) != 0;
    int checked = (dis->itemState & ODS_CHECKED) != 0;
    HDC dc = dis->hDC;
    RECT rc = dis->rcItem;

    COLORREF bg, fg;
    if(!text || !text[0]){
        bg = dark ? RGB(45, 45, 45) : GetSysColor(COLOR_MENU);
        HBRUSH br = CreateSolidBrush(bg);
        FillRect(dc, &rc, br);
        DeleteObject(br);
        int y = (rc.top + rc.bottom) / 2;
        HPEN pen = CreatePen(PS_SOLID, 1,
            dark ? RGB(80, 80, 80) : GetSysColor(COLOR_GRAYTEXT));
        HPEN oldPen = SelectObject(dc, pen);
        MoveToEx(dc, rc.left + 30, y, NULL);
        LineTo(dc, rc.right - 6, y);
        SelectObject(dc, oldPen);
        DeleteObject(pen);
        return;
    }
    if(selected) bg = dark ? RGB(70, 70, 70) : GetSysColor(COLOR_HIGHLIGHT);
    else bg = dark ? RGB(45, 45, 45) : GetSysColor(COLOR_MENU);
    if(disabled) fg = dark ? RGB(130, 130, 130) : GetSysColor(COLOR_GRAYTEXT);
    else if(selected && !dark) fg = GetSysColor(COLOR_HIGHLIGHTTEXT);
    else fg = dark ? RGB(240, 240, 240) : GetSysColor(COLOR_MENUTEXT);

    HBRUSH br = CreateSolidBrush(bg);
    FillRect(dc, &rc, br);
    DeleteObject(br);

    HFONT oldFont = SelectObject(dc, np_get_menu_font());
    int oldBk = SetBkMode(dc, TRANSPARENT);
    COLORREF oldFg = SetTextColor(dc, fg);

    if(checked){
        RECT cr;
        cr.left = rc.left + 6; cr.right = cr.left + 18;
        cr.top = rc.top; cr.bottom = rc.bottom;
        DrawTextW(dc, L"\x2713", 1, &cr,
                  DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    }
    const wchar_t *left, *right;
    np_menu_split(text, &left, &right);
    RECT lr;
    if(isBar){
        /* Flat top bar: centered label, no check gutter, no arrow. */
        lr = rc;
        lr.left += 10;
        lr.right -= 10;
        DrawTextW(dc, left, -1, &lr, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        SetTextColor(dc, oldFg);
        SetBkMode(dc, oldBk);
        SelectObject(dc, oldFont);
        return;
    }
    lr.left = rc.left + 32; lr.right = rc.right - 8;
    lr.top = rc.top; lr.bottom = rc.bottom;
    DrawTextW(dc, left, -1, &lr, DT_SINGLELINE | DT_VCENTER | DT_LEFT);
    if(right[0])
        DrawTextW(dc, right, -1, &lr, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
    /* Submenu arrow (tagged in the item data at build time; never on
     * the top bar). */
    if(isPopup && !isBar){
        RECT ar;
        ar.left = rc.right - 22; ar.right = rc.right - 6;
        ar.top = rc.top; ar.bottom = rc.bottom;
        DrawTextW(dc, L"\x25B6", 1, &ar,
                  DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    }

    SetTextColor(dc, oldFg);
    SetBkMode(dc, oldBk);
    SelectObject(dc, oldFont);
}

/* ==========================================================================
 *  Polish: toolbar, tab close buttons
 * ========================================================================== */

/* Slim text toolbar (no art assets needed, theme-safe). Buttons mirror the
 * matching menu commands. */
static void np_toolbar_create(void)
{
    HINSTANCE inst = GetModuleHandleW(NULL);
    g_np.toolBar = CreateWindowExW(0, TOOLBARCLASSNAMEW, NULL,
        WS_CHILD | TBSTYLE_FLAT | TBSTYLE_TOOLTIPS | TBSTYLE_LIST |
        CCS_NORESIZE | CCS_NODIVIDER | CCS_TOP,
        0, 0, 100, 30, g_np.mainWnd, NULL, inst, NULL);
    if(!g_np.toolBar) return;
    SendMessageW(g_np.toolBar, TB_BUTTONSTRUCTSIZE, sizeof(TBBUTTON), 0);
    static const wchar_t labels[] =
        L"New\0Open\0Save\0Cut\0Copy\0Paste\0Undo\0Find\0";
    int base = (int)SendMessageW(g_np.toolBar, TB_ADDSTRINGW, 0,
                                 (LPARAM)labels);
    static const struct { int cmd; int str; int sepBefore; } defs[] = {
        { CMD_FILE_NEW, 0, 0 }, { CMD_FILE_OPEN, 1, 0 },
        { CMD_FILE_SAVE, 2, 0 }, { 0, 0, 1 },
        { CMD_EDIT_CUT, 3, 0 }, { CMD_EDIT_COPY, 4, 0 },
        { CMD_EDIT_PASTE, 5, 0 }, { 0, 0, 1 },
        { CMD_EDIT_UNDO, 6, 0 }, { 0, 0, 1 },
        { CMD_EDIT_FIND, 7, 0 },
    };
    for(size_t i = 0; i < sizeof(defs) / sizeof(defs[0]); i++){
        TBBUTTON b;
        memset(&b, 0, sizeof(b));
        if(defs[i].sepBefore){
            b.fsStyle = BTNS_SEP;
            b.fsState = TBSTATE_ENABLED;
        }else{
            b.iBitmap = I_IMAGENONE;
            b.idCommand = defs[i].cmd;
            b.fsStyle = BTNS_BUTTON | BTNS_AUTOSIZE | BTNS_SHOWTEXT;
            b.fsState = TBSTATE_ENABLED;
            b.iString = base + defs[i].str;
        }
        SendMessageW(g_np.toolBar, TB_ADDBUTTONSW, 1, (LPARAM)&b);
    }
    SendMessageW(g_np.toolBar, TB_AUTOSIZE, 0, 0);
    HFONT hf = np_get_menu_font();
    if(hf) SendMessageW(g_np.toolBar, WM_SETFONT, (WPARAM)hf, TRUE);
    ShowWindow(g_np.toolBar, SW_SHOW);
}

static void np_toolbar_update(void)
{
    if(!g_np.toolBar) return;
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int haveSel = (s != e);
    int canUndo = (int)SendMessageW(g_np.editBox, EM_CANUNDO, 0, 0);
    int canPaste = IsClipboardFormatAvailable(CF_UNICODETEXT) ||
                   IsClipboardFormatAvailable(CF_TEXT);
    SendMessageW(g_np.toolBar, TB_ENABLEBUTTON, CMD_EDIT_CUT, haveSel);
    SendMessageW(g_np.toolBar, TB_ENABLEBUTTON, CMD_EDIT_COPY, haveSel);
    SendMessageW(g_np.toolBar, TB_ENABLEBUTTON, CMD_EDIT_DELETE, haveSel);
    SendMessageW(g_np.toolBar, TB_ENABLEBUTTON, CMD_EDIT_PASTE, canPaste);
    SendMessageW(g_np.toolBar, TB_ENABLEBUTTON, CMD_EDIT_UNDO, canUndo);
}

/* Toolbar color message: dark background + light text, else native. */
static LRESULT np_toolbar_customdraw(LPNMTBCUSTOMDRAW tb)
{
    int dark = g_np.darkActive || g_np.themeName[0] != 0;
    if(tb->nmcd.dwDrawStage == CDDS_PREPAINT){
        if(dark){
            RECT rc;
            GetClientRect(g_np.toolBar, &rc);
            HBRUSH br = CreateSolidBrush(RGB(45, 45, 45));
            FillRect(tb->nmcd.hdc, &rc, br);
            DeleteObject(br);
        }
        return CDRF_NOTIFYITEMDRAW;
    }
    if(tb->nmcd.dwDrawStage == CDDS_ITEMPREPAINT){
        if(dark) tb->clrText = RGB(240, 240, 240);
        return CDRF_DODEFAULT;
    }
    return CDRF_DODEFAULT;
}

/* Close-box geometry for a tab (16px, right-aligned, vertically centered). */
static void np_tab_x_rect(HWND tab, int item, RECT *out)
{
    RECT rc;
    SendMessageW(tab, TCM_GETITEMRECT, item, (LPARAM)&rc);
    out->right = rc.right - 5;
    out->left = out->right - 16;
    out->top = (rc.top + rc.bottom) / 2 - 8;
    out->bottom = out->top + 16;
}

static void np_tab_draw(const DRAWITEMSTRUCT *dis)
{
    int idx = (int)dis->itemID;
    int selected = (dis->itemState & ODS_SELECTED) != 0;
    int dark = g_np.darkActive || g_np.themeName[0] != 0;
    HDC dc = dis->hDC;
    RECT rc = dis->rcItem;

    COLORREF bg;
    if(selected) bg = dark ? RGB(62, 62, 62) : GetSysColor(COLOR_WINDOW);
    else bg = dark ? RGB(45, 45, 45) : GetSysColor(COLOR_BTNFACE);
    HBRUSH br = CreateSolidBrush(bg);
    FillRect(dc, &rc, br);
    DeleteObject(br);

    wchar_t label[NP_MAX_PATH + 8] = {0};
    TCITEMW item;
    memset(&item, 0, sizeof(item));
    item.mask = TCIF_TEXT;
    item.pszText = label;
    item.cchTextMax = NP_MAX_PATH + 8;
    SendMessageW(g_np.tabBar, TCM_GETITEMW, idx, (LPARAM)&item);

    COLORREF fg = dark ? RGB(240, 240, 240) : GetSysColor(COLOR_WINDOWTEXT);
    HFONT oldFont = SelectObject(dc, np_get_menu_font());
    int oldBk = SetBkMode(dc, TRANSPARENT);
    COLORREF oldFg = SetTextColor(dc, fg);
    RECT tr = rc;
    tr.left += 8;
    tr.right -= 24;
    DrawTextW(dc, label, -1, &tr,
              DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS);

    /* Close box on the selected tab and the hovered tab. */
    if(selected || idx == g_np.tabHover){
        RECT xr;
        np_tab_x_rect(g_np.tabBar, idx, &xr);
        int hot = idx == g_np.tabHover;
        if(hot){
            HBRUSH hb = CreateSolidBrush(dark ? RGB(90, 90, 90)
                                              : GetSysColor(COLOR_HIGHLIGHT));
            FillRect(dc, &xr, hb);
            DeleteObject(hb);
            SetTextColor(dc, dark ? RGB(255, 255, 255)
                                  : GetSysColor(COLOR_HIGHLIGHTTEXT));
        }
        DrawTextW(dc, L"\xD7", 1, &xr,
                  DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        SetTextColor(dc, fg);
    }
    SetTextColor(dc, oldFg);
    SetBkMode(dc, oldBk);
    SelectObject(dc, oldFont);

    /* Accent underline on the selected tab (system accent, else blue). */
    if(selected){
        COLORREF accent = RGB(LOOS_BRAND_ACCENT_R, LOOS_BRAND_ACCENT_G, LOOS_BRAND_ACCENT_B);
        HMODULE dwm = GetModuleHandleW(L"dwmapi.dll");
        if(dwm){
            typedef HRESULT (WINAPI *GCOLOR_FN)(DWORD *, BOOL *);
            GCOLOR_FN getColor = (GCOLOR_FN)GetProcAddress(dwm,
                "DwmGetColorizationColor");
            if(getColor){
                DWORD col = 0;
                BOOL opaque = FALSE;
                if(SUCCEEDED(getColor(&col, &opaque)) && col)
                    accent = RGB(GetRValue(col), GetGValue(col),
                                 GetBValue(col));
            }
        }
        RECT ar = rc;
        ar.bottom = ar.top + 2;
        ar.left += 2;
        ar.right -= 2;
        HBRUSH ab = CreateSolidBrush(accent);
        FillRect(dc, &ar, ab);
        DeleteObject(ab);
    }
}

static LRESULT CALLBACK np_tab_subproc(HWND h, UINT m, WPARAM w, LPARAM l,
                                       UINT_PTR id, DWORD_PTR ref)
{
    (void)id; (void)ref;
    if(m == WM_LBUTTONDOWN){
        POINT pt;
        pt.x = (int)(short)LOWORD(l);
        pt.y = (int)(short)HIWORD(l);
        TCHITTESTINFO hit;
        memset(&hit, 0, sizeof(hit));
        hit.pt = pt;
        int idx = (int)SendMessageW(h, TCM_HITTEST, 0, (LPARAM)&hit);
        if(idx >= 0 && !(hit.flags & TCHT_NOWHERE)){
            RECT xr;
            np_tab_x_rect(h, idx, &xr);
            if(pt.x >= xr.left && pt.x < xr.right &&
               pt.y >= xr.top && pt.y < xr.bottom){
                np_tab_close(idx);
                return 0;
            }
        }
        return DefSubclassProc(h, m, w, l);
    }
    if(m == WM_MBUTTONDOWN){
        POINT pt;
        pt.x = (int)(short)LOWORD(l);
        pt.y = (int)(short)HIWORD(l);
        TCHITTESTINFO hit;
        memset(&hit, 0, sizeof(hit));
        hit.pt = pt;
        int idx = (int)SendMessageW(h, TCM_HITTEST, 0, (LPARAM)&hit);
        if(idx >= 0 && !(hit.flags & TCHT_NOWHERE)) np_tab_close(idx);
        return 0;
    }
    if(m == WM_MOUSEMOVE){
        POINT pt;
        pt.x = (int)(short)LOWORD(l);
        pt.y = (int)(short)HIWORD(l);
        TCHITTESTINFO hit;
        memset(&hit, 0, sizeof(hit));
        hit.pt = pt;
        int idx = (int)SendMessageW(h, TCM_HITTEST, 0, (LPARAM)&hit);
        if(idx < 0 || (hit.flags & TCHT_NOWHERE)) idx = -1;
        if(idx != g_np.tabHover){
            g_np.tabHover = idx;
            InvalidateRect(h, NULL, FALSE);
        }
        TRACKMOUSEEVENT tme;
        memset(&tme, 0, sizeof(tme));
        tme.cbSize = sizeof(tme);
        tme.dwFlags = TME_LEAVE;
        tme.hwndTrack = h;
        TrackMouseEvent(&tme);
        return DefSubclassProc(h, m, w, l);
    }
    if(m == WM_MOUSELEAVE){
        if(g_np.tabHover != -1){
            g_np.tabHover = -1;
            InvalidateRect(h, NULL, FALSE);
        }
        return DefSubclassProc(h, m, w, l);
    }
    return DefSubclassProc(h, m, w, l);
}

/* ==========================================================================
 *  Overview strip (simple minimap): density blocks + click-to-jump
 * ==========================================================================
 *  A narrow owner-painted strip right of the editor. Each row aggregates
 *  one line band's character count into a brightness block; the caret band
 *  gets the accent underline. Click/drag jumps the editor there. No text
 *  is cached (lengths come straight from the EDIT control), so huge files
 *  cost nothing extra. View > Overview Strip, persisted as fOverview. */

static LRESULT CALLBACK np_overview_proc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    switch(m){
    case WM_PAINT: {
        PAINTSTRUCT ps;
        HDC dc = BeginPaint(h, &ps);
        RECT rc;
        GetClientRect(h, &rc);
        int dark = g_np.darkActive || g_np.themeName[0] != 0;
        COLORREF bg = dark ? RGB(38, 38, 38) : GetSysColor(COLOR_BTNFACE);
        HBRUSH br = CreateSolidBrush(bg);
        FillRect(dc, &rc, br);
        DeleteObject(br);
        int total = (int)SendMessageW(g_np.editBox, EM_GETLINECOUNT, 0, 0);
        if(total < 1) total = 1;
        int rows = (rc.bottom - rc.top) / 2;
        if(rows < 1) rows = 1;
        DWORD s = 0, e = 0;
        SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
        int caretLine = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, e, 0);
        for(int r = 0; r < rows; r++){
            int line = (int)((long long)r * total / rows);
            int idx = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, line, 0);
            int ink = 0;
            if(idx >= 0){
                int ll = (int)SendMessageW(g_np.editBox, EM_LINELENGTH, idx, 0);
                ink = (ll > 120) ? 120 : ll;
            }
            int shade;
            if(dark) shade = 70 + ink * 140 / 120;
            else shade = 200 - ink * 130 / 120;
            HBRUSH b = CreateSolidBrush(RGB(shade, shade, shade));
            RECT rr;
            rr.left = rc.left + 8;
            rr.right = rc.right - 8;
            rr.top = rc.top + r * 2;
            rr.bottom = rr.top + 1;
            FillRect(dc, &rr, b);
            DeleteObject(b);
        }
        /* Caret band marker. */
        {
            int r = (int)((long long)caretLine * rows / total);
            RECT rr;
            rr.left = rc.left + 4;
            rr.right = rc.right - 4;
            rr.top = rc.top + r * 2 - 1;
            rr.bottom = rr.top + 3;
            HBRUSH ab = CreateSolidBrush(RGB(0, 120, 215));
            FillRect(dc, &rr, ab);
            DeleteObject(ab);
        }
        EndPaint(h, &ps);
        return 0;
    }
    case WM_LBUTTONDOWN:
    case WM_MOUSEMOVE: {
        if(m == WM_MOUSEMOVE && !(w & MK_LBUTTON)) break;
        RECT rc;
        GetClientRect(h, &rc);
        int y = (int)(short)HIWORD(l);
        if(y < 0) y = 0;
        if(y > rc.bottom) y = rc.bottom;
        int total = (int)SendMessageW(g_np.editBox, EM_GETLINECOUNT, 0, 0);
        if(total < 1) total = 1;
        int want = (int)((long long)y * total / (rc.bottom - rc.top + 1));
        if(want >= total) want = total - 1;
        int cur = (int)SendMessageW(g_np.editBox, EM_GETFIRSTVISIBLELINE, 0, 0);
        /* Center the target line: scroll delta, then place caret. */
        {
            RECT er;
            GetClientRect(g_np.editBox, &er);
            HDC dc = GetDC(g_np.editBox);
            HFONT hf = (HFONT)SendMessageW(g_np.editBox, WM_GETFONT, 0, 0);
            HFONT old = SelectObject(dc, hf);
            SIZE sz;
            GetTextExtentPoint32W(dc, L"Ay", 2, &sz);
            SelectObject(dc, old);
            ReleaseDC(g_np.editBox, dc);
            int visLines = sz.cy > 0 ? (er.bottom - er.top) / sz.cy : 25;
            if(visLines < 1) visLines = 1;
            int top = want - visLines / 2;
            if(top < 0) top = 0;
            SendMessageW(g_np.editBox, EM_LINESCROLL, 0, top - cur);
            int idx = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, want, 0);
            if(idx >= 0) SendMessageW(g_np.editBox, EM_SETSEL, idx, idx);
        }
        SetFocus(g_np.editBox);
        InvalidateRect(h, NULL, FALSE);
        return 0;
    }
    }
    return DefWindowProcW(h, m, w, l);
}

static void np_overview_toggle(void)
{
    g_np.showOverview = !g_np.showOverview;
    if(g_np.showOverview && !g_np.overviewBar){
        g_np.overviewBar = CreateWindowExW(0, L"NPOverview", NULL,
            WS_CHILD | WS_VISIBLE, 0, 0, 60, 100,
            g_np.mainWnd, NULL, GetModuleHandleW(NULL), NULL);
    }
    if(!g_np.stickyOn){
        if(g_np.showOverview && g_np.overviewBar)
            SetTimer(g_np.mainWnd, IDT_STICKY, 500, NULL);
        else
            KillTimer(g_np.mainWnd, IDT_STICKY);
    }
    np_on_size();
    np_refresh_title();
    np_save_settings();
}

/* ==========================================================================
 *  Built-in supervisor: notepad.exe --watch [args...]
 * ==========================================================================
 *  Same binary, no separate watchdog.exe. Supervises a child instance of
 *  itself: abnormal exits (anything but code 0, e.g. 0xC0000005) relaunch
 *  with session, autosave snapshots and version history intact. Clean exit
 *  ends the watch. Crash-loop guard: 3 abnormal deaths inside 60 seconds
 *  stops with a message instead of respawning forever. Flags: --watch,
 *  /watch, -w. */

static int np_watchdog(const WCHAR *args)
{
    wchar_t self[NP_MAX_PATH], dir[NP_MAX_PATH];
    GetModuleFileNameW(NULL, self, NP_MAX_PATH);
    wcsncpy_s(dir, NP_MAX_PATH, self, _TRUNCATE);
    wchar_t *slash = wcsrchr(dir, L'\\');
    if(slash) *slash = L'\0';
    else wcscpy_s(dir, NP_MAX_PATH, L".");

    wchar_t cmd[32768];
    swprintf_s(cmd, 32768, L"\"%s\"%s%s", self,
               args[0] ? L" " : L"", args[0] ? args : L"");
    OutputDebugStringW(L"[watchdog] supervising ");
    OutputDebugStringW(self);

    DWORD crashes[8] = {0};
    int crashCount = 0;
    for(;;){
        STARTUPINFOW si;
        PROCESS_INFORMATION pi;
        memset(&si, 0, sizeof(si));
        si.cb = sizeof(si);
        memset(&pi, 0, sizeof(pi));
        wchar_t run[32768];
        wcsncpy_s(run, 32768, cmd, _TRUNCATE);
        if(!CreateProcessW(NULL, run, NULL, NULL, FALSE, 0,
                           NULL, dir, &si, &pi)){
            DWORD err = GetLastError();
            wchar_t msg[128];
    swprintf_s(msg, 128, L"Cannot start supervised LoOS Notepad (error %lu).",
                       err);
            MessageBoxW(NULL, msg, L"LoOS Watchdog", MB_OK | MB_ICONSTOP);
            return 1;
        }
        WaitForSingleObject(pi.hProcess, INFINITE);
        DWORD code = 0;
        GetExitCodeProcess(pi.hProcess, &code);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        if(code == 0) return 0;
        DWORD now = GetTickCount();
        crashes[crashCount % 8] = now;
        crashCount++;
        int recent = 0;
        for(int i = 0; i < crashCount && i < 8; i++)
            if(now - crashes[i] < 60000) recent++;
        if(recent >= 3){
            MessageBoxW(NULL,
                L"Notepad crashed repeatedly. Your session and autosave "
                L"snapshots are preserved - reopen when ready.",
                L"LoOS Watchdog", MB_OK | MB_ICONWARNING);
            return 1;
        }
    }
}

/* ==========================================================================
 *  Standard power shortcuts: duplicate/delete line, reopen closed tab
 * ==========================================================================
 *  (NP_CLOSED_MAX / NP_CLOSED / g_closed live up next to NP_TAB so the
 *  window procedure can use them.) */

/* Remember a closed tab for Ctrl+Shift+T (newest slot wins, cap 10). */
static void np_closed_push(const NP_TAB *t)
{
    int hasText = (t->text && t->text[0]) ? 1 : 0;
    if(!t->path[0] && !hasText) return;
    if(g_closedCount >= NP_CLOSED_MAX){
        free(g_closed[0].text);
        for(int i = 0; i + 1 < NP_CLOSED_MAX; i++)
            g_closed[i] = g_closed[i + 1];
        g_closedCount = NP_CLOSED_MAX - 1;
    }
    NP_CLOSED *c = &g_closed[g_closedCount++];
    memset(c, 0, sizeof(*c));
    wcsncpy_s(c->path, NP_MAX_PATH, t->path, _TRUNCATE);
    if(hasText){
        size_t n = wcslen(t->text) + 1;
        c->text = malloc(n * sizeof(WCHAR));
        if(c->text) memcpy(c->text, t->text, n * sizeof(WCHAR));
        else{ g_closedCount--; return; }
    }
    c->enc = t->enc;
    c->bom = t->bom;
    c->eol = t->eol;
    c->selS = t->selS;
    c->selE = t->selE;
    c->largeFile = t->largeFile;
}

static int np_closed_reopen(void)
{
    if(g_closedCount == 0){
        np_announce(L"Nothing to reopen");
        return 0;
    }
    NP_CLOSED *c = &g_closed[--g_closedCount];
    np_tab_new();
    wcsncpy_s(g_np.path, NP_MAX_PATH, c->path, _TRUNCATE);
    g_np.encoding = c->enc;
    g_np.bomPresent = c->bom;
    g_np.eolMode = c->eol;
    if(g_np.tabActive >= 0 && g_np.tabActive < g_np.tabCount)
        g_np.tabs[g_np.tabActive].largeFile = c->largeFile;
    if(c->largeFile && g_np.wrap){
        g_np.wrap = 0;
        ShowWindow(g_np.statusBar, g_np.showBar ? SW_SHOW : SW_HIDE);
    }
    g_np.editGuard = 1;
    SetWindowTextW(g_np.editBox, c->text ? c->text : L"");
    g_np.editGuard = 0;
    SendMessageW(g_np.editBox, EM_SETSEL, c->selS, c->selE);
    free(c->text);
    c->text = NULL;
    c->path[0] = L'\0';
    np_set_dirty(1);
    np_refresh_title();
    np_update_status();
    return 1;
}

/* Duplicate the selection, or the whole caret line with its line break. */
static void np_edit_dupline(void)
{
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    if(s != e){
        int n = (int)(e - s);
        WCHAR *sel = malloc(((size_t)n + 1) * sizeof(WCHAR));
        if(!sel){ free(buf); return; }
        memcpy(sel, buf + s, (size_t)n * sizeof(WCHAR));
        sel[n] = L'\0';
        free(buf);
        SendMessageW(g_np.editBox, EM_SETSEL, e, e);
        SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)sel);
        SendMessageW(g_np.editBox, EM_SETSEL, e, e + n);
        free(sel);
        return;
    }
    int line = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, s, 0);
    int total = (int)SendMessageW(g_np.editBox, EM_GETLINECOUNT, 0, 0);
    int start = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, line, 0);
    int endNext = (line + 1 < total)
        ? (int)SendMessageW(g_np.editBox, EM_LINEINDEX, line + 1, 0) : len;
    if(start < 0) start = 0;
    if(line + 1 < total){
        /* Copy includes the CRLF; insert right after it. */
        int n = endNext - start;
        WCHAR *cp = malloc(((size_t)n + 1) * sizeof(WCHAR));
        if(!cp){ free(buf); return; }
        memcpy(cp, buf + start, (size_t)n * sizeof(WCHAR));
        cp[n] = L'\0';
        free(buf);
        SendMessageW(g_np.editBox, EM_SETSEL, endNext, endNext);
        SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)cp);
        SendMessageW(g_np.editBox, EM_SETSEL, endNext, endNext + n);
        free(cp);
    }else{
        /* Last line has no break: append CRLF + copy. */
        int n = len - start;
        WCHAR *cp = malloc(((size_t)n + 4) * sizeof(WCHAR));
        if(!cp){ free(buf); return; }
        cp[0] = L'\r'; cp[1] = L'\n';
        memcpy(cp + 2, buf + start, (size_t)n * sizeof(WCHAR));
        cp[n + 2] = L'\0';
        free(buf);
        SendMessageW(g_np.editBox, EM_SETSEL, len, len);
        SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)cp);
        SendMessageW(g_np.editBox, EM_SETSEL, len + 2, len + 2 + n);
        free(cp);
    }
}

/* Delete the selected lines (expanded), or the whole caret line. */
static void np_edit_delline(void)
{
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    int len = GetWindowTextLengthW(g_np.editBox);
    if(len == 0) return;
    int l0 = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, s, 0);
    int l1 = (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, e, 0);
    int total = (int)SendMessageW(g_np.editBox, EM_GETLINECOUNT, 0, 0);
    if(l1 >= total) l1 = total - 1;
    int start = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, l0, 0);
    int endNext = (l1 + 1 < total)
        ? (int)SendMessageW(g_np.editBox, EM_LINEINDEX, l1 + 1, 0) : len;
    if(start < 0) start = 0;
    if(endNext <= start){
        /* Single last line without break: clear from its start. */
        SendMessageW(g_np.editBox, EM_SETSEL, start, len);
    }else{
        SendMessageW(g_np.editBox, EM_SETSEL, start, endNext);
    }
    SendMessageW(g_np.editBox, EM_REPLACESEL, TRUE, (LPARAM)L"");
    SetFocus(g_np.editBox);
}

/* ==========================================================================
 *  Dark-everywhere helpers + custom caret + owner-drawn status
 * ==========================================================================
 *  Every dialog we own (Find/Replace/GoTo/History/input/palette) routes
 *  WM_CTLCOLOR* through np_dark_ctl and themes its children once via
 *  np_dark_dlg_init. System common dialogs get the same through their
 *  hook procs. Light mode returns 0 everywhere (fully native). */

/* Shared control painter for dialog WM_CTLCOLOR*. Returns a brush, or 0
 * to let the system draw (light mode). */
static LRESULT np_dark_ctl(HDC hdc, HWND ctl)
{
    COLORREF bg = 0, fg = 0;
    int dark = 0;
    (void)ctl;
    np_effective_colors(&bg, &fg, &dark);
    if(!dark && !g_np.themeName[0]) return 0;
    SetTextColor(hdc, fg);
    SetBkColor(hdc, bg);
    if(!g_np.themeBrush){
        COLORREF b2 = 0, f2 = 0;
        int d2 = 0;
        np_effective_colors(&b2, &f2, &d2);
        g_np.themeBrush = CreateSolidBrush(b2);
    }
    return (LRESULT)g_np.themeBrush;
}

static BOOL CALLBACK np_dark_child(HWND h, LPARAM l)
{
    (void)l;
    COLORREF bg = 0, fg = 0;
    int dark = 0;
    np_effective_colors(&bg, &fg, &dark);
    if(!dark && !g_np.themeName[0]) return TRUE;
    wchar_t cls[32] = {0};
    GetClassNameW(h, cls, 32);
    if(_wcsicmp(cls, L"Button") == 0 || _wcsicmp(cls, L"ComboBox") == 0 ||
       _wcsicmp(cls, L"ListBox") == 0 || _wcsicmp(cls, L"SysLink") == 0 ||
       _wcsicmp(cls, L"Edit") == 0)
        SetWindowTheme(h, L"DarkMode_Explorer", NULL);
    return TRUE;
}

static void np_dark_dlg_init(HWND dlg)
{
    COLORREF bg = 0, fg = 0;
    int dark = 0;
    np_effective_colors(&bg, &fg, &dark);
    if(!dark && !g_np.themeName[0]) return;
    EnumChildWindows(dlg, np_dark_child, 0);
    InvalidateRect(dlg, NULL, TRUE);
}

/* --- Owner-drawn status bar: white text on dark, native otherwise --- */

static void np_status_draw(const DRAWITEMSTRUCT *dis)
{
    int part = (int)dis->itemID;
    if(part < 0 || part > 3) return;
    COLORREF bg = 0, fg = 0;
    int dark = 0;
    np_effective_colors(&bg, &fg, &dark);
    HDC dc = dis->hDC;
    RECT rc = dis->rcItem;
    if(!dark && !g_np.themeName[0]){
        /* Should not happen (parts only go owner-drawn in dark), but
         * stay readable no matter what. */
        FillRect(dc, &rc, GetSysColorBrush(COLOR_BTNFACE));
        fg = GetSysColor(COLOR_WINDOWTEXT);
    }else{
        HBRUSH br = CreateSolidBrush(bg);
        FillRect(dc, &rc, br);
        DeleteObject(br);
    }
    HFONT oldFont = SelectObject(dc, np_get_menu_font());
    int oldBk = SetBkMode(dc, TRANSPARENT);
    COLORREF oldFg = SetTextColor(dc, fg);
    RECT tr = rc;
    tr.left += 6;
    DrawTextW(dc, g_np.sbText[part], -1, &tr,
              DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS);
    SetTextColor(dc, oldFg);
    SetBkMode(dc, oldBk);
    SelectObject(dc, oldFont);
}

/* --- Custom caret: the EDIT control only draws black, invisible on dark.
 * We paint a 2px white caret on a blink timer whenever dark, focused and
 * collapsed. The black system caret hides underneath (same geometry). --- */

static void np_caret_pos(RECT *out)
{
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    DWORD pos = (DWORD)SendMessageW(g_np.editBox, EM_POSFROMCHAR, s, 0);
    HDC dc = GetDC(g_np.editBox);
    HFONT hf = (HFONT)SendMessageW(g_np.editBox, WM_GETFONT, 0, 0);
    HFONT old = SelectObject(dc, hf);
    SIZE sz;
    GetTextExtentPoint32W(dc, L"Ay", 2, &sz);
    SelectObject(dc, old);
    ReleaseDC(g_np.editBox, dc);
    out->left = (int)(short)LOWORD(pos);
    out->top = (int)(short)HIWORD(pos);
    out->right = out->left + 2;
    out->bottom = out->top + sz.cy;
}

static void np_caret_tick(void)
{
    COLORREF bg = 0, fg = 0;
    int dark = 0;
    np_effective_colors(&bg, &fg, &dark);
    if(!dark || !g_np.caretFocus){
        if(g_np.caretPhase){
            g_np.caretPhase = 0;
            RECT r;
            np_caret_pos(&r);
            InvalidateRect(g_np.editBox, &r, FALSE);
        }
        return;
    }
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    if(s != e){
        if(g_np.caretPhase){
            g_np.caretPhase = 0;
            np_update_status();
        }
        return;
    }
    g_np.caretPhase = !g_np.caretPhase;
    if(g_np.caretPhase){
        RECT r;
        np_caret_pos(&r);
        HDC dc = GetDC(g_np.editBox);
        HBRUSH br = CreateSolidBrush(RGB(255, 255, 255));
        FillRect(dc, &r, br);
        DeleteObject(br);
        ReleaseDC(g_np.editBox, dc);
        /* Remember where it is so the erase pass invalidates tightly. */
        g_np.caretRect = r;
    }else{
        InvalidateRect(g_np.editBox, &g_np.caretRect, FALSE);
    }
}

/* --- About dialog (themed, ours, with the LoOS logo) --- */

static INT_PTR CALLBACK np_about_proc(HWND dlg, UINT msg, WPARAM w, LPARAM l)
{
    (void)l;
    switch(msg){
    case WM_INITDIALOG: {
        np_dark_dlg_init(dlg);
        wchar_t info[512];
        swprintf_s(info, 512,
            L"LoOS Notepad %s\r\n\r\nNative power text editor.\r\n"
            L"No telemetry. No AI. No accounts.\r\n\r\n"
            L"Tabs, regex, macros, Markdown preview,\r\n"
            L"version history, plugins and more.",
            NP_VERSION);
        SetDlgItemTextW(dlg, 101, info);
        return TRUE;
    }
    case WM_CTLCOLORDLG: {
        LRESULT br = np_dark_ctl((HDC)w, dlg);
        if(br) return br;
        break;
    }
    case WM_CTLCOLOREDIT:
    case WM_CTLCOLORSTATIC:
    case WM_CTLCOLORLISTBOX:
    case WM_CTLCOLORBTN: {
        LRESULT br = np_dark_ctl((HDC)w, (HWND)l);
        if(br) return br;
        break;
    }
    case WM_COMMAND:
        if(LOWORD(w) == IDOK || LOWORD(w) == IDCANCEL){
            EndDialog(dlg, IDOK);
            return TRUE;
        }
        break;
        break;
    }
    return FALSE;
}

static void np_show_about(void)
{
    DialogBoxParamW(GetModuleHandleW(NULL), MAKEINTRESOURCEW(16),
                    g_np.mainWnd, np_about_proc, 0);
}

/* ==========================================================================
 *  Plugin ABI v3 services, events and idle ticks
 * ========================================================================== */

static int s_activePlugin = -1; /* dispatch/event context for private dir */

static int np_host_getLineCount(void)
{
    return (int)SendMessageW(g_np.editBox, EM_GETLINECOUNT, 0, 0);
}

static int np_host_getLine(int line, WCHAR *out, int outChars)
{
    if(!out || outChars <= 1) return 0;
    int total = np_host_getLineCount();
    if(line < 0) line = 0;
    if(line >= total) line = total - 1;
    return np_get_line_text(g_np.editBox, line, out, outChars);
}

static int np_host_getCaretLine(void)
{
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    return (int)SendMessageW(g_np.editBox, EM_LINEFROMCHAR, e, 0);
}

static void np_host_gotoLine(int line)
{
    int total = np_host_getLineCount();
    if(line < 0) line = 0;
    if(line >= total) line = total - 1;
    int idx = (int)SendMessageW(g_np.editBox, EM_LINEINDEX, line, 0);
    if(idx < 0) idx = 0;
    SendMessageW(g_np.editBox, EM_SETSEL, idx, idx);
    SendMessageW(g_np.editBox, EM_SCROLLCARET, 0, 0);
    SetFocus(g_np.editBox);
    np_update_status();
}

static void np_host_getSelText(WCHAR *out, int outChars)
{
    if(!out || outChars <= 0) return;
    out[0] = L'\0';
    DWORD s = 0, e = 0;
    SendMessageW(g_np.editBox, EM_GETSEL, (WPARAM)&s, (LPARAM)&e);
    if(s == e) return;
    int len = GetWindowTextLengthW(g_np.editBox);
    WCHAR *buf = malloc(((size_t)len + 1) * sizeof(WCHAR));
    if(!buf) return;
    GetWindowTextW(g_np.editBox, buf, len + 1);
    int n = (int)(e - s);
    if(n >= outChars) n = outChars - 1;
    if(n > len - (int)s) n = len - (int)s;
    if(n > 0){
        memcpy(out, buf + s, (size_t)n * sizeof(WCHAR));
        out[n] = L'\0';
    }
    free(buf);
}

static int np_host_inputBox(const WCHAR *title, const WCHAR *prompt,
                            const WCHAR *def, WCHAR *out, int outChars)
{
    wchar_t value[64] = {0}, unused[64] = {0};
    int r = np_input_box(title ? title : L"Plugin",
                         prompt ? prompt : L"Value:",
                         def ? def : L"", L"", L"", value, unused);
    if(r != IDOK || !out || outChars <= 0) return IDCANCEL;
    wcsncpy_s(out, outChars, value, _TRUNCATE);
    return IDOK;
}

static int np_host_messageBox(const WCHAR *text, int buttons)
{
    UINT type = MB_OK;
    if(buttons == 1) type = MB_OKCANCEL;
    else if(buttons == 2) type = MB_YESNO | MB_ICONQUESTION;
    else if(buttons == 3) type = MB_YESNOCANCEL | MB_ICONQUESTION;
    else type |= MB_ICONINFORMATION;
    return MessageBoxW(g_np.mainWnd, text ? text : L"", S_APP, type);
}

/* Per-plugin private dir: config\plugins_data\<dllname>, created. */
static void np_private_dir(int plugin, WCHAR *out, size_t outChars)
{
    wchar_t base[NP_MAX_PATH];
    np_session_dir(base, NP_MAX_PATH);
    wchar_t name[64] = {0};
    if(plugin >= 0 && plugin < g_np.pluginCount &&
       g_np.plugins[plugin].fileName[0]){
        wcsncpy_s(name, 64, g_np.plugins[plugin].fileName, _TRUNCATE);
        wchar_t *dot = wcsrchr(name, L'.');
        if(dot) *dot = L'\0';
    }else{
        wcsncpy_s(name, 64, L"shared", _TRUNCATE);
    }
    swprintf_s(out, outChars, L"%s\\plugins_data\\%s", base, name);
    {
        wchar_t tmp[NP_MAX_PATH];
        wcsncpy_s(tmp, NP_MAX_PATH, out, _TRUNCATE);
        for(wchar_t *p = tmp; *p; p++){
            if(*p == L'\\'){
                wchar_t save = *p; *p = L'\0';
                if(wcslen(tmp) > 3) CreateDirectoryW(tmp, NULL);
                *p = save;
            }
        }
        CreateDirectoryW(tmp, NULL);
    }
}

static void np_host_getPrivateDir(WCHAR *out, int outChars)
{
    if(!out || outChars <= 0) return;
    wchar_t tmp[NP_MAX_PATH];
    np_private_dir(s_activePlugin, tmp, NP_MAX_PATH);
    wcsncpy_s(out, outChars, tmp, _TRUNCATE);
}

static void np_private_ini(int plugin, WCHAR *out, size_t outChars)
{
    wchar_t dir[NP_MAX_PATH];
    np_private_dir(plugin, dir, NP_MAX_PATH);
    swprintf_s(out, outChars, L"%s\\plugin.ini", dir);
}

static int np_host_readPrivate(const WCHAR *name, WCHAR *out, int outChars)
{
    if(!name || !out || outChars <= 0) return 0;
    wchar_t ini[NP_MAX_PATH];
    np_private_ini(s_activePlugin, ini, NP_MAX_PATH);
    GetPrivateProfileStringW(L"data", name, L"", out, outChars, ini);
    return out[0] != L'\0';
}

static int np_host_writePrivate(const WCHAR *name, const WCHAR *value)
{
    if(!name) return 0;
    wchar_t ini[NP_MAX_PATH];
    np_private_ini(s_activePlugin, ini, NP_MAX_PATH);
    return WritePrivateProfileStringW(L"data", name,
                                      value ? value : L"", ini) != 0;
}

static int np_host_getEncoding(void)
{
    return (int)g_np.encoding; /* 0 ANSI, 1 UTF-8, 2 UTF-16LE, 3 UTF-16BE */
}

static int np_host_isDirty(void)
{
    return g_np.dirty ? 1 : 0;
}

static void np_host_openFile(const WCHAR *path)
{
    if(!path || !path[0]) return;
    if(GetFileAttributesW(path) == INVALID_FILE_ATTRIBUTES){
        MessageBoxW(g_np.mainWnd, L"File not found.", S_APP,
                    MB_OK | MB_ICONWARNING);
        return;
    }
    if(g_np.path[0] || g_np.dirty ||
       GetWindowTextLengthW(g_np.editBox) != 0)
        np_tab_new();
    np_load_file(path, -1, 0, 0);
}

static void np_host_init(void)
{
    memset(&g_host3, 0, sizeof(g_host3));
    /* v2 prefix (order locked to NP_HOST). */
    g_host3.version = 3;
    g_host3.editBox = g_np.editBox;
    g_host3.mainWnd = g_np.mainWnd;
    g_host3.getSel = np_host_getSel;
    g_host3.setSel = np_host_setSel;
    g_host3.getTextLen = np_host_getTextLen;
    g_host3.getText = np_host_getText;
    g_host3.replaceSel = np_host_replaceSel;
    g_host3.replaceAll = np_host_replaceAll;
    g_host3.getPath = np_host_getPath;
    g_host3.setStatus = np_host_setStatus;
    g_host3.getConfigDir = np_host_getConfigDir;
    g_host3.confirm = np_host_confirm;
    /* v3 services. */
    g_host3.getLineCount = np_host_getLineCount;
    g_host3.getLine = np_host_getLine;
    g_host3.getCaretLine = np_host_getCaretLine;
    g_host3.gotoLine = np_host_gotoLine;
    g_host3.getSelText = np_host_getSelText;
    g_host3.inputBox = np_host_inputBox;
    g_host3.messageBox = np_host_messageBox;
    g_host3.getPrivateDir = np_host_getPrivateDir;
    g_host3.readPrivate = np_host_readPrivate;
    g_host3.writePrivate = np_host_writePrivate;
    g_host3.getEncoding = np_host_getEncoding;
    g_host3.isDirty = np_host_isDirty;
    g_host3.openFile = np_host_openFile;
    /* Back-compat view for v2 plugins (identical prefix). */
    memcpy(&g_npHost, &g_host3, sizeof(NP_HOST));
}

/* Fire a notification event to every plugin that wants it. */
static void np_fire_event(int event)
{
    for(int i = 0; i < g_np.pluginCount; i++){
        if(!g_np.plugins[i].onEvent) continue;
        int saved = s_activePlugin;
        s_activePlugin = i;
        g_np.plugins[i].onEvent(event, (const NP_HOST *)&g_host3);
        s_activePlugin = saved;
    }
}

/* Idle ticks for plugins that export NpPluginIdle (2s cadence). */
static void np_idle_tick(void)
{
    for(int i = 0; i < g_np.pluginCount; i++){
        if(!g_np.plugins[i].onIdle) continue;
        int saved = s_activePlugin;
        s_activePlugin = i;
        g_np.plugins[i].onIdle((const NP_HOST *)&g_host3);
        s_activePlugin = saved;
    }
}

static int np_any_idle(void)
{
    for(int i = 0; i < g_np.pluginCount; i++)
        if(g_np.plugins[i].onIdle) return 1;
    return 0;
}

/* POWERAPPEND */
