# LoOS window manager + GDI (`crates/loos-win32k`)

Kernel-side half of user32/gdi32: lets **windowed** Win32 programs (not just console
exes) run. Single desktop, cooperative model, `no unsafe`.

| Area | Implemented |
|---|---|
| Classes | `RegisterClassEx`/`UnregisterClass`, atoms, class background (`COLOR_x+1` or brush), wnd-extra slots |
| Windows | `CreateWindowEx` (WM_NCCREATE/CREATE/SIZE/MOVE, CREATESTRUCT), top-level + child, `DestroyWindow` (children first, WM_DESTROY/NCDESTROY), `ShowWindow` (hide/show/min/max/restore), `MoveWindow`, text, user data, `SetWindowLongPtr(WNDPROC)` subclassing, `FindWindow`, enable/disable |
| Messages | per-thread queues; `GetMessage`/`PeekMessage` (filters), `PostMessage`, `PostQuitMessage`, `SendMessage` (runs on owner thread), `TranslateMessage` (WM_CHAR), `DispatchMessage`, WM_PAINT synthesised from invalid regions, WM_TIMER |
| Painting | `InvalidateRect`/`ValidateRect`/`UpdateWindow`/`BeginPaint`/`EndPaint`, WM_ERASEBKGND via class brush |
| GDI | DCs (window + memory), solid brush/pen/font(px)/bitmap, stock objects, `FillRect`, `Rectangle`, `MoveToEx/LineTo`, `Set/GetPixel`, `TextOut`, `DrawText`, `GetTextExtent`, `BitBlt(SRCCOPY)`; text uses the shell's Inter outline rasteriser |
| Input | `inject_mouse_move/button`, `inject_key`: hit-testing (`WM_NCHITTEST`), activation + raise, focus, capture, caption drag, close button → `WM_SYSCOMMAND(SC_CLOSE)` → `WM_CLOSE` → `DestroyWindow` |
| Output | `compose(&mut [u32])` renders desktop, frames, title bars, client bitmaps (children clipped to parents) |
| User-mode procs | `Proc::User(addr)` + `set_upcall`: the kernel supplies the hook that enters ring 3 to call a PE `WNDPROC` |

Also: `Button`/`Static`/`#32770` controls (BN_CLICKED → `WM_COMMAND`), menu bars + one-level drop-downs (`set_menu`), non-blocking `message_box` (modal owner disable, `message_box_result`), and `service.rs` — the syscall-style ABI (`SVC_*`, Win64 struct layouts `WNDCLASSEXW`/`MSG`/`PAINTSTRUCT`/`RECT`, UTF-16 strings via `GuestMem`) that user32/gdi32 PE stubs call.

Not yet: nested submenus, accelerators, dialog templates, clipboard, layered/transparent windows, DWM effects,
non-SRCCOPY raster ops, region clipping (update region is its bounding rect).

## Wiring to PE programs (remaining)
Done: `Win32k::service(tid, nr, regs, &mut dyn GuestMem)`; test drives a guest class/window/message loop/paint entirely through it. Remaining: the `user32.dll`/`gdi32.dll` PE export stubs (`mov eax, SVC_x; syscall`) and the kernel's ring-3 `WNDPROC` upcall. `GetMessage` returning `Idle`
means the thread blocks until input/post/timer (scheduler wait on the thread's queue).

Tests: `cargo test -p loos-win32k` (hello window paints + composes, close button,
caption drag, child hit-test/focus/keys/timers, maximise/restore, user-proc upcall).


## PE stubs (done)
`src/win32/user32/{user32,gdi32}.c` + `scripts/win32/build-user32.sh` build real PE32+ `user32.dll`/`gdi32.dll` (mingw-w64 here; clang+lld-link
works too) whose exports call `syscall` with `SVC_*` numbers from `src/win32/include/loos_win32k.h` (header is checked against
`service.rs` by `cargo test -p loos-win32k`). Not done: the kernel cannot yet *run PE processes* (no PE process model in the boot kernel), so these
DLLs are unexercised end-to-end; Linux programs already drive the same window manager through the `liblooswin` bridge in the ring-3 test.
