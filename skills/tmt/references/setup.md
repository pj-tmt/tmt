# TMT reference: Provider setup and usage

Part of the tmt skill, printed by `tmt learn --skill tmt setup`.
The rules of the skill entry (`tmt learn --skill`) still apply.

### Provider setup and usage

`tmt setup` shows detected agents and four official extensions: ops, digest,
remote and colab, all selected by default with their bundled skills. In an
interactive terminal, use ↑/↓ and space to select extensions, then Enter to
review one approval for the complete plan. Colab needs Remote: selecting Colab
selects Remote visibly; unchecking Remote also unchecks Colab. Without a terminal
or with `--json`, setup never prompts; user-authorized `--yes` applies the four
defaults. Declining changes nothing. Healthy extensions are kept without an
upgrade; missing bundled skills are retried from the verified active release.
Independent items continue after a failure; completed work stays installed and
`tmt setup --yes` retries missing work. A failed Remote installation skips Colab.
JSON adds per-extension outcomes and a nonzero exit for incomplete setup.
Interrupting installation stops later items, keeps completed work and exits 130.
For read-only installation facts, use `tmt extension ls --json`: its four official
rows include installed state, active version, `missing` and `dependencies` (Colab
needs Remote). An unhealthy installation has recovery diagnostics; it is not
reported as missing. Listing never installs or starts a service.
Claude uses a nonempty `CLAUDE_CONFIG_DIR` for settings and skills, otherwise
`~/.claude`; relative roots resolve from the command's working directory.
Setup refuses symlinked settings without changing the link or target; review
and back up the target before editing it manually. New byte-exact recovery
copies stay in private `.tmt-setup-backups` beside settings. Above 32 copies,
setup warns with that directory and a manual cleanup hint but still publishes.
TMT never automatically deletes or migrates backups.
Timeout or uncertain evidence produces no identity claim. Session-only binding
is not supported; a hook never names, transfers or resurrects an identity.
Independent Codex sessions use verified pane/process evidence. Shared-server hooks
require an existing exact thread mapping and never inherit the server's pane
identity. Unmapped shared sessions get no context; a client disconnect is not a
thread-ended signal.

Consented `tmt setup` includes the Stop hook by default, exposing `resume.usage`
(context size) and optional `resume.consumption` in ls/identity JSON. It also keeps
the agent's recent answers locally (7 days, up to 1,000, 16 KiB each) for [`responses.recent`](https://github.com/tmt-ninja/tmt/blob/main/contracts/extension-api.md#recent-agent-responses):
these are the agent's own final answers for display, not delivered replies.
Answers already kept stop appearing when they expire; later collection
removes them. With `--no-usage` the Stop hook stays for the agent's answers only: no usage,
consumption or transcript data is read or stored. An install made with `--no-usage` before that
has no Stop hook until `tmt setup` runs again. With usage on, setup also keeps a small managed
Claude Code mod in Claude's skills root that hands its rate-limit windows to TMT, so allowance
figures stay fresh; `--no-usage`, `--remove` and uninstall remove it, and a directory that is
not TMT's is left alone. Consumption reports cumulative
completed-request counters; measure changes within an epoch. Cached input
already belongs to input. Foreground `tmt run`/`resume` observes accepted
completed-request evidence every five seconds during a turn. Listing never reads
provider files. `consumption.history` in `tmt api` returns bounded closed history (fine
buckets retained two hours and queried up to one hour; longer windows read the
[rollup](https://github.com/tmt-ninja/tmt/blob/main/contracts/extension-api.md#thirty-day-rollup);
32 UUIDs per batch) with an included cumulative seed watermark. Follow [the history contract](https://github.com/tmt-ninja/tmt/blob/main/contracts/extension-api.md#consumption-history)
for coverage, partial windows and avoiding overlap with live observations.
Use its epoch/sequence and completeness/gap evidence, never context-size
differences or a missing value as zero. Hook timestamps are not heartbeats.
