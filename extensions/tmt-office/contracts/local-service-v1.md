# Local Office service v1

This source-candidate contract owns the optional installed companion's offline browser
boundary. It requires a coordinated compatible CLI/Office release before public use.
It is installation-local and does not authorize Firebase, remote publication, pairing,
agent execution or filesystem browsing.

## Process lifecycle

`tmt office start` serializes startup and launches only the verified active
`tmt-office` executable with the internal `__tmt-office-service 1 serve` vector. The
child binds IPv4 `127.0.0.1`, writes a private bounded receipt, and emits readiness only
after the listener and receipt exist. Parent readiness succeeds only after an
authenticated control health request. Reuse preserves the receipt and browser token;
restart generates new independent browser, control and nonce secrets.

The receipt schema is version 1 and contains PID, port, running companion version,
nonce, browser token and control token. It is private coordination state, never public
status. Stale absent processes may be replaced under the startup lock. A live PID
without authenticated health is uncertain and must never be signalled. Stop requires
the control bearer and nonce, waits boundedly for the authenticated process to exit,
and removes only a still-matching receipt.

## HTTP boundary

Every request is HTTP/1.1 on the bound listener and has exactly one Host equal to
`127.0.0.1:<bound-port>`. Request headers are limited to 16 KiB, bodies to 64 KiB,
with the explicit resource/preview exceptions below. Transfer encoding and ambiguous
content lengths are rejected, and every connection
handles one request before closing. There is no CORS response.

- `GET /`, `/index.html`, `/local`, `/local/*` and exact embedded `/assets/*` serve
  only compiled SPA bytes. The static shell is unauthenticated and contains no state.
  The build admits HTML plus generated JS/CSS and bundled PNG files, with safe
  flat asset names, at most 32 files and an 8 MiB combined size limit. PNG is
  served as `image/png`; the static asset path never admits uploads or executable SVG.
- `GET` and `PUT /api/v1/local/world` expose the [atomic whole-world contract](world-v1.md#local-transport).
  PUT requires the browser bearer, exact Origin and JSON content type with a
  4 MiB + 512 byte envelope. CLI and HTTP share the world storage operation.
  The former local block list, identity-block and lobby-block routes return 404;
  they are not aliases or alternate writers. Migration retains existing layouts
  until the first explicit world save retires them atomically.
- `GET` and `PUT /api/v1/local/whiteboards/<id>` expose installation-owned
  [whiteboard documents](whiteboard-v1.md#local-resource-api). Both require the
  browser bearer; saves also require exact Origin and JSON content type. Only
  an exact valid whiteboard PUT path admits the 2 MiB + 16 KiB envelope budget;
  other metadata routes retain their existing limits. Reads do not create content.
- Whiteboard [snapshot routes](whiteboard-v1.md#snapshot-http-access) use the same
  browser bearer and exact write-Origin policy. Capture POST admits 128 KiB of
  JSON; image PUT admits 8 MiB of raw `image/png` only at its exact snapshot path.
  Stored PNG reads/attachment responses contain normalized inert pixels, not
  source metadata. Image admission does not attest that pixels depict the scene.
- `POST /control/v1/health` and `/control/v1/stop` require the distinct control bearer,
  receipt nonce and an empty body.
- `GET /api/v1/local/notebooks/<uuid>` exposes the [saved notebook reader](notebook-v1.md)
  under the browser bearer. It admits only identity references, never filesystem
  paths; missing reads do not initialize files. No notebook write route exists.
- `POST /api/v1/local/props/list` and `/props/install` expose the existing custom
  prop catalog to [local pixel authoring](prop-pack-v2.md#local-pixel-workshop).
  Both require browser bearer and exact JSON Origin. List admits 4 KiB and returns
  metadata only; install admits the 512 KiB pack's escaped envelope (3 MiB + 512
  bytes), then revalidates exact bytes and uses the existing catalog revision CAS.
  Saving a pack does not change world placement; the browser adds saved art to
  the ordinary layout draft through a separate explicit action.
- `GET /api/v1/local/rooms` and conditional `PUT /api/v1/local/rooms/<uuid>` expose
  [explicit meeting membership](meeting-room-v1.md), independent of personal layout
  and presence. Writes use the same owner authority and an 8 KiB input budget.
  `POST /api/v1/local/rooms/<uuid>/retire` uses that same budget and authority for
  a revision-checked retirement. It retains data and removes the room from active lists.
- `POST /api/v1/local/dispatch` exposes [explicit request composition](dispatch-v1.md)
  with browser bearer, exact Origin and JSON admission. Its operation receipt and
  canonical inbox writes share one transaction. Only that exact POST path admits
  the dispatch envelope budget; it is not a general command-execution endpoint.
- Read-only `POST /api/v1/local/requests/list`, `/requests/show` and `/dispatch/show`
  expose [retained conversations and acceptance recovery](dispatch-v1.md#retained-conversations-and-acceptance-recovery)
  under the same `/api/v1/local` prefix and owner authority. These exact POST paths
  accept at most 4 KiB, never the dispatch message budget. Reads do not acknowledge
  or send work; history and detail share canonical request retention.

All responses set `Cache-Control: no-store`, a restrictive CSP, no-referrer, nosniff,
frame denial and same-origin resource policy. Paths do not map to the filesystem.
The CSP permits self-hosted content and data/blob images for admitted previews;
script, style and connection sources remain self-only. Image URLs grant no host
capability and are not snapshot references.
Missing, malformed, unauthorized, conflicting and unavailable results use only the
bounded status/error allowlist implemented by the service.

## Browser session

The session URL is `http://127.0.0.1:<port>/local#token=<browser-token>`. The offline
SPA validates the token, moves it to memory and removes the fragment with
`history.replaceState`. Reload therefore needs the URL printed by a later `office
start`. Data requests carry the bearer. Block watches have one request in flight, poll
from two seconds with bounded backoff, abort on disposal, and stop after authorization
failure. The existing block draft/conflict state remains the only editor state.

`/local` observes the whole-world layout, active identity profiles, meeting rooms
and admitted artwork. Profile directory rows include native `presence` (`active`, `offline`,
or `unknown`). Unknown evidence is neither confirmed offline nor permission to
remove an identity; only active observations produce map actors. All three states
remain selectable for messaging, whose acceptance is owned by the request service.
`/local/agents/<identity-uuid>` selects that agent's HUD in the same world;
`/local/lobby` opens the world overview. Neither route creates a room or identity.
A mounted-view snapshot loader owns loading, explicit refresh and stale-completion
fencing. Prop resolution deduplicates world placements and batches at most 16
unique digests per request; areas do not each start a listener.

## Persistence

Migration 011 introduced singleton local-world metadata and identity-UUID block rows;
migration 019 adds the lobby target in the same table. Canonical version-2 layouts,
revisions and timestamps live in the existing SQLite
database. CLI local block invocations and HTTP handlers call the same repository. A
missing block read does not create it; first apply creates UUID/revision 1 atomically.
Exact retries preserve the committed revision. Identity mutations recheck active identity in an
immediate transaction. Retirement retains but hides content, and a same-name new UUID
does not inherit it.
