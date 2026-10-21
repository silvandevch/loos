# LoOS Rust crates (see docs/LANGUAGE.md)

Cargo workspace for kernel pieces written in Rust. Crates are `no_std` + `alloc`, I/O-free state machines
with host `cargo test` suites (run from ctest as `rust.workspace`).

| crate | contents |
|---|---|
| `ksync` | ticket `SpinLock`; NT-style `Dispatcher` (events, semaphores, recursive mutants, wait any/all, timeouts, abandonment on thread exit) |

Layout rule: one module per concern under `src/`, integration tests under `tests/`, no `unsafe` outside
`spin.rs` (each block carries a `SAFETY:` comment).
