# Optional performance probes

Developer tools, not CI timing gates. They use the shared executable selectors and
isolated fixtures; never measure an installed host command or fall back to another
runtime.

**Startup resources (macOS).** Build a release CLI, select it, run at least twice:

```sh
cargo build --locked --release --manifest-path rust/Cargo.toml
export TMT_TEST_CLI="{\"executable\":\"$PWD/rust/target/release/tmt\",\"args\":[]}"
node typescript/scripts/benchmark-startup.mjs > "$TMPDIR/tmt-startup-run-1.json"
node typescript/scripts/benchmark-startup.mjs > "$TMPDIR/tmt-startup-run-2.json"
```

It uses macOS `/usr/bin/time -lp` with private home/config/workspace state; CPU includes
waited descendants, maximum RSS is bytes, Linux accounting is not implemented and a
denied kernel statistic is a failure, not zero.

**Private-tmux latency (Docker).** Run `scripts/dev-disk-check.sh` first, use a
worktree-named tag (see the
[disk rules](../../../../DEVELOPMENT.md#keep-local-development-from-filling-the-disk)) and
never run these tmux scenarios on the host:

```sh
docker build --build-arg TMT_NATIVE_PROFILE=release -f typescript/test/e2e/Dockerfile -t "tmt-performance:$worktree" .
docker run --rm --init --network none -e TMT_PERFORMANCE_BASELINE=1 "tmt-performance:$worktree" \
  sh -c 'cd /workspace/typescript && pnpm exec vp test run --config test/e2e/vitest.config.ts test/e2e/performance-baseline.e2e.test.ts'
docker image rm "tmt-performance:$worktree"
```

Repeat before removing the image; add `-e TMT_PERFORMANCE_SLOW_REPLY=1` for a 1,500 ms
mock reply delay. The scenario emits a `TMT_PERFORMANCE_BASELINE` JSON report and is
otherwise skipped; assertions gate causal behavior, subprocess bounds and cleanup,
never latency, and a report followed by failed teardown is invalid. Talk timings include
the 500 ms paste/Enter delay and one-second poll granularity: do not subtract the
injected peer delay and call the rest processing time.

**Comparison rules.** Keep all raw samples, the first observation, the median and the
range; record revision and dirty diff, selected executable and peer, toolchain, profile,
target and binary size; same machine and fixture sizes; interleave at least two runs of
seven samples. A repeated scoped median regression above 15% is a review trigger, not an
assertion. Do not shorten production delays or claim unmeasured throughput, memory or
platform behavior.
