# LoOS notepad.exe — Documentation

## About LoOS

LoOS is a custom desktop operating system built on top of the leaked
Windows XP / Server 2003 source code. The goals of the project are:

- **No telemetry.** No Cortana, no activity history upload, no
  diagnostic-data scheduling tasks. The components that ship with
  LoOS do not call home. Behavioural tracking services are removed
  at install time and replaced with local-only stubs.
- **Modern look and feel.** The shell, taskbar, start menu, file
  explorer and consoles are restyled to look like Windows 10 / 11
  (rounded corners, dark-mode tokens, Segoe UI Variable, Mica-style
  surfaces) while keeping the NT 5.2 kernel family underneath for
  stability and tiny footprint.
- **Modern app compatibility.** LoOS ships a *shim layer* on top of
  the NT 6.x / NT 10.x system calls so unmodified Windows 10 / 11
  user-mode binaries load and run.

`notepad.exe` is the native text editor for LoOS — it replaces
Windows' `notepad.exe` wholesale. It is a clean-room reconstruction
of the decompiled Windows 11 `notepad.exe` rewritten in a single,
human-readable C file (`notepad.c`), with the genuine dialogs and
icon (`notepad.rc`, `notepad.ico`), then extended with tabs, regex,
and pro editing tools.

Stripped out, by design: ETW telemetry (`EventWriteTransfer` and
friends — this build registers no providers), Windows Error
Reporting calls, `SetWinEventHook`, the "Send Feedback" menu item,
all Copilot/AI features (those live only in the separate XAML
Store app, which is not our target), the Bing search command and the
Store promo banner. Only local screen-reader announcements
(`NotifyWinEvent`) remain; nothing leaves the machine except through
links you click yourself (View Help).

## Building

Prerequisites on the build VM: MSVC Build Tools (`C:\BuildTools`),
Windows 10 SDK (`rc.exe`), no third-party dependencies.

```
build.bat
```

This compiles `notepad.rc` to `C:\LoOS\out\notepad.res`, then
`notepad.c` with `/W4` (zero warnings) and links
`C:\LoOS\out\notepad.exe` as a GUI-subsystem x64 binary, plus a copy of
`notepad.exe.manifest` (also linked into the exe from
`notepad.rc`, so the single binary carries comctl32 v6, PerMonitorV2
DPI and long paths by itself).

## Classic behaviour (preserved 1:1)

Window class `Notepad` with one multiline EDIT control per view.
Menus, command IDs, shortcuts, dialog layouts, message strings,
encodings and registry value names match the original binary:

| Menu | Items |
|---|---|
| File | New, New Window, New Tab, Close Tab, Open, Save, Save As, Version History, Page Setup, Print, Exit |
| Edit | Undo, Cut, Copy, Paste, Delete, Find, Find Next, Find Previous, Replace, Go To, Select All, Time/Date, Sort Lines, Evaluate Math, Macro, Columns |
| Format | Word Wrap, Font, Auto-Close Pairs (power addition) |
| View | Zoom submenu, Status Bar, Toolbar, Theme, Markdown Preview, Sticky Scroll, Split View, Distraction Free, Always on Top, Sticky Note |
| Tools | Command Palette, plugin commands |
| Help | View Help, About Notepad |

Encodings: ANSI, UTF-8 (with/without BOM), UTF-16 LE/BE, with an
encoding dropdown hooked into the Open/Save dialogs, BOM handling,
EOL detection (CRLF/LF/CR) shown live in the status bar, and a
Unicode-loss guard when saving Unicode text as ANSI.

Settings persist under `HKCU\Software\Microsoft\Notepad` using the
original value names (`fWrap`, `lfFaceName`, `searchString`, …) plus
power additions (`iTheme`, `sTheme`, `fToolbar`, `fRegex`, …).

## Power features

### Tabs and sessions
- `Ctrl+T` new tab, `Ctrl+Shift+W` close tab (dirty tabs prompt),
  `Ctrl+Tab` / `Ctrl+Shift+Tab` switch, click `×` or middle-click
  closes, per-tab encoding/dirty/selection/scroll state.
- Sessions (`%APPDATA%\LoOS\Notepad\session.ini`) restore open files,
  caret positions and untitled buffers. Every dirty tab snapshots
  every 30 s; crashes offer full recovery on next start.
- `Ctrl+P` fuzzy palette over open tabs plus the 20 most recent files
  (subsequence matching with boundary/consecutive bonuses).

### Find and replace
Literal or regex (`Use regular expressions`). Engine supports `^ $ .`
`* + ? {m,n} {m,} {m} | ( ) [ranges] [^neg]` and
`\d \D \w \W \s \S \b` plus escapes; replacements expand `$1`–`$9`
(`$$` for a literal dollar). Matching is step-capped: pathological
patterns report "too complex" instead of hanging. Live match count in
both dialogs; both windows are resizable; direction, case, whole word
and wrap-around persist.

### Editing tools
- **Sort Lines**: selection (expanded to full lines) or whole file;
  A-Z, Z-A, numeric by leading number, by length, de-dupe (first wins),
  trim trailing whitespace.
- **Evaluate Math** (`Ctrl+M`): replaces the selection — or the
  expression around the caret — with its value. Integer arithmetic
  `+ - * / % ^` with parentheses.
- **Links**: `http(s)://` and `www.` auto-highlight; click opens,
  hover previews the URL.
- **Pairs**: typing `([{"'` inserts the closer; typing the closer skips
  over it; Backspace on an empty pair deletes both. Smart Backspace
  deletes back to the 4-space tab stop. Toggle: Format menu.
- **Autocomplete**: document words, automatic after 3 chars,
  `Ctrl+Space` forces; `Enter`/`Tab` accept, arrows navigate, `Esc`
  dismisses.
- **Macros**: `Ctrl+Shift+R` toggles recording, `Ctrl+Shift+E` plays
  back; the macro persists in `%APPDATA%\LoOS\Notepad\macro.bin`.
- **Columns**: dialog-driven insert-at-column / delete-range /
  copy-range over the selected lines (1-based columns, short lines are
  space-padded).
- **Multi-caret**: `Ctrl+Click` adds carets (status bar shows the
  count); typing, Backspace and Delete apply at all of them. `Esc`
  collapses. Only the main caret is drawn (EDIT control limit).
- **Hex colors**: hovering `#RRGGBB` shows the RGB value;
  `Alt+Click` opens the color chooser and rewrites the code.

### Panes, history, modes
- **Markdown Preview** (`View`): live RichEdit pane rendering
  headings, bold/italic, code, links, quotes, lists and fenced blocks
  (debounced 400 ms; skipped for large files).
- **Version History** (`File`): every save snapshots a timestamped
  copy (newest 50 kept); restore with one click (asks first).
- **Sticky Scroll**: pins the current Markdown heading above the edit
  surface; updates on edit, scroll and caret moves.
- **Split View**: second pane bound to another tab, vertical or
  horizontal, optional scroll sync.
- **Overview Strip**: density minimap with click/drag-to-jump and a
  caret marker (`View > Overview Strip`, persisted as `fOverview`).
- **Distraction Free** (`F11`): borderless fullscreen, chrome hidden.
- **Always on Top** and **Sticky Note** (borderless, topmost, compact;
  `Alt+drag` moves it).

### Shortcuts
Standard bindings plus power keys, all live: `Ctrl+D` duplicate
line, `Ctrl+L` delete line, `Ctrl+Shift+T` reopen closed tab (last
10 kept), `Ctrl+W` closes the tab, `Ctrl+Q` quits, `Ctrl+Y`
redo-toggles the single-level Undo, `F11` distraction-free.

### Themes and UI polish
Dark mode covers every surface we own: frame, edit surface with a
white caret, owner-drawn status bar, dark menus/toolbar/tabs/banner
strip, all Find/Replace/GoTo/history/input/palette dialogs, the
encoding strip, and the Font/Print dialogs via hooks (their list
views stay native). System common dialogs are darkened best-effort.
System/Light/Dark plus `.theme` files
(`%APPDATA%\LoOS\Notepad\themes\NAME.theme` with `bg=`/`fg=` hex
under a `[theme]` section) appended to the View menu. Dark title bar
and Mica backdrop via dynamically loaded `dwmapi` (older Windows
ignores it); dark scrollbars/controls via `DarkMode_Explorer` themes;
owner-drawn dark menus; slim text toolbar (`View > Toolbar`);
tab close buttons; Segoe UI chrome font; PerMonitorV2 DPI awareness;
selection count and caret count in the status bar.

### Large files
Files ≥ 32 MB open in large-file mode: word wrap locked off, undo
buffer cleared, autosave skipped, URL detection off, Markdown preview
shows a notice. Editing stays fully functional (best effort on the
native EDIT control — no virtualized engine).

### Plugins
`%APPDATA%\LoOS\Notepad\plugins\*.dll`, up to 32 DLLs, 12 commands
each under Tools. `build_plugin.bat <name>` compiles
`plugins\<name>\*.c` with `gcc` straight into that folder (with
install instructions if `gcc` is missing). Optional event/idle entry
points get lifecycle notifications. Exports:

```c
int NpPluginVersion(void);                       // return 2 (or 1)
int NpPluginCommands(const NP_PLUGIN_CMD **out); // static array + count
void NpPluginExec2(int id, const NP_HOST *host); // v2 entry point
void NpPluginExec3(int id, const NP_HOST3 *host); // v3 entry point
void NpPluginEvent(int event, const NP_HOST *host); // optional
void NpPluginIdle(const NP_HOST *host);             // optional, ~2 s
typedef struct { int id; const wchar_t *name; } NP_PLUGIN_CMD;
```

`NP_HOST` (see `plugins/np_plugin.h`): `editBox`, `mainWnd`,
`getSel`/`setSel`, `getTextLen`/`getText`, `replaceSel`/`replaceAll`,
`getPath`, `setStatus`, `getConfigDir`, `confirm` — all dirty-flag
safe. `NP_HOST3` keeps that exact prefix (v2 plugins read it safely)
and adds: `getLineCount`/`getLine`/`getCaretLine`/`gotoLine`/
`getSelText`, `inputBox`/`messageBox`, `getPrivateDir`/
`readPrivate`/`writePrivate` (per-plugin INI storage),
`getEncoding`/`isDirty`/`openFile`. Events: `STARTUP/SHUTDOWN/OPEN/
SAVE/TAB/CHANGE` (notifications only — `CHANGE` fires per keystroke,
keep handlers cheap). The demo plugin (`plugins/demo`, 11 commands)
uses every call, event and idle tick. v1/v2 plugins still load
unchanged (best executor wins).

## Registry reference (selection)

Classic names: `fWrap`, `fStatusBar`, `iZoom`, `ShowStoreBanner`,
`fWrapAround`, `fSaveWindowPositions`, `fMatchCase`, `fWholeWord`,
`fReverse`, `iWindowPosX/Y/DX/DY`, `iMarginLeft/Top/Right/Bottom`,
`lfFaceName` (LOGFONT blob), `searchString`, `replaceString`,
`szHeader`, `szFooter`.

Power additions: `iTheme` (0 system / 1 light / 2 dark), `sTheme`
(custom theme name), `fToolbar`, `fRegex`, `fAutoPairs`,
`iPaperSize`, `iOrientation`.

Files: `%APPDATA%\LoOS\Notepad\session.ini` (session + recent),
`%LOCALAPPDATA%\LoOS\Notepad\autosave\recovery_N.*` (crash),
`%LOCALAPPDATA%\LoOS\Notepad\history\<doc>\*.txt` (versions).

## Honest limits

No fake UI is shipped for these: full syntax highlighting (deferred by
choice), font ligatures and letter-spacing (need a DirectWrite engine),
and per-tab process isolation — replaced by built-in supervision
(`notepad.exe --watch`, same binary), which restarts the app on
abnormal exits while session, autosave snapshots and version history
restore everything. The plugin API exposes text
access so external tools can integrate regardless.

## FAQ

**Does it phone home?** No. No ETW, no WER, no feedback hub, no AI
endpoints. `Search with Bing` and the Store banner button only fire
on explicit clicks.

**Where did the UI come from?** Menus, dialogs, strings and
accelerators were decoded from the genuine Windows 11
`notepad.exe.mui`. The icon is an original LoOS design, not Microsoft
art. Diffs against the decompile are documented in the source header.

**Why one giant C file?** Same reason as `loos-cmd`: reviewability,
`cl`-only builds, zero dependencies.
