#!/usr/bin/env bash
# Picks the QEMU accelerator. Usage: qemu-accel.sh [args|name|status]
#   LOOS_KVM=auto (default) use KVM when /dev/kvm is usable, else TCG
#   LOOS_KVM=on   require KVM (exit 3 if unavailable)
#   LOOS_KVM=off  always TCG (what the reproducible boot tests default to)
# `args`   -> "-enable-kvm -cpu host"  or  "-accel tcg"   (for command lines)
# `name`   -> "kvm" or "tcg"                              (for -machine accel=)
# `status` -> human readable diagnosis
mode=${LOOS_KVM:-auto}
cmd=${1:-args}

kvm_ok() { [ -c /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; }
why() {
    [ -c /dev/kvm ] || { echo "/dev/kvm missing (enable VT-x/AMD-V in firmware; modprobe kvm_intel or kvm_amd)"; return; }
    { [ -r /dev/kvm ] && [ -w /dev/kvm ]; } || echo "no access to /dev/kvm (add yourself to the 'kvm' group: sudo usermod -aG kvm \$USER)"
}

case $mode in
    off) use=tcg ;;
    on)  kvm_ok || { echo "LOOS_KVM=on but KVM is unavailable: $(why)" >&2; exit 3; }; use=kvm ;;
    auto) if kvm_ok; then use=kvm; else use=tcg; fi ;;
    *) echo "LOOS_KVM must be auto|on|off" >&2; exit 2 ;;
esac

case $cmd in
    args)   [ "$use" = kvm ] && echo "-enable-kvm -cpu host" || echo "-accel tcg" ;;
    name)   echo "$use" ;;
    status)
        echo "mode: $mode  accelerator: $use"
        if kvm_ok; then echo "/dev/kvm: usable"; else echo "/dev/kvm: unavailable - $(why)"; fi
        grep -q -E 'vmx|svm' /proc/cpuinfo 2>/dev/null && echo "cpu: hardware virtualization present" || echo "cpu: no vmx/svm flag visible (inside a VM without nested virt?)"
        command -v qemu-system-x86_64 >/dev/null && echo "qemu: $(qemu-system-x86_64 --version | head -1)" || echo "qemu: not installed"
        ;;
    *) echo "usage: $0 [args|name|status]" >&2; exit 2 ;;
esac
