# Building the pinned Gold Band on Linux

[Gold Band](https://github.com/diodeme/Gold-Band) is a third-party (AGPL) app.
Setup makes the pinned commit **`987fc3e298b15d725a7f0e9b609b0f77dba63a74`**
(tag `v0.19.1`) present on a Linux workstation: the desktop app
(`gold-band-desktop`, a Tauri 2 app) and the headless `gold-band` binary, both
built from source, both installed to `~/.local/bin`. Upstream packages are not
used. Gold Band's code is never changed and never copied into this repository.

The surface is declared in `registry/estate-tooling.yaml` (`gold_band:`); the
builder is `scripts/gold-band/build-linux.sh`. The Mac build is a separate
ticket.

## What a setup run does

1. **No clone, no surface.** Setup builds from an existing clone at
   `~/Projects/diodeme/Gold-Band` (it never clones). Without one it prints
   `gold band: skipped, no clone at ...` and the run still succeeds.
2. **Read the pin without touching the clone.** The tree at the pinned commit
   is extracted with `git archive` into
   `~/.cache/agent-kit/gold-band/<commit>/src`.
3. **crate-age over Gold Band's `Cargo.lock`.** Exit `1` (a locked version is
   younger than 7 days) stops the build: the crate is printed, setup reports a
   failure, nothing is built or installed. Exit `2` (age unprovable) fails
   closed too, with exactly two exceptions that have no publish date by
   nature: Gold Band's own path crates, and the `tao` git patch, accepted only
   while the lockfile pins it to rev `c704261c`. An HTTP 429 from crates.io is
   reported as a rate limit to retry, never as a pass.
4. **Through the crate gate.** Gold Band ships a `.cargo/config.toml` that
   points crates.io at a third-party mirror (rsproxy.cn), which would bypass
   the gate. In the build copy only, it is replaced by a config that resolves
   through the gate at `127.0.0.1:17342` (`scripts/gate/install-linux.sh`
   installs the gate and crate-age; see [RUST.md](RUST.md)). One exception, stated rather than hidden: Gold Band's
   `[patch.crates-io] tao` is a git dependency at rev `c704261c`, which cargo
   fetches from github.com directly (the gate only mirrors crates.io); the
   builder refuses any other tao source. The web UI's npm
   packages come through the packages proxy that setup already configures.
5. **Build** `npm ci`, `npm run web:build` (the UI the desktop app embeds),
   then `cargo build --release -p gold-band --bin gold-band` and
   `cargo build --release -p gold-band-desktop`. The lockfile at the pin is
   stale for Gold Band's own two crates (`0.19.0` recorded, `0.19.1` in the
   manifests), so the build runs without `--locked`; afterwards the builder
   proves that no third-party entry of the lockfile changed.
6. **Install and report.** Both binaries go to `~/.local/bin`, and
   `~/.local/state/agent-kit/gold-band/build.json` records version, tag, build
   commit, build time, the age-check result and each binary's sha256. A run
   that finds that record matching the pin does nothing (`GOLD_BAND_FORCE=1`
   rebuilds). `setup-workstation.sh --check` only reports.

Exit status of the builder: `0` built, already built, or skipped; `3` a crate
too young or unprovable (build stopped, setup fails); `1` any other failure.

## System build dependencies (NixOS)

Do not install these system-wide. `scripts/gold-band/shell.nix` is a
`nix-shell` the builder runs the build in (`nix-shell scripts/gold-band/shell.nix
-I nixpkgs=flake:nixpkgs`). Its packages are what Tauri 2 on Linux needs:

| Need | nixpkgs attribute |
| --- | --- |
| Rust toolchain (edition 2024) | `cargo`, `rustc`, `gcc` |
| Web UI build | `nodejs_22` |
| Linking | `pkg-config`, `wrapGAppsHook3` |
| WebView | `webkitgtk_4_1` (webkit2gtk-4.1), `libsoup_3` |
| Toolkit | `gtk3`, `glib`, `cairo`, `pango`, `gdk-pixbuf`, `atk`, `librsvg` |
| TLS, tray, secrets | `openssl`, `libayatana-appindicator`, `libsecret`, `dbus` |

On other distributions the equivalents are the Tauri 2 prerequisites
(`webkit2gtk-4.1`, `gtk3`, `libsoup3`, `librsvg`, `libayatana-appindicator`,
`openssl`, a C toolchain) plus Rust and Node; when `cargo`, `node` and
`pkg-config --exists webkit2gtk-4.1` already work on `PATH`, the builder uses
them directly and skips `nix-shell`.

## Build results

Linux x86_64 (NixOS), 2026-11-10, from commit `987fc3e298b15d725a7f0e9b609b0f77dba63a74`:

* **Age check:** crate-age proved 766 of 768 locked registry versions at least
  7 days old and refused the two with no publish date by nature:
  `gold-band-desktop 0.19.0` (path) and `tao 0.35.3` (git, rev `c704261c`).
  No crate was younger than 7 days.
* **Build:** `npm ci` and the web build, then both cargo builds, all crates
  resolved through the gate; the lockfile changed only in Gold Band's own two
  versions.
* **Installed:** `~/.local/bin/gold-band` (29,941,328 bytes, sha256
  `3d833cea...0f38e12`; `--help` lists `run` and `continue`) and
  `~/.local/bin/gold-band-desktop` (97,125,168 bytes, sha256
  `0c532532...2ef3b`; every shared library resolves with `ldd`).
  The desktop window itself was not launched here (no display session).
* **Reported version:** the CLI has no `--version`; the version (0.19.1) is the
  source manifest's, recorded with the commit in the build record.


## The personal instance's agent map

After the build, setup converges the `agents` map of the personal instance's
`settings.json` (`${GOLD_BAND_HOME:-~/.gold-band}/settings.json`) onto the
registry (`scripts/converge.py gold-band`). The app's map replaces its own
built-in list, so the result is exactly the declared agents plus the ones the
keep file (`~/.config/agent-kit/keep.yaml`) names:

* `gold_band.agents` in `registry/estate-tooling.yaml` declares each agent, and
  each id must be one `gold_band_instances.gold-band-personal.agents` names.
  OpenCode is the first. For an id the app ships itself, `adapter.command` and
  `adapter.args` are not written: the app strips them when it saves.
* An undeclared agent is removed unless the keep file names it. An id that
  names Codex and any id in `gold_band.retired_agents` are removed even when
  kept.
* An agent whose `binary` is not on PATH is skipped and reported
  (`skipped: <id> (<binary> is not installed)`); its existing entry is left alone.
* Every other key of `settings.json` keeps its value and its order. A second
  run prints `settings unchanged` and writes nothing. A change keeps one
  `settings.json.bak`.
* Nothing is written while the desktop app runs (setup stops and says so), and
  a machine without Gold Band gets no settings file. `--check` only reports.
