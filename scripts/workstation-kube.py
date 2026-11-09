#!/usr/bin/env python3
"""Install a device's kubectl access: read-only for agents, admin behind sudo.

Each device has two ServiceAccounts per cluster, declared in GitOps
(fleet-infra and ESA-Blueshell/website, ``cluster/flux/apps/core/workstation-access``):
``<device>-ro`` (view, no Secrets) and ``<device>-admin`` (cluster-admin).
``install`` writes the device owner's ``~/.kube/config`` with

* ``<cluster>``: the read-only token, inline. This is what agents use.
* ``<cluster>-admin``: an exec credential that runs ``sudo cat`` on a
  root-only file holding the admin token. Every use asks for the sudo
  password, and kubectl refuses it outside a terminal.

A previous ``~/.kube/config`` this tool did not write is moved, not deleted,
into the root-only ``previous/`` folder next to the admin tokens.

``install --rotate`` deletes the device's token Secrets and creates them
again, so the old tokens stop working, then installs the new ones. ``check``
reports what an agent on the device can and cannot do.

Admin access to mint tokens comes from ``--admin-kubeconfig`` (default
``~/.kube/config``), context ``--admin-context`` (default
``{cluster}-admin``). A kubeconfig only root can read is read with sudo.

Tokens never reach argv or output: they travel on stdin. Stdlib only.

Usage::

    python3 scripts/workstation-kube.py install --device macbook
    python3 scripts/workstation-kube.py install --device rx7900xtx \\
        --ssh deploy@100.112.121.102 --ssh-port 2222 --user ExtraToast
    python3 scripts/workstation-kube.py check --device macbook
"""

from __future__ import annotations

import argparse
import atexit
import base64
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

NAMESPACE = "workstation-access"
MARKER = "workstation-kube"
DEFAULT_CLUSTERS = ("personal", "blueshell")
TOKEN_WAIT_SECONDS = 60


class Failure(Exception):
    """A problem the owner has to fix; printed without a traceback."""


@dataclass(frozen=True)
class Layout:
    """Where the target device keeps its binaries and root-only files."""

    sudo: str
    cat: str
    secret_dir: str
    root_group: str


DARWIN = Layout("/usr/bin/sudo", "/bin/cat", "/private/etc/workstation-kube", "wheel")
NIXOS = Layout("/run/wrappers/bin/sudo", "/run/current-system/sw/bin/cat", "/etc/workstation-kube", "root")
LINUX = Layout("/usr/bin/sudo", "/bin/cat", "/etc/workstation-kube", "root")


class Target:
    """The device being installed: this machine, or one reached over ssh."""

    def __init__(self, ssh: str | None, ssh_port: int, user: str | None, secret_dir: str | None) -> None:
        self.ssh = ssh
        self.ssh_port = ssh_port
        owner = user or (os.environ.get("USER") if ssh is None else None)
        if not owner:
            raise Failure("--user is required with --ssh")
        self.user: str = owner
        self.layout = self._detect_layout()
        if secret_dir:
            self.layout = Layout(self.layout.sudo, self.layout.cat, secret_dir, self.layout.root_group)
        # This machine: $HOME. A remote device: the owner's passwd entry.
        self.home = str(Path.home()) if ssh is None else self.sh(f"getent passwd {shlex.quote(self.user)} | cut -d: -f6").strip()
        if not self.home:
            raise Failure(f"no home directory for {self.user} on {self.name}")
        self.group = self.sh(f"id -gn {shlex.quote(self.user)}").strip()

    def _detect_layout(self) -> Layout:
        system = self.sh("uname -s").strip()
        if system == "Darwin":
            return DARWIN
        if self.sh("test -x /run/wrappers/bin/sudo && echo nixos || true").strip() == "nixos":
            return NIXOS
        return LINUX

    def sh(self, script: str, data: str | None = None, root: bool = False, check: bool = True) -> str:
        """Run a shell script on the target, as root when asked; data goes to stdin."""
        command = ["sudo", "sh", "-c", script] if root else ["sh", "-c", script]
        if self.ssh:
            command = ["ssh", "-p", str(self.ssh_port), "-o", "BatchMode=yes", self.ssh, shlex.join(command)]
        result = subprocess.run(command, input=data, capture_output=True, text=True)
        if check and result.returncode != 0:
            raise Failure(f"on {self.name}: {script.split(';')[0]} failed: {result.stderr.strip()}")
        return result.stdout

    @property
    def name(self) -> str:
        return self.ssh or "this machine"

    def write_root_file(self, path: str, data: str) -> None:
        d = shlex.quote(self.layout.secret_dir)
        p = shlex.quote(path)
        g = self.layout.root_group
        self.sh(
            f"umask 077; mkdir -p {d}; chown 0:{g} {d}; chmod 700 {d}; cat > {p}; chown 0:{g} {p}; chmod 600 {p}",
            data=data,
            root=True,
        )

    def install_kubeconfig(self, data: str) -> str | None:
        """Write the owner's kubeconfig; move a foreign one aside. Returns where it went."""
        kube = f"{self.home}/.kube"
        config = f"{kube}/config"
        moved = None
        existing = self.sh(f"cat {shlex.quote(config)} 2>/dev/null || true", root=True)
        if existing.strip() and not is_ours(existing):
            previous = f"{self.layout.secret_dir}/previous"
            moved = f"{previous}/config.{time.strftime('%Y%m%dT%H%M%S')}"
            g = self.layout.root_group
            self.sh(
                f"umask 077; mkdir -p {shlex.quote(previous)}; chown -R 0:{g} {shlex.quote(self.layout.secret_dir)};"
                f" chmod 700 {shlex.quote(self.layout.secret_dir)} {shlex.quote(previous)};"
                f" mv {shlex.quote(config)} {shlex.quote(moved)}; chmod 600 {shlex.quote(moved)}",
                root=True,
            )
        u, g = shlex.quote(self.user), shlex.quote(self.group)
        self.sh(
            f"umask 077; mkdir -p {shlex.quote(kube)}; chown {u}:{g} {shlex.quote(kube)}; chmod 700 {shlex.quote(kube)};"
            f" cat > {shlex.quote(config)}; chown {u}:{g} {shlex.quote(config)}; chmod 600 {shlex.quote(config)}",
            data=data,
            root=True,
        )
        return moved

    def as_user(self, args: list[str], stdin_null: bool = True) -> subprocess.CompletedProcess[str]:
        """Run a command as the device owner, with no terminal, as an agent would."""
        inner = shlex.join(args) + (" </dev/null" if stdin_null else "")
        if self.ssh:
            inner = f"sudo -u {shlex.quote(self.user)} sh -c {shlex.quote(inner)}"
            command = ["ssh", "-p", str(self.ssh_port), "-o", "BatchMode=yes", self.ssh, inner]
        else:
            command = ["sh", "-c", inner]
        return subprocess.run(command, capture_output=True, text=True, stdin=subprocess.DEVNULL)


def is_ours(kubeconfig: str) -> bool:
    try:
        doc = json.loads(kubeconfig)
    except ValueError:
        return False
    return any(e.get("name") == MARKER for e in doc.get("extensions") or [])


class Admin:
    """kubectl with the owner's admin credentials, used to mint device tokens."""

    def __init__(self, kubeconfig: str, context_format: str) -> None:
        self.kubeconfig = self._readable(kubeconfig)
        self.context_format = context_format

    @staticmethod
    def _readable(path: str) -> str:
        if os.access(path, os.R_OK):
            return path
        # A break-glass kubeconfig only root can read: copy it to a private temp file.
        data = subprocess.run(["sudo", "cat", path], capture_output=True, text=True)
        if data.returncode != 0:
            raise Failure(f"cannot read admin kubeconfig {path}: {data.stderr.strip()}")
        fd, copy = tempfile.mkstemp(prefix="workstation-kube-")
        atexit.register(os.unlink, copy)
        with os.fdopen(fd, "w") as f:
            f.write(data.stdout)
        return copy

    def kubectl(self, cluster: str, args: list[str], check: bool = True) -> subprocess.CompletedProcess[str]:
        context = self.context_format.format(cluster=cluster)
        # stdin stays the terminal: an exec credential behind sudo needs it.
        result = subprocess.run(
            ["kubectl", "--kubeconfig", self.kubeconfig, "--context", context, *args], capture_output=True, text=True
        )
        if check and result.returncode != 0:
            raise Failure(f"{cluster}: kubectl {' '.join(args[:3])} failed: {result.stderr.strip()}")
        return result

    def cluster_entry(self, cluster: str) -> dict:
        view = json.loads(self.kubectl(cluster, ["config", "view", "--raw", "--minify", "-o", "json"]).stdout)
        entry = view["clusters"][0]["cluster"]
        return {"server": entry["server"], "certificate-authority-data": entry["certificate-authority-data"]}

    def require_service_accounts(self, cluster: str, device: str) -> None:
        for level in ("ro", "admin"):
            found = self.kubectl(cluster, ["-n", NAMESPACE, "get", "serviceaccount", f"{device}-{level}", "-o", "name"], check=False)
            if found.returncode != 0:
                raise Failure(
                    f"{cluster}: ServiceAccount {NAMESPACE}/{device}-{level} does not exist. Declare the device in"
                    " cluster/flux/apps/core/workstation-access and let Flux apply it first."
                )

    def recreate_token_secret(self, cluster: str, device: str, level: str) -> None:
        name = f"{device}-{level}-token"
        self.kubectl(cluster, ["-n", NAMESPACE, "delete", "secret", name, "--ignore-not-found"])
        self.ensure_token_secret(cluster, device, level)

    def ensure_token_secret(self, cluster: str, device: str, level: str) -> None:
        name = f"{device}-{level}-token"
        if self.kubectl(cluster, ["-n", NAMESPACE, "get", "secret", name, "-o", "name"], check=False).returncode == 0:
            return
        secret = token_secret(device, level)
        # A file, not stdin: stdin must stay the terminal for a sudo-backed context.
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(secret, f)
        try:
            self.kubectl(cluster, ["create", "-f", f.name])
        finally:
            os.unlink(f.name)

    def token(self, cluster: str, device: str, level: str) -> str:
        name = f"{device}-{level}-token"
        deadline = time.monotonic() + TOKEN_WAIT_SECONDS
        while True:
            got = self.kubectl(cluster, ["-n", NAMESPACE, "get", "secret", name, "-o", "json"], check=False)
            if got.returncode == 0:
                encoded = (json.loads(got.stdout).get("data") or {}).get("token")
                if encoded:
                    return base64.b64decode(encoded).decode()
            if time.monotonic() > deadline:
                raise Failure(f"{cluster}: Secret {NAMESPACE}/{name} has no token after {TOKEN_WAIT_SECONDS}s")
            time.sleep(1)


def token_secret(device: str, level: str) -> dict:
    """The token Secret exactly as GitOps declares it, so Flux adopts it unchanged."""
    labels = {"workstation-access/device": device, "workstation-access/level": level}
    return {
        "apiVersion": "v1",
        "kind": "Secret",
        "type": "kubernetes.io/service-account-token",
        "metadata": {
            "name": f"{device}-{level}-token",
            "namespace": NAMESPACE,
            "labels": labels,
            "annotations": {"kubernetes.io/service-account.name": f"{device}-{level}"},
        },
    }


def exec_credential(token: str) -> str:
    return json.dumps({"apiVersion": "client.authentication.k8s.io/v1", "kind": "ExecCredential", "status": {"token": token}}) + "\n"


def admin_token_path(layout: Layout, cluster: str) -> str:
    return f"{layout.secret_dir}/{cluster}-admin.json"


def render_kubeconfig(device: str, layout: Layout, clusters: dict[str, dict], ro_tokens: dict[str, str]) -> str:
    """The device's kubeconfig: inline read-only users, sudo-gated admin users."""
    users, contexts = [], []
    for cluster in clusters:
        users.append({"name": f"{cluster}-{device}-ro", "user": {"token": ro_tokens[cluster]}})
        users.append(
            {
                "name": f"{cluster}-{device}-admin",
                "user": {
                    "exec": {
                        "apiVersion": "client.authentication.k8s.io/v1",
                        "command": layout.sudo,
                        "args": [layout.cat, admin_token_path(layout, cluster)],
                        "interactiveMode": "Always",
                        "provideClusterInfo": False,
                    }
                },
            }
        )
        contexts.append({"name": cluster, "context": {"cluster": cluster, "user": f"{cluster}-{device}-ro"}})
        contexts.append({"name": f"{cluster}-admin", "context": {"cluster": cluster, "user": f"{cluster}-{device}-admin"}})
    doc = {
        "apiVersion": "v1",
        "kind": "Config",
        "preferences": {},
        "extensions": [{"name": MARKER, "extension": {"device": device}}],
        "current-context": next(iter(clusters)),
        "clusters": [{"name": name, "cluster": entry} for name, entry in clusters.items()],
        "users": users,
        "contexts": contexts,
    }
    return json.dumps(doc, indent=2) + "\n"


def install(args: argparse.Namespace) -> int:
    target = Target(args.ssh, args.ssh_port, args.user, args.secret_dir)
    admin = Admin(os.path.expanduser(args.admin_kubeconfig), args.admin_context)
    clusters: dict[str, dict] = {}
    ro_tokens: dict[str, str] = {}
    for cluster in args.clusters:
        admin.require_service_accounts(cluster, args.device)
        for level in ("ro", "admin"):
            if args.rotate:
                admin.recreate_token_secret(cluster, args.device, level)
            else:
                admin.ensure_token_secret(cluster, args.device, level)
        clusters[cluster] = admin.cluster_entry(cluster)
        ro_tokens[cluster] = admin.token(cluster, args.device, "ro")
        target.write_root_file(admin_token_path(target.layout, cluster), exec_credential(admin.token(cluster, args.device, "admin")))
        print(f"{cluster}: {args.device} tokens {'rotated and ' if args.rotate else ''}installed on {target.name}")
    moved = target.install_kubeconfig(render_kubeconfig(args.device, target.layout, clusters, ro_tokens))
    if moved:
        print(f"previous kubeconfig moved to {moved} (root only)")
    return report(target, args.clusters)


def check(args: argparse.Namespace) -> int:
    return report(Target(args.ssh, args.ssh_port, args.user, args.secret_dir), args.clusters)


# What an agent on the device must be able to do, and what it must not.
EXPECTED = (
    (["auth", "can-i", "list", "pods", "-A"], "yes"),
    (["auth", "can-i", "get", "secrets", "-A"], "no"),
    (["auth", "can-i", "create", "deployments", "-n", "default"], "no"),
    (["auth", "can-i", "delete", "pods", "-n", "default"], "no"),
)


def report(target: Target, clusters: list[str]) -> int:
    failures = 0
    readable = target.as_user(["cat", admin_token_path(target.layout, clusters[0])])
    failures += line("admin token unreadable without sudo", readable.returncode != 0)
    for cluster in clusters:
        for kubectl_args, want in EXPECTED:
            got = target.as_user(["kubectl", "--context", cluster, *kubectl_args]).stdout.strip().splitlines()
            failures += line(f"{cluster}: {' '.join(kubectl_args[2:])} -> {want}", (got[-1] if got else "") == want)
        refused = target.as_user(["kubectl", "--context", f"{cluster}-admin", "get", "namespaces"])
        failures += line(f"{cluster}-admin refused without a terminal", refused.returncode != 0)
    return 1 if failures else 0


def line(text: str, ok: bool) -> int:
    print(f"  {'ok  ' if ok else 'FAIL'} {text}")
    return 0 if ok else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("install", "check"):
        p = sub.add_parser(name)
        p.add_argument("--device", required=True, help="device name, as in workstation-access/device")
        p.add_argument("--clusters", type=lambda s: s.split(","), default=list(DEFAULT_CLUSTERS))
        p.add_argument("--ssh", help="user@host of a remote device; this machine when absent")
        p.add_argument("--ssh-port", type=int, default=22)
        p.add_argument("--user", help="device owner whose ~/.kube/config is written (required with --ssh)")
        p.add_argument("--secret-dir", help="root-only folder for admin tokens (default per OS)")
        if name == "install":
            p.add_argument("--rotate", action="store_true", help="replace the device's tokens; old ones stop working")
            p.add_argument("--admin-kubeconfig", default="~/.kube/config")
            p.add_argument("--admin-context", default="{cluster}-admin", help="admin context; {cluster} is replaced")
    args = parser.parse_args(argv)
    try:
        return install(args) if args.command == "install" else check(args)
    except Failure as failure:
        print(f"workstation-kube: {failure}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
