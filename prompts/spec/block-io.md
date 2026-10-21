# LoOS block I/O contract

`src/io/block.c` is the synchronous block-device boundary used by filesystem
code while the interrupt-driven queue is brought up. Device capacity and
sector size are explicit; every request must be sector-aligned and remain
inside capacity. The device supplies read/write/flush callbacks, and callback
failure becomes `LOOS_BLOCK_EIO`.

The filesystem must call `flush` at transaction barriers. No block API opens a
host path, creates a network connection, or silently rounds an unaligned
request.
