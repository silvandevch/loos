# Architecture

## Flow

```
refresh (background) ──> SystemInfo + Disk[]
                                   │
        ┌──────────────────────────┼───────────────────────────┐
        ▼                          ▼                           ▼
   Install plan              Migration plan                 Safety UI
   (immutable)               (immutable)
        │                          │
        ▼                          ▼
  worker thread              worker thread
  cancel: Arc<AtomicBool>    cancel: Arc<AtomicBool>
  progress: mpsc             progress: mpsc
        │                          │
        ▼                          ▼
  InstallOutcome             MigrationManifest
```

The GUI never performs disk I/O itself. It builds a plan, shows it, and starts
one worker. The worker sends progress over an `mpsc` channel; the UI polls each
frame and requests repaints while a job is active.

## Deployment methods

### Install media

1. Validate target: not the running system disk, not read-only, at least 2 GiB.
2. Validate the embedded payload's compressed SHA-256.
3. Stream-decompress zstd into the raw target while hashing the raw bytes.
4. `fsync`/flush, then read the written region back and compare its SHA-256.
5. On cancel, zero the first MiB so a partial image cannot masquerade as valid
   media.

The payload is a full disk image: GPT + FAT32 ESP for UEFI, or MBR + boot table
for BIOS. The LoOS kernel and installer are inside it, so installation works
offline.

### Local UEFI staging

1. Extract only `BOOTX64.EFI` and `KERNEL.ELF` from the payload's FAT32 ESP.
   The extractor parses GPT + FAT32 in Rust (no external tools at runtime).
2. Write them to `\EFI\LoOS\BOOTX64.EFI`, `\LOOS\KERNEL.ELF`,
   `\LOOS\BOOT.CFG`. The UEFI loader was patched to read its own loaded-image
   file path, so it never needs to overwrite Windows' fallback loader.
3. Add a `Boot####` firmware entry (Linux: `efibootmgr`; Windows:
   `SetFirmwareEnvironmentVariableW` with a HardDrive + FilePath device path).
4. No partition table, filesystem or user file is modified.

### Unattended selection

The kernel accepts `installer.auto.bytes=<N>`. It rejects the installation
media's own disk by GUID, matches exactly one remaining disk by byte size, and
aborts if the match is missing or ambiguous. The installed `BOOT.CFG` strips
every `installer.*` token, including credentials, after setup.

## Migration

- Walk each selected root, skipping configured cache names and (optionally)
  hidden entries. Symlinks are skipped unless explicitly followed.
- The destination layout defaults to
  `<destination>/home/<target-user>/<relative-source-path>/...`.
- Each file is copied to a `.loos-part` temporary name, fsynced, timestamped,
  then atomically renamed. Existing files are skipped, overwritten or renamed
  according to policy.
- SHA-256 is computed over source and destination; a mismatch aborts that file.
- A JSONL journal records copied files. A same-size/same-mtime destination is
  treated as already copied, so a cancelled run resumes cheaply.
- The final `manifest.json` records options, per-source totals and errors.
  `RESTORE.txt` explains the copy-only guarantee.

## Cancellation granularity

- Payload write: checked before each decompressed 4 MiB chunk.
- Verification: checked before each 4 MiB read.
- Migration: before each file and each 4 MiB read; temporary files are removed
  when a file copy is cancelled.
- Scanning: before each directory entry.

Some operations (a single 4 MiB write, an `fsync`, a firmware NVRAM call) are
atomic from the app's point of view. That is the honest limit of cancellation.

## Platform boundary

`src/platform/linux.rs` and `src/platform/windows.rs` own all OS-specific code:
disk enumeration, raw device opening, elevation, volume dismounting, ESP writes,
firmware boot entries and reboot. The rest of the engine is portable.
