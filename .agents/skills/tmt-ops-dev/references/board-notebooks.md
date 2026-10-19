# Board notebooks, detail and replies

[board.md](board.md) owns frame reset, composers and `Scrolls`. The shipped Squad
skill owns [notebook controls and link syntax](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#keep-your-notebook-current)
and [detail/reply appearance](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#board-appearance).
Effect policy belongs to [config-and-effects.md](config-and-effects.md#actions-and-effects);
the [refresh reference](refresh-and-meter.md#refresh-worker) owns worker priority/cancellation.

## Inert text and independent notebook/source utilities

The workspace has no notebook/detail pane or notebook-link activation route.
Selecting an agent never queues `notes.read`; selected worker jobs are typed
Status previews, exact Message sources and bounded recent pane-output observations. The old notebook event, eight-entry
cache, per-squad cursor/link maps and painters are removed. Notebook storage,
public source APIs and saved-identity behavior retain their independent owners.
`board::refresh` calls `observe::observe`, which reads the lead's notes only when
the board shows them or observation is active, and retains the bounded
acquired-data conversion, including missing, no-lead, temporary-identity and
failed-read distinctions; it creates no notebook. Incompatible notebook rendering/handler settings get
located migration notices without rewriting authored values.

`board::notes::sanitize` removes escapes, controls and hidden bidi/format
characters, preserves newlines and script/emoji joiners, and expands tabs to four
spaces. Shared Markdown renders headings, lists, neutral underlined links,
filled code and quote gutters; fitted tables use a boxed grid and fall back to
source when they cannot fit. Unsupported HTML, images and footnotes remain
literal. Code lines clip without wrapping, and inline code adds no padding. The retained `NotesCursor` anchors response source lines and wrapped
continuations; it reconciles nearest duplicates and clamps deleted lines.
Conversation Scrolls owns the viewport, independently of that source anchor.

Conversation bodies fold independently after 12 rendered rows (11 plus the expand
caption); the newest reply starts expanded. The conversation session retains
presentation choices by request/source or outgoing sequence and prunes vanished
bodies. Fold captions have neither exact-source nor member-action hits. A separate
anchor map preserves reading position while source mappings retain raw identities.
Visible fold controls support pointer activation and read-focus Tab/Enter/Esc;
editors, member menus and authored bindings keep priority. No fold action sends.

Markdown keeps styled text and source-line mapping; it retains no URL destinations
or URL activation hits. `member_links` resolves explicit `#tmt:` references against
worker-acquired global identity UUIDs and canonical names. Markdown wrapping carries
those tags into clipped current-frame rectangles; plain composer/Recent previews use
the same projection. Conversation source and annotation byte ranges remain unchanged.
A single existing menu surface anchors above/below the painted reference. Its actions
recheck the acquired UUID/membership and effective configured host action; explicit
Agents navigation preserves drafts and changes only session visibility/search.
Unknown references and failed inventory observations remain literal. Keyboard traversal
uses visible conversation references only; frame invalidation discards pointer hits.
The inventory generation invalidates mention-bearing render caches off refresh,
without scanning all identities on ordinary frames. Built-in destination recognition grants only a text style;
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
across room and global observations; `Detail.reply` is the newest of them. Those windows are
bounded and shared, so the painted conversation (`pane_output_enabled`, Replies pane visible)
also reads its agent's own newest eight once per activation (`SelectedRead::Finals`,
`requests.list` with `originatorId`, `recipientId`, `view:"results"`); a core that refuses
that form leaves the windows as the only source. Request bodies
load off the UI thread, newest first, one selected read at a time. Finals come from storage
alone (`requests.list` and `requests.show`): the originator notification, which is
`unavailable` for a person with no live pane, and pane output never supply one. A header
derives its age from `Reply.at_ms` at paint, so a held reply keeps its true age.
The conversation renders every loaded final as a distinct request-keyed message,
deduplicating a local outgoing reply by request ID rather than text. Replies and
pane captures preserve authored single newlines through the conversation Markdown
entry; the underlying bytes and request-source authority remain unchanged.
Unavailable/expired source is not an annotatable response.

`board::conversation` owns one session per sender/recipient UUID and paints the
same selected-agent response in the workspace Overview and Agents detail area. Each session
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
keeps long captures scrollable without displacing the composer. Overview keeps its
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
The same InputField/editor owns Overview, member focus and squad focus, with no
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
text in place. Mouse-up copies the selected plain text through the configured
clipboard worker; a failed copy does not prevent quoting. Read-focus Shift+arrows
extend the selection, `y` copies, and Esc or an outside press clears it. The
`+ Quote` control or Enter adds a literal author/excerpt chip above the input,
without replacing the draft text or granting request authority. Up from the first
input line enters that chip row; Left/Right move between chips, `x` or Delete
removes one, and Down/Esc returns to the input. Sending projects each chip as one
compact Markdown quote above the comment. Quotes retain the UUID-keyed draft's
success/failure lifecycle and need no source-identity revalidation.

`board::response` separately owns exact raw source, the full-response Reader and
request-bound row-note attachments. The configured note action may attach an
admitted response selection as exact multiline Markdown blockquotes following
`[squad · row]`; without one it keeps the existing inline tag. Task selections do
not attach automatically to notes. These exact attachments retain request,
passage and recipient revalidation; literal chat chips do not replace that owner.
Details freezes exact-note provenance only when recipient and source UUID match.
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
