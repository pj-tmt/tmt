# CI-only seed, never used by local or cache-miss builds.
FROM rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073 AS ci-chef
RUN curl --fail --location https://github.com/LukeMathWalker/cargo-chef/releases/download/v0.1.77/cargo-chef-x86_64-unknown-linux-musl.tar.xz --output /tmp/chef.tar.xz \
    && echo 'a3733ab416c3ffddd37914cd13919ca05fee1a1cf654f3016dcfe7f399d89cd1  /tmp/chef.tar.xz' | sha256sum --check - \
    && tar -xJf /tmp/chef.tar.xz --strip-components=1 -C /usr/local/bin cargo-chef-x86_64-unknown-linux-musl/cargo-chef \
    && rm /tmp/chef.tar.xz \
    && cargo chef --version

FROM ci-chef AS ci-planner
WORKDIR /native/rust
COPY rust/ /native/rust/
COPY extensions/tmt-office/rust/ /native/extensions/tmt-office/rust/
COPY extensions/tmt-ops/rust/ /native/extensions/tmt-ops/rust/
COPY extensions/tmt-remote/rust/ /native/extensions/tmt-remote/rust/
COPY extensions/tmt-colab/rust/ /native/extensions/tmt-colab/rust/
RUN cargo chef prepare --recipe-path /recipe.json

FROM ci-chef AS ci-cook
WORKDIR /native/rust
COPY --from=ci-planner /recipe.json /recipe.json
ARG CARGO_BUILD_JOBS=default
ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
# Keep dependency variants separate, like the real commands below. Chef removes
# compiled workspace libraries, but v0.1.77 leaves dummy test executables.
# Remove only those adapter executables before copying current source.
RUN mkdir -p target/debug/deps /usr/local/cargo/registry \
    && cargo chef cook --locked --recipe-path /recipe.json --examples -p tmt-adapters \
    && cargo chef cook --locked --recipe-path /recipe.json --profile dev -p tmt-cli \
    && cargo chef cook --locked --recipe-path /recipe.json --profile dev -p tmt-ops \
    && cargo chef cook --locked --recipe-path /recipe.json --profile dev -p tmt-remote \
    && cargo chef cook --locked --recipe-path /recipe.json --profile test --tests -p tmt-adapters \
    && find target/debug/deps -maxdepth 1 -type f -perm -111 -regex '.*/tmt_adapters-[0-9a-f][0-9a-f]*' -delete

# Local output exports dev dependencies only; no rootfs/graph cache or target/tmp.
FROM scratch AS dependencies
COPY --from=ci-cook /native/rust/target/debug/ /target/debug/
COPY --from=ci-cook /usr/local/cargo/registry/ /registry/
