FROM rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073 AS native-tests
WORKDIR /native/rust
COPY rust/ ./
COPY extensions/tmt-office/rust/ ../extensions/tmt-office/rust/
COPY extensions/tmt-office/contracts/ ../extensions/tmt-office/contracts/
COPY skills/ ../skills/
COPY extensions/tmt-office/skills/ ../extensions/tmt-office/skills/
# The squad extension: built for its E2E scenarios.
COPY extensions/tmt-squad/ ../extensions/tmt-squad/
COPY extensions/tmt-remote/rust/ ../extensions/tmt-remote/rust/
COPY extensions/tmt-colab/rust/ ../extensions/tmt-colab/rust/
COPY extensions/tmt-colab/contracts/ ../extensions/tmt-colab/contracts/
ARG TMT_NATIVE_PROFILE=dev
# Every cargo command below reads this; `pnpm test:e2e` forwards a host value.
ARG CARGO_BUILD_JOBS=default
RUN cargo build --locked --example tmux-probe --example runtime-caller-fixture --example claude-hook-fixture --example codex-channel-fixture \
  && case "$TMT_NATIVE_PROFILE" in dev) native_output=debug ;; release) native_output=release ;; *) exit 2 ;; esac \
  && cargo build --locked --profile "$TMT_NATIVE_PROFILE" -p tmt-cli \
  && CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked --profile "$TMT_NATIVE_PROFILE" -p tmt-office \
  && cargo build --locked --profile "$TMT_NATIVE_PROFILE" -p tmt-squad \
  && cargo build --locked --profile "$TMT_NATIVE_PROFILE" -p tmt-remote \
  && cargo test --locked --no-run -p tmt-adapters \
  && mkdir /native-artifacts \
  && cp target/debug/examples/tmux-probe /native-artifacts/tmux-probe \
  && cp target/debug/examples/runtime-caller-fixture /native-artifacts/codex \
  && cp target/debug/examples/claude-hook-fixture /native-artifacts/claude \
  && cp "target/$native_output/tmt" /native-artifacts/tmt \
  && cp "target/$native_output/tmt-office" /native-artifacts/tmt-office \
  && cp "target/$native_output/tmt-squad" /native-artifacts/tmt-squad \
  && cp "target/$native_output/tmt-remote" /native-artifacts/tmt-remote \
  && find target/debug/deps -maxdepth 1 -type f -name 'tmt_adapters-*' -executable \
    -exec cp {} /native-artifacts/adapter-tests \;

FROM node:22.23.2-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5
ARG TMUX_VERSION=3.3a-3
ARG PROCPS_VERSION=2:4.0.2-3
RUN apt-get update \
  && apt-get install --no-install-recommends -y "tmux=${TMUX_VERSION}" "procps=${PROCPS_VERSION}" bash-completion zsh fish python3 make g++ \
  && rm -rf /var/lib/apt/lists/*
RUN npm install --global pnpm@10.33.0
WORKDIR /workspace
COPY typescript/package.json typescript/pnpm-lock.yaml typescript/pnpm-workspace.yaml typescript/
COPY extensions/tmt-office/typescript/apps/office/package.json extensions/tmt-office/typescript/apps/office/package.json
RUN cd typescript && pnpm --filter tmux-team install --frozen-lockfile \
  && test ! -e ../extensions/tmt-office/typescript/apps/office/node_modules/react
COPY . .
COPY --from=native-tests /native-artifacts/tmux-probe /opt/tmt-tests/tmux-probe
COPY --from=native-tests /native-artifacts/codex /opt/tmt-tests/codex
COPY --from=native-tests /native/rust/target/debug/examples/codex-channel-fixture /opt/tmt-tests/codex-channel-fixture
COPY --from=native-tests /native-artifacts/claude /opt/tmt-tests/claude
COPY --from=native-tests /native-artifacts/claude /opt/tmt-tests/hook-runtime/codex
COPY --from=native-tests /native-artifacts/adapter-tests /opt/tmt-tests/adapter-tests
# Exercise the same unset-selector path as a native developer checkout.
# The selected dev/release profile changes bytes, not this fixture location.
COPY --from=native-tests /native-artifacts/tmt /workspace/rust/target/debug/tmt
COPY --from=native-tests /native-artifacts/tmt-office /workspace/rust/target/debug/tmt-office
COPY --from=native-tests /native-artifacts/tmt-squad /workspace/rust/target/debug/tmt-squad
COPY --from=native-tests /native-artifacts/tmt-remote /workspace/rust/target/debug/tmt-remote
ENV TMT_TEST_TMUX_PROBE='{"executable":"/opt/tmt-tests/tmux-probe","args":[]}'
# Run process lifecycle tests under the wrapper's --init, not a build-stage
# shell that may retain orphan zombies. Both suites share network isolation.
# TMT_E2E_FILES (validated plain file names) limits the suite to those files, and
# TMT_E2E_ADAPTER_TESTS=0 skips the Rust adapter tests; CI sets both per shard. Vitest
# matches a positional filter by substring, so each name becomes the anchored path
# test/e2e/<name>: a bare routing.e2e.test.ts would also select check-routing.e2e.test.ts.
CMD ["sh", "-c", "if [ \"${TMT_E2E_ADAPTER_TESTS:-1}\" = 1 ]; then /opt/tmt-tests/adapter-tests; fi && cd /workspace/typescript && for name in $TMT_E2E_FILES; do set -- \"$@\" \"test/e2e/$name\"; done && exec pnpm exec vp test run --config test/e2e/vitest.config.ts \"$@\""]
