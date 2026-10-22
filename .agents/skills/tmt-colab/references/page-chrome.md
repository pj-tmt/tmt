# Page chrome (#1586)

`colab-header.tsx` and `notice-card.tsx` adapt Colab content to the private
`@tmt/browser-ui/react` Header/Notice exports. Router chrome uses the shared Action/Toggle/List
and static classes; Theme, page icons and close use `BrowserIconAction`.
`BrowserMenu` supplies disclosure state, popup semantics, trigger ref and keyboard handling;
it owns menu dismissal and focus return. Original trusted activation events remain Colab-owned. App and reader
entry points import `/static.css`. Header/card geometry, opaque square surfaces, mark plus
visible state word and shadow-free presentation belong to that leaf; its
[package contract](../../../../design/browser-ui/README.md) owns generation and host inputs.
`colab-header.css` supplies Colab safe areas, fixed viewport placement, body offset and host
metrics; `notice-card.css` retains product page positioning and inherited state-label type. Legacy product color/font
variable names alias the shared browser roles rather than projecting another palette.

Owner headers show muted `by <name>` next to the title; ready readers retain
`By <name>` below it. Only the original author supplies the caption; with none it is absent. `page-attribution.tsx` presents that
field (`Unknown author` when absent) and the independently optional latest publisher in
About this page (Page information for read-only readers). These are escaped publisher-provided display labels, never identity,
permissions or Ask recipient selection. Long captions truncate with the full label in the tooltip and attribution panel.

The mounted app also supplies a display-only Remote session snapshot through the existing
Header status slot. It reads the admitted SDK device list, up to 20 pages of 50 rows,
on mount, visible-tab return and opening the details. One read owns all pages; a
refusal or unfinished traversal shows an unconfirmed count, never a partial sum or zero.
Current browser comes first, then recent activity; details omit zero-session rows without
changing the confirmed count.
Counts describe live Sessions, not tabs, and closing a tab may take about a minute to appear.
The read time and this limit stay visible in the tooltip. A successful complete read supplies
the same-origin `/settings` link. Hosted/detached adapters and missing Sessions show no chip.
`remote-presence.ts` owns the finite snapshot and replacement/close fences. Replacement
starts one read after any stale read settles; close prevents that successor. `mounted.ts`
uses its already-admitted Session without reopening. `remote-header.tsx` owns only presentation
and the three refresh triggers. The space home header uses the shared Tooltip chip on desktop and,
through 640px, a Remote submenu in its More menu with the same details and settings link. The page
header keeps no Remote chip: its More menu holds that Remote entry, after Share…, and the header shows
`△ Remote offline` (a mark and a word) only while the count cannot be confirmed. No shared header
API, presence stream, periodic polling or new Remote operation is introduced.

Native pairing/app-unavailable guidance stays static. `chrome.rs` compile-time embeds the same checked
CSS and Colab host/reader/notice layout styles; `/assets/chrome.css` serves those immutable
bytes even without an app build. `socket.rs` uses shared static header/notice slots, escapes
product text, and keeps the parent CSP free of inline style/script exemptions. Guidance
loads the admitted build's recovery entry only when present; otherwise its details remain
visible. Neither Cargo nor installed serving runs Node or fetches external assets.
Signed-in app-unavailable guidance shows a waiting-colored Update needed state and
asks the person to update Colab to open the page, preserving space/sign-in details and the user command.

`router.tsx` retains one active Source, Comments, Chat, Agents, Files, Export, Manage or
About overlay. `page-header-actions.tsx` owns the page header's base entry points at every
width (#4116): Search, Discussion (combined pending count; opens Chat or Comments), Agents and More.
More lists, top to bottom: Share… (the page's sharing and connection words as secondary text; it
opens the Manage dialog), Remote (the submenu above), Projects… (when the viewer writes
projects; opens the picker below), Pin to top / Unpin (below 640px, once the owner list confirms a
non-archived page the viewer can write), Files (count; while the page holds a file or the viewer can add
one), Source, Export page, a rule, then Manage page, Theme: Light/Dark/System and About this page.
The page's own projects (`page-projects.tsx`) follow the title: the first three as small chips
and a count of the rest (hidden under 640px), then the list row's picker (`PageProjectsMenu`: up to
eight projects, New project… names one and puts this page in it). The chips, the trigger and
Projects… open the same menu. The chips come from the list the space home last loaded, so arriving
from the list reads nothing extra. A page opened by a link has no list yet: it reads `spaceHome(signal)`
once on open (#4130), and leaving the page aborts that read through the signal, so no read outlives
the page. The owner list is also read when the picker opens and after each write. A viewer who cannot
write projects shows none of it. The desktop pin beside the chips and the phone's More item use
the same `PageProjects` write, owner-list refresh and refusal words; the menu only projects its
confirmed pin state and pending write. Archived pages and non-writers have no pin item. Below 480px a pin
refusal also appears in a closable shared inline Alert in flow under the header, using the same refusal
words. In the shell it joins the header slot, pushing the phone sub-bar and page down by its height. That notice alone announces the failure at that width; the header keeps its mark. Success,
leaving the page or Close clears it; at 480px and wider the existing header alert is unchanged.
Each moved action keeps its accessible name and handler. Icons are centralized inline SVG;
labels supply hover and keyboard-focus tooltips. Counts come from the existing page projection.
The floating Chat icon shares the selection control's corner and opens the same
retained drawer in one press; it hides during selection mode or an annotation.
Its mark plus `Unread` (also on the Chat menu entry) is device-local reply attention,
not the unresolved/Unseen count. Only visible Chat history at its end clears it;
toast dismissal/expiry, closing and reload do not. Close/Escape restores the opener.
`BrowserMenu` retains its row trigger for conversation actions and accepts an icon for
headers; keyboard navigation skips disabled entries, Escape returns focus, and selection
returns focus before opening a portal. Open header menus sit above the update row and
selection controls. Local choice/action menus, native actions, Ask previews, reader
information panels and management dialogs use the shared one-pixel edge roles
without shadows; keyboard focus keeps its visible outline. Every panel that floats over
page content has that one-pixel `--tmt-ui-surface-rule` edge on all four sides, once: the
shared drawers and dialogs from the leaf, and the annotation window and thread popover
(`.annotation-popover`) from `style.css`. It knowingly sits below the 3:1 boundary floor of #3959
(about 1.5:1 on `#fafafa`): a deliberate exception for floating panels (Ben, 2026-10-20), not to be reopened.
Explicit theme choices are radio menu items.
Owner page and space-home headers use one shared-height row. Inside the TMT web shell
([Shell](../../../../design/gui-style.md#shell)) `ColabHeader` renders into the shell's header slot
from whichever route draws it and draws the title alone, then muted original author: no brand, mark
or module crumb, because the rail's head carries the brand and the rail names the module (a scoped
page keeps its project crumb before the title). The display-only `local · <backendName>` identity
(`local` when absent) is not a visible element: it is in the header's accessible name
(`Colab · local · <backendName> · <title>`). The shell bar's cluster holds Search (an icon and a
quiet `⌘K` hint, hidden on coarse pointers) and the Remote presence chip (`Remote · N`, hidden
below 640px where the count stays in More); both are portaled into the shell's `tools` slot, and
the header leaves the room the shell measures for them. A standalone header (reader, mounted and
native screens) still starts with the TMT mark and the `Colab` wordmark, then `/`, and names the
brand link `Colab · local · <backendName>` (an unlinked brand is a named group with the same title);
below 480px it shows the mark only, the wordmark visually hidden and kept in that name. The shell's
module menu holds All pages,
each project with its counts, New project… and Manage projects…; the space home shows its scope as
plain text after the title. Author hides below 900px; the title truncates without
changing its DOM copy. The page header's status slot holds, in order, the needs-you entry
(only while something waits) and the attention marks, each shown only while its state holds, as
one mark plus a word: `△ Remote offline`, `◉ Shared` (a page readable by anyone but its owner must
not hide in a menu) and `○ Archived`. Below 480px a mark keeps its glyph and its word is visually
hidden, never removed from the accessible name. The connection words (`Private · Live`,
`Reconnecting…`) are not shown: they stay in the document, visually hidden, and are Share…'s
secondary text; trouble surfaces in the notice below the header. The base actions and status names stay accessible at every
width; the pin moves into More below 640px as described above, and CSS owns fitting. Header, body and
update-notice offsets use shared heights, including at 320px. Reader and guidance headers keep their defaults. No menu grants writer admission.

Header search results and page-list titles are real links with router-generated URLs.
Plain click/Enter keep the existing same-tab navigation and focus behavior. Modified clicks
retain the browser's native tab/window and context-menu actions; Ctrl/Cmd+Enter opens the
highlighted search result in another tab, and Shift+Enter in another window, leaving the
current search open. Filter suggestions remain choices rather than navigation links.
Reading links retain their existing reading hint; Needs-you links open the full page (and
retain that hint for readings), while plain menu selection still jumps to its local target.

`needs-you-header.tsx` uses the existing page status slot for one `◆ Needs you N ▾` entry.
Only the diamond takes the attention colour. Through 640px the visible label is `◆ N ▾`,
with `Needs you: N` as its accessible name. At zero nothing shows: only a one-pixel, visually
hidden element labelled `✓ Nothing waits` remains. When the list empties while the menu is open
(keyboard focus inside it), that element takes focus (`tabindex=-1`, not a Tab stop) instead of the
body, and keeps it across the header's remount while the page layers reparse; focus elsewhere,
or a menu that was already closed, is left alone. The page owns that one flag. The shared DropdownMenu supplies grouped
labels, dismissal and keyboard handling; its modal scroll lock retains Colab's fixed-header
and safe-area padding, so opening it does not shift the authored page. Menu rows contain a one-line target title and muted
agent. Groups are `Please decide` (proposals), `Please read`, `Please answer`, then `Please confirm`, each with its count.
Picking a row finishes menu dismissal before scrolling/opening the existing target and
focusing its first action. Read-only targets retain content focus and navigation, never writes.
The [contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#page-summary-2588) owns
the shared pending-state and count rules; header state is not another read or publication owner.

The page index uses leaf List/Row presentation with newest-update-first ordering,
then displayed title and page ID for ties. Active titles are links; archived titles
remain plain text with their actions menu. Tab order is title then actions for
active rows. Each row shows a relative update time with an absolute local date
in its tooltip, and Private, Shared or Archived. Missing and out-of-range times
show `Update time unknown` without a date or `time` element. On compact screens,
metadata sits beneath the title. The Actions disclosure contains Details, full
page ID, the shared expiry hint and Manage page. The index uses window scrolling;
only the header stays fixed. Neither titles nor retention hints grant access.

The browser window scrolls the author page. `renderer.ts` initially sizes the
opaque iframe to the remaining viewport, disables its scrollbar, then uses the
bootstrap's document-height reports to grow or shrink it. Reports are untrusted,
exact-shaped and bound to the current frame/render ID. The parent coalesces
updates at most every 100 ms and caps height at 1,000,000 CSS pixels. Five increasing
reports within two seconds indicate viewport-coupled growth; this page alone
falls back to a viewport-high, internally scrolling frame. Over-limit claims clamp to that hard maximum. Replacement/abort clears timers and listeners.

The bootstrap installs ResizeObserver and mutation/resize hooks before author
HTML. Its local fragment-link offsets let the parent scroll the window below the
header; fallback pages instead scroll to the target internally. Neither layout
nor anchor reports grant a publication capability. Quote capture/resolution and
the existing selection channel remain cosmetic and keep working after resize.
Selection capture waits for pointer/keyboard release; the parent Annotate control follows
its focus end. With mode off, C annotates a completed selection; otherwise it toggles mode.
It stays outside text entry, IME, repeats and modifiers;
Alt+Enter keeps its explicit annotation action. The parent-only bottom-right toggle clears
open panels spatially; overlapping anchored windows fit above its reserved bottom strip. Escape closes a nested menu or mention list, then the active drawer, then the retained anchored window, then exits mode, one level per press. Chat and anchored drafts are kept. Busy sends are not collapsed by Escape. In mode, one outline
marks the hovered target; text-bearing clicks and completed selections open the retained
anchored discussion. Textless targets show a temporary hint and create nothing. Readers
have no toggle. No selection navigation prepares, publishes or sends an Ask.
Question cards follow the actual authored group/standalone bounds measured by the
opaque frame, through the existing slot owner and its strict cosmetic protocol.
The [inline slot contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#own-stream-discussion-records-1427)
owns the exact shapes and admission limits. The parent floating-action placement
effect reuses its resize/scroll observer to clear attached visible question cards
containing footers, including answer controls, recipients and Send, and attached inline
Proposal cards, including Approve, Decline, Follow up, Resolve and the follow-up composer;
with no available space above them, the floating actions temporarily hide while header
entry points remain available. Card/FAB observers and scroll listeners clean up with that effect. This does not
change drafts, active-tab geometry retirement, selection coordinates or send policy.
Author backgrounds and widths belong to the page, except on a `column` or `full` layout
(#3913), where Colab owns the frame (below); parent theme tokens style chrome. `theme.ts` reads the parent's explicit root choice, otherwise the live OS
preference; theme actions record Light/Dark or remove the explicit choice for System only on activation. `renderer.ts` subscribes
for its mounted lifetime and projects the effective light/dark value through the
bound init/port. The bootstrap sets root `data-theme` before author code and updates
it without replacing the document. Question and Proposal cards use Colab’s effective theme throughout. Question frame
text uses generated browser sheet/text roles; parent-drawn card bodies, controls and
follow-up composers inherit the same theme. Fixed-look author pages do not override
that card theme. The [renderer contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#renderer-and-live-anchors)
owns the exact shapes and author CSS selector rule: the bundled starter follows
Colab's choice, while media-only author CSS follows the OS. Fixed-look pages use
unconditional CSS palettes, not a pinned root theme attribute; the bundled skill
shows that authoring rule. The unstyled white
canvas, sandbox attributes, source limits and CSP remain unchanged.

**Page frame (#3913).** `mountRenderer({ layout })` adds an optional `layout` to the bind; only
`column` and `full` make the bootstrap set `data-colab-layout` and the generated `src/page-frame.css`
apply, in a layer declared before author styles. The column (`min(1280px, 100% - 32px)`), the
16/8/40 px gaps and `data-width="full"` are defined there and nowhere else in the frame; the
parent's `.canvas[data-layout] .proposal-inline` rule in `style.css` repeats the column's gutter
so reading and review bands sit on it; every band also keeps out of the marker lane below. Question
blocks keep the frame's own measurement of their passage. The frame's chip marker is out of flow,
so it adds no blank line between blocks. `PageSnapshot.layout` and `PageView.layout` carry the verified
`meta.layout` (and `layoutReason` for a canvas); `freeform`, `canvas` and absent bind no column. Every
page, framed or not, gets the frame's 28 px right marker lane (`--colab-lane`) from its first paint:
the renderer stands a comment's number there, centred on its line, so it never covers text and the
first thread moves none (on a framed page 4 px after the column's end, beside its text; otherwise at
the frame's edge); threads on lines closer than one marker share a `+n` pill that posts
`colab.render.open-list`, and the parent opens the Comments list. The thread whose window is open is
the renderer handle's `select(id | null)`, which `router.tsx` calls with the open thread and which
posts `colab.render.select` (the frame accepts only the current request and an ID it holds); the
frame paints that passage in `::highlight(colab-selected)` above the hover look, or on its overlay
marks without the Highlight API.
`router.tsx` binds the renderer from the live view, so a metadata-only change remounts the
frame, and the About drawer's `PageLayoutSection` names the layout and a canvas's reason. An inline `style`
with `!important` is the one author declaration the frame cannot override.

Source revisions replace only the author renderer, not parent chrome. `router.tsx`
keeps the mounted annotation input and frozen quote/rectangle during loading;
page-ID changes reset page-local selection, panels and in-memory drafts; saved drafts
return with their own page. Existing
thread highlights are projected again when the new render is ready. A plain click or tap
on a resolved highlight opens its existing discussion, including in selection mode; native
drag selection stays available. The bounded resolver chooses the shortest quote with a
stable ID tie-break, and the parent admits only current known-thread view actions. `renderer.ts`
checks the frozen composer quote through the same bounded cosmetic resolver: the
reserved empty anchor ID resolves without painting a highlight or creating a
marker/action. Only an admitted resolution response marks the check complete;
pending checks do not show the stale-quote hint. Annotation/Chat submits and recipient selection use `BrowserAction`: the existing
Enter intent selects the one primary submit, while other submits and recipient selection
remain text actions. The ready primary Send is solid; disabled Send stays outlined and muted.
Busy, echo and page-open timing follow the shared
[transient states](../../../../design/gui-style.md#transient-states).
Export downloads use the shared Action inside the drawer, with its header providing the single close action.
`MessageComposer` configures its retained mention-aware combobox on the shared
`RichInput` editing boundary and `BrowserField`. The caller owns popup ARIA and
message policy; the shared input owns focus, caret/IME and editor history. Its
optional accessory is inside the input below text at the shared text left edge.
The creator toggle belongs to the shared message action row, with Send on the right
and recipient status in its hint line. It uses the leaf toggle-button semantics,
with a full accessible label and a compact recipient label at 640px and below.
Its preference is shared by this page/viewer device's Chat, annotations and replies;
the accessory slot remains presentation-only and separate from recipient ownership.
`BrowserDrawer` owns the shared title, audience caption and Close action; Chat
keeps its mounted `ConversationWindow` history/composer with its inner header hidden.
Close uses `BrowserIconAction`; its tooltip and focus presentation belong to the leaf.
Escape dismisses the tooltip and reaches the containing drawer or dialog in the same activation. Annotation surfaces and drawers are shadow-free.
Source loading does not block
discussion input or explicit sends, which use the frozen quote and verified
connection; renderer failures and connection errors still block them. It
releases the old channel before replacement without clearing parent selection or
removing the old frame; failures and ordinary abort/destroy still remove it. An
open annotation sits below the failure notice when renderer geometry is gone,
keeping the full notice readable with input and sending disabled by the current failure. It
retains the previous frame height during replacement and preserves the current
window offset through that replacement, clamped by the new document's bounds.
Later height reports do not replay an older offset over newer scrolling.
An opened page is idle (#4089): the frame measures its height from the body and the root's scroll
height and holds it until the body reads differently, so a last block whose bottom margin collapses
through the body's cannot make the height alternate. An idle or scrolled page sends the app no frame
message, which `large-page-idle.spec.ts` counts.
The [UI contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#own-stream-discussion-records-1427)
owns draft survival and stale-quote behavior.

The annotation window uses the same `ThreadWindow` as Comments from first
selection through the committed conversation. Its composer and message bodies use
the host field body scale (14px/1.5), shared with Chat. Mention chips inherit the
composer's font size and retain their compact line height. Captions use the browser
caption or host label tokens.
The shell derives its gaps and insets from the browser gap and host field padding,
without a larger mobile or desktop annotation font. Authored page styles remain isolated.
The composer stays mounted on first send; Close, Escape and outside press collapse without sending
or resolving. Page-owned drafts keep the exact edit and recipient for reopening
the known thread. Current renderer anchor positions locate marker-opened windows;
viewport dimensions clamp their square, shadow-free surface independently of
document height. The shell fits its content without reserved history height and
grows away from the same selection edge through first Send. At the viewport cap,
only history scrolls; the field and Send remain visible. An open annotation stays
clamped inside the viewport even when its selection endpoint is offscreen.
Its shared icon actions retain hover/keyboard tooltips; a long status title truncates before the fixed
right-aligned controls; attachment metadata stays on that same header row.
Header/composer stay in place while messages scroll, including
an associated delayed reply to an earlier turn. Unrelated live publications keep
the reader's message-history position. This changes no renderer messages,
subscription, publication admission or reply association. Comments remains the
explicit full-history/details view.

`BrowserDrawer` renders in place and native modality promotes it above the header/frame. Desktop panels float on
the right on an opaque square surface with a 1 px border on every edge. Chat,
Source, Agents, Files, Comments and Export leave 8 px below the header and at the
right and bottom edges (4 px through 480 px), plus top/right/bottom safe-area insets. All widths use native modal containment;
menus use a transparent catching backdrop and consume Escape before their drawer. Neither changes renderer
width or content layout; the header and page edge remain visible.
Panel bodies scroll independently except
Chat: its bounded flex body keeps the shared header and non-shrinking composer
visible, with only message history scrolling in the remaining height.
All widths use native modal containment without body scroll locking; resizing preserves the mounted draft and editing selection. The body contains overscroll. A header or iframe backdrop press dismisses only, never clicks through.
Close/Escape restores focus, and media listeners/dialogs clean up on close or
unmount. Chat initializes on first opening; closed Source, Comments and Chat panels retain drafts and admitted history;
closing never dispatches, abandons or retries. Export closes its preparation and
revokes download Blob URLs. Manage opens after its menu closes and initially
focuses its non-interactive heading without an outline while metadata loads;
Its title and Close stay fixed while only the management body scrolls.
Tab reaches its enabled controls with visible focus outlines, and
Close/Escape restores the trigger's focus. Safety details
remain available from About this page, reader Page information and blocked views; visibility never
substitutes for writer admission.

`agent-status-panel.tsx` presents the canonical read-only `LiveAsk` observation
inside the Agents drawer. It reads only while opened or after explicit Recheck,
retains transient directory failures as a labelled stale snapshot, and drops rows
on ended/evicted/scope-denied or changed page/client admission. Session/directory
faults never become offline presence. Pending results are fenced on close and
binding replacement; this presentation has no message or recovery capability.
The read owner and typed failure boundary are defined in [Ask agent](ask-agent.md).

`e2e/chrome.spec.ts` compares header and state-card dimensions, font metrics, state words/colors and window-scroll
containment across every screen (including the compact one-row page header) at 1440/390/320 in light/dark, including responses
from the native no-app socket fixture. Short-screen midpoint checks add an inert
scroll probe; their top captures show the natural notice layout.
`e2e/layout.spec.ts` covers window-scrolled 1440/390 light/dark short/long captures,
five icons and visible metadata, menus, overlays that preserve page geometry, and focus/Escape.
`e2e/live-update.spec.ts` covers annotation/thread/Chat draft survival, frozen-quote
sends, highlight reprojection, window offsets and page-change resets at 1440/390.
`e2e/renderer-scroll.spec.ts` covers owner and reader resizing, local anchors,
viewport-growth fallback and malformed/stale/over-limit claims. `e2e/theme.spec.ts`
covers OS defaults, explicit choices, live updates, render bindings and cleanup
without reload, plus the media-only CSS limit. `e2e/column-frame.spec.ts` and
`e2e/column-bands.spec.ts` cover the frame's column, gaps, `full` blocks and author overrides, and
the bands on it, at 1440, 1024, 390 and 2000 px; `e2e/band-clearance.spec.ts` measures, on the needs-you sample
page in `freeform` and `column` layouts at 1440, 1024 and 390 px, that no band covers page text or another band,
also after a live width change (the frame and the bands catch up with each other in 100-200 ms, so the check waits
for the clear state); `e2e/renderer.spec.ts` covers the layout bind key. Existing Ask,
discussion, management and export scenarios verify the publication controls;
run the [app/browser and real-binary acceptance gates](development.md).

Proposal cards live in trusted parent chrome with a flat shared 1px edge and state
mark plus word. The parent measures height; the one author iframe owns only inert
spacers through the existing bounded renderer channel. Approve is the filled
primary Action; Decline uses the bordered Action, Follow up the text Action, and
Resolve/Reopen muted text at the right end. Follow up opens the shared composer
inside the card below its history; Send, Escape and Cancel close it. Proposal
cards have no anchored window or Delete thread action. Missing, duplicate or
vanished slots and inner-scroll fallback show the admitted proposal in Comments. Resolve folds a proposal
on the page; in Comments it uses a plain row with full muted title, author/time,
Proposal · Resolved state and Reopen. All proposal list rows prefix their state
with Proposal, and resolved marks match ordinary threads. Reopen restores history while retaining
the final decision. Comments offers Proposals only alongside the mixed list. The
single header count uses `openThreadCount` for undecided unresolved proposals plus
unresolved ordinary annotations, excluding Chat/deleted records and decided
proposals; filtering does not change it. No new token or surface variant is introduced.

A card the parent draws over the author's page (the compact question block, group and answer batch,
a proposal card) is the page sheet of the active theme, white on light, with a 1px
`--tmt-ui-surface-rule` boundary and no dark edge (the block draws its rule as an outline inside
its box, so it moves nothing), whatever the page paints; it is not the
needs-you band (#4068 reversed #3959's band surface, which read as a dark slab). Its text keeps the
normal secondary colours, which hold 4.5:1 on the sheet. The 3:1 boundary floor does not apply to
the rule, by the owner's decision (as for floating panels, #4025).
`e2e/page-surface-contrast.spec.ts` holds text to 4.5:1 and every card to the sheet and the rule, on
a light, a dark and an unpainted page in both themes at 1440 and 390 px.

**Progressive open (#3894).** `Live.snapshot()` returns once the first admitted view has
published the document; the threads, asks and presentations follow when `#publishViews` has
verified them. Until then `PageView.discussionPending` is set and those three fields are absent,
never empty, so nothing unverified reads as verified: the router shows no comment count (the
Discussion button and its Comments row carry no number), no `Needs you` entry (not `✓ Nothing waits`),
no inline question, review, reading or proposal layer, and no thread highlights. The Comments and
Chat drawers show one quiet `Loading comments…` status instead of a list or composer. The document
frame does not move when verification arrives; the layers and the count then appear. `ThreadStore`
reports unavailable until verified, so no write can start from a pending view. `e2e/progressive-open.spec.ts`
holds the verification back through the fixture's `discussionPending` option.

`use-reply-notifications.ts` owns page-local Chat/Thread/Answer/Proposal presentation over admitted
Ask finals and discussion comments; `reply-notifications.ts` reuses authenticated
Yours refs and existing comment/Ask associations. The page read owner baselines
the binding's first current view after loading, including a cached snapshot;
initial and unavailable reads never replay history. The shared leaf owns FIFO budgets,
hover/focus pause, hidden-document suspension and once-only polite display.
ConversationWindow reports visible/following state and reveals stable record keys.
Chat activation opens its existing Drawer, while Thread activation reuses the
canonical quote/anchored-window path. Visible-at-end destinations suppress arrivals.
PageMenu and native Manage/attachment owners report actual visibility to the page.
An open anchored Thread window also holds the queue so toasts cannot cover its
history or composer. All active surfaces must close before resume rechecks the
latest admitted view; retained items preserve FIFO order and are not re-announced.
Header counts, drafts, Send and recipient admission keep their current owners.

Answer uses the question layer's checked thread association; Proposal uses its
admitted thread. Card owners suppress replies already in open, visible history.
Activation selects the answer tab or unfolds proposal history, scrolls and focuses
the existing block; detached proposals use Comments. The page supplies its rendered header's
live bottom through the notification context for reveal offsets and visible-record reporting.
The explicit arrival outline
uses the focus token and clears on blur or the next pointer/key interaction.
Busy clicks wait behind the existing send fence for at most five seconds. Pending
activation/reveal is cancelled on page change, withdrawal or timeout without late
scroll, focus or cue. Ordinary Chat/Comments and inline cards remain reachable if
a busy activation expires. One leaf polite region replaces the legacy announcer.
Proposal cards
return Cancel and completed decision actions to Follow up; while busy they keep
focus on the card and never reclaim it after the reader moves elsewhere.

Agent replies in Chat, threads and proposal Follow up share `ConversationTurn`.
An agent reply has one look, running or finished: no driver edge, colour or word, and a
running turn shows only the chip's working state (#4111). Stable turn keys preserve focus.
Comment, Chat and reply bodies render through `MarkdownText` (#4112): `markdown.ts` parses a safe
subset into a tree (never HTML), capped at 20 000 characters, depth 3 and 4 000 nodes, and a body
over a cap stays plain paragraphs. Every scan over a body is linear (a search that found nothing from
a position is not repeated; brackets are matched once), so a body of unmatched openers cannot stall a
reader. Text leaves keep their mention chips. `classifyHref` admits only `/p/<page prefix>` links of
this space as links; a plain click routes in the app through `shortLinkRoute` like a page link in a
page (#4091), and a modified click stays the browser's. Outside `https:` URLs stay inert words with
the URL until outside links are confirmed (#4083), and every other target stays literal text.
The data flow is owned by [Ask agent](ask-agent.md#verified-reply-treatment); the visual
rule is in [GUI style](../../../../design/gui-style.md#conversation-turn).

While recovery is required, the active Drawer presents the connection owner's existing
Reconnect action in a mark-and-word notice below its header. It owns no connection
state, probe or timer, and reconnect keeps the composer and draft mounted.
