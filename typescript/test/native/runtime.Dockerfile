# Verification tooling only. The product receives a separate empty PATH.
FROM node:26.12.1-alpine@sha256:ae8e1c19d56886270987e99b3cfcef13a919f0655b376ae322bc64ada76f88cd
RUN apk add --no-cache binutils
WORKDIR /verification
COPY typescript/scripts/native-runtime-proof.mjs typescript/scripts/native-database-layout.mjs typescript/scripts/verify-native-runtime.mjs typescript/scripts/packed-command.mjs typescript/scripts/
COPY typescript/test/support/performance-contract.mjs typescript/test/support/performance-contract.mjs
COPY skills/tmt/SKILL.md skills/tmt/SKILL.md
COPY skills/tmt-inbox/SKILL.md skills/tmt-inbox/SKILL.md
ENTRYPOINT ["node", "typescript/scripts/verify-native-runtime.mjs"]
