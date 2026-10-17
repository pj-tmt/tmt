# Local native-architecture artifact proof. Not a publishing workflow.
FROM node:22.23.2-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5 AS node-tooling

FROM rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073 AS build
RUN apt-get update && apt-get install --no-install-recommends -y musl-tools \
  && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-dist --version 0.32.0 --locked \
  && cargo install cargo-about --version 0.9.2 --locked --features cli
COPY --from=node-tooling /usr/local/bin/node /usr/local/bin/node
ARG TARGET_TRIPLE
ARG PRODUCT=cli
RUN test -n "$TARGET_TRIPLE" && rustup target add "$TARGET_TRIPLE"
WORKDIR /workspace
COPY rust/ rust/
COPY drivers/herdr/rust/ drivers/herdr/rust/
COPY skills/ skills/
# Workspace member: Cargo must load its manifest even when not building it.
COPY extensions/tmt-ops/ extensions/tmt-ops/
COPY extensions/tmt-remote/rust/ extensions/tmt-remote/rust/
COPY extensions/tmt-digest/rust/ extensions/tmt-digest/rust/
COPY extensions/tmt-digest/contracts/ extensions/tmt-digest/contracts/
COPY extensions/tmt-colab/rust/ extensions/tmt-colab/rust/
COPY extensions/tmt-colab/contracts/ extensions/tmt-colab/contracts/
COPY extensions/tmt-colab/firestore/ extensions/tmt-colab/firestore/
COPY extensions/tmt-digest/skills/ extensions/tmt-digest/skills/
COPY extensions/tmt-colab/skills/ extensions/tmt-colab/skills/
COPY design/tokens/tokens.json design/tokens/tokens.json
COPY design/browser-ui/generated/static.css design/browser-ui/generated/static.css
COPY extensions/tmt-colab/typescript/app/src/colab-header.css extensions/tmt-colab/typescript/app/src/colab-header.css
COPY extensions/tmt-colab/typescript/app/src/reader-style.css extensions/tmt-colab/typescript/app/src/reader-style.css
COPY extensions/tmt-colab/typescript/app/src/notice-card.css extensions/tmt-colab/typescript/app/src/notice-card.css
COPY scripts/native-cargo.sh scripts/build-native-artifact.sh scripts/
COPY dist-workspace.toml LICENSE ./
COPY typescript/package.json typescript/pnpm-lock.yaml typescript/pnpm-workspace.yaml typescript/
COPY design/browser-ui/package.json design/browser-ui/package.json
COPY contracts/ contracts/
COPY .github/components.json .github/components.json
COPY typescript/test/e2e/shard-weights.json typescript/test/e2e/shard-weights.json
COPY typescript/scripts/release-declaration.mjs typescript/scripts/release-cut-read.mjs typescript/scripts/release-upgrade.mjs typescript/scripts/ci-scope.mjs typescript/scripts/native-release-policy.mjs typescript/scripts/release-versions.mjs typescript/scripts/release-draft-assets.mjs typescript/scripts/release-source-at-ref.mjs typescript/scripts/packed-command.mjs typescript/scripts/cargo-workspace.mjs typescript/scripts/e2e-shards.mjs typescript/scripts/component-skills.mjs typescript/scripts/native-artifact-policy.mjs typescript/scripts/plan-release-builds.mjs typescript/scripts/release-read-retry.mjs typescript/scripts/release-index.mjs typescript/scripts/
RUN cd rust && cargo fetch --locked
# A standalone extension proof consumes the same frozen snapshot as the release pipeline.
RUN --mount=type=secret,id=release-version-state \
    case "$PRODUCT" in \
      ops|digest|remote|colab) \
        test -s /run/secrets/release-version-state && \
        node typescript/scripts/release-declaration.mjs generate /workspace /run/secrets/release-version-state "$PRODUCT" && \
        TMT_RELEASE_VERSION_STATE=/run/secrets/release-version-state \
          scripts/build-native-artifact.sh "$TARGET_TRIPLE" "$PRODUCT" > native-manifest.json && \
        cp /run/secrets/release-version-state target/distrib/release-version-state.json ;; \
      *) scripts/build-native-artifact.sh "$TARGET_TRIPLE" "$PRODUCT" > native-manifest.json ;; \
    esac

FROM node-tooling AS verification
RUN apt-get update && apt-get install --no-install-recommends -y binutils \
  && rm -rf /var/lib/apt/lists/*
RUN npm install --global pnpm@10.33.0
WORKDIR /verification
COPY typescript/package.json typescript/pnpm-lock.yaml typescript/pnpm-workspace.yaml typescript/
COPY design/browser-ui/package.json design/browser-ui/package.json
RUN cd typescript && pnpm --filter tmt install --frozen-lockfile --ignore-scripts
COPY .github/components.json .github/components.json
COPY typescript/test/e2e/shard-weights.json typescript/test/e2e/shard-weights.json
COPY typescript/scripts/ci-scope.mjs typescript/scripts/native-release-policy.mjs typescript/scripts/e2e-shards.mjs typescript/scripts/
COPY typescript/scripts/component-skills.mjs typescript/scripts/native-artifact-policy.mjs typescript/scripts/verify-native-artifact.mjs typescript/scripts/verify-native-installation.mjs typescript/scripts/packed-command.mjs typescript/scripts/
COPY typescript/scripts/migrated-state.mjs typescript/scripts/native-upgrade-proof.mjs typescript/scripts/native-bootstrap-proof.mjs typescript/scripts/publication-gates.mjs typescript/scripts/plan-release-builds.mjs typescript/scripts/release-cut.mjs typescript/scripts/release-draft-assets.mjs typescript/scripts/release-index.mjs typescript/scripts/release-read-retry.mjs typescript/scripts/release-source-at-ref.mjs typescript/scripts/release-versions.mjs typescript/scripts/cargo-workspace.mjs typescript/scripts/
COPY typescript/scripts/release-declaration.mjs typescript/scripts/release-cut-read.mjs typescript/scripts/release-upgrade.mjs typescript/scripts/
COPY typescript/scripts/native-runtime-proof.mjs typescript/scripts/
COPY typescript/scripts/native-install-layout.mjs typescript/scripts/native-database-layout.mjs typescript/scripts/
COPY typescript/scripts/native-application-schema.mjs typescript/scripts/
COPY typescript/test/support/performance-contract.mjs typescript/test/support/performance-contract.mjs
COPY typescript/scripts/native-bootstrap.mjs typescript/scripts/generate-native-bootstrap.mjs typescript/scripts/verify-native-bootstrap.mjs typescript/scripts/
COPY scripts/native-bootstrap.sh scripts/native-bootstrap.sh
COPY skills/tmt/SKILL.md expected-skill.md
COPY skills/tmt/SKILL.md skills/tmt/SKILL.md
COPY skills/tmt-inbox/SKILL.md skills/tmt-inbox/SKILL.md
# Squad archives are compared byte for byte with these sources (--skills).
COPY extensions/tmt-ops/skills/ expected-squad-skills/
COPY --from=build /workspace/native-manifest.json ./
COPY --from=build /workspace/rust/target/native-notices/THIRD-PARTY-NOTICES.txt expected-notices.txt
COPY LICENSE expected-license.txt
COPY --from=build /workspace/target/distrib/ artifacts/
# Pass archive and target explicitly from the generator's target selection.
ENTRYPOINT ["node", "typescript/scripts/verify-native-artifact.mjs", "--manifest", "native-manifest.json", "--skill", "expected-skill.md", "--notices", "expected-notices.txt", "--license", "expected-license.txt", "--version-state", "artifacts/release-version-state.json"]
