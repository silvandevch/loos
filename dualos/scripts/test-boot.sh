#!/usr/bin/env bash
# Boots the ISO under QEMU CPU models with and without long mode; asserts the right kernel ran.
set -u
iso=${1:-out/dualos.iso}
fail=0
run() { # name, expected-substring, qemu-binary, cpu
  local out
  out=$(timeout 60 "$3" -cpu "$4" -m 64 -cdrom "$iso" -display none -serial stdio -no-reboot \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 2>&1 </dev/null | tr -d '\r')
  if grep -q "$2" <<<"$out"; then printf 'PASS  %-34s %s\n' "$1" "$(grep -o -m1 'dualos kernel.*' <<<"$out")"
  else printf 'FAIL  %-34s\n%s\n' "$1" "$out"; fail=1; fi
}
run "i386 -cpu pentium   -> kernel32 B" "kernel32: mode=B arch=i586 cpuid=1 lm=0" qemu-system-i386 pentium
run "i386 -cpu coreduo   -> kernel32 B" "kernel32: mode=B arch=i586 cpuid=1 lm=0" qemu-system-i386 coreduo
run "i386 -cpu 486       -> refused"    "kernel32: CPU below Pentium baseline" qemu-system-i386 486
run "x86_64 -cpu qemu64  -> kernel64 A" "kernel64: mode=A arch=x86_64 cpuid=1 lm=1" qemu-system-x86_64 qemu64
run "x86_64 -cpu Nehalem -> kernel64 A" "kernel64: mode=A arch=x86_64 cpuid=1 lm=1" qemu-system-x86_64 Nehalem
exit $fail
