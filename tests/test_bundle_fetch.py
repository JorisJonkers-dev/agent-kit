"""curl | bash fetches the bundle its checksum names, past any cache.

The asset host sits behind Cloudflare, which caches the tarball for hours but
not its .sha256. Asking for the tarball under a URL keyed by that checksum
gets the release the checksum describes, never a stale cached copy.
"""

from __future__ import annotations

import hashlib
import io
import os
import subprocess
import sys
import tarfile
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

URL = "https://assets.example/agent-kit-skills.tar.gz"

FAKE_CURL = r'''#!/usr/bin/env python3
import os, shutil, sys
args = sys.argv[1:]
out = args[args.index("-o") + 1]
url = args[-1]
open(os.environ["CURL_LOG"], "a").write(url + "\n")
served = os.environ["SERVED"]
name = {"URL": "cached.tgz", "URL.sha256": "new.sha256", "URL?sha256=" + os.environ["NEW_SHA"]: "new.tgz"}
key = url.replace(os.environ["BASE"], "URL")
if key not in name:
    sys.exit(22)
shutil.copy(os.path.join(served, name[key]), out)
'''


def _bundle(marker: str) -> bytes:
    script = f"#!/bin/sh\necho {marker}\n".encode()
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        info = tarfile.TarInfo("installer/setup-workstation.sh")
        info.size = len(script)
        info.mode = 0o755
        tar.addfile(info, io.BytesIO(script))
    return buf.getvalue()


def _fn(script: str, name: str) -> str:
    return f"{name}() {{" + script.split(f"\n{name}() {{")[1].split("\n}\n")[0] + "\n}\n"


def test_a_stale_cached_tarball_is_never_what_runs(tmp_path: Path) -> None:
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()),
    )
    served = tmp_path / "served"
    served.mkdir()
    new = _bundle("NEW-BUNDLE")
    (served / "cached.tgz").write_bytes(_bundle("STALE-BUNDLE"))
    (served / "new.tgz").write_bytes(new)
    new_sha = hashlib.sha256(new).hexdigest()
    (served / "new.sha256").write_text(f"{new_sha}  agent-kit-skills.tar.gz\n")
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "curl").write_text(FAKE_CURL.replace("/usr/bin/env python3", sys.executable))
    (bin_dir / "curl").chmod(0o755)
    log = tmp_path / "curl.log"
    body = (
        f"KIT_ROOT={tmp_path}/not-a-checkout\nORIG_ARGS=()\nfailures=0\nwarnings=0\n"
        "todo_failures=()\ntodo_warnings=()\n"
        "log() { printf 'setup: %s\\n' \"$*\"; }\nok() { log \"$*\"; }\n"
        "fail() { printf 'FAIL %s\\n' \"$*\" >&2; }\n"
        + _fn(script, "use_kit_bundle") + "use_kit_bundle\n"
    )
    env = {
        **os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}", "CURL_LOG": str(log), "SERVED": str(served),
        "NEW_SHA": new_sha, "BASE": URL, "AGENT_KIT_SKILLS_BUNDLE_URL": URL,
    }
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env,
                            stdin=subprocess.DEVNULL)
    assert "NEW-BUNDLE" in result.stdout, result.stdout + result.stderr
    fetched = log.read_text().splitlines()
    assert fetched == [f"{URL}.sha256", f"{URL}?sha256={new_sha}"]
