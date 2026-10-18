#!/usr/bin/env bash
# Headless QEMU boot with required serial-output assertions.
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=../../env.sh
. "$root/env.sh"

usage() {
    cat <<'EOF'
Usage: tests/boot/qemu-serial.sh ISO --expect TEXT [--expect TEXT ...]
       [--timeout SECONDS] [--log PATH]

Boot the ISO headlessly with networking disabled. Every --expect substring
must appear on the serial console before QEMU exits or reaches the timeout.
EOF
}

iso="${1:-}"
if [[ -z "$iso" || "$iso" == --help ]]; then
    usage
    [[ "$iso" == --help ]] && exit 0
    exit 2
fi
shift

timeout_seconds="${QEMU_TIMEOUT:-60}"
serial_log="${QEMU_SERIAL_LOG:-$OUT_ROOT/logs/loos-qemu-serial.log}"
expects=()
while (($#)); do
    case "$1" in
        --expect)
            (($# >= 2)) || { echo "--expect needs text" >&2; exit 2; }
            expects+=("$2")
            shift 2
            ;;
        --timeout)
            (($# >= 2)) || { echo "--timeout needs seconds" >&2; exit 2; }
            timeout_seconds="$2"
            shift 2
            ;;
        --log)
            (($# >= 2)) || { echo "--log needs a path" >&2; exit 2; }
            serial_log="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if ((${#expects[@]} == 0)); then
    echo "at least one --expect assertion is required" >&2
    usage >&2
    exit 2
fi
if ! [[ "$timeout_seconds" =~ ^[1-9][0-9]*$ ]]; then
    echo "timeout must be a positive whole number of seconds" >&2
    exit 2
fi
[[ -f "$iso" ]] || { echo "ISO not found: $iso" >&2; exit 2; }

qemu="${QEMU:-qemu-system-x86_64}"
command -v "$qemu" >/dev/null 2>&1 || { echo "QEMU not found: $qemu" >&2; exit 2; }
mkdir -p "$(dirname "$serial_log")"
serial_log="$(cd "$(dirname "$serial_log")" && pwd)/$(basename "$serial_log")"
: >"$serial_log"

accel="${QEMU_ACCEL:-tcg}"
echo "[boot-test] ISO: $iso"
echo "[boot-test] serial log: $serial_log"
echo "[boot-test] accelerator: $accel; network disabled"

set +e
timeout --kill-after=2s "${timeout_seconds}s" "$qemu" \
    -machine "pc,accel=$accel" \
    -m "${QEMU_MEM:-512}" \
    -smp "${QEMU_CPUS:-2}" \
    -cdrom "$iso" \
    -boot order=d \
    -display none \
    -monitor none \
    -serial "file:$serial_log" \
    -no-reboot \
    -no-shutdown \
    -net none
qemu_status=$?
set -e

if [[ -s "$serial_log" ]]; then
    cat "$serial_log"
fi
for expected in "${expects[@]}"; do
    if ! grep -Fq -- "$expected" "$serial_log"; then
        echo "[boot-test] missing serial assertion: $expected" >&2
        echo "[boot-test] QEMU exit status: $qemu_status" >&2
        exit 1
    fi
    echo "[boot-test] found: $expected"
done

case "$qemu_status" in
    0|124|137) ;;
    *) echo "[boot-test] QEMU exited unexpectedly with status $qemu_status" >&2; exit "$qemu_status" ;;
esac
echo "[boot-test] PASS"
