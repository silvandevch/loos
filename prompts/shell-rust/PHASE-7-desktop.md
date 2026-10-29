# Phase 7: desktop host, backend, cleanup

Port `src/shell/desktop.c` (desktop/login/installer modes, window arrangement, config apply,
date/time, Quick Settings dispatch), `src/shell/apphost.c`, `src/shell/backend_mem.c`
(in-memory backend: config, firewall, permissions, processes) and `src/shell/compat_console.c`.

- After this phase `src/shell/` holds only headers. Decide with the team (bus topic `shell`)
  whether the headers move to `src/rust/` or stay as the C ABI of `loos-shell`; keep the C tests.
- `src/kernel/boot/shellhost.c` keeps calling `desktop_*`; do not change it.
- Remove `loos_shell`'s C sources from CMake (the target becomes an INTERFACE over `loos_rust`),
  and the shell list from `boot/build-kernel.sh`.
- Update `docs/SHELL.md`, `docs/RUST-WIRING.md`, `docs/LANGUAGE.md` (crate layout) and this README.

Done when no `.c` file remains in `src/shell/`, every verify step passes, and `./loos qemu`
shows the same desktop as before the port.
