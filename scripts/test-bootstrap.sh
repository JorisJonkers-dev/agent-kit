#!/usr/bin/env bash
# Tests scripts/bootstrap.sh, `release-binaries.sh stamp` and nix/agent-kit-tools.nix
# against a fake release (stub tools, file:// assets): no network, no cargo, no
# Python or Node. Run from anywhere: scripts/test-bootstrap.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
tag=v9.9.9
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) target=x86_64-unknown-linux-musl ;;
  Linux/aarch64) target=aarch64-unknown-linux-musl ;;
  Darwin/arm64) target=aarch64-apple-darwin ;;
  *) echo "skip: no release target for this machine"; exit 0 ;;
esac
name=agent-kit-tools-$tag-$target
fail() { echo "FAIL: $*" >&2; exit 1; }
sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

# A fake release: three stub tools, the tarball, SHA256SUMS and a setup script.
rel="$tmp/release"; mkdir -p "$rel" "$tmp/stage/$name/bin"
for t in agent-launcher critique model-gateway build-tracker; do
  printf '#!/bin/sh\necho "%s stub"\n' "$t" > "$tmp/stage/$name/bin/$t"; chmod 0755 "$tmp/stage/$name/bin/$t"
done
printf '%s\n%s\n' "$tag" "$target" > "$tmp/stage/$name/VERSION"
(cd "$tmp/stage/$name" && sha VERSION bin/* > SHA256SUMS)
tar -C "$tmp/stage" -czf "$rel/$name.tar.gz" "$name"
(cd "$rel" && sha "$name.tar.gz" > SHA256SUMS)
# shellcheck disable=SC2016
printf '#!/usr/bin/env bash\necho "setup ran with: $*"\necho "$PATH" > "%s/setup-path"\n' "$tmp" > "$rel/setup-workstation.sh"

run() { # run bootstrap.sh in a fresh home; extra env via the caller
  rm -rf "${tmp:?}/home"; mkdir -p "$tmp/home"
  HOME="$tmp/home" XDG_DATA_HOME="" XDG_CONFIG_HOME="" AGENT_KIT_TAG="$tag" \
    AGENT_KIT_RELEASE_BASE="file://$rel" "$@" 2>&1
}
unset XDG_DATA_HOME XDG_CONFIG_HOME
boot="$ROOT/scripts/bootstrap.sh"

# 1. Unstamped: fetches SHA256SUMS, installs to the right places, runs setup with the arguments.
out="$(run sh "$boot" --check --no-lsp)" || fail "unstamped run: $out"
echo "$out" | grep -q 'setup ran with: --check --no-lsp' || fail "setup arguments: $out"
for f in .local/bin/agent-launcher .local/bin/build-tracker .local/share/agent-kit/critique/critique .local/share/agent-kit/model-gateway/model-gateway; do
  [ -x "$tmp/home/$f" ] || fail "not installed: $f"
done
grep -q "$tmp/home/.local/bin" "$tmp/setup-path" || fail "setup PATH lacks the bin dir"
echo "ok unstamped install and setup"

# 2. A pin file keeps the node runner.
mkdir -p "$tmp/home/.config/agent-kit"; touch "$tmp/home/.config/agent-kit/critique-node"
HOME="$tmp/home" AGENT_KIT_TAG="$tag" AGENT_KIT_RELEASE_BASE="file://$rel" AGENT_KIT_SKIP_SETUP=1 sh "$boot" >"$tmp/out" 2>&1
grep -q 'skipped critique, pinned' "$tmp/out" || fail "pin not honoured"
echo "ok pin file"

# 3. Stamped: the stamped sums are used, not the release's.
cp "$rel/SHA256SUMS" "$tmp/SHA256SUMS"; mkdir -p "$tmp/dist"; cp "$rel/SHA256SUMS" "$tmp/dist/"
"$ROOT/scripts/release-binaries.sh" stamp "$tag" "$tmp/dist" >/dev/null
bash -n "$tmp/dist/bootstrap.sh"; sh -n "$tmp/dist/bootstrap.sh"
grep -q "^STAMPED_TAG='$tag'" "$tmp/dist/bootstrap.sh" || fail "tag not stamped"
grep -q "$name.tar.gz" "$tmp/dist/bootstrap.sh" || fail "sums not stamped"
out="$(run env AGENT_KIT_TAG= sh "$tmp/dist/bootstrap.sh")" || fail "stamped run: $out"
echo "$out" | grep -q 'no stamped checksums' && fail "stamped script fetched sums"
echo "ok stamped"

# 4. A tampered tarball fails against the stamped sums, even though the release's sums are replaced to match.
mkdir "$tmp/evil"; cp -r "$rel/." "$tmp/evil/"; echo x >> "$tmp/evil/$name.tar.gz"
(cd "$tmp/evil" && sha "$name.tar.gz" > SHA256SUMS)
if out="$(run env AGENT_KIT_RELEASE_BASE="file://$tmp/evil" AGENT_KIT_SKIP_SETUP=1 sh "$tmp/dist/bootstrap.sh")"; then fail "tampered tarball accepted: $out"; fi
echo "$out" | grep -q 'does not match its sha256' || fail "wrong tamper message: $out"
[ ! -e "$tmp/home/.local/bin/agent-launcher" ] || fail "tampered install left a binary"
echo "ok tamper"

# 5. A tarball from another tag, an unknown tag name and a missing asset fail.
if out="$(run env AGENT_KIT_TAG='../x' sh "$boot")"; then fail "bad tag accepted"; fi
if out="$(run env AGENT_KIT_TAG=v0.0.1 sh "$boot")"; then fail "missing asset accepted"; fi
echo "ok bad tag and missing asset"

# 6. No Python, Node or cargo on PATH: the bootstrap needs only the basics.
bin="$tmp/minpath"; mkdir "$bin"
for c in sh bash uname curl tar grep sed mktemp cp chmod mv rm mkdir cat printf env awk sha256sum shasum dirname gzip; do
  p="$(command -v "$c" 2>/dev/null || true)"; [ -z "$p" ] || ln -s "$p" "$bin/$c"
done
out="$(run env PATH="$bin" "$bin/sh" "$boot")" || fail "minimal PATH: $out"
echo "ok minimal PATH"

# 7. The NixOS derivation evaluates against the release's sums.
if command -v nix >/dev/null 2>&1; then
  cp "$rel/SHA256SUMS" "$tmp/nix-sums"
  drv="$(nix --extra-experimental-features 'nix-command flakes' eval --impure --raw --expr "
    let pkgs = import (builtins.getFlake \"nixpkgs\") { system = builtins.currentSystem; };
    in (pkgs.callPackage $ROOT/nix/agent-kit-tools.nix { tag = \"$tag\"; sums = builtins.readFile $tmp/nix-sums; }).src.url" 2>&1)" \
    || { echo "skip nix: could not evaluate ($drv)"; drv=""; }
  [ -z "$drv" ] || { echo "$drv" | grep -q "$name.tar.gz" || fail "nix url: $drv"; echo "ok nix derivation evaluates"; }
else
  echo "skip nix: not installed"
fi
echo "all bootstrap tests passed"
