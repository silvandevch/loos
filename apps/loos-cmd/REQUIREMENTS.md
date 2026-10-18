# LoCmd compatibility requirements

- Origin: classic NT command-processor behavior, extended for LoOS.
- Target: x86-64 Windows PE console executable (`LoCmd.exe`).
- Build imports: `kernel32`, `user32`, `shell32`, `advapi32`, `iphlpapi`,
  `netapi32`.
- Runtime: console input/output, file and process APIs, environment and
  registry APIs, and local process/network-state enumeration.
- Network clients and probes are omitted; no socket or HTTP libraries are
  linked.
- Telemetry: none.
