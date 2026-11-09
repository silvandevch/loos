#!/usr/bin/env bash
# Builds a Linux root filesystem tar for the LoOS Linux layer from host binaries and every shared library they need.
#   lin-rootfs.sh out.tar /usr/bin/echo /usr/bin/ls ...     (files land at the same path; /bin -> /usr/bin)
set -euo pipefail
out=$1; shift
here=$(cd "$(dirname "$0")" && pwd)
stage=$(mktemp -d); trap 'rm -rf "$stage"' EXIT
add() {                         # copy a file (and its symlink target) keeping its path
    local f=$1; mkdir -p "$stage$(dirname "$f")"
    [ -e "$stage$f" ] || cp -L "$f" "$stage$f"
}
for b in "$@"; do
    add "$b"
    ldd "$b" 2>/dev/null | grep -o '/[^ ]*' | sort -u | while read -r l; do [ -e "$l" ] && add "$l"; done
done
add /lib64/ld-linux-x86-64.so.2
mkdir -p "$stage/etc" "$stage/tmp" "$stage/dev" "$stage/proc" "$stage/home" "$stage/run"
echo "loos" > "$stage/etc/hostname"
printf 'root:x:0:0:root:/root:/bin/sh\nuser:x:1000:1000:user:/home:/bin/sh\n' > "$stage/etc/passwd"
printf 'root:x:0:\nuser:x:1000:\n' > "$stage/etc/group"
printf 'nameserver 10.0.2.3\n' > "$stage/etc/resolv.conf"
printf '127.0.0.1 localhost\n' > "$stage/etc/hosts"
printf 'hosts: files dns\npasswd: files\ngroup: files\n' > "$stage/etc/nsswitch.conf"
for ca in /etc/pki/tls/certs/ca-bundle.crt /etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem /etc/ssl/certs/ca-certificates.crt; do [ -e "$ca" ] && { mkdir -p "$stage$(dirname "$ca")"; cp -L "$ca" "$stage$ca"; }; done
tar --format=ustar --owner=0 --group=0 --numeric-owner -C "$stage" -cf "$out" .
echo "$out: $(tar -tf "$out" | wc -l) entries, $(du -h "$out" | cut -f1)"
