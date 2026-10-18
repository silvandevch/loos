# UI spec — custom desktop shell

The shell is **your own**, replacing the Server 2003 shell entirely. This
document tells Claude Code what to build and, crucially, what the shell requires
from the backend (so the two stay in sync). Anything marked **[you]** is a
decision to lock in; everything else is a sensible default to implement now.

---

## 1. Design intent

- Privacy-first, quiet, fast. No ads, no web views loading remote content, no
  telemetry, no account prompts.
- Local, legible, keyboard-driven. A power-user desktop, not a tablet toy.
- Visually distinct from Windows: product name **LoOS**, tagline **Local only.**
  Shared palette: accent `#3B82F6`, surface `#121824`, text `#F3F4F6` from
  `branding/brand.h`. Default concept: dark, high-contrast, accent-color-driven, generous spacing, soft
  rounded panels, one accent hue, monochrome iconography.

## 2. Surfaces to build

1. **Session / sign-in** — local accounts only; optional auto-login; no MS
   account, no PIN-to-cloud.
2. **Shell chrome**
   - **Taskbar**: app buttons, clock/calendar (local), a **network activity LED**
     that lights on *any* socket (privacy signal), tray with a hard "no network"
     toggle.
   - **Start / launcher**: search across local apps, files, and settings; no
     web results, no remote suggestions.
   - **Window manager**: snap layouts, per-monitor DPI, virtual desktops,
     focus-follows options, tiling mode.
   - **Notifications**: local sources only; a per-app permission model.
3. **Settings app** — the privacy control center:
   - Network firewall: per-process allow/deny, live connections.
   - Permissions: camera/mic/files/notifications, per app.
   - Updates: opt-in, self-hostable mirror, offline mode.
   - Appearance, input, accessibility, accounts.
4. **File manager** — native **btrfs** + mounted volumes (FAT32 ESP, optional
   read-only NTFS); no cloud integrations.
5. **Task manager / monitor** — process tree, per-process sockets and disk, no
   upload.
6. **Compatibility console** — pick the claimed Windows version per app (7/10/11),
   view shim decisions, header of "why this app needs X".

## 3. Backend contract the UI depends on

The UI is a client of these backend services. Implement them as versioned local
IPC (your own protocol; a documented message schema over a local socket or your
object-manager namespace). Claude Code must keep UI and these APIs in lockstep.

| UI need | Backend service / call |
|---|---|
| Launch/管理 apps | `ShellExecute`/`CreateProcess` equivalent, app inventory query |
| Taskbar window buttons | window enumeration + events (`WindowManager` subscription) |
| Network LED / firewall | `NetMonitor.query()` → per-process sockets; `Firewall.setRule()` |
| Notifications | `Notify.publish/local sources`, permission gate |
| Settings: permissions | `Perms.get/set` per app+resource |
| File manager | `Fs.list/stat/copy/move`, volume enumeration |
| Task monitor | `Ps.snapshot()` → processes, threads, handles, io/net/disk |
| Compat console | `Compat.getAppProfile()/setClaimedVersion()` |
| Version reporting | `RtlGetVersion`/`GetVersion` routed through compat profile |
| Theme/DPI | compositor `Theme`/`Scale` APIs from the WM |

Rules:
- **No UI action may generate network traffic** except what the user explicitly
  initiated, and the LED/firewall must reflect it truthfully.
- Every setting is **local**, persisted in the registry/hive or a config store
  you control. No cloud sync, ever, unless a future opt-in self-hosted sync mode.

## 4. Interaction & accessibility

- Full keyboard navigation; command palette (Ctrl+Space default).
- Per-monitor DPI; text scaling; reduced-motion; high-contrast themes.
- Screen-reader bridge via the UI Automation-compatible surface you expose
  (so assistive Windows tools can attach).

## 5. Tech choice for the shell

- Preferred: original C/C++/Rust against **your** user32/gdi32/compositor — do
  not build the shell on WinHTML/web runtimes. A native compositor keeps it
  honest, fast, and privacy-clean.
- **[you]** confirm language/framework; the backend contract above is
  framework-agnostic.
- The old Server 2003 shell (explorer.exe, etc.) is **not** built or shipped.

## 6. Milestones for the UI

1. U0 — compositor rectangle + a window that draws and takes input.
2. U1 — taskbar + launcher wired to the app inventory.
3. U2 — window manager features (snap, virtual desktops, DPI).
4. U3 — Settings → privacy/firewall/permissions, live LED.
5. U4 — file manager + task monitor.
6. U5 — compatibility console + accessibility pass.

Build the UI against the **backend contract**, not against a mock, so that as
kernel/subsystem pieces land the shell lights up incrementally.
