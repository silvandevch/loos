# LoOS applications

| App | Source | Target | Status |
|---|---|---|---|
| LoCmd | [`loos-cmd/`](loos-cmd/) | x86-64 PE console | Linux MinGW-w64 target and CI build |
| LoNotepad | [`loos-notepad/`](loos-notepad/) | x86-64 PE GUI | Linux MinGW-w64 target and CI build |
| LoTaskMgr | [`loos-taskmgr/`](loos-taskmgr/) | x86-64 PE GUI | Linux MinGW-w64 target and CI build |

The root CMake build exposes these targets through the `windows-mingw` preset,
which cross-compiles from Linux. Each application keeps its compatibility
notes in `REQUIREMENTS.md`.
The checked-in compatibility corpus contains no binaries; local and stock PE
files remain outside the repository.
