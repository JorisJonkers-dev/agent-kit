"""Setup builds the pinned Gold Band on Linux (scripts/gold-band/build-linux.sh).

Each test runs the real script against a scratch git repository that stands in
for the Gold Band clone, with crate-age, npm, cargo and
pkg-config replaced by stubs. What is asserted is what the host holds afterwards:
which binaries are installed, what the build record says, whether cargo was ever
called, and what the build copy's cargo config points at. The age check's own
exit codes (1 too young, 2 unprovable) come from the standards repo.
"""

from __future__ import annotations

import json
import os
import platform
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path

import pytest
from setup_harness import setup_run, stub

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "gold-band" / "build-linux.sh"
SYSTEM_BIN = "/run/current-system/sw/bin" if Path("/run/current-system/sw/bin/bash").exists() else "/usr/bin"

pytestmark = pytest.mark.skipif(
    platform.system() != "Linux" or platform.machine() != "x86_64",
    reason="the build script is for Linux x86_64",
)

TAO = (
    'source = "git+https://github.com/tauri-apps/tao?rev='
    'c704261c519c58cfdd0bc2d58ba24e06a0b71c92#c704261c519c58cfdd0bc2d58ba24e06a0b71c92"'
)
LOCK = f"""[[package]]
name = "gold-band"
version = "0.19.0"

[[package]]
name = "serde"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tao"
version = "0.35.3"
{TAO}
"""


def png(size: int) -> bytes:
    return b"\x89PNG\r\n\x1a\n" + (13).to_bytes(4, "big") + b"IHDR" + size.to_bytes(4, "big") * 2 + b"test"


ICON = png(128)
ICONS = {
    "32x32.png": png(32),
    "64x64.png": png(64),
    "128x128.png": ICON,
    "128x128@2x.png": png(256),
    "icon.png": png(512),
}
SVG = b"<svg xmlns='http://www.w3.org/2000/svg'/>"
RSPROXY = '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n'


def git(repo: Path, *args: str) -> str:
    env = {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_SYSTEM": os.devnull, "PATH": os.environ["PATH"]}
    done = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, env=env, check=True)
    return done.stdout.strip()


@dataclass
class Host:
    root: Path
    clone: Path
    commit: str
    bin_dir: Path
    state: Path
    calls: Path

    def run(self, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
        full = {
            "HOME": str(self.root / "home"),
            "PATH": f"{self.root / 'stubs'}:{SYSTEM_BIN}:/usr/bin:/bin",
            "GOLD_BAND_DIR": str(self.clone),
            "GOLD_BAND_REF": self.commit,
            "GOLD_BAND_BIN_DIR": str(self.bin_dir),
            "GOLD_BAND_SHARE": str(self.root / "share"),
            "GOLD_BAND_CACHE": str(self.root / "cache"),
            "GOLD_BAND_STATE": str(self.state),
            "CRATE_AGE": "crate-age",
            **env,
        }
        return subprocess.run(["bash", str(SCRIPT), *args], capture_output=True, text=True, env=full, check=False)


def make_host(
    tmp_path: Path,
    *,
    age_status: int = 0,
    age_output: str = "",
    proxy_token: bool = True,
    cargo_lock: str | None = None,
) -> Host:
    root = tmp_path
    (root / "home").mkdir()
    if proxy_token:
        # What setup writes for the packages proxy; the value is never read here.
        (root / "home" / ".cargo").mkdir()
        (root / "home" / ".cargo" / "credentials.toml").write_text(
            '[registries.jorisjonkers-packages]\ntoken = "Bearer test"\n'
        )
    clone = root / "clone"
    clone.mkdir()
    git(clone, "init", "-q")
    git(clone, "config", "user.email", "owner@example.invalid")
    git(clone, "config", "user.name", "Owner")
    git(clone, "config", "commit.gpgsign", "false")
    (clone / "Cargo.toml").write_text('[package]\nname = "gold-band"\nversion = "0.19.1"\n')
    (clone / "Cargo.lock").write_text(LOCK)
    (clone / ".cargo").mkdir()
    (clone / ".cargo" / "config.toml").write_text(RSPROXY)
    (clone / "src-tauri" / "icons").mkdir(parents=True)
    for name, data in {**ICONS, "logo-source.svg": SVG}.items():
        (clone / "src-tauri" / "icons" / name).write_bytes(data)
    git(clone, "add", "-A")
    git(clone, "commit", "-q", "-m", "pin")
    commit = git(clone, "rev-parse", "HEAD")

    stubs = root / "stubs"
    stubs.mkdir()
    calls = root / "calls.log"
    stub(stubs, "crate-age", f'echo "crate-age $*" >> "{calls}"\ncat <<\'OUT\'\n{age_output}\nOUT\nexit {age_status}')
    stub(stubs, "npm", f'echo "npm $*" >> "{calls}"')
    stub(stubs, "node", "exit 0")
    stub(stubs, "pkg-config", "exit 0")
    new_lock = cargo_lock if cargo_lock is not None else LOCK.replace('version = "0.19.0"', 'version = "0.19.1"')
    lock_file = root / "new.lock"
    lock_file.write_text(new_lock)
    stub(
        stubs,
        "cargo",
        f'echo "cargo $*" >> "{calls}"\n'
        f'cat .cargo/config.toml > "{root}/build-cargo-config.toml"\n'
        f'cp "{lock_file}" Cargo.lock\n'
        "mkdir -p target/release\n"
        "for b in gold-band gold-band-desktop; do\n"
        '  printf "#!/bin/sh\\necho $b\\n" > target/release/$b; chmod +x target/release/$b\n'
        "done",
    )
    return Host(root, clone, commit, root / "bin", root / "state" / "build.json", calls)


def calls_of(host: Host) -> list[str]:
    return host.calls.read_text().splitlines() if host.calls.exists() else []


def test_a_machine_without_gold_band_skips_the_surface_and_says_so(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    done = host.run(GOLD_BAND_DIR=str(tmp_path / "nowhere"))
    assert done.returncode == 0
    assert "skipped: no Gold Band clone" in done.stdout
    assert not host.bin_dir.exists()
    assert calls_of(host) == []


def test_it_builds_both_binaries_from_the_pin_and_reports_version_and_commit(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    done = host.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert (host.bin_dir / "gold-band").exists() and (host.bin_dir / "gold-band-desktop").exists()
    record = json.loads(host.state.read_text())
    assert record["version"] == "0.19.1"
    assert record["commit"] == host.commit
    assert record["gold-band"]["sha256"] and record["gold-band-desktop"]["sha256"]
    assert f"built from commit {host.commit}" in done.stdout
    cargo = [c for c in calls_of(host) if c.startswith("cargo")]
    assert cargo == [
        "cargo build --release -p gold-band --bin gold-band",
        "cargo build --release -p gold-band-desktop --features tauri/custom-protocol",
    ]


def test_cargo_resolves_through_the_packages_proxy_not_gold_bands_mirror(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    assert host.run().returncode == 0
    config = tomllib.loads((tmp_path / "build-cargo-config.toml").read_text())
    proxy = "sparse+https://packages.jorisjonkers.dev/cargo/"
    assert config["source"]["crates-io"]["replace-with"] == "jorisjonkers-packages"
    assert config["source"]["jorisjonkers-packages"]["registry"] == proxy
    assert config["registries"]["jorisjonkers-packages"]["index"] == proxy
    assert config["registry"]["global-min-publish-age"] == "7 days"
    assert "127.0.0.1" not in str(config)
    config = str(config)
    assert "rsproxy" not in config
    # The clone itself is untouched.
    assert (host.clone / ".cargo" / "config.toml").read_text() == RSPROXY
    assert git(host.clone, "status", "--porcelain") == ""


def test_a_second_run_finds_the_pin_already_built(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    assert host.run().returncode == 0
    before = len(calls_of(host))
    again = host.run()
    assert again.returncode == 0
    assert "already built: 0.19.1" in again.stdout
    assert len(calls_of(host)) == before


def test_check_builds_nothing(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    done = host.run("--check")
    assert done.returncode == 0
    assert "would build" in done.stdout
    assert calls_of(host) == []
    assert not host.bin_dir.exists()


def test_a_crate_younger_than_seven_days_stops_the_build_and_is_reported(tmp_path: Path) -> None:
    host = make_host(tmp_path, age_status=1, age_output="young: serde 1.0.200: published 2 days ago")
    done = host.run()
    assert done.returncode == 3
    assert "young: serde 1.0.200" in done.stdout
    assert "younger than 7 days" in done.stdout
    assert [c for c in calls_of(host) if c.startswith(("cargo", "npm"))] == []
    assert not host.bin_dir.exists()


def test_an_unprovable_age_other_than_the_two_known_ones_stops_the_build(tmp_path: Path) -> None:
    out = "refused: gold-band 0.19.0: path dependency: no published date exists\nrefused: foo 1.0.0: not found"
    host = make_host(tmp_path, age_status=2, age_output=out)
    done = host.run()
    assert done.returncode == 3
    assert "cannot be proven" in done.stdout
    assert not host.bin_dir.exists()


def test_a_rate_limited_age_check_is_a_failure_to_retry_not_a_pass(tmp_path: Path) -> None:
    out = "refused: serde 1.0.200: GET https://crates.io/api/v1/crates/serde/1.0.200: http status: 429"
    host = make_host(tmp_path, age_status=2, age_output=out)
    done = host.run()
    assert done.returncode == 1
    assert "rate-limited" in done.stdout
    assert not host.bin_dir.exists()


def test_only_path_crates_and_the_pinned_tao_are_allowed_to_lack_an_age(tmp_path: Path) -> None:
    out = (
        "refused: gold-band-desktop 0.19.0: path dependency: no published date exists\n"
        "refused: tao 0.35.3: git dependency: no published date exists"
    )
    host = make_host(tmp_path, age_status=2, age_output=out)
    done = host.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert "unprovable by nature" in json.loads(host.state.read_text())["age_check"]


def test_tao_at_another_git_revision_stops_the_build(tmp_path: Path) -> None:
    out = "refused: tao 0.35.3: git dependency: no published date exists"
    host = make_host(tmp_path, age_status=2, age_output=out)
    (host.clone / "Cargo.lock").write_text(
        LOCK.replace(
            "c704261c519c58cfdd0bc2d58ba24e06a0b71c92#c704261c519c58cfdd0bc2d58ba24e06a0b71c92", "deadbeef#deadbeef"
        )
    )
    git(host.clone, "commit", "-q", "-am", "moved tao")
    done = host.run(GOLD_BAND_REF=git(host.clone, "rev-parse", "HEAD"))
    assert done.returncode == 3
    assert "tao is not the pinned git revision" in done.stdout
    assert not host.bin_dir.exists()


def test_no_packages_proxy_token_stops_the_build_before_anything_is_fetched(tmp_path: Path) -> None:
    host = make_host(tmp_path, proxy_token=False)
    done = host.run()
    assert done.returncode == 1
    assert "no packages proxy token" in done.stderr
    assert [c for c in calls_of(host) if c.startswith(("cargo", "npm"))] == []


def test_a_build_that_changes_a_third_party_lock_entry_installs_nothing(tmp_path: Path) -> None:
    host = make_host(tmp_path, cargo_lock=LOCK.replace("1.0.200", "1.0.201"))
    done = host.run()
    assert done.returncode == 3
    assert "third-party Cargo.lock entry" in done.stderr
    assert not (host.bin_dir / "gold-band").exists()


def test_a_missing_commit_is_reported(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    done = host.run(GOLD_BAND_REF="0" * 40)
    assert done.returncode == 1
    assert "is not in" in done.stderr


def _path(root: Path) -> dict[str, str]:
    # NixOS keeps bash and the rest in the system profile, not /usr/bin.
    return {"PATH": f"{root / 'bin'}:{SYSTEM_BIN}:/usr/bin:/bin"}


def test_setup_says_so_when_the_machine_has_no_gold_band_and_still_succeeds(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux", args=("--check",), extra_env=_path(tmp_path))
    assert "gold band: skipped, no clone at" in run.stdout
    assert "gold band: " not in run.stderr
    assert run.returncode == 0


def test_setup_runs_the_builder_when_the_clone_exists(tmp_path: Path) -> None:
    def clone(home: Path) -> None:
        (home / "Projects" / "diodeme" / "Gold-Band" / ".git").mkdir(parents=True)

    run = setup_run(tmp_path, "Linux", args=("--check",), prepare_home=clone, extra_env=_path(tmp_path))
    # git is a recording stub, so the pin cannot resolve: the builder fails and setup reports it.
    assert "gold band: the build failed" in run.stderr


def test_a_young_crate_stops_the_build_even_when_crate_age_exits_2_for_tao(tmp_path: Path) -> None:
    out = (
        "refused: tao 0.35.3: git dependency: no published date exists\n"
        "TOO NEW serde 1.0.200: 2 day(s) old, the floor is 7"
    )
    host = make_host(tmp_path, age_status=2, age_output=out)
    done = host.run()
    assert done.returncode == 3
    assert "TOO NEW serde 1.0.200" in done.stdout
    assert [c for c in calls_of(host) if c.startswith(("cargo", "npm"))] == []


def test_a_third_party_path_dependency_without_an_age_stops_the_build(tmp_path: Path) -> None:
    host = make_host(
        tmp_path, age_status=2, age_output="refused: vendored 1.0.0: path dependency: no published date exists"
    )
    assert host.run().returncode == 3


def test_a_lock_entry_with_a_different_source_at_the_same_version_installs_nothing(tmp_path: Path) -> None:
    moved = LOCK.replace(
        "registry+https://github.com/rust-lang/crates.io-index", "git+https://example.invalid/serde#abc"
    ).replace('version = "0.19.0"', 'version = "0.19.1"')
    host = make_host(tmp_path, cargo_lock=moved)
    assert host.run().returncode == 3
    assert not (host.bin_dir / "gold-band").exists()


def share(host: Host, *parts: str) -> Path:
    return host.root / "share" / Path(*parts)


def test_it_installs_the_launcher_runtime_shell_desktop_entry_and_icon(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    done = host.run()
    assert done.returncode == 0, done.stdout + done.stderr
    launcher = host.bin_dir / "gold-band-launch"
    assert os.access(launcher, os.X_OK)
    text = launcher.read_text()
    assert f"GB_BIN='{host.bin_dir}/gold-band-desktop'" in text
    assert f"GB_RUNTIME='{share(host, 'gold-band', 'runtime.nix')}'" in text
    assert "pkg-config --exists webkit2gtk-4.1" in text
    runtime = share(host, "gold-band", "runtime.nix").read_text()
    assert "webkitgtk_4_1" in runtime and "GB_LIBS" in runtime and "GB_DATA" in runtime
    entry = share(host, "applications", "gold-band.desktop").read_text().splitlines()
    for line in (
        "Name=Gold Band",
        f"Exec={launcher}",
        "Icon=gold-band",
        "Categories=Development;",
        "StartupWMClass=gold-band-desktop",
    ):
        assert line in entry
    hicolor = share(host, "icons", "hicolor")
    for size in (32, 64, 128, 256, 512):
        assert (hicolor / f"{size}x{size}" / "apps" / "gold-band.png").read_bytes() == png(size)
    assert (hicolor / "scalable" / "apps" / "gold-band.svg").read_bytes() == SVG
    icon_name = next(line for line in entry if line.startswith("Icon=")).removeprefix("Icon=")
    assert all(p.name.startswith(f"{icon_name}.") for p in hicolor.glob("*/apps/*"))
    assert len(list(hicolor.glob("*/apps/*"))) == 6


def test_the_launcher_execs_the_binary_directly_where_webkit_is_on_the_system(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    assert host.run().returncode == 0
    desktop = host.bin_dir / "gold-band-desktop"
    desktop.write_text('#!/bin/sh\necho "ran $*" > "$OUT"\n')
    out = tmp_path / "out"
    # pkg-config is stubbed to succeed (WebKitGTK found) and nix-shell is absent.
    done = subprocess.run(
        [str(host.bin_dir / "gold-band-launch"), "--flag"],
        capture_output=True,
        text=True,
        env={"PATH": f"{tmp_path / 'stubs'}:{SYSTEM_BIN}:/usr/bin:/bin", "OUT": str(out)},
        check=False,
    )
    assert done.returncode == 0, done.stderr
    assert out.read_text() == "ran --flag\n"


def test_the_launcher_enters_runtime_nix_when_webkit_is_missing(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    assert host.run().returncode == 0
    stubs = tmp_path / "stubs"
    stub(stubs, "pkg-config", "exit 1")
    stub(stubs, "nix-shell", f'echo "nix-shell $*" > "{tmp_path}/nix.log"')
    done = subprocess.run(
        [str(host.bin_dir / "gold-band-launch"), "a b"],
        capture_output=True,
        text=True,
        env={"PATH": f"{stubs}:{SYSTEM_BIN}:/usr/bin:/bin"},
        check=False,
    )
    assert done.returncode == 0, done.stderr
    line = (tmp_path / "nix.log").read_text()
    assert "-I nixpkgs=flake:nixpkgs" in line
    assert str(share(host, "gold-band", "runtime.nix")) in line
    assert "LD_LIBRARY_PATH" in line and "XDG_DATA_DIRS" in line and "a\\ b" in line


def test_a_second_run_and_the_already_built_path_leave_the_files_alone(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    assert host.run().returncode == 0
    entry = share(host, "applications", "gold-band.desktop")
    stamp = entry.stat().st_mtime_ns
    again = host.run()
    assert again.returncode == 0
    assert "already built" in again.stdout
    assert f"up to date: {entry}" in again.stdout
    assert entry.stat().st_mtime_ns == stamp


def test_hand_installed_copies_are_replaced_and_a_stale_built_machine_gets_the_entry(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    assert host.run().returncode == 0
    entry = share(host, "applications", "gold-band.desktop")
    for path in (host.bin_dir / "gold-band-launch", entry, share(host, "gold-band", "runtime.nix")):
        path.write_text("hand made\n")
    done = host.run()
    assert done.returncode == 0
    assert f"installed {entry}" in done.stdout
    assert "hand made" not in entry.read_text()
    assert "hand made" not in (host.bin_dir / "gold-band-launch").read_text()
    assert "webkitgtk_4_1" in share(host, "gold-band", "runtime.nix").read_text()


def test_check_reports_the_desktop_files_and_writes_none(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    done = host.run("--check")
    assert done.returncode == 0
    assert f"would install {share(host, 'applications', 'gold-band.desktop')}" in done.stdout
    assert not (host.root / "share").exists()
    assert not host.bin_dir.exists()
    host.run()
    again = host.run("--check")
    assert "would install" not in again.stdout


def test_a_missing_icon_is_reported_and_does_not_fail_the_run(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    git(host.clone, "rm", "-q", "-r", "src-tauri")
    git(host.clone, "commit", "-q", "-m", "no icons")
    done = host.run(GOLD_BAND_REF=git(host.clone, "rev-parse", "HEAD"))
    assert done.returncode == 0, done.stdout + done.stderr
    assert "the icon was not installed" in done.stdout
    assert share(host, "applications", "gold-band.desktop").exists()


def test_a_stale_icon_cache_is_removed_where_no_cache_tool_exists(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    cache = share(host, "icons", "hicolor", "icon-theme.cache")
    cache.parent.mkdir(parents=True)
    cache.write_text("stale")
    assert host.run().returncode == 0
    assert not cache.exists()


def test_the_cache_tool_runs_when_present_and_its_failure_is_not_fatal(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    stub(tmp_path / "stubs", "gtk-update-icon-cache", f'echo "$*" > "{tmp_path}/cache.log"\nexit 1')
    done = host.run()
    assert done.returncode == 0
    assert (tmp_path / "cache.log").read_text().startswith("-f -t ")


def test_retire_removes_every_installed_file_and_keeps_the_binaries(tmp_path: Path) -> None:
    host = make_host(tmp_path)
    assert host.run().returncode == 0
    dry = host.run("--retire", "--check")
    assert dry.returncode == 0 and "would remove" in dry.stdout
    assert (host.bin_dir / "gold-band-launch").exists()
    done = host.run("--retire")
    assert done.returncode == 0, done.stdout + done.stderr
    assert not (host.bin_dir / "gold-band-launch").exists()
    assert not list((host.root / "share").rglob("gold-band*"))
    assert (host.bin_dir / "gold-band-desktop").exists()
