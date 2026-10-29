# LoOS Browser (and the Firefox port plan)

Status: **real pages under QEMU, native shell**. LoOS ships a browser app that
loads live HTTP(S) pages through a development-only QEMU web bridge, with a CSS
subset and meta-refresh. It is *not* Firefox and contains no Gecko code. This
document describes what ships today, the bridge, the boundary the full port will
plug into, and the staged plan for bringing Firefox/Gecko to LoOS.

- Code: `crates/loos-shell/src/apps/browser.rs` (UI + engine subset) and
  `crates/loos-shell/src/apps/browser/css.rs` (CSS subset)
- Web bridge: `crates/loos-shell/src/web.rs` (guest ABI) and `boot/web-bridge.py`
  (host HTTPS helper; QEMU only)
- Shell integration: `src/shell/desktop.c` (`open_browser`, `desktop_open_url`),
  launcher entry in `src/shell/inventory.c`
- Tests: `tests/shell/browser_test.c`, Rust unit tests in the app module, and the
  end-to-end `tests/boot/browser_qemu.py`
- Policy: no telemetry, no background connections, nothing leaves the machine
  unless the user starts it ([PRIVACY-EGRESS.md](PRIVACY-EGRESS.md))

## 1. What ships now (v0.2: real pages in QEMU)

| Area | What works |
|---|---|
| Window | Tabs, address bar, Back / Forward / Reload / Home, Go, Bookmark, status bar, scrollbar |
| Tabs | Up to 12; open (`Ctrl+T`, `+`), close (`Ctrl+W`, `x`), switch by click; per-tab history and scroll position |
| History | Per-tab back/forward stack (64 entries) and a global visited list (80 entries), persisted under `/home/user/.config/loos-browser` |
| Bookmarks | `Ctrl+D`, Bookmarks page, persisted; its default set includes Start, the local sample site and the guide |
| Addresses | `loos://` pages, `about:`, `file://`, absolute paths, `~/`, Windows paths (`C:\...`), live HTTP(S) under `./loos qemu`, bare-word local search |
| Local pages | `/home/user/Web/index.html`, `guide.html`, `privacy.html` are seeded as a sample site on the disk |
| Internal pages | `loos://home`, `loos://bookmarks`, `loos://history`, `loos://about`, `loos://search?q=...` |
| HTML subset | headings h1-h6, paragraphs, `br`, `hr`, lists (`ul`/`ol`/`li`), links, `b`/`strong`, `i`/`em`, `code`/`kbd`/`samp`, `blockquote`, `pre`, image alt text/logo fallback, form controls as placeholders, tables flattened to text, entities, comments, `<meta http-equiv=refresh>` |
| CSS subset | tag/`.class`/`#id` selectors, `display:none`, `visibility:hidden`, colour, background, font-size/weight, margins, padding/indent, border, text decoration, text-align on single-line blocks, `bgcolor`/`text` body attributes |
| Web bridge | Under `./loos qemu`, `boot/web-bridge.py` fetches HTTP(S) on the host (TLS validated), strips scripts/comments, raw-DEFLATE compresses and sends it over COM2. The guest has no socket code. `LOOS_WEBBRIDGE=0` disables it. |
| Text | Inter (regular/semibold) at logical sizes; word wrapping; language-independent layout; UTF-8 |
| Input | `Ctrl+L` focus address bar, `Ctrl+T/W/R/D`, `Alt+Left/Right`, page keys, wheel, link clicks |
| Storage | Plain TSV files in `/home/user/.config/loos-browser/`; no database, no sync |

Deliberately **not** in this build:

- JavaScript, the full CSS cascade (flex/grid/position/media queries), images,
  media, iframes, PDF, extensions, downloads, printing, zoom, find-in-page,
  devtools.
- A native LoOS TCP/IP + TLS stack. The QEMU bridge is a development
  convenience: the host fetches the page, so the guest still has no sockets and
  the LoOS firewall/NetMonitor cannot see the request. This is explicitly not
  the final network design.
- Real cookie jars, logins, client certificates or persistent site data.

This is a useful browser shell and a real page viewer for many server-rendered
sites (Google's basic page, Wikipedia, example.com). It is **not** a
compatibility claim for the modern web (see §4 for what that requires).

## 2. Architecture of the shell

```
crates/loos-shell/src/apps/browser.rs
  Browser ── tabs ── Tab { url, title, Doc, history, scroll }
     │
     ├── address layer     classify()/resolve_href()/normalize_path()
     ├── document layer    parse_html() -> Doc { title, blocks[] } -> draw
     ├── persistence       bookmarks.tsv / history.tsv through shell_backend fs_*
     ├── web bridge        loos_web_fetch() -> COM2 -> boot/web-bridge.py (QEMU only)
     └── view              immediate-mode paint through ui_ctx / shell_fb
```

The guest-side bridge client is `crates/loos-shell/src/web.rs` (no sockets, no
std): `WGET <url>\n` out, `OK <compressed_len> <raw_len> <final_url>\n` plus
raw DEFLATE back. The host helper validates TLS, follows HTTP redirects, strips
scripts/comments, refuses `file:` and non-http(s), and refuses credentials in
URLs. The browser follows `<meta http-equiv=refresh>` itself.

The app also uses two small toolkit hooks added for it: `ui_focus()` /
`ui_focused()` in `src/shell/ui.h` / `crates/loos-shell/src/ui.rs` (the address bar needs `Ctrl+L` to move
keyboard focus). Keep them when the toolkit is ported to Rust.

`Doc` is the engine boundary. A future Gecko backend will produce the same
`Doc`-equivalent (or draw into a client surface; see §5.5), while tabs,
history, bookmarks, permissions, address completion and the LoOS compositor
integration stay in the shell. That keeps the Firefox work from also having to
reinvent the browser chrome and LoOS integration.

## 3. Security and privacy posture

- The browser asks the shell for files only; it has no direct OS calls and no
  socket code.
- **QEMU web bridge egress is host-side and development-only.** It is started
  only by `./loos qemu`, makes a request only when the user navigates, uses
  certificate validation, and is absent from real-machine images. It bypasses
  the LoOS firewall because there is no guest network stack yet; this is
  called out honestly in [PRIVACY-EGRESS.md](PRIVACY-EGRESS.md).
- Production LoOS will route egress through the firewall/NetMonitor and the LED
  ([SHELL.md](SHELL.md)). A future HTTP engine must show its sockets there, and
  the hard network switch must remain authoritative.
- Site data, if added, goes under the same per-user profile directories and
  must be deletable in Settings.
- Firefox defaults that must be neutralised in any port: telemetry, crash
  reporting, Studies/Normandy, remote settings, sponsored tiles, search
  suggestions, Safe Browsing lookups (unless the user configures a local
  provider), DoH (local setting only), account sync, and automatic updates
  (mirror is user-set and opt-in, see [INSTALL.md](INSTALL.md)).
- No DRM is shipped by default. Widevine/EME is an explicit opt-in with a
  visible notice, if it is ever shipped.

## 4. Why the full Firefox port is a programme, not a patch

Firefox is not a self-contained library. A desktop Firefox needs:

- a full POSIX/Linux runtime (glibc, threads, futex, signals, `/proc`,
  `/dev`, Unix sockets, `epoll`, `timerfd`, memory-mapped files);
- a toolkit stack (GTK, glib, pango, harfbuzz, fontconfig, freetype, cairo,
  ATK, X11 and/or Wayland);
- NSPR/NSS, SQLite, ICU, zlib, libffi, libevent, and many Rust crates
  (Firefox already contains a large Rust component: style, WebRender, etc.);
- media (libvpx, dav1d/ffmpeg, PulseAudio/PipeWire, ALSA);
- a graphics path (EGL/GL, WebRender) and a compositor;
- the browser chrome (toolkit), content processes, IPC, sandboxing, and the
  whole build system (`mach`, `mozbuild`, clang, Rust pin, Node, Python).

LoOS today has a compositor, a shell, a filesystem, and a PE loader, but not
yet a network stack, a Linux ABI runtime, or the desktop toolkit. The honest
shortest path is therefore to **use the "Lin" compatibility layer** planned in
[LINUX-COMPAT.md](LINUX-COMPAT.md) to bring stock desktop Firefox up first,
then replace layers with native LoOS implementations where it pays off.

### Options considered

| Option | What it means | Verdict |
|---|---|---|
| A. Stock Firefox ESR on Lin | Port the Linux runtime + GTK/X11/Wayland, run unmodified Firefox ESR, patch only integration points | **Recommended first target.** Fastest way to a real Gecko; reuses upstream work; patches stay small |
| B. Embed Gecko in a LoOS app | Use the Gecko embedding API / `GeckoView`-like content process | Gecko desktop embedding is not a supported upstream product; GeckoView targets Android. High maintenance, no upstream guarantees |
| C. Port Gecko natively without Linux | Write LoOS implementations of every `#ifdef XP_UNIX` platform layer | Correct long-term, but years of work before the first page renders; do it incrementally after A |
| D. Reimplement the engine (Servo) | Own engine, not Firefox | Not what was asked; a different project. Useful only as an experiment |

The plan below is **A + incremental C**: get real Firefox running on LoOS via
Lin, keep the LoOS browser shell as the chrome integrator, and migrate
platform layers to native LoOS services when they are stable.

## 5. Firefox port plan

The work is split into phases with hard exit criteria. Every phase must keep
the tree buildable and the privacy lint green.

### Phase 0 — browser shell and boundary (done, this changeset)

Deliverables:
- Native browser app, HTML-subset engine, tabs/history/bookmarks, local pages,
  offline web handling.
- `Doc` boundary and shell integration; test coverage.

Exit criteria:
- `cargo +1.97.1 test -p loos-shell` passes (browser tests included).
- `ctest -R shell.browser_test` passes.
- A web address never creates a socket (privacy lint + code review).
- Visual check: `LOOS_BROWSER_PPM=out.ppm tests/shell/browser_test`.
- Browser appears in Start, opens from the taskbar, and can be handed a URL via
  `desktop_open_url()`.

### Phase 1 — runtime prerequisites (Lin L0-L4)

Depends on the Lin programme ([LINUX-COMPAT.md](LINUX-COMPAT.md)).

- ELF loader for `ET_DYN`/PIE and the stock glibc `ld-linux`.
- Linux x86-64 syscalls used by glibc and Firefox: `mmap`, `clone`, `futex`,
  signals, `epoll`, `eventfd`, `timerfd`, `getdents64`, `statx`, `ioctl`,
  `getrandom`, `prlimit64`.
- `/proc/self/{maps,status,fd}`, `/dev/{null,zero,random,urandom,shm,tty}`,
  `/tmp`, `$HOME`.
- `.deb` unpacking into the Lin root and a launcher.

Exit criteria:
- Stock Debian `bash`, `ls`, then a threaded GTK "hello" binary run.
- Firefox's own build tools (`mach`, clang, Rust pin) run in the Lin root.

### Phase 2 — toolkit and display bring-up

- GTK3 (and the glib/pango/harfbuzz/fontconfig/freetype stack) running on a
  Lin X11 server or a Wayland compositor.
- **X11 served on the LoOS compositor** (Lin L7) is the smallest first step;
  Wayland follows.
- Clipboard, cursor, keyboard layout, HiDPI, and window resize.

Exit criteria:
- Stock `gtk3-demo` renders into a LoOS window at 100% and 200% scale.
- Input, resize and focus work through the LoOS shell.

### Phase 3 — SpiderMonkey

- Build SpiderMonkey for the Lin runtime from the pinned Firefox ESR source
  (`js/src`, `mach build`) with the `--enable-posix-nspr-emulation` path as
  needed.
- Ship the `js` shell; run the test262 subset and the Firefox JS test suite.
- Add a LoOS-callable embedding (C++ ABI is fine; keep it behind a small C ABI)
  so native LoOS apps can host JS without the browser.

Exit criteria:
- `js` runs test262 (agreed subset) with no regressions from upstream.
- A LoOS test app evaluates a script, returns a value, and survives OOM/stack
  limits.

### Phase 4 — Gecko core without chrome

Build the Firefox ESR tree for Lin and bring up, in order:

1. XPCOM + NSPR + NSS + `xpcshell`.
2. Necko (HTTP/1.1, HTTP/2, TLS) against a **local** test server; firewall and
   NetMonitor see every socket; hard-off refuses.
3. Layout/style + text/font shaping; render static HTML from a local file.
4. WebRender: GL/EGL first, then a software fallback. GPU drivers are a LoOS
   milestone ([DRIVERS.md](DRIVERS.md)); do not block the port on them.
5. `dom/` + event loop + timers + workers.

Exit criteria:
- `xpcshell` passes its upstream smoke tests.
- A local HTML page with CSS and images matches the Linux ESR screenshot
  (perceptual diff tolerance documented in CI).
- A page loads over HTTP from `127.0.0.1` with the firewall showing the
  connection; with the hard switch on, it fails and the LED goes red.
- JavaScript on the page runs (SpiderMonkey from Phase 3).

### Phase 5 — browser chrome integration

Two sub-options, in order:

1. **Fastest:** run Firefox's own `browser/` chrome as a Lin app, with its
   windows on the LoOS compositor. LoOS provides the window manager, tray,
   notifications and firewall; Firefox remains mostly stock. This gives a
   testable daily browser sooner.
2. **Native chrome:** reuse `crates/loos-shell/src/apps/browser.rs` as the
   LoOS-native chrome and embed a Gecko content view (client surface / IPC).
   This is the long-term shape: LoOS tabs, history, bookmarks, permissions,
   downloads and settings; Gecko only renders content.

Exit criteria for (1):
- Stock Firefox opens, renders a page, and its profile lives under the LoOS
  user directory.
- Privacy defaults are a build-time patch: telemetry/crash reports/Studies/
  sponsored content/remote settings disabled; search and updates user-set.
- The firewall attributes Firefox's sockets to the process and the hard switch
  wins.

Exit criteria for (2):
- Tabs/history/bookmarks stay in the LoOS shell across Gecko restarts.
- A content process crash does not take down the shell; tab restore works.
- The browser can run with the network hard switch on and remain fully usable
  for local files/internal pages.

### Phase 6 — product completion

- Content processes, site isolation, sandbox (process per site, seccomp-like
  LoOS filters where available).
- Downloads, printing, PDF (pdf.js), find-in-page, zoom, session restore,
  private windows.
- Extensions (WebExtensions) with a LoOS permission prompt bridge.
- Media: codecs, audio routing (PipeWire/ALSA), captions.
- Accessibility: Gecko a11y -> LoOS accessibility tree (`ui_a11y`/UIA bridge).
- Certificate store integration with the LoOS trust store; enterprise policy
  via the registry hive; per-site permissions surfaced in Settings.

### Phase 7 — performance and hardening

- JIT hardening, ASLR, W^X, CFG/CET where available.
- Startup and memory budgets; GPU compositing; media hardware decode.
- Reproducible builds, SBOM, signed release, fuzzing (IPC parsers, HTML, JS).

## 6. Milestone table

| # | Milestone | Gate | Suggested owner |
|---|---|---|---|
| B0 | Native shell + local engine | browser tests + privacy lint | claude/sol |
| B1 | Lin runtime for glibc/GTK | threaded GTK demo on LoOS | astra/sol |
| B2 | X11/Wayland on the compositor | gtk3-demo in a LoOS window | claude/sol |
| B3 | SpiderMonkey | test262 subset | astra/sol |
| B4 | XPCOM/NSS/Necko | local HTTPS + firewall proof | sol |
| B5 | Layout/WebRender | screenshot parity for a static page | astra/sol |
| B6 | Stock Firefox on Lin | daily driver on a test machine | sol |
| B7 | Native chrome + Gecko view | tabs survive content crash | claude/sol |
| B8 | Sandbox/site isolation | escape-test corpus | astra |
| B9 | Media/a11y/extensions | feature matrix signed off | sol/claude |

## 7. Porting mechanics

- **Pin:** Firefox ESR (exact tag recorded in `docs/PROVENANCE.md` / release
  notes). No floating branch.
- **Build:** upstream `mach` in a Linux container with the same clang/Rust
  versions Firefox pins. The LoOS port consumes the resulting tree; LoOS
  patches are a short, ordered series under `ports/firefox/patches/` when the
  directory is created, each with an upstreamability note.
- **Platform layer:** start with Lin. Add `#ifdef LOOS` only for integration
  points (firewall/permissions, file pickers, notifications, profile paths,
  update channel). Do not fork layout/JS.
- **Third-party:** every bundled library gets a `THIRD-PARTY.md` row and a
  license note (MPL-2.0 for Firefox, plus each dependency's license).
- **CI:** privacy lint; unit tests; JS conformance; WPT subset; screenshot
  parity; egress test with the hard switch; crash-restore test; startup budget.
- **Reproducibility:** record toolchain versions and hashes; build twice and
  compare.

## 8. Risks and mitigations

| Risk | Why it hurts | Mitigation |
|---|---|---|
| Toolkit dependency explosion | GTK/glib/pango/fontconfig drag in a large stack | Use stock Debian libraries on Lin first; replace only measured hot spots |
| No network stack yet | Necko can't be validated | Phase 1/4 gate with localhost servers; integrate the LoOS firewall early |
| GPU/WebRender | blank or slow pages | Software WebRender fallback; GPU comes with [DRIVERS.md](DRIVERS.md) |
| Sandbox expectations | Firefox security model assumes OS primitives | Content-process isolation + LoOS job/token model; document gaps honestly |
| Codec/DRM licensing | cannot ship some media | Ship unencumbered codecs; EME opt-in only |
| Fork drift | patches rot | keep patches small, integration-only, and upstream what is general |
| Disk/time | Firefox tree and builds are huge | build out-of-tree on the DATA drive; CI caches; no full tree in the repo |

## 9. What we will not claim

- The current browser is not Firefox and does not render the modern web.
- "Firefox support" means a tested ESR build on LoOS with the gates above, not
  a source drop.
- No privacy claim survives if a port silently enables telemetry, Safe
  Browsing lookups, remote settings or auto-update. Those are release blockers,
  not settings.

## 10. Verifying the current browser

```bash
# Crate tests (HTML parser, URLs, tabs/history/bookmarks, offline behaviour)
cd crates && cargo +1.97.1 test -p loos-shell

# Shell integration test (build the host tree first, see DEVELOPMENT.md)
ctest --test-dir out/cmake/host-clang -R shell.browser_test --output-on-failure

# Visual review of the client surface
LOOS_BROWSER_PPM=out/browser.ppm ./out/cmake/host-clang/tests/shell/browser_test

# The full QEMU path: bridge starts automatically, COM2 carries compressed pages
./loos qemu

# Optional end-to-end QEMU browser test (needs host network)
python3 tests/boot/browser_qemu.py
```

Inside LoOS: Start -> **LoOS Browser**, press `Ctrl+L`, type `google.com` or
`example.com`. Local pages work with no bridge at all. Other apps can hand the
browser a URL with `desktop_open_url(&desktop, "/home/user/Web/index.html")`.

## Related

- [SHELL.md](SHELL.md) — shell/app toolkit and built-in apps
- [LINUX-COMPAT.md](LINUX-COMPAT.md) — Lin, the prerequisite for stock Firefox
- [PRIVACY-EGRESS.md](PRIVACY-EGRESS.md) — egress rules
- [DRIVERS.md](DRIVERS.md) — GPU/audio drivers
- [SOURCE-POLICY.md](SOURCE-POLICY.md) — code-use rules and licensing
- [THIRD-PARTY.md](../THIRD-PARTY.md) — borrowed components (Firefox rows land
  when code is imported, not before)
