# Candidate discovery v1

`tmt discover [--host HOST] [--socket PATH] [--json]` observes candidate panes.
It never registers identities, reconciles bindings, initializes server metadata,
installs hooks/drivers/runtimes or launches an agent. This slice supports tmux;
Herdr and other explicit hosts return `unsupported` without calling a driver.
Without explicit selection, the caller host is used. An explicit socket selects
that exact tmux server; there is no socket-directory search.

## Observation

JSON is `{version:1,observedAtMs,sources,candidates,truncated}`. Each source has
`host`, nullable `socket`, `status` (`ok`, `unsupported`, `permission_denied`,
`offline` or `unknown`) and nullable `reason`. Failure is data, not an empty
successful observation. Failed child cleanup is a command failure.

A candidate has `host`, `socket`, `server` (native PID/start and nullable TMT
server UUID), `session`, `window`, stable `pane`, host `target`, native pane
PID/start, `runtime`, nullable verified `identity` (UUID/name/binding UUID),
`state`, `reason`, `duplicate`, `display`, `adoptable` and nullable `expect`.
Missing native evidence has null server/native/expect and an explicit reason.
`runtime.driver` is an executable hint; `state` remains `unknown`, and its
`recordedState` is stored lifecycle evidence only. Neither a hook nor an argv
hint proves readiness or transfers authority. An explicit binding request may
select a native verified pane even when its runtime is unknown; binding does
not start that runtime or grant routing readiness.

`state` distinguishes `unknown`, `known`, `stale`, `hook_only` and `duplicate`.
Only a fresh marker/native observation agreeing with a stored binding supplies
`identity`. Conflicting stored owners, malformed ownership metadata and stale or
unverified markers refuse adoption. Linked occurrences of one identical pane
are deduplicated and marked `duplicate`; contradictory occurrences are unknown.
Display titles, executable text, working directories and host coordinates are
untrusted literal strings. JSON escapes framing characters, human output quotes
controls, and consumers must render text inert. Names never select identities.

A scan has one aggregate 2,000 ms observation deadline, at most 256 candidate
rows and at most 1 MiB of subprocess/result data. Display fields retain at most
1,024 UTF-8 bytes and flag `display.truncated`. Candidate/time/output exhaustion
sets `truncated` and a source reason; retained incomplete-scan rows have no
adoption expectation. Bounds include native process observation and existing
read-only storage; no database is created or migrated. Native children use the
bounded process owner's termination/reaping, including on failure. No detached
Core worker or persisted scan token is created. A consumer starts this finite
CLI in its background worker and discards superseded generations; the Ops UI
owns its paint/input responsiveness tests.

## Guarded existing binder

```
tmt add %3 worker --expect <digest> [--socket PATH] [-s] [--json]
tmt add %3 --identity <uuid> --expect <digest> [--socket PATH] [-s] [--json]
```

The name and UUID forms are mutually exclusive. Name mode creates a new
identity and refuses any existing canonical name, even an offline saved one.
UUID mode links exactly that unretired identity, independent of rename or a
same-name successor. `-s/--save` retains its meaning: otherwise new identities
are temporary; saving promotes an existing temporary UUID without downgrading
saved identities. Ordinary unguarded `add`/`name` retain their existing policy.

`expect` is a lowercase SHA-256 digest in a discovery-specific namespace, of
length-framed host/socket, native server/pane incarnations, executable hint and
pane-marker ownership UUIDs/PID (including the marker's server UUID). It excludes
the server's TMT UUID, display names, titles, cosmetic/activity fields and scan
time. The fence is host-neutral:
- The host is its token: `tmux` or the driver's name.
- The server is its socket, its native incarnation (PID and core's own start
  token) and the host's server start time. For an external host, that start
  time is core's start token too, never a driver's `startTime`.
- The pane is its stable host pane ID. The host `target` is display only and can
  move.
- The process is the pane PID and core's own start token for it.
- The ownership is the marker's identity, binding and server UUIDs and its PID.

A pane that is closed and reopened with the same ID and a reused PID has a new
start token, so the digest changes. A PID reused within one start-token tick is
not detected, as for every binding. Initializing server metadata while adopting
one unmarked pane preserves the original expectations for other panes from that
scan; native server replacement still requires rescan. A changed process,
marker or admission input requires rescan. Missing/unknown native or ownership
evidence cannot authorize adoption.

Guarded add reobserves before storage initialization and twice inside the
binding transaction before identity creation/promotion, server-marker
initialization or publication. Native target and ownership checks remain in the
shared binding/publication path. It refuses live ownership elsewhere and never
steals a pane. With a fresh matching expectation, the same UUID/target returns
`already_bound` without a second binding or publication; an older pre-adoption
expectation refuses. No name lookup is used for UUID authority.

Successful JSON has `{version:1,status:bound|already_bound,bound:true,id,name,
pane,lifetime}`. Failed JSON adds `status:refused|partial` and `pane` to the
existing error document. Refusal before admitted host effects leaves identities,
bindings and markers unchanged. Failure after admission may leave host effects;
refusals returned inside the SQL transaction roll back its identity/binding
mutations, and commit/close uncertainty is reported as `partial`. The result never
claims rollback of host publication or a confirmed SQL commit after an error. The command never replays a
partial attempt. Each invocation owns one item's result; a consumer issuing
several requests retains their individual results.

Invalid selectors/digest syntax are `USAGE_ERROR`. Drift uses `PANE_CHANGED`
(exit 3), absent UUID `NAME_NOT_FOUND` (3), identity/name conflicts
`NAME_ALREADY_ACTIVE` or `PANE_ALREADY_BOUND` (5), and uncertainty/storage
failures the existing reconciliation/storage codes (1). No new error-code family,
store schema or driver permission is introduced.

Normative state and digest vectors are in [discovery-v1.json](discovery-v1.json). Core owns
bounds and the digest fence; adapters own read-only host/storage acquisition;
the CLI owns grammar and JSON. The Herdr protocol inventory operation is deferred
to its epics #2097/#2890 and requires its own uniform driver-contract plan.
