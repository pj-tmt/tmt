# Focused Rust checks

Model-free, deterministic checks for driver, runtime and request behavior. Run from
`rust/` with `CARGO_BUILD_JOBS=2`; shared gates are in
[DEVELOPMENT.md](../../../../DEVELOPMENT.md#rust-checks). None of these starts a
model or reads provider credentials.

## Codex driver

- Queue transport: `cargo test --locked -p tmt-adapters drivers::codex::queue`, `...::transport`
  (loopback peers; receipt loss and absolute deadlines) and `...::delivery` (peer
  readiness, channel-gated final process observation). Preparation and delivery each keep a
  three-second absolute bound, so a send can spend six seconds; talk's observer deadline
  still starts before the synchronous send and is not a transport cancellation boundary.
  Budgets and terminal uncertainty are owned by the
  [Codex contract](../../../../contracts/codex-channel-v1.md#transport-and-qualification).
  The refusal fixture holds a bound, non-listening socket through the connect attempt
  so a parallel test cannot claim the port.
- Enrollment state: `drivers::codex::record` (isolated records, injected liveness;
  lock-scoped changes and replacement preservation, not real crashed-server recovery).
- Startup and attachment planning: `drivers::codex` (isolated shell stand-ins; cwd probes
  compare relative and absolute `-C`).
- Fresh bootstrap: `drivers::codex::supervisor`, `::channel_hooks`, `::record`, `::server`,
  and `cargo test --locked -p tmt-cli --bin tmt run_command::channel`; plus
  `cargo test --locked -p tmt-cli --test architecture`,
  `cargo test --locked -p tmt-cli private_hook_worker_budget` and
  `cargo test --locked -p tmt-cli budget_tests`. The TERM-ignoring child test checks group
  SIGKILL and reaping. These do not replace the built-driver eager-TUI and Idle proof.

- Folder-trust advice: `cargo test --locked -p tmt-adapters drivers::codex::trust`,
  `... drivers::codex::channel` and `cargo test --locked -p tmt-cli run_command::channel` (temporary
  config fixtures; the injected version runner permits only `--version`).

## Reply notices and requests

- Notice presentation and inlined reply bodies: `cargo test --locked -p tmt-adapters delivery::notices` and
  `... storage::requests::service_tests::notification` (body limits per transport, quoting, batch budget,
  unchanged attention/retention after a read; the architecture suite also confines display-width
  dependencies to the two presentation owners).

- Reply-notice waiter timing: `cargo test --locked -p tmt-cli --bin tmt reply_notice_command::tests`
  (injected monotonic clock and sender gate over real isolated SQLite claims; the old
  three-second grace must fail the positive control), and
  `cargo test --locked -p tmt-adapters runtime::tests::maximum_send_duration`.
  `storage::requests::service_tests::notification` owns dead-sender recovery and
  attempted-frame no-replay.
- Request-ID prefix selection: `cargo test --locked -p tmt-core --lib request::service::responses`,
  `cargo test --locked -p tmt-adapters storage::requests::service_tests::response`,
  `... retained_request_id_sample_and_overflow_count` and
  `cargo test --locked -p tmt-cli --test result_prefix` (real SQLite, injected clock, indexed
  ID ranges for sample and overflow count).
- Completed-request counters: `cargo test --locked -p tmt-adapters runtime::consumption` and
  `cargo test --locked -p tmt-cli --bin tmt output::tests`. For a manual release-mode scan
  measurement run the ignored `runtime::consumption::tests::streaming_scan_measurement`
  with `TMT_SCAN_MIB` (`1`, `10`, `100`) and `TMT_SCAN_MIX` (`foreign`, `usage`, `long`);
  measurements are evidence, never scan-budget calibration.

## Consumption history

`cargo test --locked -p tmt-adapters consumption`, `... runtime::sampling` and
`... process::interactive` cover history reduction, source admission and foreground
tick/exit/degraded cleanup; history and API tests consume the shared vector
`contracts/consumption-history-v1.json` (the closed counter is the live baseline even when
`ls` is newer). Public history reads must not renew retention or touch provider files.
Verify schema 46 against the independent native storage fixture and its change-cursor
triggers. `usage-hooks.e2e.test.ts` verifies admitted hooks, unchanged context usage and
compaction; foreground sampling scenarios hold a deterministic mock turn open, observe
advancing public counters before Stop, prove no sample/Stop double count and assert cleanup
and the original nonzero child status. Run one focused selection of affected Docker scenarios in the
infra heavy slot with isolated HOME/provider roots; run it twice only when a
lifecycle or transport change needs repeat evidence. Never sample a real account. Old wrappers
and hook-only launches stay Stop-only until relaunched, and `--no-usage` or the legacy opt-out
must suppress foreground collection too.

## Targets, hosts and setup

- Explicit tmux target errors: `cargo test --locked -p tmt-adapters tmux::io_tests` and
  `cargo test --locked -p tmt-cli --test target_resolution` (slow stand-in under an
  isolated HOME; no real tmux server).
- External-host fixtures: `cargo test --locked -p tmt-adapters host::external::tests`. Success
  cases use a thirty-second test runner budget; conformance timing uses scripted elapsed
  values and the late-answer case keeps the production runner and deadline.
- Completion setup: `cargo test --locked -p tmt-adapters completion_install` and
  `cargo test --locked -p tmt-cli --test completion_install` (read-only startup-file
  evidence, shell detection, framework-managed zsh; a real PTY checks the help hint).
  Fixtures own their terminals and HOMEs; never use a real shell startup file. Run the CLI
  style and architecture guards with these checks.
- Skill reminder hints: `cargo test --locked -p tmt-cli --bin tmt skill_reminder`.
