# LoOS — tools reference

One command does most things: **`./loos`**. Everything else is below it.

## `./loos` — the CLI

```
./loos status            where things live, disk, tool check
./loos setup             fetch srv03rtm + patchkit, verify layout
./loos prep              run the whole prep pipeline (see scripts/)
./loos verify            pre-flight: tools, wine prefix, paths
./loos build             build under Wine (hours)
./loos iso               (re)make the bootable ISO
./loos qemu              boot the ISO
./loos fixwine           fix Wine .cmd/.bat association
./loos vm-build          build on the Windows VM over SSH (all steps)
./loos vm <sync|build|iso|fetch|qemu|all>   VM sub-steps
./loos docs [topic]      print a doc from docs/
./loos relocate SRC OUT  change roots (writes .loosrc)
```

Config is `env.sh` + optional `.loosrc` — see [LAYOUT.md](LAYOUT.md).

---

## Prep scripts (`scripts/`)

| Script | Does | Example |
|---|---|---|
| `00-fetch.sh` | clone/update `srv03rtm` + `patchkit` into `SRC_ROOT` | `./loos setup` |
| `01-structure.sh` | verify the assembled layout + toolchains | `bash scripts/01-structure.sh` |
| `02-overlay-patchkit.sh` | apply patchkit (additive; `--force` overwrites) | `bash scripts/02-overlay-patchkit.sh` |
| `11-resolve-patchkit-conflicts.sh` | resolve the overlay conflicts in favour of patchkit (backup + EOL-normalise) | `bash scripts/11-resolve-patchkit-conflicts.sh --dry-run` |
| `03-patch-oobe.sh` | OOBE fixes at `base/ntsetup/oobe` | `bash scripts/03-patch-oobe.sh` |
| `04-certs.sh` | regenerate test-signing certs via openssl | `bash scripts/04-certs.sh` |
| `05-mediatags.sh` | write `WIN51`/`WIN51IA`/`WIN51IS` | `bash scripts/05-mediatags.sh` |
| `06-stubs.sh` | materialise/stub missing binaries from `tools/missing.cmd` | `RETAIL_ISO=… scripts/06-stubs.sh` |
| `run-all.sh` | the whole prep pipeline in order | `./loos prep` |
| `09-build-all.sh` | Wine build (razzle init + `_build-pass.cmd`) | `./loos build` |
| `10-verify-env.sh` | pre-flight checks | `./loos verify` |
| `fix-wine-assoc.sh` | stop Wine opening `.cmd` in notepad | `./loos fixwine` |
| `_iso-run.sh` | make ISO under Wine (uses `_iso.cmd`) | `./loos iso` |

Runs **inside** razzle (invoked via `exec`, never directly): `_build-pass.cmd`
(prebuild + build), `_iso.cmd` (oscdimg), `build-amd64.cmd` (razzle wrapper),
`build-all.cmd` (entry point).

## Remote / ISO / QEMU (`remote/`)

| Tool | Does | Example |
|---|---|---|
| `loos-build.py` | SSH to the VM: sync → build → fetch → qemu | `./loos vm all` |
| `build-in-vm.cmd` | runs on the VM inside razzle (prebuild+build+oscdimg) | copied by `loos-build.py` |
| `build-wine.sh` | build + ISO on this host under Wine | `remote/build-wine.sh srv` |
| `make-iso.sh` | Linux-only ISO from a layout dir (mkisofs) | `remote/make-iso.sh <dir> out.iso` |
| `run-qemu.sh` | boot an ISO with an install disk | `ISO=$OUT_ROOT/loos.iso remote/run-qemu.sh` |
| `config.example.env` | VM/host/paths config | copy to `config.env` |

`loos-build.py` subcommands: `sync build iso fetch qemu all`. Its config values
are also overridable as environment variables of the same name.

## Agent bus (`agents/`)

| Tool | Does |
|---|---|
| `agents/busd.py` | broker: SQLite store + TCP server |
| `agents/agentbus` | CLI (also `./loos bus …`) |
| `agents/client.py` | `Bus` class for agent code |

```bash
./loos bus start                      # start the broker
./loos bus send --from astra --to claude --topic kernel "…"
./loos bus inbox --for claude --unread
./loos bus thread kernel
./loos bus task ls
```

See [../agents/README.md](../agents/README.md) and [AGENTS.md](AGENTS.md).

## Environment variables you may care about

| Var | Default | Meaning |
|---|---|---|
| `SRC_ROOT` | NVMe `~/LoOS-build` | source + patchkit |
| `OUT_ROOT` | `$DATA/loos/out` | outputs, ISOs, disks |
| `NT_SRC` | `$SRC_ROOT/srv03rtm` | the source tree |
| `VARIANT` | `srv` | SKU for `oscdimg` |
| `ISO_OUT` | `$OUT_ROOT/loos.iso` | ISO destination |
| `RETAIL_ISO`/`RETAIL_SRC` | — | missing-binary source |
| `WINEPREFIX` | `~/.wine` | Wine prefix |
| `AGENTBUS_DIR` | `.agentbus` | agent message bus dir |
| `CFG_CROSS` | `0` | force `win64 amd64` cross build |
