# Build notes

## Why the build script changes directory

The parent LoOS workspace has `.cargo/config.toml`:

```toml
[source.crates-io]
replace-with = "loos-vendor"
```

The vendor directory does not contain GUI crates. Cargo config is discovered
from the current directory, so `scripts/*.sh` build from `$HOME` with an
explicit `--manifest-path`. Cargo still writes artifacts to
`migrationapp/target` because `CARGO_TARGET_DIR` is set.

## Windows cross-compile

The workspace-local Rust 1.97.1 toolchain and `x86_64-pc-windows-gnu` std are
installed under `migrationapp/build/rustup` because the user's rustup home is
read-only in this environment. `scripts/build-windows.sh` uses that toolchain.

`build.rs` compiles `assets/loos-switch.rc` with
`x86_64-w64-mingw32-windres`, embedding:

- the multi-resolution application icon;
- a manifest with `requestedExecutionLevel=requireAdministrator`;
- Windows 10/11 compatibility and PerMonitorV2 DPI awareness;
- version information.

## AppImage

`scripts/package-linux.sh` assembles an AppDir, then runs the official
`appimagetool` with `APPIMAGE_EXTRACT_AND_RUN=1` so FUSE is not required on the
build host. The resulting AppImage is self-contained; it still needs a working
OpenGL driver at runtime (egui/glow), like any native GUI.

## Payload reproduction

`scripts/rebuild-payload.sh` runs the LoOS UEFI and BIOS build scripts with
`--cmdline installer`, compresses each image with zstd, and rewrites
`assets/payload.json`. The app verifies both compressed and raw SHA-256 values
at runtime.
