#!/usr/bin/env bash
# Vendors build-std dependencies into ./vendor. Runs from /tmp so the parent LoOS .cargo/config.toml
# (which replaces crates-io with its own vendor dir) does not apply.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
tc=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$here/rust-toolchain.toml")
src=$(rustc +"$tc" --print sysroot)/lib/rustlib/src/rust/library/Cargo.toml
cd /tmp && cargo +"$tc" vendor --manifest-path "$src" --versioned-dirs "$here/vendor" >/dev/null
