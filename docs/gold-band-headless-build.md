# Building the headless Gold Band binary (#347)

The Gold Band workflow tests run a headless `gold-band` binary that the
environment supplies. It is built from the audited commit
`987fc3e298b15d725a7f0e9b609b0f77dba63a74` (tag v0.19.1), through the crate
gate, after `crate-age` has checked Gold Band's own lockfile. Nothing here
changes Gold Band's Rust.

## Steps

```bash
git clone https://github.com/diodeme/Gold-Band gold-band-src
cd gold-band-src && git checkout 987fc3e

# 1. Gold Band ships a project cargo config that replaces crates.io with the
#    rsproxy.cn mirror. That would bypass the crate gate, so move it aside.
mv .cargo/config.toml .cargo/config.toml.upstream-rsproxy

# 2. The age check over Gold Band's lockfile (crate-age is built from the
#    standards repo, scripts/crates/crate-age).
crate-age --root .

# 3. The headless binary only: the root package, not the Tauri app.
cargo build --release -p gold-band --bin gold-band
./target/release/gold-band --help
```

## Result of the build on 2026-11-08

- **Age check:** `crate-age` proved 766 of the 768 locked registry versions at
  least 7 days old and exited 2 for the other two, which have no publish date
  at all: `gold-band-desktop 0.19.0` (Gold Band's own Tauri crate, a path
  dependency) and `tao 0.35.3` (a git patch pinned to tauri-apps/tao rev
  `c704261c`). No crate is younger than 7 days, so nothing went to the owner
  as too young. The git patch is the one unproven dependency of the build.
- **Lockfile:** at the audited commit the lockfile is stale for Gold Band's own
  two crates (it records `0.19.0`, their manifests say `0.19.1`), so the build
  runs without `--locked`. The resulting lockfile diff is exactly those two
  version lines; no third-party version changes.
- **Crate gate:** the build exposed a gate bug, fixed in the standards repo
  (branch `fix/crate-gate-dl-index`): every crate whose name starts with `dl`
  (here `dlopen2`) got a 404, because the gate routed every `/dl/` path as a
  download. A second gate limit remains: a download over 10 MiB is refused
  with 502 (seen for `windows 0.56.0` during `cargo metadata`); the macOS
  build does not need that crate.
- **Binary:** `target/release/gold-band`, 25,514,016 bytes, sha256
  `16696be5db938bd2efbbeeb22f078b2ad970b7fa86f4b9e574eaba35415af034`.
  `--help` lists `run` and `continue`.

Point the headless tests at the binary with `GOLD_BAND_BIN=<path>`; without it
they are skipped with that reason.
