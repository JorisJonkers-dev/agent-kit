#!/bin/sh
# Builds the Docker Sandboxes template images under images/sbx/<client>/ on this
# machine, as agent-kit/<client>:<version> and agent-kit/<client>:current. The
# version is the Dockerfile's `ARG VERSION=` line. Only docker build runs; no
# other network call is made here.
#
#   build-sbx-images.sh [--dry-run] [--client NAME]
#
#   --dry-run        print the docker build commands; build nothing, need no docker.
#   --client NAME    build one client (a directory under images/sbx/).
#
# Exit status: 0 built (or printed); 1 a build or a bad Dockerfile failed, or
# docker is missing; 2 bad usage.
set -eu

HERE=${0%/*}
case "$HERE" in
  "$0") HERE=. ;;
esac
ROOT=$(cd "$HERE/.." && pwd)
IMAGES=$ROOT/images/sbx

dry_run=0
only=
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry_run=1 ;;
    --client)
      if [ $# -lt 2 ]; then
        echo "build-sbx-images: --client needs a name" >&2
        exit 2
      fi
      only=$2
      shift
      ;;
    -h|--help)
      sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "build-sbx-images: unknown argument: $1" >&2
      exit 2
      ;;
  esac
  shift
done

if [ "$dry_run" = 0 ] && ! command -v docker >/dev/null 2>&1; then
  echo "build-sbx-images: docker is not on PATH; install Docker (or Docker Desktop) and start it, then re-run" >&2
  exit 1
fi

if [ -n "$only" ] && [ ! -f "$IMAGES/$only/Dockerfile" ]; then
  echo "build-sbx-images: no image for client '$only' (expected $IMAGES/$only/Dockerfile)" >&2
  exit 2
fi

built=0
for dockerfile in "$IMAGES"/*/Dockerfile; do
  [ -f "$dockerfile" ] || continue
  client=$(basename "$(dirname "$dockerfile")")
  if [ -n "$only" ] && [ "$client" != "$only" ]; then
    continue
  fi
  version=$(sed -n 's/^ARG VERSION=//p' "$dockerfile" | head -n 1)
  case "$version" in
    ''|*[!A-Za-z0-9._-]*)
      echo "build-sbx-images: $dockerfile has no usable 'ARG VERSION=' line" >&2
      exit 1
      ;;
  esac
  context=$(dirname "$dockerfile")
  if [ "$dry_run" = 1 ]; then
    echo "docker build -f $dockerfile -t agent-kit/$client:$version -t agent-kit/$client:current $context"
  else
    echo "build-sbx-images: building agent-kit/$client:$version" >&2
    docker build -f "$dockerfile" -t "agent-kit/$client:$version" -t "agent-kit/$client:current" "$context"
  fi
  built=$((built + 1))
done

if [ "$built" = 0 ]; then
  echo "build-sbx-images: no images/sbx/<client>/Dockerfile found${only:+ for $only}" >&2
  exit 1
fi
