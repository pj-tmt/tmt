# Board notebooks, detail and replies

[board.md](board.md) owns frame reset, composers and `Scrolls`. The shipped Squad
skill owns [notebook controls and link syntax](../../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md#keep-your-notebook-current)
and [detail/reply appearance](../../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md#board-appearance).
Effect policy belongs to [config-and-effects.md](config-and-effects.md#actions-and-effects);
the [refresh reference](refresh-and-meter.md#refresh-worker) owns worker priority/cancellation.

## Lead notebook and links

- The notes pane reads the lead's saved-identity notebook, with no separate squad
  notebook. `observe` chooses `Member::is_lead` and reads the UUID via bounded public
  `notes.read`, never creating a file. `refresh::lead_notes` distinguishes missing,
  no-lead and failed reads; hidden notes use `Notes::NotShown`. A temporary lead
  remains an API failure.
- `board::notes::sanitize` removes escape sequences, controls and hidden bidi/format
  characters from agent-written text, preserving newlines and script/emoji joiners
  and expanding tabs to four spaces. `board::markdown` renders the supported subset
  over sanitized input and leaves unsupported constructs as source. Plain mode
  creates no interactive links. `view::notes` caches lines, source indices, links
  and wrapped display-cell ranges by width/look in the immutable view.
- `links` classifies explicit destinations and admits configured handlers;
  `markdown::render_links` keeps destination occurrence and source-line identity
  through wrapping. `App` selects/previews before activation and validates built-in
  row/lead targets and bounded composer text; custom argv comes only from the loaded
  user-file handler. Unknown/invalid schemes and relative local paths are inert.
  Parsing/painting fetches nothing and executes nothing; activation reuses existing
  prompts, request pickers, opening and reaping owners.
- `App.note_cursors` keeps one cursor per retained squad, anchored to the complete
  sanitized source line with a wrapped continuation offset. Refresh chooses the
  nearest duplicate match or clamps after deletion. `Scrolls` reveals the cursor;
  wheel scrolling suspends following until cursor movement. Source mapping comes
  from the same rendering pass, with a fixed two-cell gutter before wrapping;
  only visible continuations receive selection decoration.
- Notebook annotations use the existing composer/sender and current lead. The
  `[<squad> · notes L<one-based line> <JSON quote>] ` tag is shared by send and
  `requests::apply`; display quotes are separate. Bounded room-history projection
  provides open markers. Painting matches the nearest source line starting with
  the retained quote; answered annotations disappear on refresh. No notebook write,
  extra read or acknowledgement is introduced.

## Selected detail

- `view::detail` appends full projected column values in column order after fields
  already represented by header/task/activity/links, escaping and wrapping without
  grid fitting, source lookup or provider calls. The displayed lead occurrence uses
  compact detail and separate notes/replies panes; `App::selected_is_lead` uses
  `RowOrigin::Lead`, including retained views/search, rather than guessing from name.
- `App::notebook_identity` requests only a saved selected member with visible,
  expanded effective detail (including view preview, tabs focus and last painted
  viewport). Temporary identities get a placeholder without a read; HOME/leads have
  no member notebook. `refresh::Deferred::Notebook` uses bounded cancellable public
  `notes.read` (1 MiB API limit), never creating a notebook. Full loads outrank
  selection jobs, which collapse to the latest; generation and selection/refresh
  revision fence events, and each snapshot revalidates the visible selection.
- `board::notes::Notebooks` retains the last eight identities' sanitized results,
  preserving rendered bodies for unchanged content. Width/look/render-mode changes
  invalidate rendering; failed reads replace the selected entry. Both notebook
  panes share Markdown/plain rendering and missing-file styles.

## Replies and verification

`requests::replies` admits only recipient finals (`retained`, `expired`,
`unavailable`) from the user's squad-room requests, ordered by submission time.
Originator withdrawal (`withdrawn`, with reason/time) remains Core history and
never becomes a recipient reply, body read or approval. Open requests,
announcements and unknown final states are excluded from this projection.

`requests::bodies` considers only the newest `BODIES` (eight) finals and loads those
still retained through `requests.show`. Its refresh-worker cache uses immutable
request IDs and prunes to that same eight-item window. `view::replies` sanitizes
and wraps them through the notebook renderer. The immutable view's
`Derived.replies` caches bodies by request ID, width and look; headers/prompts are
assembled per frame so ages advance without reparsing. New views discard that
render cache. Reply scrolling belongs to `Scrolls`; reading acknowledges nothing.

Relevant tests are `notes` (sanitizer, cursor and bounded cache), `markdown`, `links`,
`app` (composer/link fences), `refresh` (selection jobs) and `view/tests` (detail,
notebook selection, replies and hits). [development.md](development.md#checks) owns
verification commands and [TUI development](../../tmt-tui/references/development.md#board-parity-baseline)
the frozen parity gate.
