# Local verification only, not a production deployment image.
FROM node:22.23.2-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5
RUN npm install --global pnpm@10.33.0
WORKDIR /workspace/typescript
COPY typescript/package.json typescript/pnpm-lock.yaml typescript/pnpm-workspace.yaml ./
COPY extensions/tmt-office/typescript/apps/office/package.json /workspace/extensions/tmt-office/typescript/apps/office/package.json
RUN pnpm office:install \
  && test ! -e node_modules/better-sqlite3 \
  && test ! -e /node_modules \
  && test ! -e /workspace/rust/target/debug/tmt
COPY extensions/tmt-office/typescript/apps/office /workspace/extensions/tmt-office/typescript/apps/office
COPY extensions/tmt-office/contracts /workspace/extensions/tmt-office/contracts
CMD ["sh", "-c", "pnpm office:check && pnpm office:test && pnpm office:build"]
