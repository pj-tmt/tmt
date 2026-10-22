# Local and CI runner for the Colab Rules emulator suite (suite.mjs).
# Emulator prerequisites: temurin 21 JRE,
# node 26.12.1, firebase-tools 15.29.0. Emulators only; never mount host Firebase credentials.
FROM eclipse-temurin:21-jre-jammy@sha256:54b382c9d790d3201004d4e3921f4eff7dc7d67e1ce21a2da8d8faaaf63c85fc AS java
FROM node:26.12.1-bookworm-slim@sha256:28147155d47dc31abe0a46e0d12b59b2fb3d0a2dcc5dc65fce76a074180b16fe
RUN set -eu; corepack_dir=$(mktemp -d); trap 'rm -rf "$corepack_dir"' EXIT; \
  npm pack corepack@0.36.0 --pack-destination "$corepack_dir" --silent; \
  node -e 'const fs = require("node:fs"), crypto = require("node:crypto"); require("node:assert/strict").equal(crypto.createHash("sha512").update(fs.readFileSync(process.argv[1])).digest("base64"), "SiiJsBhZqdBiPHTEl6OT3sASrRrKIcYTQMsVGXx6EE/gM8WFMwYjeIX8Tt8RiU4Iv2J6LbT8KpGfCOsBpRWB/w==", "Corepack tarball integrity");' "$corepack_dir/corepack-0.36.0.tgz"; \
  npm install --global --ignore-scripts "$corepack_dir/corepack-0.36.0.tgz"; \
  corepack enable
COPY --from=java /opt/java/openjdk /opt/java/openjdk
ENV JAVA_HOME=/opt/java/openjdk
ENV PATH="/opt/java/openjdk/bin:${PATH}"
RUN npm install --global firebase-tools@15.29.0
USER node
WORKDIR /home/node
RUN firebase setup:emulators:firestore --non-interactive
ENV GCLOUD_PROJECT=demo-tmt-colab
