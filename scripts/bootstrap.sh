#!/bin/sh
# Bootstrap for a machine with no Python, Node or cargo (#807, part of #802):
# downloads this release's prebuilt static tools, verifies their sha256,
# installs them and runs setup (the release's setup-workstation.sh). POSIX sh; needs curl (or wget), tar, and
# sha256sum or shasum.
#
#   sh bootstrap.sh [setup-workstation.sh options]
#
#   AGENT_KIT_TAG           the release (default: the one stamped in, else the latest)
#   AGENT_KIT_RELEASE_BASE  where the assets are (default: the GitHub release)
#   AGENT_KIT_BIN_DIR       where the tools go (default ~/.local/bin)
#   AGENT_KIT_SKIP_SETUP=1  install the tools and stop
#
# The release ships this file with the tag and the SHA256SUMS lines stamped in
# below (scripts/release-binaries.sh stamp), so the tarball is checked against a
# value that came with the script. The copy in the repository is not stamped: it
# fetches SHA256SUMS from the release instead, which only the transport secures.
# docs/RUST.md, "Bootstrap".
set -eu

# --- stamped by release-binaries.sh stamp ---
STAMPED_TAG=''
STAMPED_SUMS=''
# --- end stamped ---

REPO=JorisJonkers-dev/agent-kit
die() { printf 'bootstrap: %s\n' "$*" >&2; exit 1; }
say() { printf 'bootstrap: %s\n' "$*"; }

case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) target=x86_64-unknown-linux-musl ;;
  Linux/aarch64 | Linux/arm64) target=aarch64-unknown-linux-musl ;;
  Darwin/arm64) target=aarch64-apple-darwin ;;
  *) die "no prebuilt tools for $(uname -s)/$(uname -m); build from a checkout with cargo" ;;
esac

if command -v sha256sum >/dev/null 2>&1; then sha_check() { sha256sum -c "$@"; }
elif command -v shasum >/dev/null 2>&1; then sha_check() { shasum -a 256 -c "$@"; }
else die "neither sha256sum nor shasum is installed"; fi

if command -v curl >/dev/null 2>&1; then fetch() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then fetch() { wget -q -O "$2" "$1"; }
else die "neither curl nor wget is installed"; fi

tag=${AGENT_KIT_TAG:-$STAMPED_TAG}
if [ -z "$tag" ]; then
  [ -z "${AGENT_KIT_RELEASE_BASE:-}" ] || die "set AGENT_KIT_TAG with AGENT_KIT_RELEASE_BASE"
  command -v curl >/dev/null 2>&1 || die "set AGENT_KIT_TAG; resolving the latest release needs curl"
  tag=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") || die "could not resolve the latest release"
  tag=${tag##*/}
fi
case $tag in "" | */* | *[!A-Za-z0-9._-]*) die "not a release tag: $tag" ;; esac
base=${AGENT_KIT_RELEASE_BASE:-https://github.com/$REPO/releases/download/$tag}
name=agent-kit-tools-$tag-$target
bin_dir=${AGENT_KIT_BIN_DIR:-$HOME/.local/bin}
data_dir=${XDG_DATA_HOME:-$HOME/.local/share}/agent-kit
conf_dir=${XDG_CONFIG_HOME:-$HOME/.config}/agent-kit

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM
say "fetching $name.tar.gz"
fetch "$base/$name.tar.gz" "$work/$name.tar.gz" || die "could not download $base/$name.tar.gz"
if [ -n "$STAMPED_SUMS" ] && [ "$tag" = "$STAMPED_TAG" ]; then
  printf '%s\n' "$STAMPED_SUMS" > "$work/SHA256SUMS"
else
  say "no stamped checksums for $tag; using the release's SHA256SUMS"
  fetch "$base/SHA256SUMS" "$work/SHA256SUMS" || die "could not download $base/SHA256SUMS"
fi
grep " $name.tar.gz\$" "$work/SHA256SUMS" > "$work/mine" || die "SHA256SUMS has no line for $name.tar.gz"
(cd "$work" && sha_check mine >/dev/null) || die "$name.tar.gz does not match its sha256"
say "sha256 verified"

tar -C "$work" -xzf "$work/$name.tar.gz"
kit=$work/$name
[ "$(sed -n 1p "$kit/VERSION")" = "$tag" ] || die "the tarball is not release $tag"
(cd "$kit" && sha_check SHA256SUMS >/dev/null) || die "the tarball's own SHA256SUMS does not match"

# One tool into place, atomically; the pin files setup's installers honour keep
# the node runners for critique and model-gateway.
install_one() { # tool dest_dir pin
  [ -z "${3:-}" ] || [ ! -e "$3" ] || { say "skipped $1, pinned by $3"; return 0; }
  mkdir -p "$2"
  tmp=$(mktemp "$2/.$1.XXXXXX")
  if cp "$kit/bin/$1" "$tmp" && chmod 0755 "$tmp" && mv -f "$tmp" "$2/$1"; then :; else rm -f "$tmp"; die "could not install $2/$1"; fi
  say "installed $2/$1"
}
for path in "$kit"/bin/*; do
  tool=${path##*/}
  case $tool in
    critique) install_one "$tool" "$data_dir/critique" "$conf_dir/critique-node" ;;
    model-gateway) install_one "$tool" "$data_dir/model-gateway" "$conf_dir/model-gateway-node" ;;
    build-tracker) install_one "$tool" "$bin_dir" "$conf_dir/build-tracker-node" ;;
    *) install_one "$tool" "$bin_dir" "" ;;
  esac
done

if [ "${AGENT_KIT_SKIP_SETUP:-}" = 1 ]; then say "tools installed; setup skipped"; exit 0; fi
command -v bash >/dev/null 2>&1 || die "setup needs bash"
say "running setup"
fetch "$base/setup-workstation.sh" "$work/setup-workstation.sh" || die "could not download $base/setup-workstation.sh"
PATH=$bin_dir:$PATH
export PATH
# Not exec, so the temp directory is removed afterwards.
rc=0
bash "$work/setup-workstation.sh" "$@" || rc=$?
exit "$rc"
