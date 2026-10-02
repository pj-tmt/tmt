# Office contract boundary

Private-world and home-block document contracts are implemented in the browser
and Rules. The remote work-handoff protocol remains a proposal, not a deployed API.

The [agent resource grant v1](agent-grant-v1.md) adds server-enforced access to
assigned UUID blocks. [Pairing approval and claim v1](pairing-v1.md) defines the
locally tested trusted issuer. Native pairing and the agent-block UI remain
separate work; the browser currently edits only the owner's home block.
The [native pairing contract](native-pairing.md) defines deployment discovery and
the in-progress command/credential boundary, not shipped command guidance.
The [local Office service v1](local-service-v1.md) defines the source-candidate optional
loopback service, browser session and shared SQLite authority; it is not remote pairing.
The [local request dispatch v1](dispatch-v1.md) capability owns explicit-recipient
composition and replay receipts over the existing inbox, not a shell or room registry.
The [local meeting-room resource](meeting-room-v1.md) supplies explicit membership
and a revision/audience fence for that same dispatch capability.
The [local map v1 foundation](map-v1.md) defines user-built topology, derived walls
and draft projection. Its containing world owns persistence;
[rooms and walls](rooms-and-walls.md) owns the full delivery goal.
The [world layout contract](world-v1.md) composes that topology with ordered floor,
wall and functional placements in one SQLite revision. Its protected local API
and browser editor are implemented; complete CLI cutover and acceptance remain.

The separately versioned [private world document v1](private-world.md) is the
direct-Firestore contract. It uses native Firestore timestamps and Rules,
not the work-handoff HTTP/JSON envelopes below. Its actual client adapter and
Rules are exercised together in `extensions/tmt-office/typescript/apps/office/e2e/world-rules.spec.ts`.

The [home block document v1](block-v1.md) extends that owner-only world with
bounded, revision-checked decoration. Its vectors are shared by client and
real Rules tests, not derived from implementation output.

The [local presentation profile v1](profile-v1.md) defines the shipped local-only,
UUID-owned profile resource. [`profile-v1.vectors.json`](profile-v1.vectors.json) is the
literal Rust/Office conformance source for its catalog, deterministic defaults and safe
validation examples. It is independent from block layout, role, notes and presence.

The [data-only prop pack v1](prop-pack-v1.md) and local
[block layout v2](block-v2.md) own #238's file, digest, catalog, placement,
placeholder and built-in migration boundary. Their source implementation does not
imply availability in an installed release, a remote catalog, or a second
layout/renderer owner.
[`prop-block-vectors.json`](prop-block-vectors.json) is their compact shared Rust/Office
value and full-capacity conformance source. Raw JSON duplicate-key admission remains a
Rust file-reader proof because JavaScript object decoding occurs after JSON parsing.

The local [avatar pack v1](avatar-pack-v1.md) contract owns strict data-only character
art admission, its independent installation catalog, expiring local preview and immutable
profile selection with stored-default fallback.
[Avatar pack v2](avatar-pack-v2.md) adds 32×48/256-color rasters within the same
file/cell/catalog budgets, retaining v1 and the existing physical character size.
[`avatar-pack-vectors.json`](avatar-pack-vectors.json) is the shared Rust/Office
projection source; the exact-byte sample freezes the separately framed avatar digest.
The managed `tmt-avatar-create` guidance documents authoring and installation. Remote
publication, animation, movement and runtime plugins remain out of scope.

## Single source of truth

The accepted [World capabilities and functional extensions](functional-props.md)
design owns the composition boundary and primary visual references. The
[v1 discussion binding](extension-v1.md) admits separate definition/instance
records; it does not extend prop-pack fields. Broader host actions, external
extension installation remain planned, not implemented public APIs. The
[whiteboard scene v1](whiteboard-v1.md) has local Rust/browser admission with
[literal vectors](whiteboard-vectors.json) and an [all-tools example](whiteboard-scene-v1.json).
Its editor, persistent resource and snapshot delivery are implemented locally;
this does not imply release availability.

The [saved notebook resource](notebook-v1.md) binds furniture to existing UUID-owned
Markdown through a bounded read-only local adapter, without another content store.

The [native companion handshake](native-companion.md) is an implemented internal
local boundary, independently versioned from the remote work-handoff proposal.

- `v1.schema.json` is JSON Schema 2020-12 for the initial work handoff ingress:
  negotiation offer, human work submission, connector evidence and final export.
- `examples.json` contains valid wire examples. Authentication is supplied by
  the transport, never by an `actorUid`, display name or local receipt field.
- `scenarios.json` assigns behavioral acceptance vectors to implementation
  issues. Its assertions are requirements, **not passing policy tests**.
- [Design](../docs/design.md) owns semantics and trust boundaries;
  [commands](../docs/commands.md) owns planned CLI UX.

Browser, service and Rust connector representations must derive from this schema
or run these fixtures and boundary mutations through their actual serializer and
validator. Do not maintain parallel hand-written contracts without conformance.
No shared runtime package, Firebase dependency or Rust crate is invented merely
to host this file. Pairing endpoints, read projections, signed/scoped permit
transport and board payloads must extend this owner in their implementing issue
before their first consumer ships. They are not implicitly defined by Firebase
collection shapes or by these four ingress message types.

## Compatibility and interpretation

The stable negotiation offer carries supported exact `major.minor` versions and
required capabilities. A service chooses the highest mutually supported exact
version only if it supports every required capability. Currently the design has
only `1.0` and capability `work-handoff` (review requests). An incompatible offer
is structurally valid but gets `409` and no session/work execution. Capability
negotiation never grants the caller permission to perform that operation.

Unknown message kinds, versions and fields reject; they are not silently ignored.
Future additive fields require a new minor schema and explicit negotiation;
semantic breaking changes require a new major. Keep older released schemas
immutable once consumers ship. Do not infer compatibility from semver package
versions or fall back to a local CLI protocol. Review metadata binds the exact
revision, not repository access or permission to publish feedback.

Strict JSON cannot contain duplicate object keys, non-finite numbers or invalid
Unicode scalar sequences. Reject duplicate keys during decoding before a normal
object parser discards them; JSON Schema alone cannot detect that condition.
Timestamps are safe-integer UTC milliseconds. Service time validates freshness,
deadline range and leases; a valid timestamp type is not evidence of freshness.
Empty and whitespace-only text remain valid exact text, not silently trimmed
content or a synthetic withheld result. This preserves native text semantics.

Human work submissions return `201` with the accepted exchange ID and `queued`
routing state; an exact authorized retry returns `200` with the original ID and
current projection. Reports/final retries also return the original projection;
they cannot replace stored content or advance unauthorized state. The service
derives the actor/device from authentication and resolves current grants and
assignment before these operations. Errors follow the design's HTTP classes;
do not forward native CLI error details to remote clients.

## Local conformance

```sh
(cd typescript && corepack pnpm exec vitest run test/tooling/office-contracts.test.ts)
(cd typescript && corepack pnpm check:tooling)
```

The TypeScript tooling suite uses [Ajv JSON Schema validation](https://ajv.js.org/json-schema.html) in strict mode rather than a
home-grown schema validator. It checks actual examples, required/unknown fields,
versions, local-metadata injection and Unicode size boundaries, plus scenario
reference integrity. It cannot prove server authorization, stale-token rejection,
tmux delivery or crash recovery. Each downstream issue must implement the owned
behavioral cases using its real rules/service/connector and causal observations.
