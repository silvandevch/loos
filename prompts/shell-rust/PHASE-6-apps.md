# Phase 6: built-in apps → Rust

Port `src/shell/settings.c` (all pages incl. Personalize: presets, accent HSL, shape/effects,
taskbar options, opacity, icon shape, wallpaper, per-token colors, export/import; `settings_quick`),
`src/shell/filemgr.c`, `src/shell/taskmon.c`, `src/shell/session.c` (login, lockout) and
`src/shell/installer.c` (installer UI; the disk-writing backend stays in `src/kernel/boot/install.c`).

- Keep `settings.h`, `filemgr.h`, `taskmon.h`, `session.h`, `installer.h` APIs.
- Security-relevant logic in `session.c` (password check delegation, lockout timing) must keep its
  exact behavior; port it first and test it on its own. Never log or store passwords.
- `installer.c` talks to the backend through the `inst_*` callbacks; keep them as C function pointers.

Done when the five C files are gone and `settings_test`, `session_test`, `installer_test`,
`apps_test` pass unchanged, and an unattended install still works:
`bash boot/bios/install_test.sh && bash boot/uefi/install_test.sh`.
