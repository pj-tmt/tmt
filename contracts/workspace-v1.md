# Workspace recovery v1

Workspace documents are private recovery input. They never prove presence,
identity ownership, delivery readiness or permission to launch. Every restore
rechecks current native host and identity evidence before its own effects.

## Documents and publication

`ConfigPaths` owns `<global_dir>/workspace/<sha256(socket-bytes)>/`. Each socket
has one stable, nonblocking `publication.lock`. Capture skips a busy lock rather
than queuing work. The directory and documents must be owned by the user and
private; symlinks, malformed documents and uncertain ownership are refused.

The existing snapshot codec remains version 1, bounded to 1 MiB and the existing
pane limit. It records capture time, the exact socket and native server
incarnation, session/window links, names, layouts, dimensions, selection, pane
geometry and cwd. Optional identity annotations contain UUID and remembered
session coordinates; external-command annotations contain verified literal TMT
argv. Neither transcript, provider environment nor credentials are recorded.

Automatic generations have immutable `auto-<sha256(bytes)>.json` names. Their
public selector is `g:<sha256(bytes)>`. A separate version 1 `generations.json`
index records publication order, capture time, pane and unique identity counts,
shrink classification and the protected generation. These counts describe
recovery content, never live agents. A valid legacy `latest.json` is adopted
before its first replacement. Snapshot codec v1 stays readable.

A generation's exclusive private stage is written and file-synced before it is
published without replacement; the directory is then synced. Only complete
immutable bytes may enter the index. The index is staged, file-synced, renamed
and directory-synced before the complete legacy `latest.json` projection is
updated and pruning begins. A crash may leave an unreferenced generation or an
old legacy projection, but the committed index names complete documents. A
reader does not fall back from an invalid index to uncertain legacy bytes.
Foreign files, symlinks and unverified fragments are preserved.

Canonical recovery content excludes capture time when checking for change. An
unchanged capture does not replace a generation, index, legacy projection or
recovery staging file, and preserves their bytes, inodes and timestamps.

## Capture triggers and public backstop

Capture rides admitted binding, unbinding, launch and lifecycle transitions and
provider hook invocations. No global tmux settings, server hooks, scheduled tmux
job, Core timer, daemon or detached worker is installed. Provider context is
written and flushed before optional capture, with existing cleanup/output
headroom. Unadmitted callbacks and the storage-only ToolFinished no-op shortcut
do not request probes. Capture errors never change the admitted hook's result.

Provider hooks coalesce to one attempt per 30 seconds per socket and native
PID/start incarnation, through the publication try-lock. `capture-attempt.json`
is a private, ephemeral version 1 coordination record containing `pid`, `start`
and `attemptedAtMs`. It alone is excluded from the no-write claim for unchanged
recovery content. It is replaced at most once per due interval, including failed
attempts; concurrent or not-yet-due calls do not rewrite it. A new incarnation or
backwards wall clock admits one fresh attempt and then coalesces again. Invalid
or symlinked coordination is refused unchanged. No generation is published unless
canonical topology or annotations changed; a layout-only change need not advance
the Core SQL cursor, so a due attempt reads topology once to learn its fingerprint.

Public API `workspace.capture` accepts only an empty `input` object, with no
identity, originator, socket, host selector or timeout override. It returns
`{attempted,published,skipped}`. It examines at most 64 entries under the private
workspace root and considers only recognized, owned socket records. Counts refer
to those examined records: `attempted` counts admitted capture callbacks (including
failed/unchanged attempts), `published` counts changed generations, and `skipped`
counts recognized records refused before their callback. It is advisory, not an
exhaustive host inventory or confirmation that a snapshot is current.

All candidates share one 200 ms acquisition deadline and the same 30-second
coordination as hooks. Before querying a recorded socket, verify its saved native
PID/start; after acquisition, require the same server identity/socket/incarnation.
An absent root returns zero counts without initialization. Unknown, stopped,
replaced, malformed or busy candidates never enroll a server, start one or adopt
its current metadata. The API opens no Core write store and creates no timer.

A running Ops minute worker may consume this optional API after feature discovery,
with cancellation and no paint/input work or visible output. Its [Ops consumer](https://github.com/pj-tmt/tmt/issues/4205) ships
separately; an absent/older Core must remain a silent no-op. The existing command
refresh setting and binding/launch event triggers retain their own policy.

## Retention and shrink protection

Retain the union of the last 10 distinct publications, the newest publication
in each of the last 24 UTC hour buckets, and one protected baseline: at most 35
unique automatic generations. Publication order remains authoritative if the
wall clock moves backwards. Pruning happens only after the new index commits,
and removes only owned, unreferenced, valid documents whose exact content digest
matches their automatic filename. A later update can remove complete crash
orphans under the same lock. Manual saves are independent of this bound.

Compare each capture with the protected baseline (or the preceding capture
before a baseline exists). Losing at least half of either nonzero pane count or
unique annotated identity count marks a shrink. The smaller capture is retained,
while the baseline remains pinned across server restarts, repeated small
captures and hourly expiry. An equally rich capture replaces the baseline. A
completed explicit generation restore may accept a smaller baseline, only if
the baseline has not changed since acquisition; it never overwrites a newer
concurrent recovery choice.

## Selection and consent

`tmt workspace ls [--socket PATH] [--json]` lists retained automatic generations
in reverse publication order, then manual names. Legacy stores without an
index list latest first. Each JSON entry includes `name`, `generation`, `digest`,
`capturedAtMs`, `panes`, `agents`, `protected` and `shrink`; manual entries have no
generation selector. Invalid recognized documents fail rather than appearing
usable. No list or preview captures, initializes or repairs storage.

`show [<name-or-generation>]` and `restore [<name-or-generation>]` acquire one
bounded document and its digest once. An explicit generation must exist in the
committed index; it never falls back to another generation. Without a selector,
choose the newest agent-bearing generation, preferring the protected baseline
when the newer choice is a shrink. If no generation has agent annotations,
full automatic restore refuses with `WORKSPACE_NO_AGENT_GENERATION`; a user may
choose layout-only recovery explicitly. Legacy and manually named snapshots
retain their existing decoding and name semantics.

A choice smaller than the protected baseline requires interactive confirmation
before any identity or host effect. JSON and noninteractive calls return
`WORKSPACE_SMALLER_CONSENT_REQUIRED` with the exact explicit selection and
`--expect <digest>` retry. An explicit selector with its matching digest supplies
consent; a mismatched digest returns `WORKSPACE_SNAPSHOT_CHANGED` before effects.
A declined choice restores nothing. No broad consent flag is added.

Version 1 list JSON is `{version,socket,snapshots}`. Save adds its entry fields
to `{version,socket}`; remove returns `{version,socket,name,removed:true}`. Missing
explicit selections use `WORKSPACE_SNAPSHOT_NOT_FOUND` (exit 3). Busy manual
publication is `WORKSPACE_BUSY` (exit 5), duplicate names are
`WORKSPACE_SNAPSHOT_EXISTS` (exit 5), and the manual cap is
`WORKSPACE_SNAPSHOT_LIMIT` (exit 1). Digest mismatch is exit 5. Invalid selector
or digest syntax remains a parser `USAGE_ERROR`.

The digest is lowercase SHA-256 of the exact acquired bytes, including
whitespace, not a re-encoding. There is no lock held through launch or new token
store. An explicit socket selects tmux recovery outside its original pane;
implicit external-host recovery returns `WORKSPACE_HOST_UNSUPPORTED`, never an
empty inventory or default-host fallback.

Manual names contain 1–64 lowercase ASCII letters, digits, underscores or
hyphens, starting with a letter or digit; `latest` is reserved. Manual
`save <name>` still copies the published legacy projection byte-for-byte,
without refreshing it. Its 16-name cap, no-replacement publication, explicit
removal and names disjoint from `g:` are unchanged. Manual saves cannot be
removed by automatic retention.

## Restore authority

Preview and restoration keep the existing native incarnation, socket, layout,
UUID and current-session fences. Existing sessions are skipped whole. Ordinary
panes preserve the user's shell; full restore uses ordinary resume or generic
external dispatch only in invocation-created panes. Missing, renamed, stale or
sessionless identities need the user. Snapshot names never resurrect another
UUID or complete an exchange. Partial effects are retained and reported; a
partial restore cannot accept a smaller protected baseline.

The implementation and verification owners are described in the
[workspace runtime reference](../.agents/skills/tmt-core-runtime/references/workspace.md).
