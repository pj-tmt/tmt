# Local emulators only; never mount host Firebase credentials into this image.
FROM eclipse-temurin:21-jre-jammy@sha256:54b382c9d790d3201004d4e3921f4eff7dc7d67e1ce21a2da8d8faaaf63c85fc AS java
FROM node:22.23.2-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5 AS emulator-tools
COPY --from=java /opt/java/openjdk /opt/java/openjdk
ENV JAVA_HOME=/opt/java/openjdk
ENV PATH="/opt/java/openjdk/bin:${PATH}"
RUN npm install --global firebase-tools@15.29.0
RUN mkdir -p /home/node/office && chown node:node /home/node/office
USER node
WORKDIR /home/node/office
RUN firebase setup:emulators:firestore --non-interactive
ENV GCLOUD_PROJECT=demo-tmt-office

FROM emulator-tools AS emulators
COPY --chown=node:node extensions/tmt-office/typescript/services/office/firebase.json extensions/tmt-office/typescript/services/office/firestore.rules ./
COPY --chown=node:node typescript/scripts/verify-office-emulators.mjs ./
CMD ["firebase", "emulators:start", "--only", "auth,firestore", "--project", "demo-tmt-office", "--config", "firebase.json", "--non-interactive"]

# Opt-in browser proof reuses the emulator owner; no host credentials or volumes.
FROM emulator-tools AS browser-tests-base
USER root
RUN apt-get update && apt-get install -y --no-install-recommends dbus git gnome-keyring libglib2.0-bin libsecret-tools sqlite3 tmux && rm -rf /var/lib/apt/lists/*
RUN npm install --global pnpm@10.33.0 && mkdir -p /workspace/typescript /workspace/extensions/tmt-office/typescript/apps/office /workspace/extensions/tmt-office/typescript/services/office/functions && chown -R node:node /workspace
WORKDIR /workspace/typescript
COPY --chown=node:node typescript/package.json typescript/pnpm-lock.yaml typescript/pnpm-workspace.yaml ./
COPY --chown=node:node extensions/tmt-office/typescript/apps/office/package.json /workspace/extensions/tmt-office/typescript/apps/office/package.json
COPY --chown=node:node extensions/tmt-office/typescript/services/office/functions/package.json /workspace/extensions/tmt-office/typescript/services/office/functions/package.json
USER node
RUN pnpm office:install && test ! -e node_modules/better-sqlite3 && test ! -e /node_modules
RUN cd /workspace/extensions/tmt-office/typescript/services/office/functions && pnpm install --ignore-workspace --lockfile-dir "$PWD/../../../../../../typescript" --frozen-lockfile
# Combined native/browser acceptance uses root-owned process/archive helpers.
# sqlite3 supplies the independent hook oracle; no native Node addon is built.
RUN pnpm --filter tmux-team install --frozen-lockfile --ignore-scripts
ENV PLAYWRIGHT_BROWSERS_PATH=/opt/playwright-browsers
USER root
RUN pnpm --filter @tmt/office exec playwright install --with-deps chromium
USER node
COPY --from=emulators --chown=node:node /home/node/office/ /home/node/office/
COPY --chown=node:node typescript/scripts/lint-config.mjs typescript/scripts/lint-config.d.mts /workspace/typescript/scripts/
COPY --chown=node:node extensions/tmt-office/typescript/apps/office /workspace/extensions/tmt-office/typescript/apps/office
COPY --chown=node:node extensions/tmt-office/contracts /workspace/extensions/tmt-office/contracts
COPY --chown=node:node typescript/test/e2e test/e2e
COPY --chown=node:node typescript/test/support test/support
# test/support/native-artifact.ts reads the archive inventory from its one owner (#1163).
COPY --chown=node:node typescript/scripts/native-artifact-policy.mjs scripts/native-artifact-policy.mjs
COPY --chown=node:node extensions/tmt-office/typescript/services/office/functions /workspace/extensions/tmt-office/typescript/services/office/functions
COPY --chown=node:node extensions/tmt-office/typescript/services/office/firebase.json extensions/tmt-office/typescript/services/office/firestore.rules /workspace/extensions/tmt-office/typescript/services/office/
COPY --chown=node:node extensions/tmt-office/typescript/services/office/with-test-keyring.sh /workspace/extensions/tmt-office/typescript/services/office/with-test-keyring.sh
RUN --network=none pnpm --filter @tmt/office-service check \
  && pnpm --filter @tmt/office-service test && pnpm --filter @tmt/office-service build
RUN --network=none pnpm office:check && pnpm office:test \
  && pnpm --filter @tmt/office type:check:e2e \
  && pnpm --filter @tmt/office test:browser:partitions \
  && pnpm office:build:local \
  && pnpm --filter @tmt/office exec vp build --outDir dist-preview \
  && pnpm --filter @tmt/office exec vp build --mode emulator \
  && pnpm --filter @tmt/office exec vp build --mode cloud --outDir dist-cloud

FROM rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073 AS native-office
WORKDIR /workspace
COPY rust rust
COPY extensions/tmt-office/rust extensions/tmt-office/rust
COPY skills skills
COPY extensions/tmt-office/skills extensions/tmt-office/skills
COPY contracts contracts
COPY extensions/tmt-office/contracts extensions/tmt-office/contracts
# Workspace member: Cargo loads every member's manifest for --workspace builds.
COPY extensions/tmt-squad extensions/tmt-squad
COPY extensions/tmt-remote/rust extensions/tmt-remote/rust
COPY extensions/tmt-colab/rust extensions/tmt-colab/rust
COPY extensions/tmt-colab/contracts extensions/tmt-colab/contracts
COPY extensions/tmt-colab/skills extensions/tmt-colab/skills
COPY --from=browser-tests-base /workspace/target/office-spa /workspace/target/office-spa
ENV TMT_OFFICE_SPA_DIR=/workspace/target/office-spa
RUN cd rust && cargo +1.97.0 build --locked --workspace --release --features tmt-office/local-service \
  && target/release/tmt-office __tmt-office-service 1 asset-probe \
  && cargo +1.97.0 build --locked --release -p tmt-adapters --example runtime-caller-fixture

FROM browser-tests-base AS browser-tests
COPY --from=native-office --chown=node:node /workspace/rust/target/release/tmt /workspace/rust/target/debug/tmt
COPY --from=native-office --chown=node:node /workspace/rust/target/release/tmt-office /workspace/rust/target/debug/tmt-office
# test/support/cli-process launches native scenarios through this neutral caller fixture.
COPY --from=native-office --chown=node:node /workspace/rust/target/release/examples/runtime-caller-fixture /workspace/rust/target/debug/examples/runtime-caller-fixture
CMD ["sh", "/workspace/extensions/tmt-office/typescript/services/office/with-test-keyring.sh", "firebase", "emulators:exec", "--only", "auth,firestore,functions", "--project", "demo-tmt-office", "--config", "/workspace/extensions/tmt-office/typescript/services/office/firebase.json", "--non-interactive", "pnpm --filter @tmt/office test:browser"]

# Keep the existing default build and compose behavior lightweight.
FROM emulators AS default
