# PROVENANCE — verbatim copies

Every place code was copied **1:1** (should be rare — see
[`../docs/SOURCE-POLICY.md`](../docs/SOURCE-POLICY.md)). One row per copy.
Borrowed open-source projects go in `THIRD-PARTY.md` instead.

| Date | LoOS file | Copied from (project + file) | Why no rewrite possible | License |
|---|---|---|---|---|
| _(example)_ | `src/kernel/hal/cpuids.def` | vendor CPUID table (public spec) | mechanical value table | public spec |
| 2026-10-18 | `src/win32/apisets/apiset.map` | contract-name -> host-module table from Windows 10 19041.1 AMD64 ApiSetSchema (apisets.info dump) | interface fact table (names only, no code); needed for import compat | public data, no license text; names are ABI |
| 2026-10-18 | `src/win32/include/loos_nt.h`, `loos_win.h`, `ntdll/errmap.c` | NTSTATUS / Win32 error / access-mask / VER_* numeric values and struct layouts | public ABI constants needed for interop (ntstatus.h, winerror.h, winnt.h contract); read in the NT 5.2 study tree, written fresh | public ABI |
| 2026-10-18 | `src/win32/compat/compat.c` | `<supportedOS>` GUIDs and Windows 7/8/8.1/10/11 version+build numbers | public documented facts | public docs |
| 2026-10-18 | `src/win32/ntdll/version.c` | VerifyVersionInfo comparison cascade (major, minor, service pack) | behaviour learned from `base/ntos/rtl/version.c` in the study tree; re-implemented (explicit lexicographic walk, numeric minor compare, strict-condition fix) | n/a, not copied |
| 2026-10-18 | `src/win32/crt/chkstk.S` | `__chkstk` stack probe loop | the standard x64 probe algorithm, written fresh | n/a |
| 2026-10-18 | `apps/loos-notepad/notepad.rc` | Microsoft Windows 11 install.wim image 1, Notepad dialog/menu resources | retained template/control resource IDs consumed by the port; labels and behavior are LoOS-rebranded | Microsoft proprietary resource data; no redistribution license asserted |
| 2026-10-18 | `apps/loos-notepad/notepad.c` | Microsoft Windows 11 `notepad.exe.mui`, string table and accelerator IDs | retained resource IDs and translated user-facing strings used by the port | Microsoft proprietary resource data; no redistribution license asserted |
| 2026-10-19 | `src/libc/src/time.c` (days_from_civil) | Howard Hinnant, chrono-Compatible Low-Level Date Algorithms | re-implemented, not copied; credited for the algorithm | public domain |
| 2026-10-19 | `src/libc/src/math.c` (fmod loop) | musl `src/math/fmod.c` (algorithm only) | re-implemented in own code; credited | MIT |
| 2026-10-20 | `boot/uefi/efi.h` | UEFI 2.x specification: table layouts, protocol GUIDs, function order | spec-defined binary interface; written from the spec, not copied from gnu-efi/EDK II | public spec |
| crates/loos-security/src/x25519.rs | field arithmetic structure after TweetNaCl (public domain, Bernstein et al.) | adapted, rewritten in Rust | public domain | RFC 7748 vectors in tests |
| | | | | |

Prefer a link/reference to a public spec over a copy. If you copy, name the exact
source file and add the license.
