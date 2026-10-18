# LoNotepad

LoNotepad is the LoOS text editor, built from `notepad.c`, its dialog resources,
manifest, and LoOS icon. It provides multi-tab editing, encoding-aware file
handling, regex search, themes, recovery, and local editing tools. ETW,
Windows Error Reporting, Copilot, feedback collection, and telemetry are not
included.

## Build

A CMake target produces `LoNotepad.exe` from Linux with MinGW-w64:

```sh
./loos source-build windows-mingw
```

`build.bat` remains a local MSVC convenience. Executables are build outputs,
not source files.
