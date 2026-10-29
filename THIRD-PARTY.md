# THIRD-PARTY — borrowed open-source components

One row per component borrowed into LoOS. See
[`docs/SOURCE-POLICY.md`](docs/SOURCE-POLICY.md). Keep this current: if you
borrow, you credit.

| Component | Version | License | Used in (files/modules) | Notes |
|---|---|---|---|---|
| _(none yet)_ | | | | The btrfs reader uses public on-disk format facts; it does not copy or link Linux/btrfs-progs code. |

License note if any GPL/LGPL code is included: state the resulting obligation for
the LoOS component that links it.

| Inter (glyph outlines in `crates/loos-shell/src/font_data.rs`, generated from `scripts/gen_font_outline.py` output by `crates/loos-shell/tools/gen_font_data.py`) | SIL Open Font License 1.1 | outlines (Regular, SemiBold) embedded and rasterised at run time; license text at https://openfontlicense.org |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | `crates/loos-shell` (browser web bridge) | vendored crate; raw-DEFLATE decompression of pages received from the QEMU web bridge |
