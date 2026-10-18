# LoOS — disk layout & keeping it small

Two roots, so a big source tree and big build output never fight for the same
disk. `env.sh` sets these; `.loosrc` (gitignored) overrides them; `./loos
relocate` writes `.loosrc` for you.

| Root | Default | Holds | Typical size | Why |
|---|---|---|---|---|
| `LOOS_ROOT` | this repo on **DATA** | scripts, docs, prompts | a few MB | small, shareable |
| `SRC_ROOT` | `/home/silvan/LoOS-build` (NVMe) | `srv03rtm`, `patchkit` | ~6.5 GB | fast disk = fast builds |
| `OUT_ROOT` | `$LOOS_DATA/loos/out` | build output, ISOs, VM disks | 5–40 GB | big & disposable → DATA |

Check current usage any time:

```bash
./loos status
```

## What lives where

```
/run/media/silvan/DATA/LoOS/          LOOS_ROOT  (this repo)
  env.sh  loos                         config + CLI
  scripts/  remote/  prompts/          tooling
  docs/                                this documentation
  certs/  missing/  logs/  backups/    small build inputs/outputs
  .agentbus/  .loosrc                  agent bus + your path overrides

/home/silvan/LoOS-build/              SRC_ROOT   (fast internal NVMe)
  srv03rtm/                            the NT 5.2 source tree (~6 GB)
  patchkit/                            openxp/patchkit

/run/media/silvan/DATA/loos/out/      OUT_ROOT   (external DATA, 355 GB free)
  binaries.<arch><type>/               build output
  loos.iso                             bootable image
  loos.qcow2                           QEMU install disk
```

## Low-space rules

1. **Never duplicate the source tree.** One copy, referenced by path. (An exact
   second copy on the slow drive is what wasted time before — don't do it.)
2. **Outputs go to DATA.** Build artifacts, ISOs and VM disks are large and
   disposable; they default to `OUT_ROOT` on DATA.
3. **Tarball for transport, not storage.** `loos-build.py sync` makes a temp
   `.tgz`, sends it, and deletes it. Don't keep it.
4. **Reclaim when done.** Safe to delete: `$OUT_ROOT/*` (rebuildable),
   `.agentbus/messages/*.tmp`, `logs/`. Never delete `SRC_ROOT/srv03rtm` unless
   you can re-fetch it (`./loos setup`).
5. **exFAT caveats.** DATA is exFAT: no symlinks, ~80 metadata ops/s. Fine for
   outputs and ISOs; do **not** check the 250k-file git tree out onto it — keep
   the tree on the NVMe `SRC_ROOT`.

## Moving roots (if internal disk gets tight)

```bash
# point source at DATA too (slower builds, more free space on internal disk)
./loos relocate /run/media/silvan/DATA/loos/src /run/media/silvan/DATA/loos/out
rsync -a --info=progress2 /home/silvan/LoOS-build/srv03rtm/ \
      /run/media/silvan/DATA/loos/src/srv03rtm/
```

## Fresh-machine bootstrap

```bash
./loos setup      # clones srv03rtm + patchkit into SRC_ROOT
./loos prep       # overlays patches, certs, tags, missing binaries
./loos verify     # checks tools + paths
```
