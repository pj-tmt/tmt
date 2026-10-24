# The ordinary E2E image stays unchanged. Provider acquisition is an independent
# pinned stage and its payload is copied only into this advisory derived image.
ARG E2E_BASE=tmt-real-harness-base
FROM node:26.12.1-bookworm-slim@sha256:28147155d47dc31abe0a46e0d12b59b2fb3d0a2dcc5dc65fce76a074180b16fe AS providers
COPY typescript/test/e2e/real-harness-pins.json /pins.json
COPY typescript/scripts/acquire-real-harness.mjs /acquire.mjs
RUN node /acquire.mjs /pins.json /opt/providers
# The verified matching platform payload is already installed: the wrapper may
# not download a replacement during its own installer.
RUN --network=none node /opt/providers/node_modules/@anthropic-ai/claude-code/install.cjs \
  && chmod -R a+rX /opt/providers

FROM ${E2E_BASE}
COPY --from=providers /opt/providers /opt/real-harness
RUN mkdir -p /opt/real-harness/bin \
  && ln -s /opt/real-harness/node_modules/@anthropic-ai/claude-code/bin/claude.exe /opt/real-harness/bin/claude \
  && ln -s /opt/real-harness/node_modules/@openai/codex/bin/codex.js /opt/real-harness/bin/codex \
  && cp -a /root/.cache/node/corepack /opt/corepack \
  && chmod -R a+rX /opt/corepack \
  && mkdir -p /workspace/typescript/node_modules/.vite-temp \
  && chown node:node /workspace/typescript/node_modules/.vite-temp
ENV PATH=/opt/real-harness/bin:/usr/local/bin:/usr/bin:/bin \
  HOME=/tmp/real-harness-home \
  COREPACK_HOME=/opt/corepack COREPACK_ENABLE_NETWORK=0 \
  TMT_AGENT=1 BROWSER=/bin/true TMT_REAL_HARNESS=1
USER node
CMD ["pnpm", "--dir", "/workspace/typescript", "exec", "vp", "test", "run", "--config", "test/e2e/real-harness.config.ts"]
