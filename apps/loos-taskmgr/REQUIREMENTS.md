# LoTaskMgr compatibility requirements

- Origin: classic Windows 7 / early Windows 10 Task Manager behavior, rewritten
  and rebranded for LoOS.
- Target: x86-64 Windows PE GUI executable (`LoTaskMgr.exe`).
- Build imports: `kernel32`, `user32`, `gdi32`, `comctl32`, `comdlg32`,
  `shell32`, `advapi32`, `psapi`, `wtsapi32`, `version`, `shlwapi`, `iphlpapi`,
  `pdh`, and `setupapi`.
- Runtime: process/thread/service/session enumeration, local TCP/UDP table
  display, performance counters, registry startup entries, and GUI controls.
- Connection addresses remain numeric. DNS lookup, online search, and external
  file-reputation lookup are excluded.
- ETW and App history tracking are absent; telemetry: none.
