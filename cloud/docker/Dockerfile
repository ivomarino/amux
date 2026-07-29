FROM python:3.11-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    tmux curl git procps ca-certificates gnupg \
    && rm -rf /var/lib/apt/lists/*

# Node 22 — Debian's `nodejs` package is far too old for the Claude Code CLI.
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

RUN pip install --no-cache-dir requests

# Claude Code. This MUST run the postinstall: it downloads the platform-native
# binary. The previous build used `--ignore-scripts ... 2>/dev/null || true`,
# which skipped that download and swallowed the error, so every cloud container
# shipped a claude that failed at runtime with "native binary not installed" —
# and the smoke test never noticed, because it only asserted that a session
# produced *some* output, which a bare shell prompt satisfies.
RUN npm install -g @anthropic-ai/claude-code

# Fail the BUILD, not the customer's session, if the CLI is not actually runnable.
RUN claude --version && echo "claude CLI verified at build time"

WORKDIR /app
COPY amux-server.py .

ENV AMUX_PORT=8822
EXPOSE 8822

CMD ["python", "amux-server.py", "8822", "--no-tls"]
