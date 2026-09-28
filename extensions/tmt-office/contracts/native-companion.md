# Native companion handshake v1

This internal local protocol is separate from the proposed remote work-handoff
schema. It grants no Office access and does not replace pairing.

The core-owned `OfficeInvocation` has exact operations `probe`, `capabilities`, `pair-begin`,
`pair-poll`, `pair-status`, `unpair`, `inspect`, `sync`, `block-show`, `block-apply`,
`local-profile-show`, `local-profile-apply`,
`local-world-show`, `local-world-apply`,
`local-prop-validate`, `local-prop-install`, `local-prop-remove`, `local-prop-list`,
`local-prop-show`, `local-avatar-validate`, `local-avatar-install`,
`local-avatar-remove`, `local-avatar-list`, `local-avatar-show`, `board-post`,
`board-list`, `board-show`, `board-reply`, `board-edit`,
`board-delete` and `board-categories`. Its argument vector is
`__tmt-office`, `1`, `<operation>`. Unknown versions, operations,
extra arguments and non-UTF-8 arguments fail with exit 1, empty stdout and a brief
stderr diagnostic. There is no arbitrary argv forwarding or shell evaluation.

Successful probe output is exactly two LF-terminated UTF-8 lines:

```text
TMT-OFFICE/1
0.1.0-alpha.2
```

The second line is the companion's canonical package version, independent of
the CLI version. The adapter allows at most 1024 bytes per output stream and a
five-second execution deadline. Success requires exit 0, empty stderr, an exact
supported handshake and a version matching the verified receipt or candidate
artifact during pre-activation validation.
The fixed header identifies this explicit child response, never terminal scrollback.

The additive `capabilities` operation leaves those probe bytes unchanged. Its
only accepted response is the bounded exact text below:

```text
TMT-OFFICE-CAPABILITIES/1
office_board_v1
```

Before any board dispatch the CLI selects one verified immutable release and
directly executes this versioned capability check against that selected executable;
it does not need to run the legacy version probe first. It requires the exact known value. Missing,
duplicate, malformed or unknown capabilities fail as `OFFICE_INCOMPATIBLE` with
no board operation launch. Old CLIs continue to use only the unchanged probe;
new CLIs fail closed against old companions. Capability support is not inferred
from package semver.

Board requests use bounded JSON up to 64 KiB. Board responses use a shared 2 MiB
ceiling, derived conservatively from the maximum legal root plus 50-reply page
after JSON escaping and envelope overhead, while content and page limits remain
unchanged. This keeps the companion transport bounded without rejecting legal
maximum pages. The companion JSON adapter
maps them to the single `office_board` domain/repository owner; it does not create
a second board policy or allow an unverified executable.

Only a verified owned release or staged candidate may be probed. The installer
lock covers each verified child launch, and board dispatch rechecks that the
same pinned release which answered the capability probe is still active. An
install switch or uninstall between those launches fails closed with no board
mutation dispatch or automatic retry. The lock protects active-release
verification and child launch, not the subsequent
wait. The result describes the version selected at launch; concurrent upgrade
or deactivation may change the current installation before completion. A staged
candidate's pre-activation probe retains the publisher's existing lock scope.
No PATH lookup or fallback to another executable is permitted. The existing
subprocess owner handles deadline, output limits and cleanup. Handshake success says nothing about
pairing, remote availability, granted capabilities or service startup.

`tmt-office-model::office_protocol` owns request encoding and response validation, with
literal positive/negative test vectors. The separate `tmt-office` executable
depends on core and the shared adapters, with Office-specific deployment decoding
enabled only for that consumer. Its probe performs no Firebase, SQLite, network
or credential-store operations. Local cargo-dist packaging is available, while
public availability remains whatever the repository's current verified release
page and native installation guidance state. Synthetic archive process tests exercise
the real compiled companion. The independent native artifact verifier separately
checks actual Office archives; neither constitutes public publication.

## Pairing operations

Remote block operations use the same protected scope selectors and 4096-byte internal
message/output bounds. `block-show` additionally accepts nullable `blockId`;
`block-apply` also requires `layout` (exact readable `objects`) and
`expectedRevision`. Input files are validated before encoding this compact
message; the 64 KiB file limit is not an expanded companion payload allowance.
Success returns exactly `blockId`, `revision` and readable `objects`; the host
validates the response and supplies catalog/limits from the pure domain owner.
Errors use the same allowlisted envelope, including `OFFICE_LAYOUT_INVALID`
and `OFFICE_REVISION_CONFLICT`. `OFFICE_BUSY` denotes proven pre-execution local
lock contention; unclassified process/transport failures after launch never
become a busy/no-write claim.
Unsupported operations from older companions
are failures, never evidence of a saved layout. See [block v1](block-v1.md).

The local whole-world operations replace local block editing; the old private
`local-block-show` and `local-block-apply` names are rejected as unknown operations.
`local-world-show` accepts exactly `{}`. `local-world-apply` accepts the strict
world save envelope (`expectedRevision`, `legacyBasis`, `layout`), bounded to
4 MiB plus 512 bytes. Replies are bounded to 4 MiB plus 1024 bytes and decoded
as one strict world snapshot or an allowlisted world failure. Fresh previews
may have `worldId: null`; revision zero requires the source fingerprint, while
saved revisions require a world UUID and null fingerprint. The same adapter
storage operation serves CLI and HTTP. See [world v1](world-v1.md). These calls
need neither identity selection nor an HTTP service or network connection.

Local profile, prop, and avatar operations use their exact bounded contracts rather than
the remote block envelope. Profile source files are bounded to 8 KiB while their
private companion JSON remains under the legacy 4096-byte ceiling. Prop candidate
payloads carry validated data-only bytes through the private child boundary and
remain bounded to the 128 KiB source limit plus encoding overhead; catalog reads
return exact allowlisted projections. See [profile v1](profile-v1.md),
[prop pack v1](prop-pack-v1.md), [avatar pack v1](avatar-pack-v1.md), and
[local block v2](block-v2.md). Avatar candidate payloads use a separate 32 KiB source
limit plus base64/JSON overhead and an independent typed catalog/cursor namespace.

Pairing and inspect operations consume one bounded JSON object on stdin (4096 bytes):
`world`, `identityId`, `emulator` and `readOnly`, with no unknown or duplicate
fields. Selectors contain no credentials. The adapter accepts at most 4096 bytes
per output stream and uses the existing subprocess deadline/cleanup owner.
The companion receives at most 25 seconds for a single operation; the public
observer's shorter deadline still wins. A successful process emits only a public
JSON result: local `state`, pending `state` plus `approvalUrl`, `blockExists`, or
a known `error` code. Stderr must be empty. Credentials and provider diagnostics
never cross this boundary. Public CLI output uses its existing error envelope.

The adapter validates this allowlist; the CLI additionally checks that the result
belongs to the requested operation. The protected state machine and HTTP/vault
adapters stay in the Office feature, not the CLI or core. See
[native pairing](native-pairing.md) for scope, deadlines and server authority.
Future operations require a reviewed typed contract before implementation;
the probe response is never a generic payload or remote authentication.

`unpair` uses the same scoped input. Success returns `state: revoked`; a denied
pending cancellation returns `state: pending` plus the original `approvalUrl`
for owner cancellation, never a cleanup confirmation. Other failures retain the
normal error result. An older companion rejects `unpair`; the host must not
interpret unsupported-operation failure as successful revocation.

`sync` consumes exactly `{}`; it requires no active identity or world selector.
It returns exactly `completed`, `failed`, `pending` (unsigned counts) and
`failureCode` (a known Office code for failed attempts, otherwise null), or the
same known `error` result. At most 16 attempts are made within the existing
25-second operation budget. It uses the same verified child, limits and cleanup
owner as pairing. An older companion rejects this additive operation; the host
does not infer successful cleanup from an unsupported response.
