# Ask agent real-binary acceptance (#1110)

`extensions/tmt-colab/typescript/app/acceptance/` drives the built `tmt`, `tmt-remote` and
`tmt-colab` with real Chromium devices. It needs no Docker and adds no production seam.

## Run

Build the app before the binaries so the native build embeds the current assets.
From the repository root, point `TMT_ACCEPTANCE_BIN_DIR` at the directory holding the
three binaries (default `rust/target/debug`):

```bash
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match build
corepack pnpm@10.33.0 --dir typescript --filter @tmt/remote-client... install --frozen-lockfile --ignore-scripts
corepack pnpm@10.33.0 --dir typescript --filter @tmt/remote-client exec vp build --config vite.firebase.config.ts
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match build:hosted
TMT_COLAB_HOSTING_DIR="$PWD/extensions/tmt-colab/typescript/app/dist-hosted" \
CARGO_BUILD_JOBS=2 TMT_COLAB_APP_DIR="$PWD/extensions/tmt-colab/typescript/app/dist" \
  cargo build --locked --manifest-path rust/Cargo.toml -p tmt-cli -p tmt-remote -p tmt-colab --bins
cargo build --locked --manifest-path rust/Cargo.toml -p tmt-adapters --example provider-script-fixture
cargo build --locked --manifest-path rust/Cargo.toml -p tmt-remote --example compose_hosting
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match test:acceptance
```

The provider build is Core's interpreted-provider fixture, its own invocation (an example selected
next to `tmt-cli` enables test-isolation features that make `tmt` refuse to run in a world).
A recipient started as a `claude` or `codex` driver is that fixture copied under the provider
basename, with `TMT_TEST_PROVIDER_INTERPRETER` set to the absolute Node: it accepts Core's managed
launch options and runs the recipient script. The harness reads it from `examples/` of the binaries
directory (so a dedicated execution directory carries `examples/provider-script-fixture` too), and
a missing one fails with the build command. The fixture reaps its child; the world keeps the
process-group and tmux-server cleanup and the no-leftovers assertion.

The hosted-bundle case also needs Remote's `examples/compose_hosting`, built separately,
and the hosted inventory embedded by `TMT_COLAB_HOSTING_DIR`; the ordinary app distribution
alone cannot qualify that case. Freeze both examples with the three executables.

If also running [native Chromium fixtures](development.md#app-and-browser-client),
compile those fixtures before the final binary build: their test configuration can
replace the regular `tmt-colab` artifact in a shared target directory. Copy the final
three binaries and both examples into a dedicated execution directory, select it with
`TMT_ACCEPTANCE_BIN_DIR`, and record the tested head, binary and embedded asset hashes
before execution. Keep that directory unchanged throughout both acceptance runs.

Run it twice for lifecycle acceptance; it remains a recorded manual gate on the PR head.
The separate advisory `colab-browser.yml` acceptance job runs once weekly, manually,
or for a PR carrying `colab-acceptance`, as two shards of whole spec files balanced by
measured time (`typescript/scripts/colab-acceptance-shards.mjs`); each shard uses one
app/native build and one Playwright worker. PR runs test the default merge ref and record
both the PR head and tested merge SHA in each shard's `tested-head.txt`. It retains exact build hashes, test outcomes and bounded assertion locations/timeouts
without private error values, traces or profiles. An unexpected outcome also prints this
bounded summary even if the list reporter's detailed failure is absent from the job log;
selection and concurrency are owned by the [CI reference](../../tmt-release/references/ci-selection.md#advisory-browser-selection).
The acceptance job records its start timestamp before checkout. The ordinary suite is interrupted
at +1,560 seconds on shard 1 or +1,740 seconds on shard 2, including setup/build time. The runner
owns a new process group: SIGINT allows up to 30 seconds for final JSON and cleanup, then SIGKILL
bounds that group only. Deadline and job-signal interruption remain nonzero even if the child
exits zero. Shard 1 reserves a further 180 seconds for the hosted-reader proof; both shards reserve
30 seconds for public reporting/upload before the unchanged 30-minute job ceiling. A forced group
kill does not reach harness-detached browser/tmux groups and is not proof of clean world teardown;
the hosted runner's job lifetime bounds those groups. These are CI budgets, not spec timeouts.

The public `outcomes.json` includes a closed `run` status (`passed`, `failed` or `incomplete`)
and reason alongside sanitized test rows. Missing, malformed, empty or interrupted final JSON,
missing execution evidence and deadline/startup failures are explicitly incomplete, including when
no test rows flushed. Never infer a green run from an empty inventory or only the passing rows of
an interrupted report. The private execution verdict, raw JSON, traces and generic errors are not
uploaded. Weight provenance lives in `acceptance/shard-weights.json`; its balance is a model until
an actual hosted run measures the new split.

The one synthetic detached-eviction/refused-tab soak case additionally emits a separate
`colab-soak-failure-N` artifact only on failure, retained for seven days. It contains at most
16 KiB: a fixed assertion ID/sentence, allowlisted owner-source error codes/messages,
owned child exit/signal and socket/door liveness, up to 40 timestamped error/exit records,
and up to 128 closed Playwright API spans from the last 30 seconds. The reporter never
retains test-authored step titles, action arguments, network payloads, DOM, URLs or raw
trace archives. Unknown text becomes unrecognized; truncation is explicit. Only exact
owned-world regular log files are read, from at most their last 64 KiB, without following
symlinks. The public report reconstructs every field independently and keeps this artifact
separate from the unchanged outcome-only results directory. The vocabulary extractor reads
explicit owner files and named static literals; source drift fails tests instead of widening
what can be published. World and collector share one lazily loaded vocabulary; a load failure
disables this optional evidence without changing the scenario outcome. This evidence observes
failure and never retries an operation.

An own-publication timeout may also retain its closed diagnostic: record root/kind, own stream
ID, submitted/admitted sequence numbers and whether a leader receipt arrived. A replayed receipt
may have no retained submitted sequence. The failure hook reads only this typed parent attribute
before browser cleanup; the report independently validates it and drops other fields. Record
keys, values, generic errors and URLs remain excluded. This metadata never acknowledges a write
or advances admission; the exact local verified fold still owns publication completion.
A failed `expectLive` check (the soak tests' Live status) retains `liveStates`: the header state it
stayed in (`live`, `opening`, `stopped`, `reconnecting`, `archived`, `none` or `other`), whether the
renderer frame existed, the tab's visibility and `ended`, the failure cause named by the terminal
notice's `Code:` label (a `FailureCause` key, `none` without a notice or `other`). The harness maps
the parent's status and reference labels to those closed sets and never reads frame text; the
report drops any other value. Its cause list is its own copy, and the app unit test
`failure-reference-report.test.ts` requires every key of `text.failureReference` in it, so a cause
the product adds fails there until the report lists it.
Set `TMT_ACCEPTANCE_KEEP=1` to keep a world's root (counter rows, `*.stderr`) after a run.
Use that setting only for diagnostics: the harness's injected-failure case requires root deletion,
so a retained-root run cannot satisfy the complete suite's cleanup gate. A Colab PR handoff
names the acceptance specs it ran and the exact tested executable/asset hashes.

## Hosted producer/reader emulator proof

`hosted-reader.proof.ts` runs through the unchanged
`node extensions/tmt-colab/scripts/hosted-reader-emulator.mjs` wrapper in CI:
selected `colab-hosted` PR jobs and shard 1 of full label/weekly/manual acceptance,
after ordinary acceptance, whatever its result, unless the run is cancelled. The shared action prepares
Java 21, pinned Firebase tools and the emulator JAR; the wrapper owns private state,
loopback demo Auth/Firestore, real native producer builds and Chromium cleanup.
Finite Firestore REST calls use one fresh loopback HTTP connection each, without retries,
forwarding the original status, headers and body. Upstream failure aborts the browser request
and fails the proof; only Listen long-polls use the pooled Playwright fetch path.
This advisory proof is separate from the signed-fixture browser suites; Remote owns
its producer scenario. Its last phases are Colab's: a door holder and a hosted holder of one
editor link each send two edit requests, every request is applied, and a second hosted holder (a
browser profile of its own, so a Firebase uid of its own) then alternates sends with the first on
the link's one hosted stream. The owner applies every request, and its store holds the door stream
with two receipts and the hosted stream with five, none frozen. Selection and lane topology stay in the
[CI reference](../../tmt-release/references/ci-selection.md#advisory-browser-selection).

## World

Each scenario gets one world (`harness/world.ts`) under a short `/tmp` root, because Unix
socket paths are limited to about 100 bytes:

- private HOME and XDG roots, and a fixed `<world>/tmux.sock` selected with `-S`.
  The shared `privateTmux` helper asserts the socket private at construction and before
  world commands; the native-child `PATH` wrapper uses `scripts/private-tmux.sh` for the
  same check, accepting Core's explicit `-S` only when it matches the world socket.
  Both remove inherited `TMUX` and `TMUX_PANE` before invoking tmux. The tmux
  `TMUX` session value passed to agent callers has no `$` prefix;
- an agent environment (`TMT_AGENT=1`, `BROWSER`) with stand-in `open`, `xdg-open` and `wslview`
  first on `PATH` that only append to `opener.log`; `dispose()` fails the run if that log
  exists, so no command in a world can reach the host browser;
- the real `tmt-remote` door with the real `tmt-colab` mounted, started by `startDoor`;
- one Chromium profile per paired device (`pairBrowser`: the real `pair --json` ceremony), so
  two viewers have separate keys, IndexedDB and cookies;
- a recipient pane (`harness/recipient.mjs`) that appends a durable `received` row before it
  replies through the real `tmt reply`. Count work from those rows, never from terminal echo;
  a duplicate wake is a second row for the same request;
- `TMT_EXECUTABLE` for Remote and Colab is a wrapper (`harness/core-barrier.mjs`) that
  forwards to the real core, records every launch (`coreCalls()`), and can park one
  `dispatch.create` before or after the core acts (`armBarrier`, `barrierEntered`,
  `releaseBarrier`) so a scenario kills and restarts a process there.

`withWorld` always disposes. Disposal closes the browsers, signals each owned agent pane and
waits for its completion while tmux can still reap it, then stops process groups and the tmux
server. A pane exceeding its completion bound is a reported leak; so is any remaining process
naming the root or tmux socket, or any other socket, including after a failed scenario.
Provider panes explicitly exec `tmt run`, so the pane leader is the wrapper rather than a shell.
`harness.spec.ts` proves the harness, including the sensitivity of that check.

## Pages

A page needs a real owner command: `tmt colab page create --title <title> --layout column|full|canvas [--reason <text>]
[--file <path|->] [--json]` (empty source by default; a file or stdin otherwise), which works while `serve`
runs. `--json` returns `{spaceId,pageId,title,path,operationId,membershipHead}`; open `path`
under the Remote door address (`createPage` and `openPage` in `harness/ask.ts` do this).
`tmt colab ls --json` and `tmt colab page read <page>` verify it. Create the page before the
browsers register: each paired device registers when it first opens the app.

## Cases

`object-channel.spec.ts` uses the frozen three binaries for first-demand channel activation,
replacement and original-ID upload recovery. Its standalone browser test client compiles the
production registration, Connection, crypto, fold, Writer and attachment consumers; only that
client's module assets are fixture-served. The mounted document, SDK, native admission,
object backend and replies are real. It refuses incomplete/unpublished reads, verifies
committed bytes before publishing a reference, reads through the authenticated reference,
and checks revoke plus world cleanup. The fixture never enters the production app build.
Discovery attachments record child-process spans and first-demand success under Remote's
250 ms setup budget; they do not qualify Darwin first-exec or preempt OS delays.

`attachments.spec.ts` drives Chat and document attachments through the browser, export and
`attachment read` byte parity, and native attach (#2291): `tmt colab attachment attach` lists a
file for a second paired device without a reload, downloads and reads back byte-equal, and the CLI
killed after the serve sealed an 8 MiB file leaves the serve to finish while `--resume <slot>`
returns the same attachment, once. Its refusals (missing, link, directory, oversized file, unknown
page or slot, another page's slot, archived page) stage nothing; a finished slot holds only
`slot.json`. The same spec revokes a registered device after a native attach: it waits on the
published document reference, checks a new epoch and attachment ID, and requires one complete
reference in every sampled export. A read-only SQLite receipt count proves exactly one content
update swapped the descriptor. The surviving browser and CLI read byte-identical files; the
old reference is stale, and subsequent reads leave the single swap unchanged.

`agent-status.spec.ts` reads the real admitted directory through the Agents drawer,
checks its served asset hashes and CSP, and injects a labelled context-read refusal
without sending or reopening. By default it compares every served asset with the app's
current `dist` build. For a relocated frozen binary run, set `COLAB_STATUS_ASSET_MANIFEST`
to an independently retained build report containing all current `distAssets` paths and SHA256
digests, including `THIRD-PARTY-NOTICES.txt`; optional
`COLAB_STATUS_NATIVE_CAPTURE_DIR` saves 1440/390 light/dark originals. Its injected
refusal does not diagnose an existing browser's session failure.

`one-command.spec.ts` (#1584) starts only `tmt-colab serve` (`startServe`): Colab starts the
real `tmt-remote` door through the real core's public CLI, a paired device opens the printed
link and a created page, and stopping Colab closes the door it started (`doorAnswers`).
`world.linkExtensions()` puts `tmt-remote` and `tmt-colab` on the world's PATH, which is how
the core resolves `tmt remote`. `page create --json` prints the door's full `link` and `paired` (#1614), and the link opens as the
paired device. A second case drives `tmt colab stop` (#1594): the started door closes, a second
stop is `not-running`, and the pairing stays listed. A third attaches to a door started outside
Colab: stop and exit leave that door running.

`mention-send.spec.ts` creates a page from a real agent pane and verifies its removable creator mention,
one comment with three distinct Asks, live recipient bytes, durable offline inbox delivery,
and reload without replay. `createPage` links the extensions onto the isolated world's PATH
so native creator provenance resolves through the real core.

`discussion.spec.ts` covers two paired writers and comment-origin Ask. Its module
contracts and focused cases are described in [discussion.md](discussion.md).

`ask.spec.ts` also proves verified treatment across served Remote: provider fixtures
execute public setup's installed SessionStart hook under real `tmt run`; Core
admits live claude/codex sessions. Ordinary recipients omit the field. Real replies
and reload keep the expected treatment with one dispatch. No directory or SDK
response is patched for this proof.

`ask.spec.ts` also pins prompt original-ID replies after sixty seconds of pending
observation, in idle Chat and in an annotation after a page write. The real recipient
reply reaches the open page within five seconds (the three-second read cap plus
native publication/decoding), with one dispatch and no reload.

`ask.spec.ts` holds the Ask cases: direct send (the recipient's received text is the oracle for the exact bytes) and a second viewer, browser
reload, Remote restart after the core accepted, Remote restart before dispatch, Colab restart,
device revocation and two tabs of one browser staying live at once. They
drive direct Chat (`composeChat`, `sendChat`, `askEntry`, `askState`) and run against the built binaries.
A composer is filled only through `annotationInput` (which `composeChat` uses): a fresh composer seeds its
default mention when the agent directory resolves, and Playwright's fill (select all, then insert) lets that
seed land between its two steps.
Remote stores sessions and door cookies in memory. The mounted owner can replace a verified
ended Session after a restart without a reload.
The cases admit that in-place recovery (the page's automatic reconnect, or its explicit
Reconnect action) before checking the original Ask outcome. They observe a fresh successful mounted device registration after
Remote dies, then require live status and no stopped preview; an old iframe is not recovery.
The restored ask is observed read-only under its original operation ID: accepted, or uncertain
with abandon recorded as `MAY_HAVE_BEEN_DELIVERED`, never a second dispatch. An in-flight send
stays "dispatching" until the SDK deadline, so a case reconnects instead of waiting for it.
The held case waits for a Remote-provided hold fixture, with held behavior covered by unit
tests. Both Remote restart cases are active pass-required cases. A fetch failure during
replacement leaves explicit Reconnect available. They retain their original-operation,
uncertain, recheck, abandon, no-effect, accepted, one-dispatch and one-wake assertions.
Explicit-Reconnect draft cases keep the same Chat and anchored composer at 1440/390,
including a mid-text caret through a failed click and a verified in-place replacement.
They count the original Ask and the next explicit Send separately, verify a new
registration without main-frame navigation, and exercise a plain-comment Send before an anchored Ask.
Enable a case by making its body pass, never with
a stand-in. Assert the recipient's text equals the disclosed bytes captured on Enter, including the
`[remote: <device>]` line, and that no delivery state is shown (presence only).

`withWorld` always disposes, and `test.afterEach(disposeActiveWorlds)` does too after a test
timeout. Playwright abandons a timed-out scenario without cancelling it, so a disposed world
refuses new children, tmux calls and closers (the scenario then fails on its own), each closer is
bounded to 10 s, and `disposeActiveWorlds` throws the leak report instead of dropping it.
All disposal callers join one promise; the world stays registered until cleanup settles,
and simultaneous scenario and cleanup failures are both reported. Allowlisted lifecycle
records link the case/worker to its root and spawned groups, private tmux identity and
root-attested serve-lock owner without URLs or seeds. Emergency cleanup stops only those
recorded owners, rechecking current serve-lock ownership before signalling a retained PID;
unrelated processes sharing an executable remain untouched.
`harness.spec.ts` also runs an excluded child fixture through real Playwright timeouts,
including a timeout while cleanup is already running, and checks process absence. A worker that Playwright kills
outright (a hook timeout) can still leave a world behind; no hook can run then.

`reader.spec.ts` (#1545) drives the read-only share link: the page, `share link add`/`reset` and `page write`
are real owner commands, and each reader is an unpaired Chromium profile (`openReaderLink`) that
never paired with the door. It asserts that an unpaired browser gets no owner file, the fragment
leaves the address bar, the page shows read-only and live, an owner edit reaches it, no request
carries the seed, Reset ends the open reader ("Access ended") and the old link, and the
replacement link opens in a third profile.
Its editor-link case adds `share link add --role editor`: the reader says "Can edit", its Source drawer
saves page text through the serve, the owner's CLI reads the saved page, and a viewer link to the same page
shows the edit with no Source control.

`export.spec.ts` (#1574) has two paired writers discuss a selection, one asks the recipient
agent about the comment, then exports through the browser's Export panel (real downloads) and
the CLI `tmt colab export`. It asserts the page, both conversation files and the manifest
(except `exportedAtMs`) are byte-identical between the two paths, and that the export holds both
writers' comments, the accepted Ask and the stored reply.

`tabs.spec.ts` pins the Remote contract the Ask design depends on: several
sessions of one device can coexist; at the configured limit a detached session is evicted
and reports the active limit, while an attached page is never evicted: a new page is refused
(#2566) and every open page stays live.

`idle-tab.spec.ts` (#2170) leaves one live tab idle for 150 s, past the door's 120 s tunnel
limit, and counts the page's `/sync` sockets: none may close or reopen, because the server pings
every 30 s. It takes about 2.5 minutes, so run it only for changes to the sync keepalive, the
tunnel limits or the live reconnect path.

`connection-reconnect.spec.ts` (#2572) restarts Remote (SIGKILL, same port) under an open owner
page and expects the same tab, with no Reconnect click and no reload, to return to Live, read the
next `page write` and deliver a Chat message to an agent.

`serve-restart.spec.ts` (#2929) installs a newer Colab release under an attached door and holds
the replacement down for longer than the readmission (`READMIT_DELAYS_MS` plus margin). The open
page stays `Reconnecting…` with no card, returns to Live when the replacement starts, and then
reads a CLI write, saves a browser edit and delivers one Chat message to an agent, each once and
with no navigation. A device revoked while the replacement is down still ends with the access
card.

`connection-readmit.spec.ts` (#2557) opens an owner page, advances the page epoch by adding and
removing a share link, and expects the same tab, with no reload, to return to Live, read the next
`page write` and deliver a Chat message to an agent. An epoch advance or authority recheck
(`STALE_EPOCH`, `DENIED`, `EXPIRED`) on an open page reopens it; only a refusal that ends access is terminal, and the
rule is in [Owner-page readmission](../../../../extensions/tmt-colab/contracts/colab-v1.md#owner-page-readmission-2557).

`connection-soak.spec.ts` (#2574) applies real page writes, Colab/Remote restarts, epoch and
grant-revision changes, offline recovery and detached grace expiry. Each trigger must leave the
same tab reading a new source, delivering one explicit Ask and showing its matching reply,
without reload or duplicate dispatch/wake. The capacity case evicts only a detached SDK session,
keeps attached pages usable and verifies a new tab's shipped refusal. Revoke ends access with
zero new effects; there is no unpair case because no public unpair action ships. For Colab
restart, the browser observes Reconnecting before replacement startup, then Live
before the next write; mounted stdout alone is not catch-up readiness. The case holds real
replacement frames to prove that distinction, and explicit Enter waits for the enabled Send
control. Socket closure is observed before the grace-duration wait; `idle-tab.spec.ts` separately
pins attached keepalive.
Chromium's offline emulation leaves established WebSockets open. Only the two offline cases
therefore pair it with a routed cut of both socket sides; online frames pass through unchanged
from the real server. The Colab restart separately holds replacement catch-up frames; all other
triggers use direct sockets.

`chat.spec.ts` (#1645) covers one null-anchor thread per asking device, two paired
viewers, page-visible history, Comments exclusion, exact follow-up context, retained
drafts, current-app-dir native CSP privacy and 1440/390 light/dark captures. Its
reload/restart checks count actual recipient rows and core dispatches, never terminal
echo. The restart suite can arm the next dispatch before Enter generates its ID,
then verifies that the parked operation matches the admitted Ask record.

## Storage lifecycle

`storage-lifecycle.spec.ts` proves the local object-storage flow on the real binaries; it adds
no product hook. Each row maps a behavior to its evidence (conformance names are in
`tmt-remote` `tests/objects/conformance.rs`, run for `local.rs` and `memory.rs`).

| Behavior                            | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Chat, annotation and Files attach   | `attachments.spec.ts` (Chat, document Files); `storage-lifecycle.spec.ts` annotation case with PNG/JPEG/WebP previews                                                                                                                                                                                                                                                                                                                       |
| Video and CSP                       | the same case: video is download-only, the CSP header is equal before and after, and no `blob:`/`unsafe-inline`, embed or video element appears; every object URL is revoked                                                                                                                                                                                                                                                                |
| Owner, reader, read-only, revoked   | `storage-lifecycle.spec.ts` principals case: owner, second paired device, read-only link; `devices revoke` and link removal end access; the owner's open page re-admits on the epoch change                                                                                                                                                                                                                                                 |
| Interruption, original-ID recovery  | `object-channel.spec.ts`; conformance `lost_acknowledgments_observe_the_original`, `restart_preserves_originals_and_progress`, `discard_closes_the_original`                                                                                                                                                                                                                                                                                |
| Quota                               | conformance level only: `entries_are_reserved_before_bytes_at_every_level`, `bytes_saturate_exactly_at_every_level`, `many_tiny_and_empty_objects_are_charged_each`, `retained_identities_are_never_evicted`                                                                                                                                                                                                                                |
| Deletion, epoch change              | `storage-lifecycle.spec.ts` cross-resource case (`COLAB_PAGE_DELETED` for export and read after `delete --yes`); native `archive_preserves_reads_and_delete_denies_admission_catchup_and_queued_delivery`; a rotated page re-seals its document attachments under the new epoch (`attachments.spec.ts` rekey case, #2360), message attachments keep theirs                                                                                  |
| Cross-resource refusal              | the same case: another page's document/message reference, a forged descriptor hash and a forged message ID are refused with their pinned codes; nothing is written                                                                                                                                                                                                                                                                          |
| Staging expiry vs page expiry       | staging: conformance `expiry_closes_staging_for_good`; page expiry is advisory only: native `expiry.rs` `verified_projection_reports_the_cloud_expiry_the_last_confirmed_page_row_made` (the `not-in-cloud` warning leaves content and edit authority) and the expired-retention step of `registration.rs` `authenticated_attachment_reads_survive_rotation_archive_and_reject_stale_disclosure` (archived attachment reads still open)     |
| No completed-object expiry or GC    | conformance `completed_objects_do_not_expire`, `namespace_removal_closes_for_good`                                                                                                                                                                                                                                                                                                                                                          |
| Configuration names backend, bounds | `object-channel.spec.ts` `config`: backend `local-fs` (default, not editable), capabilities, `payloadBytes` 12 MiB, `chunkBytes` 32 KiB                                                                                                                                                                                                                                                                                                     |
| Export and native temp-file cleanup | the cross-resource case: output dir unchanged and no `.tmt-colab-export-*` staging after every refusal; the harness leak report fails a passing run that leaks a process, tmux server or socket. a timed-out test is covered too (#2365): `disposeActiveWorlds` throws the leak report, a disposed world refuses late children, and `harness.spec.ts` drives those paths, though a worker Playwright kills outright can still leave a world |

The clock cannot move on a real binary, so staging expiry and quota stay at conformance level.

### Frozen-binary protocol

1. Build the SPA and `tmt-cli`, `tmt-remote` and `tmt-colab` once from the delivered head and
   copy the three binaries into a dedicated directory; record the head, each binary's SHA256 and
   the served asset hashes.
2. Run `storage-lifecycle`, `attachments`, `object-channel` and `export` twice with
   `TMT_ACCEPTANCE_BIN_DIR` set to that directory and one Playwright worker.
3. Never rerun blindly or relax an assertion. Record an original failure with its command and
   output apart from the evidence of the fixing head.
4. Capture Files, Chat attachments and Export at 1440/390 in light and dark for UX review.
