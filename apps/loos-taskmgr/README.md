# LoTaskMgr

LoTaskMgr is the native LoOS task manager, built from `taskmgr.c`, dialog
resources, manifest, and LoOS icon. It shows processes, performance, startup
entries, users, details, and services. Its per-process connection view reads
the local TCP/UDP tables and displays numeric addresses. It does not resolve
hostnames or send process names or file hashes to external services.

ETW telemetry, XAML/DirectUI, App history tracking, feedback links, online
search, third-party file-reputation lookups, shell reset, and firewall containment are
omitted until LoOS publishes those native interfaces.

## Build

A CMake target produces `LoTaskMgr.exe` from Linux with MinGW-w64:

```sh
./loos source-build windows-mingw
```

`build.bat` remains a local MSVC convenience. Executables are build outputs,
not source files.
