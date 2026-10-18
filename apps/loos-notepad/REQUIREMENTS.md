# LoNotepad compatibility requirements

- Origin: Windows 11 Notepad behavior, rewritten and rebranded for LoOS.
- Target: x86-64 Windows PE GUI executable (`LoNotepad.exe`).
- Build imports: `kernel32`, `user32`, `gdi32`, `comdlg32`, `comctl32`,
  `shell32`, `advapi32`, `winspool`, `ole32`, and `uxtheme`.
- Runtime: Unicode GUI, common controls/dialogs, file dialogs, printing,
  registry settings, and optional dynamic DWM theme support.
- ETW, Windows Error Reporting, Copilot, and feedback telemetry are absent.
- Telemetry: none.
