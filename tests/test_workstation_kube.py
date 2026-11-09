"""workstation-kube: agents on a device read the clusters; only the sudo password administers them."""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = KIT_ROOT / "scripts" / "workstation-kube.py"

FAKE_KUBECTL = r'''#!/usr/bin/env python3
"""A stand-in for kubectl over a JSON state file: ServiceAccounts, token Secrets, contexts."""
import json, os, sys
state_path = os.environ["FAKE_KUBE_STATE"]
state = json.load(open(state_path))
args = sys.argv[1:]
with open(os.environ["FAKE_ARGV_LOG"], "a") as log:
    log.write("kubectl " + " ".join(args) + "\n")
def opt(name):
    if name in args:
        i = args.index(name); value = args[i + 1]; del args[i:i + 2]; return value
context = opt("--context") or "personal"
opt("--kubeconfig")
ns = opt("-n")
admin = context.endswith("-admin")
cluster = context[: -len("-admin")] if admin else context
def save():
    json.dump(state, open(state_path, "w"))
if admin and os.environ.get("FAKE_AS_AGENT") and not os.isatty(0):
    sys.exit("exec plugin cannot support interactive mode: standard input is not a terminal")
if args[:2] == ["config", "view"]:
    print(json.dumps({"clusters": [{"name": cluster, "cluster": {"server": f"https://{cluster}:6443", "certificate-authority-data": f"CA-{cluster}"}}]}))
elif args[:2] == ["auth", "can-i"]:
    print("yes" if admin or args[2:4] == ["list", "pods"] else "no")
elif args[:2] == ["get", "serviceaccount"]:
    sys.exit(0 if f"{cluster}/{args[2]}" in state["sas"] else 1)
elif args[:2] == ["get", "secret"]:
    key = f"{cluster}/{args[2]}"
    if key not in state["secrets"]:
        sys.exit(1)
    import base64
    print(json.dumps({"data": {"token": base64.b64encode(state["secrets"][key].encode()).decode()}}))
elif args[:2] == ["delete", "secret"]:
    state["secrets"].pop(f"{cluster}/{args[2]}", None); save()
elif args[:1] == ["create"]:
    doc = json.load(open(args[args.index("-f") + 1]))
    state["minted"] += 1
    state["secrets"][f"{cluster}/{doc['metadata']['name']}"] = f"TOKEN-{cluster}-{doc['metadata']['name']}-{state['minted']}"
    state.setdefault("created", []).append(doc); save()
elif args[:2] == ["get", "namespaces"]:
    print("default")
else:
    sys.exit(f"fake kubectl: unhandled {args}")
'''

# sudo and ssh run their command here; chown only records, since tests are not root.
FAKE_SUDO = """#!/bin/sh
echo "sudo $*" >> "$FAKE_ARGV_LOG"
while [ "${1#-}" != "$1" ]; do [ "$1" = "-u" ] && shift; shift; done
exec "$@"
"""
FAKE_SSH = """#!/bin/sh
echo "ssh $*" >> "$FAKE_ARGV_LOG"
for last; do :; done
exec sh -c "$last"
"""
FAKE_CHOWN = """#!/bin/sh
echo "chown $*" >> "$FAKE_ARGV_LOG"
"""
# Tests are not root, so file modes cannot stop an agent; this cat stands in for them.
FAKE_CAT = """#!/bin/sh
if [ -n "$FAKE_AS_AGENT" ]; then
  for a; do case "$a" in "$FAKE_ROOT_ONLY"*) echo "cat: $a: Permission denied" >&2; exit 1;; esac; done
fi
exec /bin/cat "$@"
"""


def load_module():
    spec = importlib.util.spec_from_file_location("workstation_kube", SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules["workstation_kube"] = module
    spec.loader.exec_module(module)
    return module


class Device:
    def __init__(self, tmp: Path) -> None:
        self.tmp = tmp
        self.home = tmp / "home"
        self.home.mkdir()
        self.secret_dir = tmp / "root-only"
        bin_dir = tmp / "bin"
        bin_dir.mkdir()
        for name, body in {"kubectl": FAKE_KUBECTL, "sudo": FAKE_SUDO, "ssh": FAKE_SSH, "chown": FAKE_CHOWN, "cat": FAKE_CAT}.items():
            (bin_dir / name).write_text(body)
            (bin_dir / name).chmod(0o755)
        self.state = tmp / "state.json"
        self.log = tmp / "argv.log"
        self.log.write_text("")
        self.set_state(sas=[f"{c}/macbook-{lvl}" for c in ("personal", "blueshell") for lvl in ("ro", "admin")])
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "HOME": str(self.home),
            "FAKE_KUBE_STATE": str(self.state),
            "FAKE_ARGV_LOG": str(self.log),
            "FAKE_ROOT_ONLY": str(self.secret_dir),
        }

    def set_state(self, sas: list[str]) -> None:
        self.state.write_text(json.dumps({"sas": sas, "secrets": {}, "minted": 0}))

    def run(self, *args: str, agent: bool = False) -> subprocess.CompletedProcess[str]:
        env = {**self.env, "FAKE_AS_AGENT": "1"} if agent else self.env
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args, "--device", "macbook", "--secret-dir", str(self.secret_dir)],
            capture_output=True,
            text=True,
            env=env,
            stdin=subprocess.DEVNULL,
        )

    def install(self, *extra: str) -> subprocess.CompletedProcess[str]:
        # The owner's admin context is a plain one here: the fake has no terminal to offer sudo.
        result = self.run(
            "install", "--admin-kubeconfig", str(self.tmp / "admin-config"), "--admin-context", "{cluster}", *extra, agent=True
        )
        assert result.returncode == 0, result.stderr + result.stdout
        return result

    def kubeconfig(self) -> dict:
        return json.loads((self.home / ".kube" / "config").read_text())

    def tokens(self) -> dict[str, str]:
        return json.loads(self.state.read_text())["secrets"]


@pytest.fixture
def device(tmp_path: Path) -> Device:
    d = Device(tmp_path)
    (tmp_path / "admin-config").write_text("{}")
    return d


def user(config: dict, name: str) -> dict:
    return next(u["user"] for u in config["users"] if u["name"] == name)


def test_an_agent_context_holds_the_read_only_token_inline(device: Device) -> None:
    device.install()
    assert user(device.kubeconfig(), "personal-macbook-ro") == {"token": device.tokens()["personal/macbook-ro-token"]}


def test_an_admin_context_reads_its_token_with_sudo_from_the_root_only_folder(device: Device) -> None:
    device.install()
    exec_ = user(device.kubeconfig(), "blueshell-macbook-admin")["exec"]
    assert exec_["args"][-1] == f"{device.secret_dir}/blueshell-admin.json"
    assert exec_["command"].endswith("/sudo")
    assert exec_["interactiveMode"] == "Always"


def test_the_admin_token_file_is_an_exec_credential_holding_the_admin_token(device: Device) -> None:
    device.install()
    credential = json.loads((device.secret_dir / "personal-admin.json").read_text())
    assert credential["kind"] == "ExecCredential"
    assert credential["status"]["token"] == device.tokens()["personal/macbook-admin-token"]


def test_the_admin_token_file_is_owner_only(device: Device) -> None:
    device.install()
    assert (device.secret_dir / "personal-admin.json").stat().st_mode & 0o077 == 0
    assert device.secret_dir.stat().st_mode & 0o077 == 0


def test_every_context_is_listed_for_both_clusters(device: Device) -> None:
    device.install()
    names = [c["name"] for c in device.kubeconfig()["contexts"]]
    assert names == ["personal", "personal-admin", "blueshell", "blueshell-admin"]


def test_no_token_ever_reaches_a_command_line(device: Device) -> None:
    device.install()
    logged = device.log.read_text()
    assert device.tokens()
    for token in device.tokens().values():
        assert token not in logged


def test_rotate_replaces_both_tokens_of_the_device(device: Device) -> None:
    device.install()
    before = device.tokens()
    device.install("--rotate")
    after = device.tokens()
    assert all(after[k] != before[k] for k in before)
    assert user(device.kubeconfig(), "personal-macbook-ro")["token"] == after["personal/macbook-ro-token"]


def test_a_recreated_token_secret_matches_what_gitops_declares(device: Device) -> None:
    device.install()
    created = json.loads(device.state.read_text())["created"][0]
    assert created["type"] == "kubernetes.io/service-account-token"
    assert created["metadata"]["annotations"] == {"kubernetes.io/service-account.name": "macbook-ro"}
    assert created["metadata"]["labels"] == {"workstation-access/device": "macbook", "workstation-access/level": "ro"}


def test_a_kubeconfig_it_did_not_write_is_moved_to_the_root_only_folder(device: Device) -> None:
    kube = device.home / ".kube"
    kube.mkdir()
    (kube / "config").write_text("clusters: [old-admin-cert]\n")
    device.install()
    moved = list((device.secret_dir / "previous").iterdir())
    assert [m.read_text() for m in moved] == ["clusters: [old-admin-cert]\n"]


def test_its_own_kubeconfig_is_replaced_without_keeping_a_copy(device: Device) -> None:
    device.install()
    device.install()
    assert not (device.secret_dir / "previous").exists()


def test_a_device_gitops_does_not_declare_fails_naming_where_to_declare_it(device: Device) -> None:
    device.set_state(sas=[])
    result = device.run("install", "--admin-kubeconfig", str(device.tmp / "admin-config"))
    assert result.returncode == 1
    assert "cluster/flux/apps/core/workstation-access" in result.stderr


def test_a_remote_device_is_installed_over_ssh(device: Device) -> None:
    device.install("--ssh", "deploy@desktop", "--user", os.environ.get("USER", "nobody"))
    assert "ssh -p 22 -o BatchMode=yes deploy@desktop" in device.log.read_text()
    assert (device.secret_dir / "personal-admin.json").exists()


def test_check_passes_when_an_agent_can_read_but_not_administer(device: Device) -> None:
    device.install()
    result = device.run("check", agent=True)
    assert result.returncode == 0, result.stdout
    assert "FAIL" not in result.stdout


def test_check_fails_when_an_admin_context_works_without_a_terminal(device: Device) -> None:
    device.install()
    result = device.run("check")  # not an agent: the fake lets the admin context through
    assert result.returncode == 1
    assert "FAIL personal-admin refused without a terminal" in result.stdout


def test_a_nixos_device_runs_sudo_and_cat_from_the_nix_paths() -> None:
    module = load_module()
    config = json.loads(
        module.render_kubeconfig("rx7900xtx", module.NIXOS, {"personal": {"server": "s", "certificate-authority-data": "c"}}, {"personal": "t"})
    )
    exec_ = user(config, "personal-rx7900xtx-admin")["exec"]
    assert [exec_["command"], *exec_["args"]] == [
        "/run/wrappers/bin/sudo",
        "/run/current-system/sw/bin/cat",
        "/etc/workstation-kube/personal-admin.json",
    ]
