# Console device contract (src/win32/include/loos_console.h)

Owner of the kernel side: astra (io/devices). Client: `src/win32/kernel32/console.c`. Test double:
`tests/win32/kernel32/nt_fake.c` (look at its NtDeviceIoControlFile for exact expected behaviour).

- **Handles.** A process starts with console handles in `RTL_USER_PROCESS_PARAMETERS.StandardInput/Output/Error`
  and `.ConsoleHandle` (any console handle; used for process-wide queries: code pages, title). Names
  `\??\CONIN$` / `\??\CONOUT$` open the process's console (kernel32 maps `CONIN$` / `CONOUT$` to them).
- **Data.** `NtWriteFile` to an output handle takes UTF-8; `NtReadFile` from an input handle returns UTF-8. In
  line mode one read returns at most one line ending `\r\n`; 0 bytes = end of input (Ctrl+Z / closed).
- **Control.** `NtDeviceIoControlFile`, device type `0x50`, METHOD_BUFFERED: GET/SET_MODE (per handle direction),
  GET_INFO (output; same 22-byte layout as CONSOLE_SCREEN_BUFFER_INFO), SET_CURSOR, SET_ATTR, GET/SET_TITLE (UTF-8,
  no terminator), GET/SET_CP, FLUSH_INPUT, INPUT_COUNT (bytes), FILL. A non-console handle must answer
  `STATUS_INVALID_DEVICE_REQUEST` (kernel32 turns that into ERROR_INVALID_HANDLE, which is how
  `GetConsoleMode` tells the C runtime "this is a file or pipe").
- **Device type for GetFileType.** `NtQueryVolumeInformationFile(FileFsDeviceInformation)` must report
  `FILE_DEVICE_CONSOLE` (0x50) for console handles, `FILE_DEVICE_DISK` for files, `FILE_DEVICE_NAMED_PIPE`
  for pipes, `FILE_DEVICE_NULL` for NUL.
- **Ctrl+C / Break / Close.** The console subsystem starts a thread in the target process at
  `kernel32!CtrlRoutine(LPVOID)` with the event type (0 C, 1 Break, 2 Close) as the argument.
- Not done: input records (`ReadConsoleInput`), screen-buffer reads (`ReadConsoleOutput`), scrolling, multiple
  screen buffers, `AllocConsole` / `AttachConsole`.
