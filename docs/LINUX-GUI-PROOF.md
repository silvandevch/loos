# Linux GUI bridge proof

The Linux GUI path runs a ring-3 x86-64 program from an initrd. The program
uses the public `src/lin/liblooswin.h` ABI, creates and shows a window, waits
for `WM_PAINT`, fills a known green rectangle, writes a label, destroys the
window, observes quit, and exits successfully through the bridge.

Run from the repository root:

```bash
PATH=/root/.cargo/bin:$PATH boot/lin_gui_test.sh out/lin-gui shots/linux-gui.png
```

The QEMU monitor socket is deliberately created below `/tmp/opencode` (or
`$LOOS_QEMU_RUNTIME`), because the repository is commonly on exFAT and exFAT
does not permit Unix-domain socket nodes. The build output and screenshot can
remain on the workspace mount.

The harness asserts all of the following:

1. The guest prints `LIN_GUI: window painted`.
2. The Linux process exits with status `0`.
3. The kernel reports non-zero composed green pixels.
4. The QEMU `screendump` contains exact `(0, 200, 0)` pixels, proving the
   composed Linux window reached the real framebuffer rather than only an
   in-memory test surface.

The test sources are `tests/lin/gui.c` and the no-libc
`tests/lin/looswin_client.c`; the kernel passes its video backbuffer to
`loos_lin_boot`, which composes the shared Win32k desktop into that buffer and
then presents it through the normal video path. `loos.lin.stop=1` stops the
stage-0 test immediately after presentation so the ordinary desktop cannot
overwrite the captured Linux frame.
