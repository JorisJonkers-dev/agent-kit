# Template image for the OpenCode worker. The gateway starts one fresh Docker
# Sandboxes microVM from it per run and runs OpenCode with `sbx exec`; no process
# starts here.
# Build: scripts/build-sbx-images.sh --client opencode

ARG NODE_IMAGE=node:24-bookworm-slim
# The client's own version: names the image tag agent-kit/opencode:<VERSION>.
# npm `opencode-ai` is OpenCode 1.x (1.19.6 is its newest release); the host's
# Homebrew OpenCode 2 is not published to npm.
ARG VERSION=1.19.6

FROM ${NODE_IMAGE}

ARG VERSION

ENV DEBIAN_FRONTEND=noninteractive
# Packages come through the estate package proxy when the packages-netrc secret is
# given (scripts/build-sbx-images.sh, AGENT_KIT_PACKAGES_NETRC); otherwise the
# default Debian mirrors. The base image has no CA bundle and its sources are
# plain http, so ca-certificates is installed first over the default mirror, then
# the sources move to the proxy's https URLs and the package list is installed.
RUN --mount=type=secret,id=packages-netrc,target=/etc/apt/auth.conf.d/packages.conf,required=false \
    apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && if [ -s /etc/apt/auth.conf.d/packages.conf ]; then \
      sed -i -e 's|http://deb.debian.org/debian-security|https://packages.jorisjonkers.dev/debian/security|' \
             -e 's|http://deb.debian.org/debian|https://packages.jorisjonkers.dev/debian|' \
             /etc/apt/sources.list.d/debian.sources \
      && apt-get update; \
    fi \
 && apt-get install -y --no-install-recommends git curl ripgrep \
 && rm -rf /var/lib/apt/lists/*

# Install scripts stay on: opencode-ai's postinstall copies the platform binary
# from its optional dependency into bin/opencode.exe. Without it the bin is a stub
# that refuses to run.
RUN --mount=type=secret,id=packages-npmrc,target=/run/secrets/packages-npmrc,required=false \
    if [ -s /run/secrets/packages-npmrc ]; then export NPM_CONFIG_USERCONFIG=/run/secrets/packages-npmrc; fi \
 && npm install -g "opencode-ai@${VERSION}" \
 && npm cache clean --force

RUN useradd --create-home --home-dir /home/agent --shell /bin/bash agent
ENV HOME=/home/agent
USER agent
WORKDIR /home/agent

CMD ["sleep", "infinity"]
