# Compatibility corpus runner

`run.py` validates a JSON corpus manifest and, when given a launcher, runs its
selected cases. Each case has a stable id, era (`7`, `10`, `11`, or `loos`),
executable path, optional argv, and optional working directory. Paths in the
manifest are relative to the manifest file unless absolute.

The launcher receives the executable and its arguments as separate argv items:

```sh
python3 tests/compat/run.py --manifest tests/compat/corpus.json --era 10 \
  --launcher ./path/to/loos-app-launcher
```

The launcher is responsible for booting or attaching to LoOS and returning a
nonzero status if the app fails its launch/window assertion. Without a launcher,
`--list` prints the selected cases and validation only checks manifest shape and
file presence; it does not claim that apps ran.

Set `LOOS_COMPAT_MANIFEST` to use a local manifest outside the repo, for
example one that points to privately held stock Windows binaries. The shared
manifest contains only repository-owned LoOS apps and never stores vendor
binaries.
