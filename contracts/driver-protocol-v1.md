# Driver protocol v1

A driver is an executable that tells TMT about a terminal host that TMT does
not build in, for example Herdr. This document owns the wire format. The
`tmt-driver-protocol` crate encodes it, and guides link here instead of
repeating it.

**Status:** the format, the crate, the approval registry and the client that
runs one driver call exist. Core uses an approved driver for its host: it
finds the caller's pane through `caller` and explicit targets through
`resolve-target`, lists bindings through `snapshot`, keeps markers with
`publish` and `clear`, records the server through `server`, and delivers,
reads and focuses through `prompt`, `input`, `capture` and `focus`. Users
approve, list and remove drivers with `tmt driver install|ls|rm`. Herdr is
served only by its first-party driver, `tmt-driver-herdr`, which ships in the
CLI release (#1082).

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
| `no_agent` | `prompt` only: the host recognizes no agent in the pane. Nothing was written. |
| `blocked` | `prompt` only: the agent waits on its user, for an approval or an answer. Nothing was written. |
| `not_ready` | `prompt` only: the agent can't take a prompt now, for example because it isn't in the foreground or is still starting. Nothing was written. |

`no_agent`, `blocked` and `not_ready` are answers of `prompt` alone. From any
other operation they are a failure.

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
| `input`, `prompt` | 2 s | 4 KiB |

Requests are at most 1 MiB.

**Member names:** members are camelCase. Both sides ignore members they don't
know. A protocol-1 addition is therefore optional by construction; a new
required member needs protocol 2.

## Versions and the compatibility window

`capabilities` lists every protocol the driver speaks. Core uses the highest
protocol that both sides speak. Each `tmt` release speaks the current protocol
and the previous one. When protocol 2 ships, protocol 1 stays supported for at
least one more minor `tmt` release.

## Operations

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
  where `{n}` stands for 1–9 digits. It starts with a letter, holds at least one
  `{n}`, and never has two `{n}` in a row.
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
 "panePid": 4242, "suggestedName": null, "marker": Marker | null}
```

- **`id` and `target`:** follow the declared syntax.
- **`cwd`:** an absolute path, or `null`.
- **`command`:** the foreground command's name, at most 256 bytes.
- **`panePid`:** the pane's shell. Like every pid, it is in 1 through 2^53 − 1.
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

Checks that need a bound pane belong to the host's own fixture. A driver
conforms when `check` reports no findings.
