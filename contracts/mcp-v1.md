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

The current surface is five read-only tools. `tools/call.params` has required
string `name`, optional object `arguments` (default `{}`), and optional object
`_meta`. Other properties are rejected. Every arguments object has
`additionalProperties: false`; optional properties may be absent, not null.
Required properties are listed below. ID/filter strings are nonempty and at most
256 UTF-8 bytes. No tool accepts an `identity` property.

| Tool            | Arguments                                                            | Existing owner and identical JSON resource                                               |
| --------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `tmt_list`      | `{}`                                                                 | `tmt identity list --json`; non-retired records, without presence                        |
| `tmt_operation` | Required string `operationId`: canonical lowercase, non-nil UUID     | `tmt api` operation `dispatch.show`                                                      |
| `tmt_inbox`     | Optional string `from`; optional integer `limit`, 1–200 (default 50) | `tmt inbox --identity <bound UUID> --json`                                               |
| `tmt_request`   | Required string `requestId`                                          | `tmt x show <id> --incoming --identity <bound UUID> --json`, including its reply receipt |
| `tmt_result`    | Required string `requestId`                                          | `tmt result <id> --json`, including exact retained final text                            |

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
`minimum: 1`, `maximum: 200`. All five tools advertise `readOnlyHint: true`,
`destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`;
annotations describe behavior and grant no authority.

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

`operation` only recovers the immutable dispatch receipt; it cannot dispatch or
wake again. An unavailable `result` is not evidence of cancellation or permission
to resend. Transport timeout is not cancellation. Existing terminal uncertain
receipts remain terminal. Recovery and final-response ownership are defined by
the [request/response contract](request-response-v1.md); MCP adds no exchange
states, receipt type, storage or retry semantics.

## Boundaries and later slices

The CLI owns launch selection and composes existing command services in-process.
`tmt-adapters::mcp` owns only bounded framing, MCP lifecycle and typed schemas;
core owns exchange decisions. All are core paths. The private
[Claude channel](claude-channel-v1.md) server is unchanged and shares no extracted
framing with this server. Native Claude and [Codex](codex-channel-v1.md) channels
are host wake/delivery mechanisms. This MCP interface is an agent's pull/read
interface; launching it neither installs those channels nor wakes an unloaded
model. The [remote door](remote-channel-v1.md) is a separate authenticated channel
boundary, not an HTTP mode of this process.

Writing tools (`tmt_send`, `tmt_answer`, `tmt_ack`) are a separate implementation
slice under #477 and are **not advertised or accepted here**. `send` will mean
existing dispatch acceptance; the name `talk` is reserved for a later blocking
operation. Waiting talk and consented provider setup/uninstall remain separately
tracked work. No claim of automatic provider registration or full team enrollment
is made by this read-only slice.
