# Docker E2E scenarios and harness

Run-count requirements and isolation are in
[DEVELOPMENT.md](../../../../DEVELOPMENT.md#docker-e2e).

## Selection and sharding

`TMT_E2E_FILES="ops.e2e.test.ts"` (space-separated plain file names) limits the
run; the image anchors each name as `test/e2e/<name>` because vitest filters by
substring. `TMT_E2E_ADAPTER_TESTS=0` skips the Rust adapter tests, and a host
`CARGO_BUILD_JOBS` (a positive count or `default`) limits the image's builds. CI
runs two shards behind the required `Docker E2E` gate, balanced by
`typescript/test/e2e/shard-weights.json` through `typescript/scripts/e2e-shards.mjs`
(refresh weights from a full run when shards drift; a guard fails if a scenario is
in no shard or two).

## Scenario ownership

Name scenarios for the behavior or adapter they verify. Native-process tests inspect
public contracts and independent stored state; Docker adds real tmux and process
ordering. Consolidate only after mapping setup, causal action, assertions and
failure/cleanup observations; the basic badge case belongs to `pane-badge`.

| E2E file(s)                                                    | Distinct evidence                                                                                                                                                                                                                                                 |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `binding-reconciliation`                                       | Publication races, uncertain endpoint evidence, scoped/global discovery, binding transitions, live listing                                                                                                                                                        |
| `marked-binding`                                               | Explicit marked-pane selection, frozen target evidence, server isolation, mark preservation                                                                                                                                                                       |
| `durable-identity`, `identity-lifecycle`, `identity-retention` | Stored identity/profile continuity, pane/server lifecycle, retirement                                                                                                                                                                                             |
| `check-routing` vs `capture-limits`                            | Real target/server resolution and capture failures vs numeric/configuration bounds                                                                                                                                                                                |
| `profile-binding` vs `role-lifecycle`                          | Verified caller/retired-name ownership vs restart, file input, validation, concurrent writes                                                                                                                                                                      |
| `talk-completion` vs `durable-talk`                            | Observer interruption, attribution, uncertain delivery vs inline input, size bounds, concurrency, retries                                                                                                                                                         |
| `exchange-watermarks` vs `exchange-attention`                  | Gated revision/watermark state, late finals vs config isolation, redaction, rebind access                                                                                                                                                                         |
| `tmux-adapter`, `transport-adapter`                            | Explicit adapter-probe evidence (caller/inventory/markers; delivery/capture stages), not public CLI success                                                                                                                                                       |
| `pane-badge`                                                   | Pane-only badge composition, user ownership, shared theme preservation, rendering, conflicts, cleanup                                                                                                                                                             |
| `executable-selection`, `smoke`                                | Harness selection, causal nested replies, startup and cleanup controls                                                                                                                                                                                            |
| `claude-channel`, `codex-channel`                              | Channel delivery against a mock `claude` or model-free `codex-channel-fixture`: no paste to an opted-in pane, crash cleanup, plain paste kept                                                                                                                     |
| `reply-batching`                                               | Fixed pane windows, disabled grouping, binding-replacement fencing, real attached PTY key debounce, worker/log cleanup; its Python fixture owns and reaps a tmux client so real key bytes reach `client_activity` (control-mode `send-keys` is not user activity) |

## Harness rules

Hosted Docker E2E, Packed install musl and dependency-archive seed jobs use a best-effort registry mirror for unchanged digest-pinned base images; admitted-archive and seed Buildx builders use a digest-pinned mirrored builder image and retain the original setup on failure. Local `pnpm test:e2e` is unchanged.

- CI may restore an exact main-owned Actions archive of dev Cargo dependencies before compiling current source; misses use the original single Docker build, and the default local command does not read this archive.

- Hosted and local `pnpm test:e2e` images disable incremental compilation in `native-tests` and debug symbols for its dev/test fixtures; debug assertions remain enabled for those profiles, with fewer source details in backtraces.
- `typescript/test/e2e/cli-assertions.ts` owns `expectJsonResult` (success envelope
  only); scenarios keep their exact payload assertions and independent SQL oracles.
  Do not build a permissive shared schema or import product types to manufacture
  expected results.
- Scenario imports use the `typescript/test/e2e/harness.ts` facade; fixture,
  readiness, cleanup and types live under `typescript/test/e2e/harness/`
  ([ownership](../../../../ARCHITECTURE.md#testing-and-evidence-boundaries)).
  Synchronous fixture tmux calls fail after five seconds and kill the client;
  scenario timeouts stay required. Install a fixture's tmux trace once and reuse
  `clear()`.
  Private-server pane borders are visible by default; the badge hint case opts out.
- Shared cross-suite utilities go in `typescript/test/support/`; helper tests go in
  `typescript/test/tooling/` and must prove rejection as well as positive behavior.
  The import-direction guard checks these boundaries without Docker.
- Codex channel scenarios build both `tmt-cli` and the `codex-channel-fixture` example
  into the E2E image only; explicit `--channel` sessions wait for the private Ready
  record matched to the foreground, and plain controls keep their foreground-only gate.
- A performance probe never gates on latency; see
  [performance probes](performance-probes.md).
