# Local MCP v1

`tmt mcp --identity <saved-name-or-uuid>` exposes the existing local exchange
through agent-launched stdio. It is a same-user process interface, not a network
endpoint or an authentication service. The CLI JSON commands and
[local API](extension-api.md) retain ownership of resources and semantics.

## Transport and lifecycle

The supported protocol revisions are `2025-11-25` and `2025-06-18`. Qualification
covers the stdio, initialization, ping and tools subset through native protocol
and real CLI integration tests; it does not claim qualification of particular
agent products. The wire follows MCP's
[stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports),
[lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)
and [tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).

Each UTF-8 JSON-RPC 2.0 message occupies one line, terminated by LF. There are no
JSON batches, embedded literal newlines, Content-Length headers or stdout logs.
Request IDs are strings or integers within the JavaScript safe-integer range;
null IDs are invalid. Duplicate envelope and typed argument fields are rejected.
`initialize` requires `protocolVersion`, an object `capabilities`, and
`clientInfo` containing string `name` and `version`. A supported requested
revision is echoed; otherwise the server proposes `2025-11-25` and the client
must disconnect if it cannot use that revision. The server advertises only
`tools: {}`. After `notifications/initialized`, clients may use `tools/list` and
`tools/call`. `ping` is available before and after initialization. Tool discovery
is fixed and unpaginated. There are no server requests, subscriptions, list-change
notifications or channel notifications.

Notifications never execute tools. Unsupported request methods return `-32601`;
invalid parameters return `-32602`; invalid JSON returns `-32700`; invalid RPC
or lifecycle use returns `-32600`. Tool operation failures instead return a
normal MCP result with `isError: true` and the existing structured TMT error.
Successful results contain `isError: false`. In both cases `structuredContent`
is the existing JSON resource and `content` contains one text item encoding that
same resource as JSON. A transport error exits nonzero with a diagnostic on
stderr. Clean EOF exits successfully; a partial final frame fails. Terminal
stdin or stdout is refused. EOF, a broken pipe or process termination owns only
this server's lifetime; it does not cancel, acknowledge or complete exchanges.

## Identity and local trust

Startup resolves an existing, active **saved** identity once, then pins its UUID
and the application data paths for the server lifetime. There is no pane lookup,
identity creation, provider login, runtime enrollment or global configuration
change. Each tool call rechecks that exact UUID. Renaming an identity does not
rebind it; retirement makes later calls fail. There is no display-name fallback
for a pinned UUID. A tool argument, client metadata or `_meta` cannot select an
identity, application directory, executable, file path or environment.

This is attribution and incoming-request scoping, not isolation from another
process running as the same OS user. That user chooses the launch identity and
already has access to the same local database and ordinary CLI. `tmt_list`,
`tmt_operation` and `tmt_result` preserve their existing local lookup scope;
`inbox` and incoming `request` are scoped to the pinned participant. Reply
receipts are correlation proofs, not login credentials. A future authenticated
remote interface must enforce its own principal and permissions.

## Tools and schemas

The current surface is eight tools: five reads and three writes. `tools/call.params` has required
string `name`, optional object `arguments` (default `{}`), and optional object
`_meta`. Other properties are rejected. Every arguments object has
`additionalProperties: false`; optional properties may be absent, not null.
Required properties are listed below. ID/filter strings are nonempty and at most
256 UTF-8 bytes. No tool accepts an `identity` property.

| Tool            | Arguments                                                            | Existing owner and identical JSON resource                                                                 |
| --------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `tmt_list`      | `{}`                                                                 | `tmt identity list --json`; non-retired records, without presence                                          |
| `tmt_operation` | Required string `operationId`: canonical lowercase, non-nil UUID     | `tmt api` operation `dispatch.show`                                                                        |
| `tmt_inbox`     | Optional string `from`; optional integer `limit`, 1–200 (default 50) | `tmt inbox --identity <bound UUID> --json`                                                                 |
| `tmt_request`   | Required string `requestId`                                          | `tmt x show <id> --incoming --identity <bound UUID> --json`, including its reply receipt                   |
| `tmt_result`    | Required string `requestId`                                          | `tmt result <id> --json`, including exact retained final text                                              |
| `tmt_send`      | Required strings `operationId`, `recipientId`, `message`             | `tmt api` operation `dispatch.create`, one `recipientIds` entry, `kind: request`, server-selected identity |
| `tmt_answer`    | Required strings `requestId`, `message`                              | `tmt answer --request <id> --identity <bound UUID> --stdin --json`; derives the existing recipient proof   |
| `tmt_ack`       | Required string `requestId`; required integer `revision`             | `tmt x ack <id> --incoming --revision <revision> --identity <bound UUID> --json`                           |

For `send`, both UUIDs must be canonical lowercase and non-nil, and `message`
must be nonblank. Send returns durable dispatch acceptance with the existing
optional first advisory wake, not blocking talk completion. A caller must retain
its operation UUID before invoking it. `answer` always names one incoming request
and submits its exact final; empty finals are allowed. It never selects a latest
request or takes a receipt, sender, file path or stdin selector from tool arguments.
Identical retained finals replay and different finals conflict. `ack` acknowledges
only the bound participant's incoming view at a supplied positive safe-integer
revision (1–9,007,199,254,740,991). Stale revisions cannot suppress later attention.
Neither answer nor reads implicitly acknowledge work. Messages are UTF-8 strings
bounded to 1,048,576 bytes independently of escaping; BOM, CRLF, NUL and Unicode
are preserved. Other underlying service admission rules still apply.

A tool-call document (serialize it on one line for stdio):

```json
{
  "jsonrpc": "2.0",
  "id": "inbox-1",
  "method": "tools/call",
  "params": { "name": "tmt_inbox", "arguments": { "limit": 20 } }
}
```

The published `inputSchema` is an object with the properties and required fields
above. Strings use `type: "string"`, `minLength: 1`, `maxLength: 256`; the byte
bound is checked separately. `operationId` uses `type: "string", format: "uuid"`
and the canonical restriction above. `limit` uses `type: "integer"`,
`minimum: 1`, `maximum: 200`. Message schemas use `type: "string"`,
`maxLength: 1048576`; core's byte limit and send's nonblank restriction are checked
separately. `recipientId` has the same UUID schema as `operationId`. `revision`
uses `type: "integer"`, `minimum: 1`, `maximum: 9007199254740991`. The five reads
advertise `readOnlyHint: true`; send/answer/ack advertise `readOnlyHint: false`.
All eight advertise `destructiveHint: false`, `idempotentHint: true`,
`openWorldHint: false`. Idempotence requires retaining all supplied arguments,
especially the send operation UUID and acknowledged revision. Annotations describe
behavior and grant no authority.

## Bounds and recovery

One input frame, excluding its LF, is bounded to 6,365,184 bytes
(`api::INPUT_LIMIT + 65,536`). One output frame including LF is bounded to
38,010,880 bytes (`3 * api::OUTPUT_LIMIT + 65,536`). The existing resource itself
must fit the API's 12,648,448-byte output bound; otherwise the tool reports
`MCP_OUTPUT_TOO_LARGE` rather than returning truncated content. Canonical
[exchange text limits](request-response-v1.md#durable-final-submission) still apply
independently of JSON escaping. These are transport ceilings, not permissions
to enlarge any underlying command's resource.

The server waits indefinitely between frames. Once the first byte of a frame
arrives, acquisition has a five-second deadline; publishing a response has a
separate five-second deadline. Oversize or stalled framing terminates the server.
There are no detached workers, listeners or persistent open database handles.
Reads retain ordinary storage opening, migration, expiry and retention behavior;
they never acknowledge an attention revision or submit a final.

`send` uses the existing dispatch service: identical operation UUID, originator
and normalized intent returns the immutable acceptance, without another wake;
changed intent conflicts. No automatic retry creates a new UUID. A lost response
is recovered with `operation`, which cannot dispatch or wake again. This remains
true after process restart, across concurrent clients, and after uncertain wake
classification. An unavailable `result` is not evidence of cancellation or
permission to resend. Transport timeout or MCP cancellation notification does
not cancel durable work. Existing terminal uncertain receipts remain terminal.
Final submission and attention revision decisions stay with the shared
[request/response contract](request-response-v1.md); MCP adds no exchange states,
receipt type, storage or retry semantics.

## Boundaries and later slices

The CLI owns launch selection and composes existing command services in-process.
`tmt-adapters::mcp` owns only bounded framing, MCP lifecycle and typed schemas;
core owns exchange decisions. All are core paths. The private
[Claude channel](claude-channel-v1.md) server is unchanged and shares no extracted
framing with this server. Native Claude and [Codex](codex-channel-v1.md) channels
are host wake/delivery mechanisms. This MCP interface lets an agent pull and act on exchanges. Send may use the
existing advisory wake; launching the server neither installs those channels nor
wakes an unloaded model. The [remote door](remote-channel-v1.md) is a separate authenticated channel
boundary, not an HTTP mode of this process.

All tools compose existing command owners. The API wire still has the same
operations; its internal dispatch selector distinguishes ordinary local name/UUID
selection from a pinned saved UUID, which never falls back to a display name.
Answer and ack use the same exact saved selection in their existing CLI owners.
The name `talk` is reserved for a later blocking operation. Waiting talk and
consented provider setup/uninstall remain separately tracked work. No automatic
provider registration or runtime enrollment is supplied by this server.
