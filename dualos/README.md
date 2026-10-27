# dualos

Dual-mode OS: native x86-64 kernel on long-mode CPUs; software-translated x86-64 guest execution on 32-bit-only x86 CPUs.

**Status: Milestone 2 (boot foundation).** Both kernels boot and log over serial; there is no emulator, scheduler or memory manager yet.
Design: [docs/adr/0001-architecture.md](docs/adr/0001-architecture.md).

```bash
./scripts/check-env.sh      # toolchain/hardware assumptions
./scripts/vendor-std.sh     # one-time: vendor build-std deps (needs network)
make iso                    # out/dualos.iso (GRUB2 + kernel32 + kernel64)
make test                   # boot ISO under 5 QEMU CPU models, assert selected kernel
make qemu32 / make qemu64   # run one configuration
```

## Milestone 2 results (executed 2026-10-26, QEMU 10.2.2, TCG)

| QEMU config | Result |
|---|---|
| `qemu-system-i386 -cpu pentium` | GRUB selects kernel32: `mode=B lm=0 cx8=1` |
| `qemu-system-i386 -cpu coreduo` | kernel32: `mode=B lm=0` |
| `qemu-system-i386 -cpu 486` | kernel32 refuses: no CMPXCHG8B (below Pentium baseline) |
| `qemu-system-x86_64 -cpu qemu64` | GRUB selects kernel64: `mode=A lm=1` (long-mode stub, identity map 1 GiB) |
| `qemu-system-x86_64 -cpu Nehalem` | kernel64: `mode=A lm=1` |

Not verified: genuine hardware; kernel64's own no-long-mode refusal path (GRUB never selects it on such CPUs, so it is untested); kernel32 under an `lm` CPU (only warns). kernel32 is checked at ISO build time for absence of SSE/MMX instructions.
