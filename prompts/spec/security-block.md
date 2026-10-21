# Kernel crypto and block-core contract

`src/security/crypto.c` provides a freestanding SHA-256 context API and a
constant-time byte comparison. The implementation has no dynamic state,
network access, or platform calls.

`src/io/block.c` validates capacity and power-of-two sector alignment before
calling a device's read/write callback. It never rounds requests. Flush is an
explicit callback and returns an error when unavailable or failed. Filesystem
transactions must use it as their persistence barrier.
