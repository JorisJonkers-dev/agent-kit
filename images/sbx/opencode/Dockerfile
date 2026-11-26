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
RUN apt-get update \
 && apt-get install -y --no-install-recommends git ca-certificates curl ripgrep \
 && rm -rf /var/lib/apt/lists/*

# Install scripts stay on: opencode-ai's postinstall copies the platform binary
# from its optional dependency into bin/opencode.exe. Without it the bin is a stub
# that refuses to run.
RUN npm install -g "opencode-ai@${VERSION}" \
 && npm cache clean --force

RUN useradd --create-home --home-dir /home/agent --shell /bin/bash agent
ENV HOME=/home/agent
USER agent
WORKDIR /home/agent

CMD ["sleep", "infinity"]
