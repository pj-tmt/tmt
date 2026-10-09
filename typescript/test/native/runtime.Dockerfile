# Verification tooling only. The product receives a separate empty PATH.
FROM node:22-alpine@sha256:c610fcdfb1d5b4740dd70c284ed3cb16bb857e0f7166196e36a5501df7a3aa32
RUN apk add --no-cache binutils
WORKDIR /verification
COPY typescript/scripts/native-runtime-proof.mjs typescript/scripts/verify-native-runtime.mjs typescript/scripts/packed-command.mjs typescript/scripts/
COPY typescript/test/support/performance-contract.mjs typescript/test/support/performance-contract.mjs
COPY skills/tmt/SKILL.md skills/tmt/SKILL.md
COPY skills/tmt-inbox/SKILL.md skills/tmt-inbox/SKILL.md
COPY extensions/tmt-office/skills/tmt-office/SKILL.md extensions/tmt-office/skills/tmt-office/SKILL.md
ENTRYPOINT ["node", "typescript/scripts/verify-native-runtime.mjs"]
