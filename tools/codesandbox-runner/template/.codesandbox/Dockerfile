# Symphony cloud agent template (built with `pnpm --filter @symphony/codesandbox-runner build:template`).
#
# It carries the three agents a Symphony task can run in a CodeSandbox sandbox:
#   - mini-swe-agent (default), with Symphony's checkpointing / commit-after-every-step agent class
#   - the Pi coding agent
#   - Pi Durable (crash-resumable Pi harness)
# plus the drivers in /opt/symphony/drivers that clone the task branch, run the chosen agent and
# commit + push after every edit.
#
# Versions are pinned; bump them deliberately and rebuild the template.
FROM node:24-bookworm

# zsh is CodeSandbox's default shell; the rest is what the agents and the drivers use.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      zsh bash git curl ca-certificates python3 python3-venv procps ripgrep jq \
 && rm -rf /var/lib/apt/lists/*

# mini-swe-agent, isolated in its own virtualenv.
ARG MINI_SWE_AGENT_VERSION=2.4.6
RUN python3 -m venv /opt/mini \
 && /opt/mini/bin/pip install --no-cache-dir "mini-swe-agent==${MINI_SWE_AGENT_VERSION}" \
 && ln -s /opt/mini/bin/mini /usr/local/bin/mini

# Pi coding agent (CLI).
ARG PI_VERSION=1.4.0
RUN npm install --global --ignore-scripts "@earendil-works/pi-coding-agent@${PI_VERSION}"

# Drivers, and the Pi Durable packages they import.
COPY drivers/ /opt/symphony/drivers/
RUN cd /opt/symphony/drivers \
 && npm install --omit=dev --ignore-scripts --no-audit --no-fund \
 && chmod +x /opt/symphony/drivers/*.sh

# mini-swe-agent asks for a model on first start unless it is told it is configured.
ENV MSWEA_CONFIGURED=true \
    MSWEA_SILENT_STARTUP=1 \
    PI_OFFLINE=1
