# Runtime performance

Performance probes are optional developer tools, not timing gates in ordinary CI.
Use the [shared executable selectors](../DEVELOPMENT.md#selecting-the-cli-under-test)
and existing isolated fixtures. Do not restore the retired TypeScript runtime as
a fallback or measure an installed host command by accident.

## Retained comparison

The paired observations below compare both runtimes from source
`e3c8bd7e9a2fdaab0ccfe59a03be7967a7719fcb` before TypeScript source retirement.
These are historical results, not current-source timings or release-archive acceptance.

Collected on 2026-09-08 with an Apple M4 Pro, macOS arm64/Darwin 25.6.0,
Node 24.20.0 tooling and Rust 1.97.0 release builds (thin LTO, stripped debug
information). Collection order was TS, native, native, TS, with seven
fresh-process samples per scenario per run. No other task-owned workload ran;
host-wide quiescence and dropped OS caches are not claimed.

| Scenario              | TS median ms, runs 1 / 2 | Native median ms, runs 1 / 2 | TS median RSS MB, runs 1 / 2 | Native median RSS MB, runs 1 / 2 |
| --------------------- | ------------------------ | ---------------------------- | ---------------------------- | -------------------------------- |
| Help                  | 156.72 / 158.82          | 14.85 / 15.03                | 100.73 / 100.37              | 8.39 / 8.36                      |
| Fresh storage create  | 162.37 / 165.69          | 18.90 / 19.16                | 102.25 / 102.04              | 11.17 / 11.16                    |
| Existing storage show | 158.66 / 158.81          | 15.41 / 16.42                | 100.30 / 100.56              | 10.47 / 10.47                    |

The macOS executable was 7,344,960 bytes. RSS is a process-tree maximum, not
summed concurrent memory; CPU readings rounded to 0.00 are not zero CPU usage.
Wall time includes measurement wrappers. Existing-storage checks verify the
exact identity UUID/content created by the preceding process.

The paired private-tmux series used identical network-isolated Bookworm
containers on a 12-CPU, 8,319,238,144-byte Docker VM, without additional container
limits. Its native target was aarch64 Linux glibc, not the published static-musl
archive. Small/201-pane native whoami medians were 7.99–10.77 ms and check medians
9.68–13.11 ms, with unchanged scoped tmux invocation counts as panes grew.
Constant invocation count does not prove constant internal tmux traversal time.
All eight paired reports passed exact reply/result, independent identity-state,
no-tmux and cleanup assertions.

Talk timings include the 500 ms paste/Enter delay and observer policy. Mock
reply arrival relative to the one-second poll interval can move completion by
an entire interval. Do not subtract the injected peer delay and call the
remainder pure processing time. Docker request-tree CPU/RSS, sustained
contention, multiple active peers and cross-platform distributions are unmeasured.

## Startup resources (macOS)

From the repository root:

```sh
cargo build --locked --release --manifest-path rust/Cargo.toml
export TMT_TEST_CLI="{\"executable\":\"$PWD/rust/target/release/tmt\",\"args\":[]}"
node typescript/scripts/benchmark-startup.mjs > /tmp/tmt-startup-run-1.json
node typescript/scripts/benchmark-startup.mjs > /tmp/tmt-startup-run-2.json
```

The probe uses macOS `/usr/bin/time -lp` and the bounded packed-command runner.
It creates and removes private home/config/workspace state, omits caller context,
blocks PATH-based tmux execution and checks output. Each create uses fresh
storage; show reopens that identity. It never installs globally. CPU includes
waited descendants; maximum RSS is in bytes on macOS. Linux resource accounting
is not implemented. Denied kernel statistics are a measurement failure, not zero.

## Private-tmux latency (Docker)

Use a task-owned image, tagged for the worktree as described in
[DEVELOPMENT.md](../DEVELOPMENT.md#keep-local-development-from-filling-the-disk);
never run these tmux scenarios on the host:

```sh
docker build --build-arg TMT_NATIVE_PROFILE=release -f typescript/test/e2e/Dockerfile -t "tmt-performance:$worktree" .
docker run --rm --init --network none \
  -e TMT_PERFORMANCE_BASELINE=1 "tmt-performance:$worktree" \
  sh -c 'cd /workspace/typescript && pnpm exec vitest run --config test/e2e/vitest.config.ts test/e2e/performance-baseline.e2e.test.ts'
docker image rm "tmt-performance:$worktree"
```

Repeat the run before removing the image. Add
`-e TMT_PERFORMANCE_SLOW_REPLY=1` for the controlled 1,500 ms mock reply delay.
The image places the selected profile at the shared selector's default path;
mock replies inherit it. Normal regression builds remain debug.

The opt-in scenario emits `TMT_PERFORMANCE_BASELINE` JSON and is otherwise
skipped. Setup of 200 extra unbound panes is outside measurements. Trace counts
include fixture caller-session discovery, not all OS subprocesses. Assertions
gate causal behavior, scoped subprocess bounds and cleanup, never latency.
A report followed by failed teardown is invalid evidence.

## Comparison rules

- Keep all raw samples, first observation, median and range. A fresh process is
  not a cold machine/cache; seven samples are not a tail-latency confidence study.
- Record source revision/dirty diff, selected executable and peer, toolchain,
  locked dependencies, profile, target/linkage and binary size. Use the same
  machine/resources, fixture sizes and command defaults; interleave at least
  two runs with seven samples per repeated scenario.
- Keep correctness, compatibility and cleanup gates mandatory. Investigate a
  repeated scoped median regression above 15%; this is a review trigger, not a
  flaky timing assertion. Report talk delay/poll policy separately.
- Preserve deterministic scoped subprocess bounds and inspect output size/scope.
  Do not reduce production delays to advertise speed or claim unmeasured
  throughput, memory, platform or release behavior.
