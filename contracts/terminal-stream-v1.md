# Terminal stream v1

The stream between a built-in-host member's supervisor and a terminal view
(`tmt attach`, the Ops Terminal tab). It shows the member's current screen,
carries its live output, and types into and resizes its terminal. It is local and
owner-only, and it grants no authority over identities, requests or delivery: those
stay with Core, which uses separate control frames on the same socket that a stream
client never sends.

The frames are defined by the dependency-free `tmt-pty-protocol` crate; this
contract owns their meaning. A client needs that crate (or the table below) and a
terminal parser, nothing from Core. Use the same parser the workspace uses,
`vt100` at `=0.16.2` (the workspace pin), so a snapshot and live output are read by
the code that produced them.

## Finding a member's stream

A stream is one Unix socket per member. Core publishes where it is; a client never
derives the path. Discovery is the extension API operation
[`terminal.stream`](extension-api.md#terminal-stream): `{identityId}` answers
`{identityId, host: "pty", socket, pane}` for a member whose current binding is on
the built-in host and whose supervisor is running. Any other member, one with no
binding, or one that has ended, is `TERMINAL_STREAM_UNAVAILABLE`, and a view shows no
terminal for it.

## Access

- The socket's directory is owner-only (0700) and the socket is 0600. A peer whose
  user differs from the supervisor's is refused (`Owner`).
- At most 16 clients are attached; another is refused (`Full`).
- A client that has not sent `Hello` within 5 s is dropped. The first frame must be
  `Hello`; any other first frame is refused (`Handshake`), a version other than 1
  is refused (`Version`), and a second `Hello` closes the connection.

## Frames

Every frame is `kind: u8`, `length: u32` (big endian), then `length` payload bytes;
a payload is at most 64 KiB. Decoding is strict: an unknown kind, a payload of the
wrong size, an out-of-range value or an empty data frame closes the connection.
A size is `cols: u16`, `rows: u16`, each 1 to 1000.

Client to supervisor:

| Kind | Frame | Payload | Meaning |
|---|---|---|---|
| `0x01` | `Hello` | `version: u16` (1) | The first frame. |
| `0x02` | `Input` | 1 to 16 KiB | Bytes typed into the program, in arrival order across clients. |
| `0x03` | `Resize` | size | This client's window. The terminal takes the size of the client that last resized or typed. |
| `0x04` | `TakeSole` | none | Disconnect every other client. A terminal view never sends it. |
| `0x05` | `Detach` | none | Leave; the program keeps running. |
| `0x06` | `Snapshot` | none | Send the screen as it is now. |

Supervisor to client:

| Kind | Frame | Payload | Meaning |
|---|---|---|---|
| `0x81` | `Welcome` | `version: u16`, size | Accepted; the size now in force. |
| `0x82` | `Output` | 1 to 16 KiB | Program output, passed through untouched. |
| `0x83` | `Exited` | tag `0` code or `1` signal, `i32` | The program ended; the supervisor closes after this. |
| `0x84` | `Evicted` | `0` taken over, `1` too slow | This client is disconnected. |
| `0x85` | `Refused` | `0` version, `1` handshake, `2` owner, `3` full | The connection is refused. |
| `0x86` | `Resized` | size | The size in force changed. It always reports the actual size, whichever client won. |
| `0x87` | `SnapshotStart` | none | A snapshot begins. |
| `0x88` | `SnapshotData` | 1 to 16 KiB | The next bytes of the snapshot. |
| `0x89` | `SnapshotEnd` | size | The snapshot is complete and was taken at this size. |

Kinds `0x20` to `0x2f` and `0xa0` to `0xaf` are Core's controls on the same socket.
They are not part of this stream and a view must not send or interpret them.

## Snapshot

`Snapshot` is how a view that connects mid-session, or reconnects, gets the current
screen without waiting for the program to draw and without any redraw nudge.

- The reply is `SnapshotStart`, any number of `SnapshotData` chunks of at most 16 KiB,
  then `SnapshotEnd(size)`. Together the chunks are terminal escape sequences that
  turn an empty terminal of that size into the screen: the visible cells with their
  attributes, the cursor position and visibility, and the input modes (application
  keypad, application cursor, bracketed paste, mouse protocol and encoding). A program
  on the alternate screen is reported by a leading `ESC[?1049h`, and only that screen is
  included. There is no scrollback.
- A client buffers the data, and at `SnapshotEnd` builds a terminal of the reported size,
  applies the bytes, and then applies every `Output` that follows on top. Frames received
  before `SnapshotStart` are superseded by the snapshot, and a second `SnapshotStart`
  discards a snapshot not yet complete. `Resized` that follows the end reports later
  changes. The CLI terminal relay instead paints each bounded chunk as it arrives,
  without an aggregate buffer, and follows live output only after the end. If an
  initial snapshot exceeds its 1 MiB safety bound, it clears the partial paint,
  discards the rest of that snapshot and continues live; the member is not stopped
  and attachment does not fail because of screen size.
- Nothing is lost or repeated between the snapshot and live output: the supervisor
  formats the screen in the same step that orders output, so the screen holds every
  byte queued before the request and none queued after it.
- Every request gets the screen as it is at that moment; none is kept waiting. A snapshot
  is bounded by the screen's size.
- A client that asks while 256 KiB of earlier frames are unread is evicted (`TooSlow`),
  like any client that cannot keep up.

## Size

The terminal has one size, that of the client that last sent `Resize`, typed after
another had, or took sole use. A view sends `Resize` before it types, and learns any
other client's change from `Resized`; it clips or pads its own display to the size in
force. A client that never sends `Resize`, such as a delivery, never moves the window.

## Flow control and loss

Output is read whether or not anyone is attached and is not retained. A client with
256 KiB of frames queued pauses the program, so a slow view slows the program instead
of losing output. One that reads none of what waits for 3 s is evicted (`TooSlow`). A
view that loses the stream, or is evicted, reconnects with a new `Hello` and a new
`Snapshot`; connection loss never proves the agent ended. `Exited` ends the stream and
carries the exit code or the signal.

## What the stream does not offer

- No scrollback and no replay of earlier output. A view that wants history keeps its
  own from what it received.
- No redraw nudge, and no change of the terminal's size except by `Resize`.
- No Core authority: it cannot resolve identities, send requests or change bindings.
