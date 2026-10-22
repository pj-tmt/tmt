# Verification tooling only. The product receives a separate empty PATH.
FROM node:26.12.1-alpine@sha256:40cd011b55415bc894e45b02fdd5a5dfde056921b7d23a64c74f80497af48797
RUN set -eu; corepack_dir=$(mktemp -d); trap 'rm -rf "$corepack_dir"' EXIT; \
  npm pack corepack@0.36.0 --pack-destination "$corepack_dir" --silent; \
  node -e 'const fs = require("node:fs"), crypto = require("node:crypto"); require("node:assert/strict").equal(crypto.createHash("sha512").update(fs.readFileSync(process.argv[1])).digest("base64"), "SiiJsBhZqdBiPHTEl6OT3sASrRrKIcYTQMsVGXx6EE/gM8WFMwYjeIX8Tt8RiU4Iv2J6LbT8KpGfCOsBpRWB/w==", "Corepack tarball integrity");' "$corepack_dir/corepack-0.36.0.tgz"; \
  npm install --global --ignore-scripts "$corepack_dir/corepack-0.36.0.tgz"; \
  corepack enable
RUN apk add --no-cache binutils
WORKDIR /verification
COPY typescript/scripts/native-runtime-proof.mjs typescript/scripts/native-database-layout.mjs typescript/scripts/verify-native-runtime.mjs typescript/scripts/packed-command.mjs typescript/scripts/
COPY typescript/test/support/performance-contract.mjs typescript/test/support/performance-contract.mjs
COPY skills/tmt/SKILL.md skills/tmt/SKILL.md
COPY skills/tmt-inbox/SKILL.md skills/tmt-inbox/SKILL.md
ENTRYPOINT ["node", "typescript/scripts/verify-native-runtime.mjs"]
