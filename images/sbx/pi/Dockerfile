# Template image for the Pi worker. The gateway starts one fresh Docker Sandboxes
# microVM from it per run and runs Pi with `sbx exec`; no process starts here.
# Build: scripts/build-sbx-images.sh --client pi
# The pins must match installer/setup-workstation.sh (scripts/crates/ci-helpers/tests/sbx_images.rs checks).

ARG NODE_IMAGE=node:24.22.3-bookworm-slim
# The client's own version: names the image tag agent-kit/pi:<VERSION>.
ARG VERSION=1.5.0
ARG PI_ACP_VERSION=0.0.34

FROM ${NODE_IMAGE}

ARG VERSION
ARG PI_ACP_VERSION

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

# Scripts off, as in setup: Pi needs none to run.
RUN --mount=type=secret,id=packages-npmrc,target=/run/secrets/packages-npmrc,required=false \
    if [ -s /run/secrets/packages-npmrc ]; then export NPM_CONFIG_USERCONFIG=/run/secrets/packages-npmrc; fi \
 && npm install -g --ignore-scripts \
      "@earendil-works/pi-coding-agent@${VERSION}" \
      "pi-acp@${PI_ACP_VERSION}" \
 && npm cache clean --force

RUN useradd --create-home --home-dir /home/agent --shell /bin/bash agent
ENV HOME=/home/agent
USER agent
WORKDIR /home/agent

CMD ["sleep", "infinity"]
