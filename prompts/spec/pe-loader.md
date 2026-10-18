# PE parser, relocator, import binder (src/win32/pe)

Pure C99, no allocation/OS calls; used by the loader and (later) Mm's SEC_IMAGE
path. Image bytes are untrusted: every RVA/offset is bounds-checked.

**Layouts.** `PE_LAYOUT_FILE` (raw file) and `PE_LAYOUT_MAPPED` (sections at
RVAs, `SizeOfImage` bytes). `pe_map` converts file -> mapped; `pe_relocate`,
`pe_bind_imports` require mapped.

**pe_open checks.** MZ/PE magic; machine/magic pairing (i386<->PE32,
amd64<->PE32+, else UNSUPPORTED); power-of-two alignments, FileAlignment
512..64K; `SizeOfImage` multiple of SectionAlignment; headers+section table fit
in `SizeOfHeaders`; sections ascending, aligned, non-overlapping, inside the
image; raw data inside the file (file layout).

**Imports.** One walker (`walk_imports`) serves enumerate and bind. Each DLL
name goes through `apiset_resolve`: `api-ms-`/`ext-ms-` names map to a host
module, unknown contracts fail `PE_ERR_APISET`, normal DLLs are lower-cased.
The resolver callback supplies each symbol address; it is written to the IAT
(4 or 8 byte slots). Failure fills `pe_bind_error` with dll/symbol. Uses the
INT (OriginalFirstThunk) when present, else the IAT. Limits: 4096 DLLs, 65536
thunks each.

**Relocations.** Types ABSOLUTE, HIGH, LOW, HIGHLOW, DIR64 (64-bit only). A
rebase of an image with no reloc directory fails `PE_ERR_NO_RELOCS`. PE32 bases
must fit 32 bits. `ImageBase` in the header is updated.

**Exports.** Binary search over the sorted name table, or by ordinal; forwarder
strings returned (`ex.forwarder`, `rva == 0`).

**Delay imports.** `pe_enum_delay_imports` / `pe_bind_delay_imports` use the same
walker (32-byte descriptors, IAT at +12, INT at +16). Attributes bit0 clear =
legacy VA-based descriptor, converted via ImageBase. Eager bind fills the delay
IAT; a loader wanting lazy binding just enumerates.

**Bound imports.** `pe_enum_bound_imports` is informational only; the binder
always rewrites the IAT, so a stale timestamp binding can never be trusted.

**TLS.** `pe_get_tls` returns template RVA/size, zero-fill, index slot RVA,
alignment and up to 32 callback RVAs (directory VAs converted against the
*current* image_base, so call it after `pe_relocate`). `pe_tls_init_block`
writes one thread's initial block. Running callbacks and allocating the index
is the loader's job (ntdll).

**Not yet:** ordinal forwarder chasing, resources, security directory/signature checks, W^X section
protection mapping (Mm's job), `/GS` cookie init.

**Verified against:** synthetic PE32/PE32+ built in the test, 6 real Wine
builtin PEs (all named exports found, imports enumerated, rebase round-trip
restores bytes), and 40k mutated-header fuzz cases under ASan+UBSan.
Delay/bound/TLS: synthetic PE32/PE32+ plus real `amdxcffx64.dll` (TLS); no real
sample with delay imports was available. Run: `tests/win32/run.sh`.
