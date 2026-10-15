# Driver protocol v1

A driver is an executable that tells TMT about something TMT does not build
in. A host driver (`kind: "host"`) knows a terminal host, for example Herdr.
A runtime driver (`kind: "runtime"`) knows a coding agent: how to recognize
its command, read its hooks, resume its sessions and find its files. This
document owns the wire format. The `tmt-driver-protocol` crate encodes it,
and guides link here instead of repeating it.

**Status:**

- **Host drivers:** the format, the crate, the approval registry and the
  client that runs one driver call exist. Core uses an approved driver for its
  host: it finds the caller's pane through `caller` and explicit targets
  through `resolve-target`, lists bindings through `snapshot`, keeps markers
  with `publish` and `clear`, records the server through `server`, and
  delivers, reads and focuses through `prompt`, `input`, `capture` and
  `focus`. Users approve, list and remove drivers with
  `tmt driver install|ls|rm`. Herdr is served only by its first-party driver,
  `tmt-driver-herdr`, which ships in the CLI release (#1082). TMT approves
  that shipped driver itself on first use in a Herdr pane, unless the user
  removed it. The Herdr driver implements `name-agent` and a pane's
  `agentName`. Core calls `name-agent` after an explicit bind or rename and
  before it clears a marker; it shows `agentName` in `tmt ls` (#2892).
- **Runtime drivers:** the format, the crate's encoding and the conformance
  harness, approval registry and bounded client exist. `tmt driver install|ls|rm`
  accepts either kind. Runtime approval calls `locations`, validates its write
  targets and discloses them before consent. Runtime recognition, launch,
  hooks and setup/detection integration are not wired yet (#1266).

## Invocation

```sh
tmt-driver-<name> __tmt-driver <protocol> <op>
```

- **Request:** core writes one JSON request on stdin and closes it.
- **Answer:** the driver prints one JSON answer on stdout and exits 0.
- **Malformed invocation:** a wrong subcommand, protocol or arity still gets an
  error answer, with exit status 2.
- **stderr** is discarded.
- **Arguments and environment:** core passes no other arguments. The driver
  runs with core's environment, plus `TMT_DRIVER_CALL=1`. A `tmt` that sees
  `TMT_DRIVER_CALL` runs no command except help and `--version`; anything
  else fails with `DRIVER_CALL_REFUSED` before any effect. A driver that runs
  `tmt` can therefore neither recurse into itself nor change TMT's state.
  Core runs the driver in its own working directory, so relative environment
  paths resolve against the invoking TMT process's cwd.
  `TMT_DRIVER_CALL` belongs to the call only: a driver must not pass it to a
  process that outlives the call, such as a host server it starts.

## Answers

A driver prints exactly one of these:

```json
{"ok": { … }}
{"error": {"code": "not_found", "message": "The pane closed."}}
```

Error codes:

| Code | Meaning |
|---|---|
| `unsupported` | The driver does not implement the operation. Core falls back as for a host without it: a message that cannot be pasted goes to the inbox. |
| `bad_request` | The request did not parse or broke this contract. |
| `unavailable` | The host server cannot be reached. |
| `not_found` | The pane or server is gone, or is no longer the one asked about. |
| `failed` | Anything else. |
| `no_agent` | `prompt` and `name-agent` only: the host recognizes no agent in the pane. No text was written and no agent name was set. |
| `blocked` | `prompt` only: the agent waits on its user, for an approval or an answer. Nothing was written. |
| `not_ready` | `prompt` and `name-agent` only: the agent can't take a prompt or a name now, for example because it isn't in the foreground or is still starting. No text was written and no agent name was set. |
| `invalid_name` | `name-agent` only: the host does not accept this agent name. Nothing was changed. |
| `name_taken` | `name-agent` only: another agent on the server has this agent name. Nothing was changed. |
| `foreign_name` | `name-agent` only: the driver observed an agent name that TMT did not set. Nothing was changed. |

`no_agent` and `not_ready` are answers of `prompt` and `name-agent` alone,
`blocked` of `prompt` alone, and `invalid_name`, `name_taken` and
`foreign_name` of `name-agent` alone. From any other operation they are a
failure.

- `message` is at most 512 bytes and contains no control characters. Core shows
  it as untrusted text.
- Anything else is a failure: an answer that is not exactly one such object, is
  over its bound, or is late. Core never uses part of an answer.

Every request carries `deadlineMs`, the time the driver has left. Core kills a
driver that runs past its operation's deadline.

| Operation | Deadline | Answer bound |
|---|---|---|
| `capabilities` | 1 s | 4 KiB |
| `caller`, `server`, `resolve-target`, `publish`, `clear`, `focus` | 1 s | 4 KiB |
| `snapshot`, `probe`, `capture` | 2 s | 1 MiB |
| `input`, `prompt`, `name-agent` | 2 s | 4 KiB |

Requests are at most 1 MiB.

**Member names:** members are camelCase. Both sides ignore members they don't
know. A protocol-1 addition is therefore optional by construction; a new
required member needs protocol 2.

## Versions and the compatibility window

`capabilities` lists every protocol the driver speaks. Core uses the highest
protocol that both sides speak. Each `tmt` release speaks the current protocol
and the previous one. When protocol 2 ships, protocol 1 stays supported for at
least six months and at least one more minor `tmt` release, whichever is
later. Host and runtime drivers share the protocol number.

## Operations

These are a host driver's operations. A runtime driver has its own, under
[Runtime drivers](#runtime-drivers).

### `capabilities`

The request is `{}`. The answer:

```json
{"protocols": [1], "kind": "host", "name": "herdr", "version": "0.1.0",
 "ops": ["caller", "server", "resolve-target", "snapshot", "probe", "publish", "clear"],
 "paneId": {"prefix": "term_"}, "target": "w{n}:p{n}",
 "callerEnv": ["HERDR_PANE_ID", "HERDR_SOCKET_PATH"]}
```

- **`kind`:** `host`.
- **`name`:** `[a-z][a-z0-9-]{0,31}`. It is also the host token stored with a
  binding.
- **`version`:** 1–64 bytes of text.
- **`ops`:** the operations the driver implements besides `capabilities`. Any
  other operation answers `unsupported`.
- **`paneId.prefix`:** 2–16 characters of `[a-z0-9_-]`. It starts with a letter
  and ends in `_` or `-`. A pane ID is the prefix followed by 1–64 characters of
  `[0-9a-z]`. The suffix has no `_` or `-`, so every pane ID has exactly one
  prefix.
- **`target`:** how a user names a pane, or `null` when panes have no public
  name. It is a template of up to 32 characters: literal `[a-z:]` and `{n}`,
  where `{n}` stands for 1–9 characters of `[0-9A-Z]`. It starts with a letter,
  holds at least one `{n}`, and never has two `{n}` in a row. A target is
  case-sensitive: text with a lowercase letter where a `{n}` stands does not
  match this declared syntax.
- **`callerEnv`:** at most 4 environment variables that `caller` reads, each
  `[A-Z][A-Z0-9_]*` and never `TMT_*`.

Installation refuses a driver when any of these hold:

- its declaration is invalid;
- it uses a built-in's name;
- its name or prefix is already used by another installed driver;
- its sample target (every `{n}` as `1`) is a target of tmux or of another
  driver, or the reverse.

### `caller`

- **Request:** `{"env": {"HERDR_PANE_ID": "w1:p2", …}}`. It holds only the
  declared variables that are set.
- **Answer:** `{"pane": {"id", "socket", "shellPid"}}`, or `{"pane": null}` when
  the environment names no pane of this host.

Core counts the pane only when `shellPid` is an ancestor of the caller, which it
checks itself.

### `server`

- **Request:** `{"socket": "/path" | null}`. `null` asks for the driver's
  default server.
- **Answer:** `{"server": {"socket", "pid", "startTime"}}`, or `{"server": null}`
  when no server runs there.

Core gives each incarnation its own server UUID. The incarnation is the
socket plus core's own observation of `pid`: the process's start, as core
reads it. `startTime` is advisory; core never stores it or sends it back. A
server whose process core cannot observe stays unresolved, and a restarted
server is a new incarnation.

### `resolve-target`

- **Request:** `{"socket", "target"}`.
- **Answer:** `{"paneId": "…" | null}`.

### `snapshot`

- **Request:** `{"socket", "panes": ["…"] | null}`. `null` asks for every pane.
- **Answer:** `{"panes": [Pane]}`, listing only panes that were asked about.
  There are at most 4096 panes, and each ID appears once.

A `Pane`:

```json
{"id": "term_7", "target": "w1:p2", "cwd": "/src", "command": "claude",
 "panePid": 4242, "suggestedName": null, "agentName": "reviewer",
 "marker": Marker | null}
```

- **`id` and `target`:** follow the declared syntax.
- **`cwd`:** an absolute path, or `null`.
- **`command`:** the foreground command's name, at most 256 bytes.
- **`panePid`:** the pane's shell. Like every pid, it is in 1 through 2^53 − 1.
- **`agentName`:** optional. The name the host gives the agent it recognizes
  in the pane, 1–256 bytes. It is `null` or absent when the host has no agent
  names (tmux), recognizes no agent in the pane, the agent has no name, or
  the driver could not read it. A driver that cannot read agent names still
  answers the snapshot. It is the name of the agent, never a label of the
  pane, and it is not evidence: it proves no binding and names no identity.
  Core only shows it; it never copies a host's name into an identity.
- **Text:** all text is free of control characters.

### `probe`

- **Request:** `{"server": {"socket", "pid", "startTime"}, "panes": ["…"]}`.
- **Answer:** one of:
  - `{"state": "live", "panes": [Pane]}`, where the panes are among those asked
    about;
  - `{"state": "dead"}`;
  - `{"state": "unknown"}`.

`dead` means the incarnation is gone. Core believes it only when its own check
of the recorded server process agrees, and treats it as `unknown` otherwise.

`probe` is optional. Core leads every probe itself and doesn't call this
operation: it checks the recorded server process. If that process is gone, or
another process now has its pid, the server is dead. If it is the same
process, the driver's `snapshot` of the scoped panes on that socket decides
which panes are live. If core can't tell, the probe is `unknown`. Unknown
never proves loss.

### `publish` and `clear`

A marker lets any process see that a pane is bound:

```json
{"name", "canonicalName", "identityId", "bindingId", "serverId", "panePid"}
```

The IDs are UUIDs. The driver stores the marker on the pane and returns it
unchanged in `snapshot` and `probe`. It never interprets the marker.

- **`publish`:**
  - request: `{"socket", "paneId", "panePid", "marker"}`;
  - answer: `{}`;
  - the driver refuses with `not_found` unless the pane still exists and runs
    `panePid`.
- **`clear`:**
  - request: `{"socket", "paneId", "bindingId"}`;
  - answer: `{"cleared": bool}`;
  - the driver removes a marker only when it carries that `bindingId`; a missing
    pane is `{"cleared": false}`.

### `capture`, `input` and `focus`

| Operation | Request | Answer |
|---|---|---|
| `capture` | `{"socket", "paneId", "lines"}` | `{"text"}` |
| `input` | `{"socket", "paneId", "text", "enter"}` | `{}` |
| `focus` | `{"socket", "paneId"}` | `{}` |

- **`input`:** the driver pastes `text` literally, then presses Enter when
  `enter` is true. With `enter: false` it must not submit anything: core
  stages a message as `input(text, enter: false)`, its delay, then
  `input("", enter: true)`, and a driver that submitted on the first call
  would deliver before core's delay. Core has already applied its delivery
  policy (`!` protection, staging, size), so the driver adds and interprets
  nothing.
- **Missing pane:** each of these answers `not_found`.

**Core directional focus.** `tmt focus <target> --into <left|right|up|down>` is a
Core host action, not an external-driver wire operation. External drivers,
including Herdr, default to `HOST_UNSUPPORTED`. On tmux, Core verifies the caller
pane and target on the same server, reads the caller window layout once, and
swaps the target with its visible directional neighbour using `swap-pane -d`.
The caller keeps focus and no pane is killed; the target may be in another window.
Touching edges cross one border cell and perpendicular spans must overlap. Core
prefers the neighbour containing the caller midpoint, then largest overlap, then
smallest left/top. It never wraps and refuses a zoomed window as no neighbour.
Success is `{"swapped":{"pane":"<target>"},"with":{"pane":"<neighbour>"}}`;
already beside is `{"swapped":null,"with":{"pane":"<neighbour>"},"already":true}`.
Refusals use the ordinary focus error envelope: `HOST_UNSUPPORTED` for an
unsupported host, no caller pane or the caller as target; `PANE_NOT_FOUND` for a
missing target or visible neighbour. Binding and evidence errors retain focus's
existing codes. A name or UUID on another server remains inactive.

**Delivery outcome.** An `input` answer tells core whether text reached the
pane:

- **Not sent:** the answer is `unsupported`, `not_found`, or `bad_request`. A
  driver checks the request completely before any effect, so `bad_request`
  always means nothing was pasted.
- **Sent:** the answer is `{}`.
- **Uncertain:** anything else. That covers `failed`, `unavailable`, a timeout,
  a killed driver, and an answer that doesn't decode. Core reports
  `DELIVERY_UNCERTAIN` and never retries.

A driver never retries an `input` itself: a second paste could duplicate the
message. A driver that also lists `prompt` gets an `input` only after a
`prompt` answered `no_agent` or `unsupported`.

**Paste, then Enter.** Core may paste and press Enter in one call
(`enter: true`). Core may also stage them: `input(text, enter: false)`, then
after its paste-to-Enter delay `input("", enter: true)`. The delay stays core
policy, and a driver adds no delay of its own.

### `prompt`

| Operation | Request | Answer |
|---|---|---|
| `prompt` | `{"socket", "paneId", "text"}` | `{}` |

`prompt` is optional. Core calls it only when `capabilities.ops` lists it.

- **What it does:** the driver hands `text` to the agent the host recognizes in
  the pane, and the host submits it, pasting and pressing Enter in one step.
  Core has already applied its delivery policy (`!` protection, size), so the
  driver adds and interprets nothing, as for `input`. There is no `enter`
  member and no staging.
- **Size:** `text` is bound by the same limits as `input`'s `text`: core's
  message size policy and the 1 MiB request bound.
- **Order:** core sends a `prompt` only after the evidence and runtime checks it
  makes before an `input`. It falls back to `input` only when the answer is
  `no_agent` or `unsupported`.

**Delivery outcome.** A `prompt` answer tells core whether the agent got the
text:

- **Sent:** the answer is `{}`.
- **Not sent, core falls back to `input`:** the answer is `no_agent` or
  `unsupported`.
- **Not sent, final:** the answer is `blocked` (core reports the agent as
  awaiting approval), `not_ready`, `not_found` or `bad_request`. Core never
  types around an agent that refused.
- **Uncertain:** anything else. That covers `failed`, `unavailable`, a timeout,
  a killed driver, and an answer that doesn't decode. Core reports
  `DELIVERY_UNCERTAIN` and never retries.

A driver never retries a `prompt` itself and never falls back to raw input on
its own. Falling back is core's decision, because only core holds the runtime
evidence. Each request makes one attempt; a `blocked` agent is not prompted
again until a new request.

### `name-agent`

| Operation | Request | Answer |
|---|---|---|
| `name-agent` | `{"socket", "paneId", "panePid", "bindingId", "name"}` | `{}` |

`name-agent` is optional. Core calls it only when `capabilities.ops` lists it.
A host without agent names does not list it.

- **What it does:** with a `name`, the driver gives that agent name to the
  agent the host recognizes in the pane. With `name: null`, it takes TMT's
  agent name away. It changes the host's agent name only. A host may also
  have a label for the pane itself; a driver never reads or writes that label
  for this operation.
- **Fence:** the driver refuses with `not_found` unless the pane still exists,
  runs `panePid` and carries a marker with `bindingId`. Core publishes the
  marker first and takes the name away before it clears the marker.
- **When core calls it:** with the identity's name after a committed explicit
  bind (`tmt name`, `this`, `add`, `marked`) and after `tmt mv`; with `null`
  just before `clear`. `tmt run`, `tmt resume`, read commands and provider
  hooks never call it. Every answer leaves the binding as it is: after a bind or
  rename core shows a refusal in human output as one warning with its fix, and before
  `clear` it goes on with the clear whatever the answer, except when the
  driver process could not be cleaned up, which stops the clear. The call
  before `clear` gets at most half of the time left in the command's budget.
- **Ownership:** the driver keeps a record on the pane of the agent name the
  host confirmed for this `bindingId`, and treats only that name as TMT's. A
  record of another binding is no record. When the driver observes an agent
  name that differs from the record, someone else set it: the driver answers
  `foreign_name` and does not change it. This holds for `name: null` too;
  core then leaves that name and goes on with its own work, such as `clear`.
  When the agent already has the requested name, the answer is `{}` and
  nothing is written; without a record that name stays someone else's, so a
  later `name: null` answers `foreign_name`. `clear` removes the record with
  the marker and leaves the agent name.
- **Order:** the driver reads the agents first and the fence last, then
  renames, and writes the record only after the host confirmed the rename.
  A refused name therefore leaves no record, and a request's intent never
  becomes ownership. To replace TMT's own name, the driver renames the old
  name to the new one in one step, so a refusal leaves the old name and its
  record. To take the name away, it removes the name first and the record
  after it. When the record fails after a confirmed rename, the answer tells
  where it failed. If the write or the read-back itself fails, the answer is
  the code the host's error maps to (such as `failed`, or `not_found` for a
  pane that is gone), and the record's state is unknown, because the host may
  have stored it. If the read-back shows the pane bound again to another
  binding, the answer is `not_found`. If the read-back shows no record of
  this binding, the answer is `failed`: the record is confirmed missing, so
  the name is set but not TMT's, and TMT does not change it again. In all
  cases the next request decides ownership from a fresh read of the pane.
- **Evidence:** the driver changes a name only when the host's list says
  exactly what the pane hosts. A list that it cannot read, that has more than
  one agent for the pane, or whose name for the pane is not a valid agent
  name answers `failed`, and nothing is written. At the record's read-back
  the driver checks again that the marker carries `bindingId`; a pane bound
  again meanwhile answers `not_found`.
- **Repeat:** after a successful request, the same request with unchanged
  evidence changes nothing and answers `{}`. Core may repeat a request after
  any answer; a request that was refused may be refused again.
- **`name: null`:** `{}` means the agent has no TMT agent name now: it was
  removed, the agent had none, or the host recognizes no agent in the pane.
- **Limits (best effort):** these guards use what the driver observes, and a
  host may give it less than a guarantee needs.
  - A host may have no compare-and-set for agent names. The driver reads the
    agents, then the pane and its shell, then renames: a rename by someone
    else after the agent list was read (two host calls before the rename) is
    not seen, and the driver's rename replaces it. The driver never widens
    that window with a retry.
  - A host may give no stable identity for one agent's lifetime. When an
    agent exits and another starts in the same pane under the same binding,
    and someone gives that agent the name in the record, the driver takes
    the name as TMT's. The same holds when a name was taken away but its
    record could not be removed.

Answers with a `name`:

| Answer | Meaning |
|---|---|
| `{}` | The agent has this agent name. |
| `invalid_name` | The host's rule for agent names refuses it. |
| `name_taken` | Another agent on the server has it. |
| `no_agent` | The host recognizes no agent in the pane, so there is nothing to name. |
| `not_ready` | The agent is still starting. |
| `foreign_name` | The agent has another agent name that TMT did not set. |
| `not_found` | The fence failed, or the pane was bound again before the record was read back. |
| `failed` | The driver could not read exact evidence, or the host set the name but did not keep the record. |

With `name: null` the answers are `{}`, `foreign_name`, `no_agent` (the
agent went away between the read and the rename `--clear`), `not_ready`,
`not_found` and `failed`.

The Herdr driver runs `herdr agent rename <target> <name>` or `--clear`, and
reads names with `herdr agent list`. A Herdr agent name matches
`[a-z][a-z0-9_-]{0,31}`; the driver checks that rule before any call. Herdr
removes an agent name itself when the agent exits or is replaced. The record
is the pane token `tmt_agent_name` under the marker's source, with the value
`<bindingId>:<name>`. Herdr 0.9.3 has no compare-and-set for agent names and
lists no stable identity for an agent's lifetime, so both limits above apply.
The calls are: `agent list`, `pane list`, `pane process-info`, then
`agent rename`, then `pane report-metadata` and a `pane list` read-back.

## Runtime drivers

A runtime driver tells TMT about one coding agent. Most of what core needs
happens on an agent's hook, where every call to a new process would cost 25
to 300 ms of a 2 s budget. So the driver declares that part once, in
`capabilities`, and core applies the declaration itself without starting the
driver: recognizing the agent's command, naming the caller's session and
decoding hooks. Core runs the driver only for the work a declaration can't
describe: finding the agent's files, building a resume command and reading
usage from a transcript.

A runtime driver delivers nothing. Core delivers to an agent through its
host, with its own delivery policy. Channels such as Claude's MCP channel and
Codex's app-server stay built in.

### Runtime `capabilities`

```json
{"protocols": [1], "kind": "runtime", "name": "kimi", "version": "0.1.0",
 "ops": ["locations", "resume", "usage"],
 "executables": ["kimi"],
 "env": ["KIMI_HOME"],
 "sessionEnv": "KIMI_SESSION_ID",
 "hooks": Hooks | null}
```

- **`kind`:** `runtime`.
- **`name`, `version` and `protocols`:** as for a host driver. The name is
  also the harness ID stored with a session.
- **`ops`:** `locations` is required; `resume` and `usage` are optional.
  `usage` needs `hooks` with an `idle` event and a `transcript` field.
- **`executables`:** 1 to 4 distinct bare command names, each 1–64 of
  `[a-z0-9._-]` and starting with a letter or digit. A pane command whose last
  path component equals one of them is this agent. `tmt run` with no command
  starts the first.
- **`claims`:** optional boolean, default `true`. `false` disables command
  recognition without changing the declared operations; skills-only drivers
  use it to avoid claiming a runtime.
- **`env`:** at most 4 environment variables that `locations` reads, each
  `[A-Z][A-Z0-9_]*` and never `TMT_*`.
- **`sessionEnv`:** the variable that holds the caller's provider session ID
  in the agent's environment, or `null`. It follows the `env` rules. Core
  reads it itself. A session ID is 1–256 bytes, not blank, with no control
  characters.

A `Hooks` declaration:

```json
{"format": "sessionHooksJson",
 "fields": {"event": "/hook_event_name", "session": "/session_id",
            "model": "/model", "transcript": "/transcript_path",
            "turn": "/turn_id"},
 "events": [
   {"name": "SessionStart", "effect": "start", "by": "/source",
    "values": {"startup": "started", "resume": "resumed", "clear": "cleared",
               "compact": "compacted", "fork": "forked"}},
   {"name": "SessionEnd", "effect": "end", "by": "/reason",
    "values": {"clear": "cleared", "resume": "resumed", "logout": "ended"}},
   {"name": "UserPromptSubmit", "effect": "working"},
   {"name": "Stop", "effect": "idle"}]}
```

- **`format`:** `sessionHooksJson` only. `tmt setup` writes one command entry
  per declared event into the settings file that `locations` names, in the
  layout the built-in Claude and Codex hooks use. It asks before it writes,
  as it does for those.
- **`fields`:** where each value is in a hook's JSON payload, as JSON Pointers
  ([RFC 6901](https://www.rfc-editor.org/rfc/rfc6901)) of at most 128 bytes.
  `event` and `session` are required; `model`, `transcript` and `turn` are
  optional.
- **`events`:** 1 to 8, with distinct names of 1–64 characters of
  `[A-Za-z0-9_]`. Each has an `effect`:

| Effect | Meaning | `by` and `values` |
|---|---|---|
| `start` | A session began or changed. | Required. Each value is `started`, `resumed`, `cleared`, `compacted` or `forked`. |
| `end` | A provider session ended or changed; the runtime process may remain alive. | Required. Each value is `cleared`, `resumed` or `ended`. |
| `working` | A turn began. | Not allowed. |
| `idle` | A turn ended. | Not allowed. |

`by` is a JSON Pointer like the fields, and `values` holds 1 to 16 entries
whose keys are 1–64 bytes of text.

A verified provider end retains the exact process/session key and launch owner,
records its session transition, and leaves stored runtime readiness Unknown until a
fresh SessionStart admits the same live incarnation's next session. Only
conclusive process loss or owned-child exit makes the runtime Ended; provider
end never proves it. Stale session keys and unverified replacement processes
cannot change that observation.
Delivery probes can report Running after provider end (including legacy Ended)
only for an exact live process under the verified pane and an exact live launch
owner when recorded, without rewriting storage; shared servers outside that
ancestry remain unverified.

**Decoding a hook.** Core reads the payload: at most 64 KiB and one JSON
object. It finds the event whose `name` equals the string at `fields.event`,
and reads the session ID at `fields.session`. For `start` and `end` it maps
the string at `by` through `values`. Core ignores the payload, changing
nothing, when:

- no event matches;
- a required value is missing or isn't a valid string;
- a `by` value isn't in `values`.

An optional field that is missing or invalid reads as absent: `model` is 1–256
bytes of text, `transcript` an absolute path, and `turn` follows the session
ID rule. What a decoded event does to a binding is core's policy, the same as
for the built-in drivers' hooks.

### Runtime operations

| Operation | Deadline | Answer bound |
|---|---|---|
| `locations` | 1 s | 4 KiB |
| `resume` | 1 s | 4 KiB |
| `usage` | the hook's remaining time, at most 1 s | 4 KiB |

A driver answers `unsupported` to any operation it doesn't list, including
every host operation. The agent and name codes (`no_agent`, `blocked`,
`not_ready`, `invalid_name`, `name_taken` and `foreign_name`) are a failure
from these operations.

#### `locations`

- **Request:** `{"home": "/Users/ann", "env": {"KIMI_HOME": "~/kimi"}}`. `env`
  holds only the declared variables that are set.
- **Answer:**

```json
{"configDirs": ["/Users/ann/.kimi"], "skills": "/Users/ann/.kimi/skills",
 "hookSettings": "/Users/ann/.kimi/settings.json",
 "transcriptRoot": "/Users/ann/.kimi/sessions"}
```

Every path is absolute, at most 1024 bytes of text, with no `.` or `..`
component. `configDirs` holds 1 to 4 paths.

- **`hookSettings`:** required when `hooks` is declared, otherwise `null`. It
  lies inside one of `configDirs`.
- **`skills`:** lies inside one of `configDirs`, or is the skills root that
  agents share, `<home>/.agents/skills` (Codex, Gemini CLI and OpenCode read
  that one).
- **`transcriptRoot`:** required when `usage` is declared, otherwise `null`.

`tmt setup` writes to `skills` and `hookSettings`, so a driver can point it
only at the agent's own directories or the shared skills root.

Core uses each one:

- **`configDirs`:** the agent counts as installed when one exists.
- **`skills`:** where `tmt setup` installs TMT's skill.
- **`hookSettings`:** where `tmt setup` writes hooks.
- **`transcriptRoot`:** the only directory whose files core passes to `usage`.

#### `resume`

- **Request:** `{"session": "…", "model": "…" | null}`. `model` is only a
  model a hook reported for that session; otherwise it is `null`.
- **Answer:** `{"argv": ["kimi", "--resume", "…"]}`.

`argv` holds 1 to 64 strings without NUL. Its first is one of the declared
`executables`, as written there. Core starts it directly and never through a
shell. Any error answer, a timeout or an invalid `argv` means the resume
failed: core reports it and starts nothing. A failed resume never starts a
fresh session.

#### `usage`

- **Request:** `{"session": "…", "transcript": "/Users/ann/.kimi/sessions/x.jsonl"}`.
  `transcript` is the path an `idle` hook reported. Core has resolved it and
  checked that it lies inside `transcriptRoot`.
- **Answer:** `{"contextTokens": n | null}`, where n is 0 through 2^53 − 1.
  `null` means the transcript shows no usage.

An error answer or a timeout leaves the remembered usage unchanged.

### States and retries

A runtime driver keeps no state between calls. Core stores everything, so a
driver or `tmt` that is killed and started again loses nothing. A failed or
late operation is not a cancel: it changes no session state, and core never
repeats it on its own. Late output from a killed driver is discarded.

### Runtime trust boundary

Everything in the host [trust boundary](#trust-boundary) applies, and:

- core checks an `argv` against the declared `executables` before it starts
  anything, and checks every path against the rules above before using it;
- a driver receives only what each request lists: never message text,
  tokens, receipts or other identities' data;
- core alone decides what a decoded hook does to a binding, and keeps
  binding checks, storage, `!` protection, delivery and retry policy.

**Installation refuses a runtime driver when any of these hold:**

- its declaration is invalid;
- it uses a built-in driver's name, or another installed driver's;
- one of its executables is claimed by a built-in driver or by another
  installed runtime driver, when `claims` is true. In this protocol a runtime driver cannot take
  over a command that a built-in driver handles.

Approval also calls `locations` for the current home and declared environment,
and applies `LocationsResponse::within(home)` before accepting the skills target.
The shared locations client is the same admission boundary setup must use before
writing. The consent view lists executables, claims, the direct argv rule, operations,
hook events, `configDirs`, skills, hook settings, transcript root, declared `env`
names and `sessionEnv`. It never prints environment values. Approval writes no
provider files; setup retains its separate consent.

An upgrade that declares more (an executable, an environment variable, a hook
event, or an operation) reads as `changed` and isn't run
until it is approved again. For the first-party driver that release ships,
TMT approves it again itself when a command that works with hosts next runs,
and prints one line saying so; a first-party driver the user removed with
`tmt driver rm` is approved again only by `tmt driver install`.

## Trust boundary

Everything a driver prints is untrusted input. Core bounds it, parses it
strictly, and checks every ID, pid and string against the declared grammar
before use. Core also decides from its own evidence:

- a pane is bound only when core's check of the process identity matches;
- the server's and each pane shell's incarnation are core-observed: core takes
  its own start token for the pid a driver names and never stores or compares
  a driver's `startTime`;
- runtime and liveness come from core's own process inspection of the pane's
  shell (status is not a driver operation);
- a server's loss is proved only by core's own process check. A pane is lost
  only when a snapshot of that same, core-verified server omits it or shows
  another shell. A driver that is missing, changed, failing or out of time
  leaves a binding Unknown, never retired;
- deliveries are accepted only through core's receipt logic.

**What a driver receives:** it never receives tokens, receipts, requests or
other identities' data. `caller` receives only the variables the driver
declared.

**What a driver must not do:**

- open or write TMT's database or files;
- change consent;
- run as another user.

This is a contract that core's checks enforce at the boundary above. It is not
an operating-system sandbox: the driver runs as the user.

**Consent:** an installed driver is consented and fingerprinted like an
extension hook, in `<global>/drivers.json` (mode 0600, replaced atomically).
Approval requires a regular executable owned by the user, with neither it nor
its directory writable by anyone else. It records the path, the SHA-256
digest, the metadata fingerprint and the capabilities. Checks at use:

- **Every call:** ownership and the fingerprint (a stat).
- **Once per process:** the digest, on the driver's first use. Hashing a large
  driver on every call would cost more than the calls themselves.

A changed executable is not run until it is approved again. There is no PATH
discovery, and at most 16 drivers can be approved.

## Conformance

The crate's `conformance::check` runs a driver through any invoker, a spawned
executable or `serve` in process. It checks:

- the capabilities and the envelope;
- `unsupported` for undeclared operations;
- `bad_request` for malformed requests;
- deadlines and bounds;
- every declared operation, against a pane and a target that don't exist, so
  running it against a live server changes nothing there.

Checks that need a bound pane belong to the host's own fixture. A host
driver conforms when `check` reports no findings.

`conformance::check_runtime` does the same for a runtime driver. It runs every
operation, declared or not, and checks:

- the declaration, including its hooks;
- `locations` for a fixture home and an empty environment: every path must
  meet the rules, and the paths a declaration requires must be there;
- `resume` for a fixture session: either an `argv` that starts with a declared
  executable, or an error answer;
- `usage` for a transcript inside `transcriptRoot` that doesn't exist: either
  `null` or an error answer.

Nothing it asks changes the agent's files. A runtime driver conforms when
`check_runtime` reports no findings.
