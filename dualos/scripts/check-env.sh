#!/usr/bin/env bash
# Milestone 1: verify the hardware/toolchain assumptions the ADR depends on.
# Prints PASS/FAIL per check; exits non-zero if any REQUIRED check fails.
set -u
fail=0
ok()   { printf 'PASS  %s\n' "$1"; }
bad()  { printf 'FAIL  %s\n' "$1"; fail=1; }
note() { printf 'INFO  %s\n' "$1"; }
have() { command -v "$1" >/dev/null 2>&1; }

# Host CPU: decides which mode the host itself would boot.
if grep -qw lm /proc/cpuinfo; then note "host CPU has long mode -> Mode A (native) path is testable under KVM/TCG"
else note "host CPU lacks long mode"; fi

# Emulators: Mode B must be tested on CPU models without long mode.
for b in qemu-system-i386 qemu-system-x86_64; do
  have "$b" && ok "$b present ($($b --version | head -1))" || bad "$b missing"
done
if have qemu-system-i386; then
  models=$(qemu-system-i386 -cpu help)
  for m in pentium coreduo; do
    grep -qw "$m" <<<"$models" && ok "qemu CPU model '$m' available (no-lm status verified at boot in M2)" || bad "qemu CPU model '$m' missing"
  done
fi

# Boot path: GRUB2 Multiboot2 ISO with cpuid probe (one ISO, two kernels).
have grub2-mkrescue && ok "grub2-mkrescue present" || bad "grub2-mkrescue missing"
have xorriso        && ok "xorriso present"        || bad "xorriso missing"
for m in cpuid multiboot2 serial iso9660; do
  [ -f /usr/lib/grub/i386-pc/$m.mod ] && ok "GRUB i386-pc module $m" || bad "GRUB i386-pc module $m missing"
done

# Rust: pinned nightly + build-std for custom i586/x86_64 bare-metal targets.
if have rustup && rustc --version | grep -q nightly; then ok "pinned nightly active ($(rustc --version))"
else bad "pinned nightly not active (see rust-toolchain.toml)"; fi
if rustup component list --installed 2>/dev/null | grep -q '^rust-src'; then ok "nightly rust-src (needed for build-std)"
else bad "nightly rust-src missing: rustup component add rust-src"; fi
ls "$(rustc --print sysroot)"/lib/rustlib/*/bin/rust-lld >/dev/null 2>&1 && ok "rust-lld bundled" || bad "rust-lld missing"

exit $fail
