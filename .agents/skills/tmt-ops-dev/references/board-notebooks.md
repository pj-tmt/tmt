# Board notebooks, detail and replies

[board.md](board.md) owns frame reset, composers and `Scrolls`. The shipped Squad
skill owns [notebook controls and link syntax](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#keep-your-notebook-current)
and [detail/reply appearance](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#board-appearance).
Effect policy belongs to [config-and-effects.md](config-and-effects.md#actions-and-effects);
the [refresh reference](refresh-and-meter.md#refresh-worker) owns worker priority/cancellation.

## Inert text and independent notebook/source utilities

The workspace has no notebook/detail pane or notebook-link activation route.
Selecting an agent never queues `notes.read`; selected worker jobs are typed
Status previews, coherent selected-conversation projections, exact Message sources for
compact details/readers, and bounded recent pane-output observations. The old notebook event, eight-entry
cache, per-squad cursor/link maps and painters are removed. Notebook storage,
public source APIs and saved-identity behavior retain their independent owners.
`board::refresh` calls `observe::observe`, which reads the lead's notes only when
the board shows them or observation is active, and retains the bounded
acquired-data conversion, including missing, no-lead, temporary-identity and
failed-read distinctions; it creates no notebook. Incompatible notebook rendering/handler settings get
located migration notices without rewriting authored values.

`board::notes::sanitize` removes escapes, controls and hidden bidi/format
characters, preserves newlines and script/emoji joiners, and expands tabs to four
spaces. Shared Markdown renders headings, lists, underlined links,
filled code and quote gutters; tables use a boxed grid that shrinks the widest
columns and wraps their cells to fit the pane (inside quotes and lists too), and
fall back to source only when even two-cell columns cannot fit. Unsupported HTML, images and footnotes remain
literal. Code lines clip without wrapping, and inline code adds no padding. The retained `NotesCursor` anchors response source lines and wrapped
continuations; it reconciles nearest duplicates and clamps deleted lines.
Conversation Scrolls owns the viewport, independently of that source anchor.
Openable links (`effects::openable`: `http`, `https`, `mailto`) are recorded by the
renderer as `member_links::Target::Url` cells beside the member mentions, for a label,
an autolink and a bare address outside code. They share the hit table, hover, focus
and Enter with mentions; a `Url` hit opens the popover of `member_links/popover.rs`
(a `Menu`: the origin as an inert row, then Open, Copy, Show full). `effects::open_argv`
fills `{url}` as one argument; `detect_opener` picks the default per `OpenHost`.
`board/hyperlinks.rs` also lets a supporting terminal click an `http(s)` label: `paint_hits`
leaves the cells alone and `Hit`s of `Target::Url` become `Span`s (`State::web_spans`),
which the draw closure hands to `hyperlinks::Links` and `terminal::Backend::draw` wraps
around each changed run in OSC 8 (one writer, `hyperlinks::sequence`, http/https only,
no control byte, percent-encoded non-ASCII, at most 2000 bytes). Support is decided once
at start by `hyperlinks::supported` (`FORCE_HYPERLINK`, positive terminal signals, tmux
3.4+ with the `hyperlinks` client feature); off, the backend's bytes are unchanged.

Conversation Markdown preserves authored soft and hard line breaks. A continuation
inside a quote carries only its list indent forward; the quote gutter is added
once per displayed line. Source mappings and copied message bytes stay unchanged.
Conversation bodies always render in full, with no message fold or expansion
control, and the pane scrolls. A separate anchor map preserves reading position
while source mappings retain raw identities.

Whole-token absolute Unix file paths in sent and received message prose render as
atomic basename chips using the registered attachment mark. Matching single or double
quotes delimit a path with spaces on one authored line; their contents stay literal.
Bare paths need at least two nonempty segments, so slash commands such as `/compact`
stay literal. Leading `(`, `[` and `{` and trailing `.`, `,`, `;`, `:`, `)`, `]` and `}`
are prose outside a bare path's source range; quoted paths keep literal punctuation
and may contain one segment.
Code, links, URLs, relative paths and substrings of prose words stay unchanged. A chip
moves intact on wrap and ellipsizes within the usable width. It retains its full path,
occurrence and original token's byte range: message selection copies the original
token including wrapping quotes, while adjacent prose keeps exact partial selection.
The file popover offers Copy full path and Show full only; it never opens a file or
probes the filesystem. Live pane output keeps its literal path presentation.

Markdown keeps styled text, source-line mapping and typed member, URL and file hits.
`member_links` resolves `#tmt:`, canonical `@name`, and bare
canonical names containing a dash, underscore or digit against the worker-held
identity inventory. Matching is case-sensitive, longest-first and word-bounded;
simple bare words and unavailable or ambiguous identities stay literal. Source
and annotation byte ranges never change. Normal-weight text-colour underlines
carry UUID tags into visible wrapped/clipped cells; hover and keyboard focus use
the selected-row background. Conversation/pane-output Markdown, plain previews,
board task summaries, KeyValue rows and notices share that resolver and hit owner.
Rendered projections are cached by text, width/look and inventory generation;
plain fitted fields resolve their complete original text before deriving visible
hits, so clipping cannot create a shorter member name. Unused plain projections
are evicted after leaving the visible frame set.

One anchored menu offers Message, Jump to host, Copy, Note to lead and Show in
Agents, hiding unavailable actions. It uses existing row action owners and
rechecks UUID/membership and effective bindings on activation. Explicit Agents
navigation preserves drafts and changes only session visibility/search. Conversation references use Left/Right and Enter. On a selected row, `@` opens
a sole visible name's menu, or enters name focus for several: Left/Right chooses,
Enter opens, and Escape or row movement leaves. Ordinary tree fold/expand keys
remain unchanged outside name focus. The optional `@ names` footer never evicts
core hints; authored bindings and editors retain priority. Invalidation discards hits.
Built-in destination recognition grants only a text style;
custom handlers remain authored Config data with migration notices. The selected
agent's attached URL uses existing Open/opener effects and global argv settings;
a compatible explicit binding defines its own action. The removed notebook
adapter does not supply navigation or execution authority.
Legacy note tags and their request projection remain independent source data;
workspace response annotations use exact request IDs and raw source ranges.

## Replies and verification

`requests::replies` admits only recipient finals (`retained`, `expired`,
`unavailable`) from the user's squad-room requests, ordered by submission time.
Originator withdrawal (`withdrawn`, with reason/time) remains Core history and
never becomes a recipient reply, body read or approval. Open requests,
announcements and unknown final states are excluded from this projection.

`requests::bodies` retains the bounded room-final acquisition. The refresh worker
also requests one bounded originator-results page for HOME, member boards and
boards configured with a Replies pane. `home_leads::State` fences that projection by sender
UUID. `row_detail` keeps the newest eight finals (`Detail.finals`, oldest first, one per
request ID, with author, exact text and `submittedAtMs`) for the selected recipient UUID
across room and global observations; `Detail.reply` is the newest of them. In the Squads tree every
open request of the agent to the user (`pending_question`, from `waitingOnYou`) is placed among
them by its send time, so an agent that answers by sending new requests still shows in the
thread. Those windows are
bounded and shared, so the painted conversation (`pane_output_enabled`, Replies pane visible)
also reads its agent's own newest eight once per activation (`SelectedRead::Finals`,
`requests.list` with `originatorId`, `recipientId`, `view:"results"`); a core that refuses
that form leaves the windows as the only source. Once the finals are in, the same
conversation reads the user's own sends to that agent on activation and after
an acquired workspace refresh
(`SelectedRead::Sent`): one `requests.list` of the agent's recipient history filtered to this
sender (newest 20 of core's newest-50 page), then `requests.show` for at most five full prompts the
worker has not read yet, newest first; the worker caches texts (and gone prompts) by request ID, so
a later read of the same agent fetches the next unseen ones and a settled selection costs one list.
A prompt not read yet or no longer retained keeps its listed preview, marked truncated with
`tmt x show <request>`. They are
`Detail.sent` and render as time-sorted "you" blocks among the finals; a live session entry
with the same request ID replaces its restored copy, so sends survive an Ops restart and are not
capped by the session's own list. The same read carries Core's
[message state](../../../../contracts/request-response-v1.md#message-state) and passthrough
capture by exact sender/recipient/request ID. Delivery labels keep `reason.text`
verbatim, beside a mark from Core's state and its supplied countdown; capture and
local activity never override that line. The ordered send worker also observes the
accepted request once through `requests.show`. Missing state keeps the legacy
local label. The same `messageState.canSendNow` of the newest own message drives the Send now
chip on the conversation's footer row, with `Ctrl-S` (an authored binding wins outside the
editor): one `tmt send-now` per press as the person, the chip disabled until Core answers, and
the notice is Core's sentence or the re-read state label, never local wording. Answered captures show bounded safe Markdown as `turn output` (with
Core's truncation flag), and `no_answer` names the missing output. One turn can answer several sends
(typed into the pane while it worked): Core binds the final to the first, so
`conversation/answer_order.rs` groups every later direct send with no capture of its own that
was sent before the answered turn settled (a pending or bound first send groups optimistically
and regroups when a later send gets its own turn) into one message block: one header, bodies
stacked without a separator (one blank line when either body has several paragraphs), one status line (the newest line that is not `✓ delivered`) and
the answer after the last body. Annotate, ask-lead and failed sends never group; live entries
and restored sends use the same placement and keep each body's own source lines for copy and
quote. A bound turn has
no local ten-minute expiry. Captures never become ordinary replies or exact response attachments;
no pane observation decides capture or settlement. A pane observation is paired once with an answered capture in that exact
sender/recipient conversation, using identical provider, raw text and truncation
plus its public bound-to-settled window (or identical end time for older data).
Only unambiguous one-to-one pairs are omitted from recent pane output; equal
text from other turns and ambiguous observations stay visible. This is display
reconciliation, never request/source authority. A failed refresh preserves acquired sends until the next normal refresh.
Request bodies load off the UI thread on the selected-read lane, newest first, one selected read at a time.
Until Core has answered for a conversation's exchanges (the agent's own finals read, or the shared
originator page where there is none) it shows nothing for the first 400 ms, then `Loading
exchanges…`; "Exchange history unavailable" appears only after a failed read, and never beside
held messages. Finals come from storage
alone (`requests.list` and `requests.show`): the originator notification, which is
`unavailable` for a person with no live pane, and pane output never supply one. A header
derives its age from `Reply.at_ms` at paint, so a held reply keeps its true age.
The conversation renders every loaded final as a distinct request-keyed message,
deduplicating a local outgoing reply by request ID rather than text. Replies and
pane captures preserve authored single newlines through the conversation Markdown
entry; the underlying bytes and request-source authority remain unchanged.
Unavailable/expired source is not an annotatable response.

`board::conversation` owns one session per sender/recipient UUID and paints the
same selected-agent response in the Agents detail area for both list modes. Each session
retains the response, source anchor, follow state and Markdown render cache keyed
by source, width and look. Bounded outgoing turns use typed acceptance and exact
request correlation, without a transcript store. A later question from the same agent is a
block below the earlier sends and never clears them; the session keeps up to 512 sends
and 4 MiB. Reading older text pauses follow;
a replacement result is indicated without replacing the displayed request until
End or intentional downward scrolling to the bottom resumes follow. Resize and
new output alone do not resume it; Send enables follow.
Until an exact acquired reply replaces its feedback, a sent turn projects only
held provider activity: current working time, an earlier turn, an observed
current-working-to-idle completion, or honest unknown evidence. Task state and
request attention never establish this progress. Missing/stale evidence clears
unfinished progress; a later idle read alone cannot prove completion. Typed
queued/offline/held/uncertain outcomes keep their own delivery meaning.
Only painted status rows admit the existing frame clock; spinner frames decorate
one glyph without rebuilding the body. The existing token-rate reduced-motion
preference applies even with sampling disabled (selected room override, otherwise
global), and NO_COLOR keeps a static glyph with elapsed text. These clocks acquire
nothing and persist no transcript or provider history.
The shared exchange painter uses Accent and bold for your sender/replier name,
Text for the recipient and other names, and Working for the spinner, `is working`
and `✓`; arrow/time metadata and queued feedback keep their neutral roles.
Status spans are styled before the shared word wrapper so narrow continuations
retain their role without parsing member names or changing layout.
`Scrolls` remains the sole viewport owner. Rendering and reading acknowledge
nothing. Layout uses terminal cells and degrades side/stack preferences to one
focused pane when dimensions cannot support a split.
In Agents member focus, the same selected worker reads [recent pane output](../../../../contracts/extension-api.md#recent-agent-responses)
after capability discovery, by exact identity UUID. Each session projects at most
ten provider-final captures, oldest capture first without a timestamp sort. It
stably merges the exchange and capture heads by their literal times, exchange
first on ties. Final message blocks use their settled times; capture order stays
as returned by Core through clock rollback. A capture never absorbs a final,
even when both contain identical text.
Neutral `agent · pane output · time` headers use the exchange age wording, including `ago`; a muted `… truncated` caption
separate these observations from replies. Pending and observed-empty pane reads add no line; unsupported/failed reads remain distinct. A failed read says `Recent pane output unavailable`;
an empty conversation requires both sources observed empty. Pane text creates no
request-source, reply, acknowledgment or annotation authority. The existing
UUID/revision/cancellation fences reject obsolete reads, and the shared viewport
keeps long captures scrollable without displacing the composer. The flat Agents list keeps its
question-only presentation and does not acquire or paint these captures.

Unread markers are separate from Core acknowledgments. After a successful draw,
App admits only the exact actor/member/request submission of an acquired displayed
reply in the Agents conversation while the reader holds focus; moving the list
selection onto a row never reads it, and leaving the reader makes the next opening
a fresh one. Loading, hidden, modal-covered or paused newer text cannot mark
unseen output. A guarded `j` seals the actor/member and newest observed
submission before leaving the UI; only successful focus publishes that marker.
The existing ordered lane max-merges shared Config markers. Quiet coalesced writes
own no action-progress overlay; visible unread clears only after successful
publication. A failure retains unread evidence and pending updates for an explicit
reopen/j retry, without an automatic retry loop. Shutdown drains admitted writes
after restoring the terminal. Shared markers survive restart, named-view changes,
reset-view and exec reload. Pane captures never supply request read watermarks.

Agents focus titles its currently actor-only history `Exchanges with you`; an
all-party title waits for #3217's read contract. It creates no other-pair rows or
qualification sentence. The existing conversation session supplies grouped
exchanges and pane-output observations. Its filled three-row input follows the
last observed group with one blank row above; only overflow pins it to the bottom.
The same InputField/editor owns the flat list, member focus and squad focus, with no
frame or title rule. Factory Enter replies to an observed exact question or
messages the selected member/current squad lead from either pane; active Enter
submits, and Esc keeps the draft and returns to the originating list. Common pool
and an unobserved or replaced lead grant no substitute recipient. The effective
footer owns key hints, including the active terminal-capability newline key; authored bindings retain their meaning. The effective Member/Reader
`view-reply` binding (default `v`) opens the full response reader without changing
its exact-source admission; a missing response is explained in the status line.

The composer opens the selected workspace UUID and room directly; flat row
indices or equal payloads never choose a different membership. `RowSend`
retains the observed sender UUID and squad-note recipient identity.
Submission rechecks the current roster membership, sender and recipient before
producing an effect; a matching name cannot authorize a replacement UUID.
Retained row text alone grants no authority after its membership disappears.

`board::conversation::source` selects the already rendered conversation and task
text in place. Both painters use `Look::selection()` for the selection style,
including its reverse-video fallback when no selection background is available.
Mouse-up copies the selected plain text through the configured
clipboard worker; a failed copy does not prevent quoting. Read-focus Shift+arrows
extend the selection, `y` copies, and Esc or an outside press clears it. A task
field is frozen as the cells it displayed. The transcript changes and scrolls under
the pointer, so its selection (`source/anchored.rs`) is anchored to transcript
lines and cell columns instead: a progress tick, new output, a growing input band
or a scroll keeps the highlight and the button, text out of view is clipped and
comes back with its line, and a rebuilt transcript finds the line again by its
displayed text: the same line, else the line it grew into while the agent streams,
else the same line with only its tail changed (an age stamp). Pointer events use the
last painted transcript until the next frame rebuilds it, so output that lands
between a press, a drag and the release never drops a press, the drag or the copy.
A drag reads the pointer against the transcript as it is now and, at the pane's
first or last row, scrolls one line per pointer event, so holding the pointer still
at the edge does not keep scrolling. The selection ends on Esc, an outside press, a
new drag, a conversation switch, or when it can no longer be shown: its lines are
gone, an end's text changed before its end, or a resize re-wrapped the transcript.
A selection opens the Selection menu (`conversation/source/menu.rs`): a bordered box with
`q› Quote` and `y  Copy` on their own lines and a dim `Esc clears`, directly below the
selection end. It flips left of the end near the right edge and opens above the first
selected row near the bottom, so it never covers selected text; it stays inside the
pane and falls back to the frame when the pane is too short (no room at all, no menu; the
keys still work). Keys and mouse run the same entries; a press on the border or the hint
neither acts nor starts a selection.
Copy and Quote carry the Markdown source of the selected messages
(`conversation/copy.rs`; each displayed line keeps its message and source line): tables,
code (with its fences), headings, lists and quotes as whole source lines, and a selection
that starts or ends inside a plain paragraph line as the exact source characters. Rows
without a source (headers, status lines, pane output) keep their displayed text. Quoting never
depends on the agent's state. The
`q Quote` entry, `q` or Enter inserts a literal quote fold after the logical line
containing the composer caret. The whole current line and later text remain;
the fold occupies its own line and the caret lands on the following empty answer
line. An empty draft starts with the fold and answer line. Repeating the gesture
keeps quotes and answers in draft order. Before admission, quote CRLF and lone CR
breaks become LF and one terminal break is removed; additional empty lines remain.
Editor owns those admitted bytes and fold ranges; `composer::chat_quotes` retains
only typed provenance by fold ID. The
shared paste/file fold painter, current-hit fences, atomic × removal and full-source
preview also serve quotes. Caret motion skips collapsed folds; there is no undo
facility. Literal quotes grant no request authority and need no source revalidation.

`board::conversation::screen` shows the selected agent's `pane.screen` in the Talk
pane while `l` is on (`screen_mode`); it replaces the chat and every chat read,
and the Agents-tab Talk pane draws it through `agents_focus`. It is one
selected-read lane, `SelectedRead::PaneScreen(key, round)`: a read is requested
only when the pane is visible and `screen_due_ms` has passed, `round` makes each
request distinct, and a result applies only while the same read is still
selected. A failed read keeps the last good screen marked stale and doubles the
wait up to eight seconds. `board::screen_view` is the trust boundary: it keeps SGR
and drops every other escape and control from the foreign text, and maps colour
by the terminal's depth. The screen is not selectable.

`board::response` separately owns exact raw source, the full-response Reader and
request-bound row-note admission. Its selections insert through the same inline
fold owner, retaining request, passage and recipient revalidation. Sending replaces
each quote fold in place: rendered literal selections use the existing flattened
200-character Markdown excerpt, while exact selections use full multiline
blockquotes. Preview shows the complete admitted text. Exact quotes keep their original source
key and byte range; send admission compares the current source with its normalized
admitted form. Plain text and other
folds remain in order. Details freezes exact-note provenance only when recipient
and source UUID match. Quote metadata does not retain a second payload or choose
a new recipient. UUID draft and submitted-revision fences also cover quote folds.
Alt-Backspace deletes the previous word in the draft.
Native terminal selection remains available with the
terminal's mouse-reporting override. Colab URLs use existing link effects;
native Colab page/thread writes are not implemented.

Relevant tests are `notes` (sanitizer and response anchors), `markdown`, `links`,
`response`, `conversation` and workspace App tests (source authority, composer and
hits), plus `refresh` (selected-message coalescing, generation and child shutdown).
[development.md](development.md#checks) owns verification commands and
[TUI development](../../tmt-tui/references/development.md#board-parity-baseline)
the frozen parity gate.
