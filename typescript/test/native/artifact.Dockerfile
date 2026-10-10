# Local native-architecture artifact proof. Not a publishing workflow.
FROM rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073 AS build
RUN apt-get update && apt-get install --no-install-recommends -y musl-tools \
  && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-dist --version 0.32.0 --locked \
  && cargo install cargo-about --version 0.9.2 --locked --features cli
ARG TARGET_TRIPLE
ARG PRODUCT=cli
RUN test -n "$TARGET_TRIPLE" && rustup target add "$TARGET_TRIPLE"
WORKDIR /workspace
COPY rust/ rust/
COPY extensions/tmt-office/rust/ extensions/tmt-office/rust/
COPY skills/ skills/
COPY extensions/tmt-office/skills/ extensions/tmt-office/skills/
# Workspace member: Cargo must load its manifest even when not building it.
COPY extensions/tmt-ops/ extensions/tmt-ops/
COPY extensions/tmt-remote/rust/ extensions/tmt-remote/rust/
COPY extensions/tmt-colab/rust/ extensions/tmt-colab/rust/
COPY extensions/tmt-colab/contracts/ extensions/tmt-colab/contracts/
COPY extensions/tmt-colab/firestore/ extensions/tmt-colab/firestore/
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
COPY extensions/tmt-office/typescript/apps/office/package.json extensions/tmt-office/typescript/apps/office/package.json
COPY extensions/tmt-office/typescript/apps/office/ extensions/tmt-office/typescript/apps/office/
COPY contracts/ contracts/
COPY extensions/tmt-office/contracts/ extensions/tmt-office/contracts/
RUN cd rust && cargo fetch --locked
RUN scripts/build-native-artifact.sh "$TARGET_TRIPLE" "$PRODUCT" > native-manifest.json

FROM node:22.23.2-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5
RUN apt-get update && apt-get install --no-install-recommends -y binutils \
  && rm -rf /var/lib/apt/lists/*
RUN npm install --global pnpm@10.33.0
WORKDIR /verification
COPY typescript/package.json typescript/pnpm-lock.yaml typescript/pnpm-workspace.yaml typescript/
COPY design/browser-ui/package.json design/browser-ui/package.json
COPY extensions/tmt-office/typescript/apps/office/package.json extensions/tmt-office/typescript/apps/office/package.json
RUN cd typescript && pnpm --filter tmt install --frozen-lockfile --ignore-scripts
COPY .github/components.json .github/components.json
COPY typescript/test/e2e/shard-weights.json typescript/test/e2e/shard-weights.json
COPY typescript/scripts/ci-scope.mjs typescript/scripts/native-release-policy.mjs typescript/scripts/e2e-shards.mjs typescript/scripts/
COPY typescript/scripts/component-skills.mjs typescript/scripts/native-artifact-policy.mjs typescript/scripts/verify-native-artifact.mjs typescript/scripts/verify-native-installation.mjs typescript/scripts/packed-command.mjs typescript/scripts/
COPY typescript/scripts/migrated-state.mjs typescript/scripts/native-upgrade-proof.mjs typescript/scripts/native-bootstrap-proof.mjs typescript/scripts/publication-gates.mjs typescript/scripts/plan-release-builds.mjs typescript/scripts/release-cut.mjs typescript/scripts/release-draft-assets.mjs typescript/scripts/release-index.mjs typescript/scripts/release-source-at-ref.mjs typescript/scripts/release-versions.mjs typescript/scripts/cargo-workspace.mjs typescript/scripts/
COPY typescript/scripts/native-runtime-proof.mjs typescript/scripts/
COPY typescript/scripts/native-application-schema.mjs typescript/scripts/
COPY typescript/test/support/performance-contract.mjs typescript/test/support/performance-contract.mjs
COPY typescript/scripts/native-bootstrap.mjs typescript/scripts/generate-native-bootstrap.mjs typescript/scripts/verify-native-bootstrap.mjs typescript/scripts/
COPY scripts/native-bootstrap.sh scripts/native-bootstrap.sh
COPY skills/tmt/SKILL.md expected-skill.md
COPY skills/tmt/SKILL.md skills/tmt/SKILL.md
COPY skills/tmt-inbox/SKILL.md skills/tmt-inbox/SKILL.md
COPY extensions/tmt-office/skills/tmt-office/SKILL.md extensions/tmt-office/skills/tmt-office/SKILL.md
# Squad archives are compared byte for byte with these sources (--skills).
COPY extensions/tmt-ops/skills/ expected-squad-skills/
COPY --from=build /workspace/native-manifest.json ./
COPY --from=build /workspace/rust/target/native-notices/THIRD-PARTY-NOTICES.txt expected-notices.txt
COPY LICENSE expected-license.txt
COPY --from=build /workspace/target/distrib/ artifacts/
# Pass archive and target explicitly from the generator's target selection.
ENTRYPOINT ["node", "typescript/scripts/verify-native-artifact.mjs", "--manifest", "native-manifest.json", "--skill", "expected-skill.md", "--notices", "expected-notices.txt", "--license", "expected-license.txt"]
