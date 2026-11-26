# Template image for the Pi worker. The gateway starts one fresh Docker Sandboxes
# microVM from it per run and runs Pi with `sbx exec`; no process starts here.
# Build: scripts/build-sbx-images.sh --client pi
# The pins must match installer/setup-workstation.sh (scripts/crates/ci-helpers/tests/sbx_images.rs checks).

ARG NODE_IMAGE=node:24-bookworm-slim
# The client's own version: names the image tag agent-kit/pi:<VERSION>.
ARG VERSION=1.5.0
ARG PI_ACP_VERSION=0.0.34

FROM ${NODE_IMAGE}

ARG VERSION
ARG PI_ACP_VERSION

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends git ca-certificates curl ripgrep \
 && rm -rf /var/lib/apt/lists/*

# Scripts off, as in setup: Pi needs none to run.
RUN npm install -g --ignore-scripts \
      "@earendil-works/pi-coding-agent@${VERSION}" \
      "pi-acp@${PI_ACP_VERSION}" \
 && npm cache clean --force

RUN useradd --create-home --home-dir /home/agent --shell /bin/bash agent
ENV HOME=/home/agent
USER agent
WORKDIR /home/agent

CMD ["sleep", "infinity"]
