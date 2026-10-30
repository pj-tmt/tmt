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
(a `Menu`: the origin as an inert row, then Open, Copy, Show full). A bare address is a
word less its closing punctuation (`member_links::word_token`) that `is_bare_url` admits;
plain rows (`paint_plain_hits`: notices, key/value fields, workspace rows, the squad
focus) use the same rule from the row's complete source, cached with its member names,
and keep only the painted part of a cut address as the hit, opening the whole one. `effects::open_argv`
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
the selected-row background. A reference grants no new delivery authority and does no paint/input I/O. Conversation/pane-output Markdown, plain previews,
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
`Detail.sent`; the legacy renderer places them among acquired exchanges by their
public clocks, while a committed timeline owns its own order; a live session entry
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
the notice is Core's sentence or the re-read state label, never local wording.

When Core publishes `responses.timeline`, the selected worker acquires its committed
conversation stream and canonical response bodies together. `conversation::committed`
groups only consecutive inputs with the same explicit `inputBatchId`; finals end an
input group. Core's ascending entry order is preserved, including clock rollback and
interleaved batches. A final appears once by opaque `responseId`; an explicit reply
appears once by `requestId`, independently of any provider final. No body, time or
request-only provider-final matching is allowed. A pruned canonical final may use the
request's retained first-answer copy only through the same `responseId`.

`conversation::timeline` is the single renderer for this stream and the legacy
observations. Inputs and agent replies use the existing reply-block primitives;
captured answers have an `in turn` header without a second `turn output` or `pane
output` heading. Sender and source attribution come from the frozen acknowledgment
keys where complete: the viewer's key keeps normal styling, only other senders mute
the reply. Empty keys, or a truncated set without the viewer, use the immediately
preceding same-batch input group; unknown page-start attribution remains normal.
Unknown input senders stay unknown, uncertain delivery is labelled, and only explicit
Core failure entries paint a failure caption. Local outgoing entries own optimistic
input/send state, never a second reply body. Each body keeps source/copy/quote
provenance; captions and headers grant none.

Without the capability, existing request and recent-response observations use the
same renderer, keeping their acquired ordering and explicit source identities.
Unlinked answers remain visible; different response IDs stay separate even when
text is identical. No settlement-window grouping or inferred batch is retained.
Missing or truncated output keeps its coverage caption. A failed refresh retains
acquired content; no pane observation decides capture, settlement or reply authority.
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
Unavailable/expired source is not an annotatable response. Conversation Checklist changes by
someone other than the Core user appear as muted `≡ actor · checklist · action title · age`
rows. Current held titles identify items; missing items or actor labels say `an item` or
`an agent`. Reorders say `reordered the checklist`. Activity, gap and failure captions
share the transcript viewport but carry no message-source or acknowledgment authority.
The [checklist owner](data-and-state.md#checklist-storage-and-service) holds the public
mutation log cursor and bounded facts; the transcript neither infers changes from snapshots
nor performs reads while painting.

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
In Agents member focus, the selected worker discovers the public conversation
capability by exact member UUID: prefer [ordered captured conversation](../../../../contracts/extension-api.md#ordered-captured-conversation),
otherwise use [recent agent responses](../../../../contracts/extension-api.md#recent-agent-responses).
One page starts the session; deliberate upward scrolling requests the older page
on the same selected lane, one read in flight. Timeline cursors stay opaque and
older entries prepend in committed order. The legacy numeric cursor and returned
capture order keep their existing meanings. Core owns retention; Ops never expires
or count-prunes acquired history or sorts a timeline by time or opaque IDs.

`recent_responses` admits page metadata and reconciles cursor chains. An ordinary
latest read splices at the held stable entry IDs (timeline) or one exact held-newest
answer record including provider/truncation metadata (legacy). Ambiguous/missing alignment (including an unalignable retention cut)
or an explicit refresh key/action rebuilds only the held window serially and cancellably:
stop at the first page boundary covering the held item count (with the existing
oldest-time fence only for the legacy response protocol), keeping `nextBefore` for deliberate older paging. Ordinary
snapshot, theme and member refresh effects keep the anchored read. Old content stays
held until the coherent result arrives. Coverage changes update the held history caption
without re-walking an otherwise aligned chain. Failed reads retain loaded answers and the cursor;
a failed older page waits for another deliberate scroll or refresh, without a retry
loop. Actor/member/revision/cursor fences reject obsolete completions.

Agent reply headers use the existing exchange age wording when a source clock is published. A
truncated answer has `answer truncated · full text in the agent's pane` as its
last muted line inside the answer gutter. History captions are inset one cell
and carry a registered state mark. Only the final page can show `older answers not kept` at the top of history;
Core's supplied time is qualified as `at or before`, including migrated floors.
`capture.enabled=false` adds `answers are not captured for this agent: run tmt setup`
while keeping retained answers; absent capture metadata adds no hint and never
causes setup. Pending/observed-empty, unsupported and failed reads remain distinct.
A failed latest read says `Recent pane output unavailable`; a failed older page
names the failure and scroll retry at the older-history boundary. An empty conversation requires both sources
observed empty. Pane text creates no request-source, reply, acknowledgment or
annotation authority. The shared anchor/viewport keeps prepended answers readable
without moving the composer. The flat Agents list keeps its question-only
presentation and does not acquire or paint these captures.

Core acknowledgment is the only unread projection. After a successful draw, the
conversation owner admits exact body rows inside the clipped viewport: final or
captured answer, actor/member and source identity. Timeline final/reply bodies carry
the producing transaction's frozen `acknowledgements` revisions directly; they never
substitute a newer revision or infer keys from nearby text. Inputs and failures have
no keys. Only the viewer's listed keys are admitted, at most 50; explicit Mark read
handles a truncated remainder. Legacy bodies retain their public timestamp and held
attention-revision checks. Both placements use the same provenance helper.
Headings, separators and live screens grant no acknowledgment; legacy provider
observations without published frozen keys grant none either.
Loading, hidden, modal-covered, unfocused or failed draws admit nothing; moving the
list selection or jumping with `j` does not mark unseen answers read.

The existing ordered action lane batches 1–50 distinct request/revision pairs through
public `requests.ack`, with at most one batch in flight until its result is adopted.
Acknowledged/already settle the exact revision; notFound settles without retry;
conflict adopts current and keeps it unread until fresh body evidence exists. A late
page cannot regress a held revision or its successful acknowledgment. Failures never
claim read and do not cause an automatic retry. Reopening the reader or explicit
Mark read supplies a new admission. Failed work does not block other conversations.

Every painted `● N unread` (top bar, an Agents row's STATE, the conversation header,
the composer info line) records an `unread_hits` area. The one under the pointer
(`App::unread_hover`, while the board takes input) is restyled after the frame's counts
with the composer chip's Hot style. A press on one, or the `unread` action (`u`), opens the Unread overlay
(`board/unread_overlay`): the scope's unread answers, grouped by agent, oldest first,
read per agent on the selected lane while it is open (`SelectedRead::UnreadBodies`,
keyed by overlay session; a failed read is asked again only after scrolling away and
back). It feeds readable body rows and terminal unavailable-caption rows painted in its
viewport to the same revision-bound writer. Loading and failed-read captions grant
no evidence; a newer revision cannot inherit an earlier draw. After the scope's held
entries settle, `SelectedRead::UnreadPage` follows one `requests.list` answers cursor
on that same lane, fenced by actor and overlay session. Entries append below the retained
blocks; HOME head-page refreshes do not reset this continuation. Reading metadata never
acknowledges it, and a failed continuation waits for reopening rather than looping.
Catalog action `mark-read` and the member menu still explicitly
acknowledge the selected conversation's unread answers without showing them or
focusing the composer. `● 0 unread` is not a target. Counts use one bounded summary page,
plus older unread metadata already retained by the open overlay (without duplicates);
explicit Mark read and a visible older body resolve further metadata in bounded public
pages, yielding between lane steps, without a history cache or per-item show calls.
Paint and hover do no I/O. Shutdown joins the lane before draining admitted writes and
reporting unresolved failures. Legacy-cutoff consumption belongs to
[the Config owner](config-and-effects.md), never a second unread authority.

Agents focus titles its history `Conversation` on both Core paths. The session
supplies one ordered transcript; its filled three-row input follows the last
observed block with one blank row above, and only overflow pins it to the bottom.
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
`q› Quote`, `y  Copy` and `n  Add to checklist` on their own lines and a dim `Esc clears`, directly below the
selection end. It flips left of the end near the right edge and opens above the first
selected row near the bottom, so it never covers selected text; it stays inside the
pane and falls back to the frame when the pane is too short (no room at all, no menu; the
keys still work). Keys and mouse run the same entries; a press on the border or the hint
neither acts nor starts a selection.
Copy and Quote carry the Markdown source of the selected messages
(`conversation/copy.rs`; each displayed line keeps its message and source line). A selection
end inside any source line (list item, quote, heading, table row, code or paragraph) cuts
there, so a few words are exactly those characters; painted-only glyphs (bullets, quote bars,
borders) are skipped when aligning, and text that cannot be aligned copies what it shows.
Markers come with a line selected whole, and code fences or a table's header and delimiter
only with a whole line of that block. Rows
without a source (headers, status lines, pane output) keep their displayed text. Quoting never
depends on the agent's state. The
`q Quote` entry, `q` or Enter inserts a literal quote fold after the logical line
containing the composer caret. The whole current line and later text remain;
the fold occupies its own line and the caret lands on the answer line after it. At the
end of the draft an empty line separates the fold from that answer line, so typed text
starts its own paragraph; inside the draft the following line break already does. Sending
also ends each quote with an empty line when the text after it would otherwise continue
the quote. An empty draft starts with the fold, an empty line and the answer line. Repeating the gesture
keeps quotes and answers in draft order. Before admission, quote CRLF and lone CR
breaks become LF and one terminal break is removed; additional empty lines remain.
Editor owns those admitted bytes and fold ranges; `composer::chat_quotes` retains
only typed provenance by fold ID. The
shared paste/file fold painter, current-hit fences, atomic × removal and full-source
preview also serve quotes. Caret motion skips collapsed folds; there is no undo
facility. Literal quotes grant no request authority and need no source revalidation.

Each squad has its room checklist, separately from each member's conversation list.
The squad row's `○ N todo` count opens the exact room list; the squad detail keeps a
visible Checklist button, including empty or unobserved lists. A known count appears
inline as `Checklist · N todo`. The existing Actions entry opens the same controller.
Current-frame UUID hits, loading and overlay guards prevent a stale row from choosing
another room. A row's optional count yields before the existing state/unread words.

Every open, nonarchived item is a todo, including an item with a reference. Completed
and archived items are absent from board lists and counts. The chip keeps its effective
keycap and reads `Checklist · N todo`, or `Checklist · empty` when none are open.
Completing a row removes it after the admitted write and shows `✓ Completed <title>`;
reopening is a CLI action. Storage and completed-item references remain unchanged.
The global header includes
each room list and each admitted room/member conversation once, independent of search,
collapse or viewport. Unknown and partial totals retain their existing presentation.
[Acquisition ownership](board-home.md#checklist-todo-projection) stays on the existing lane.

The conversation entry is `C  Checklist · …` (the key as a keycap, none when unbound); its modal is `Checklist · <member>`
(with the item count), and a source form is `Add to checklist · <member>`.
The list is one list in stored order (`rows.rs`, no kind grouping), with no header, in a sheet
that asks for the whole board body; the modal then keeps its usual 90% cap, or the full width below
the md step (the width is part of the template key, so a resize recompiles it). Each item is a two-line `flex-col` row, and the selection covers both lines.
Line 1 is the cursor, `OPEN_MARK` (`○`, `waiting` role) and the wrapped title. Line 2, under the
title, is `LINK_MARK` (`↗`) and `address()` in the `link` role, then the byline in `dim` capped at
24 cells. `address()` gives a GitHub issue/PR as `owner/repo#N` and any other reference whole
without its scheme. A todo's line 2 is only the byline. The link text carries the `link` hit id; the
controller keeps its painted areas (`link_hits`) and the item under the pointer (`hovered`, from
`App::move_pointer` while Notes is the top overlay). A press on a link selects its row and opens it;
the cursor row's link and the hovered link paint in the selection colour, underlined.
The title text carries the `title` hit id; its bare web links (`member_links::bare_urls`,
the transcript's rule) map onto its painted glyphs the same way and become links 1.. of
the row (`LinkHit::link`; 0 is the item's own link), each hovered on its own. Shift+↑↓ swaps
with the neighbouring row of the same list. Captions
resolve held preview/roster data by the frozen room/member UUID, falling back to
`member <short UUID>` when no name is held. Rendering never reads Core or borrows
another selected member's name. Both form variants label the first field Title.
The canonical authored action is `member-checklist`, bound to `C` for members
and the Reader (`N` stays create-squad on a squad row); the chip leads with that key as a
keycap, taken from the effective bindings. This breaking rename before launch has no `member-notes` alias;
an authored former action gets the ordinary unknown-action notice.

Enter on a link row follows the global `board.checklist_enter` (`open` by default, or `tick`; a todo
row always ticks). It is a `View` field built beside `ask_lead`, edited on the `,` page and by
`tmt ops sq config set`; a bad authored value is noticed and reads as `open`. Space always ticks and
`o` always opens the link, so both actions stay reachable under either value; the footer names
`Enter tick · o open` only under `tick`.

`n Add to checklist` in the conversation Link popover or Selection menu opens the existing
checklist draft. Links preselect Link and keep the full URL, with its last path segment
(or host) as the editable title; selections preselect Todo with the selected source
text flattened into the title. Kind, Title, Link and From share the conversation checklist modal;
Tab changes fields; Kind uses the shared flat chips, with Left/Right or a current
chip click selecting the explicit kind. An existing
unsent draft is retained instead of being replaced. The held room/member/actor
must still match at entry and submission; an unread checklist cannot be written.
The conversation checklist records descriptive message provenance in the existing item body;
its render metadata grants no request authority and introduces no schema or store.
`conversation::note_source` maps rendered rows to held message metadata, separately
from exact response/quote authority. From shows author, kind and time; the existing
checklist body records one human-readable Source line with the exact request ID or
pane-observation context and timestamp. Mixed and missing sources stay explicit.
The link popover retains its opened source through Show full/back while the transcript
refreshes; another clicked hit replaces it even for the same URL. No URL/text
matching chooses a message source, and source descriptions grant no jump-back
authority. All reads and writes retain the existing checklist worker, revision and
failure ownership.

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
