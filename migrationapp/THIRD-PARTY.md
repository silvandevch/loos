# Third-party notices

LoOS Switch is MIT licensed. It bundles or links the following:

| Component | License | Use |
|---|---|---|
| [egui / eframe](https://github.com/emilk/egui) | MIT OR Apache-2.0 | GUI and native window/GL backend |
| [Inter](https://rsms.me/inter/) | SIL Open Font License 1.1 | bundled UI typeface (`assets/fonts/`) |
| [zstd / zstd-safe](https://github.com/gyscos/zstd-rs) | MIT OR BSD-3-Clause | embedded payload compression |
| [sha2](https://github.com/RustCrypto/hashes) | MIT OR Apache-2.0 | payload and migration verification |
| [image](https://github.com/image-rs/image) | MIT OR Apache-2.0 | PNG decoding/encoding |
| [serde / serde_json](https://serde.rs/) | MIT OR Apache-2.0 | manifests and payload metadata |
| [walkdir](https://github.com/BurntSushi/walkdir) | Unlicense OR MIT | migration tree walking |
| [fs2](https://github.com/danburkert/fs2-rs) | MIT OR Apache-2.0 | free-space query |
| [windows-sys](https://github.com/microsoft/windows-rs) | MIT OR Apache-2.0 | Windows raw disk and firmware APIs |
| [dirs](https://github.com/dirs-dev/dirs-rs) | MIT OR Apache-2.0 | platform user folders |

The embedded LoOS installer payload is built from this LoOS tree. Its own
third-party notices are in the parent repository's `THIRD-PARTY.md`.

The font licence text is in `assets/fonts/LICENSE.txt`.
