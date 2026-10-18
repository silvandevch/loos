# LoCmd

LoCmd is the LoOS command processor. It is built from `cmd.c` and provides
classic batch commands, local file and process tools, an interactive line
editor, tabs, persistent history, arrays, objects, arithmetic, and user-defined
functions.

Network clients and probes (`PING`, `TRACERT`, `NSLOOKUP`, `CURL`, and `WGET`)
are excluded from the LoOS build. Local adapter and connection-table display
commands do not open network connections.

## Build

A CMake target produces `LoCmd.exe` from Linux with MinGW-w64:

```sh
./loos source-build windows-mingw
```

`build.bat` remains a local MSVC convenience. Executables are build outputs,
not source files.
