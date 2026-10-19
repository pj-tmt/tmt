# TMT browser interface style

This document owns how TMT's browser surfaces look and behave: Colab, the Remote
pages and any later TMT page in a browser. It defines the principles,
the shared components and the interaction rules. Terminal output and the Squad
board follow the [CLI style](cli-style.md). Colors, type, sizes and breakpoints
come from [`tokens/tokens.json`](tokens/tokens.json). Use shared roles and the leaf
contract's explicit host metric inputs; do not invent a product palette.
Component extraction and consumer ownership are recorded in the [browser component contracts](gui-components.md).

**Adoption status:** the [browser component contracts](gui-components.md#current-consumers-and-ownership)
own current consumers and integration boundaries. Shipped Colab components are
named below; explicitly labelled design targets remain unshipped.

## Principles

1. **Square and flat.** Corners are square except Colab annotation number badges (round) and the Annotate affordance (rounded pill). The browser leaf and Colab
   menus, previews and dialogs use opaque surfaces without shadows.
2. **Edge to edge.** The page fills the window. There is exactly one window
   scrollbar; no region under the header scrolls on its own, except an overlay's
   body.
3. **One sticky row.** Only the one-row header is sticky. Everything else that
   appears on demand is an overlay that never reflows the page.
4. **Light actions.** Actions are light text buttons. A disabled action keeps its
   place and shows why. One primary filled button per card at most.
5. **Mark plus word.** A state is a mark and a word (`◆ waiting`, `✗ failed`).
   Color is never the only signal.
6. **Our controls only.** Use custom choices, toggles and icon help,
   without native selects, date pickers, checkboxes or confirm dialogs. Clipped
   titles may retain native full-text hints.
7. **One name per surface.** The header names the product once (`Colab`,
   `Remote`); the owner-header exception is defined under [Header](#header).
8. **Say the consequence.** Copy that asks for a decision states what happens
   and how to undo it, in plain words. No internal terms (baseline, epoch,
   descriptor) in human text.

## Tokens

Use the leaf's [roles and generation](browser-ui/README.md#entries-and-generation)
and [host metric inputs](browser-ui/README.md#host-metric-inputs); those are the
single owner of browser projection and metrics. Light/system-dark defaults and
explicit theme overrides use the same tokens. State marks retain their words;
[CLI marks](cli-style.md#marks) own the canonical meanings. Browser marks may use
the matching Lucide icon, followed by the state word.

## Icons

Lucide (`lucide-react`), with square line caps and miter joins, 18 px at a
1.75 stroke in the header and 16 px inline, colored with `currentColor`. An icon
never stands alone where a word fits; an icon-only button needs an accessible
name and a visible tooltip.

## Layout

- **Widths:** compact through the shared header's `compact-max-width`, full
  above it. Content columns center at up to 1080 px; the page canvas behind them is
  edge to edge.
- **Header height:** 56 px, 48 px when compact (`header` tokens).
- **Overlays** open over the page from the right without reflow; see [Overlay](#overlay) for geometry and focus.
- **Spacing** steps are 4, 8, 12, 16, 24 and 32 px.
- **Responsive:** every component works from 320 px up and sizes from its own
  container (CSS container queries), not only the viewport, because the same
  component appears full width and inside a 400 px overlay. Nothing overflows
  horizontally outside a tab strip: rows wrap, long titles truncate with `…`, and mono text breaks
  anywhere. Header actions overflow into More; the state chip stays visible.
  The [Header](#header) rule owns the Colab compact fitting order.

## Components

Each component lists its job, its required states and its rules. "Basis" is how
a shared package should build it (proposal): **own** means plain React and CSS
on tokens; **Radix** means a Radix primitive restyled to this document, in the
shadcn manner (code copied into our package, not a themed dependency).

### Header

The one sticky row on every screen: TMT mark, product word and plain screen
title on the left, without a default divider; status group, then light actions on
the right. Use the same height, type and order across screens. Actions that do not fit overflow into More;
status remains visible as mark plus word.

- Colab owner pages and space home are the exception to the leading mark: neutral
  `◉ local · <backend name>` (`local` when absent), Colab, `/`, title and muted
  `by <name>` (original author only) replace the default left group. Attribution is display-only, with no
  switcher or menu; full labels remain accessible. Hide author below 900 px, then
  attribution text below 640 px; keep the Remote mark, title and state chip. At
  640 px and narrower, Discussion (count) and More remain; Agents, Files (count),
  Source and Export move into More with their current authority and disabled
  states. The title truncates within the shared-height row. Reader and native
  guidance headers retain the default mark and caption layout.
- Primitive: shared Header; host-owned optional attribution and title-prefix slots.
- Shipped as `ColabHeader` (`colab-header.tsx`). Basis: own.

### Product switcher

The product word in the header is a menu trigger (`Colab ▾`) listing every TMT
browser surface: Colab, Remote and Settings (core), each with its state
as mark plus word (`● running`, `○ not installed`) and one action (`Open`,
`Set up`). The current product is selected. Products that aren't installed
stay listed and open the setup guide. Same position on every product; compact
width shows `tmt ▾`. Not shipped. Basis: Radix (Dropdown Menu).

### Remote switcher

Design target for [#2463](https://github.com/pj-tmt/tmt/issues/2463) and the
[#2461 browser registry](https://github.com/pj-tmt/tmt/issues/2461); not shipped.
Remote owns the registry, registration, static header and Remotes view; Colab owns
the React header glue and page-follow behavior. Use **remote** throughout UI, CLI help and
handbook copy, **Local** / **Firebase** for kinds and **this machine** for the host.

- **Header:** immediately after the logo, before the separate Product switcher:
  `[● studio ▾] [Colab ▾] <page title>`. Show the current origin's live state:
  `● connected`, `◌ connecting`, `○ unreachable`, `✗ access ended` (revoked/expired).
  Never show `not checked` in the chip. Compact width keeps the mark and a
  truncated name; tooltip and accessible name retain the full name and state
  (`Remote: studio, connected`). State words remain available; colour alone never
  carries state.
- **Menu:** Local rows first, then Firebase rows with their project ID. Every
  registered remote stays listed; the selected one has a check. A foreign row
  shows muted `not checked`, without a state mark, and is selectable with `Open`:
  navigation checks it at its own origin, with no foreign probe. Never claim
  Connected, Revoked, Not paired or last seen for an unchecked remote. Attention
  rows this origin knows are broken cannot be selected until fixed:
  `Unreachable · Retry`, `Access ended · Pair again`, `Sign-in needed · Sign in`.
  Retry only rechecks the current origin with its own keys. Footer actions are
  `Add remote…` and `Manage remotes`, opening Remote settings' Remotes section.
  No browser default mark; Manage remotes keeps
  the copyable `tmt remote settings default-remote <ref|local>` command for the
  separate CLI/API default. Never silently fall back.
- **Remotes list:** the current remote's observed connected state is
  `● Connected`; a foreign row is `not checked`, without a state mark or probe.
  Row actions are `Rename` and `Remove`, with full accessible names
  `Rename <name>` and `Remove <name>`. Only foreign rows have `Open`; the current
  row has no Open action.
- **Add remote:** the dialog has `Remote name` and `Door address or remote link`
  fields, with Continue / Cancel. A Local door asks `Add remote <name>?`, showing
  its address; a Firebase remote link asks `Add Firebase remote <project ID>?`,
  with the public Hosting address `<project>.web.app` on the next line. Add /
  Cancel confirms a public reference; pairing, sign-in and space admission stay
  separate. No free-form JSON field. Invalid input says
  `The remote name or link is invalid`; capacity refusal says
  `This browser can store at most 16 remotes. Remove one before adding another.`
- **Catalog handoff:** before storing anything, show
  `Add these remotes to this browser?`, the names, kinds and addresses, and
  `Only names and addresses are copied. No keys, pairings or drafts.` with
  Add / Cancel. The unverified source host and a count are absent from the
  confirmation. With nothing addable, Add is disabled and says `Nothing new to add.`
  A name used for another target says
  `You already have a remote named <name>` with an editable `<name>-2` to confirm,
  or Skip. A duplicate canonical target says `Already added as <name>` and is
  excluded from Add. Invalid, oversize or colliding packets are refused
  as a whole with one sentence and no partial import. Removal asks
  `Remove <name> from this browser?` and
  `This only removes the entry here. It does not sign out or revoke access.`
  with Remove / Cancel.
- **Per-remote states:** this origin's observed state shows `Not paired` →
  `Pairing…` → `Connected · read only` / `Connected · can send`. Firebase adds
  `Sign-in needed` → `Signing in…` → `Signed in`; anonymous Auth has no account
  name, and sign-in alone grants no space access. Show
  `Unreachable · last seen <age>` or `Access ended · Pair again` only from this
  origin's observed state. `Over budget today · resets at <time>` appears only
  after an actual budget refusal here, never on a foreign row; no counter,
  percentage or budget-ok state. With only the current host registered, show
  `● this machine`, its single menu row and Add remote, without another banner.
- **Page states and switching:** an unreachable selected remote keeps the last
  verified page read-only and its draft, with
  `<remote> is unreachable. Changes can't be saved.` and Retry / Switch remote.
  Access ended hides page content and says
  `Access to <remote> ended. Pair again or switch remote.` Colab shows
  `This page is not on <remote>` when its space is not published there, with
  Publish for the owner or Back to pages. Switching navigates in the same tab to
  the selected remote's own origin; separate tabs stay independent. Drafts and
  device keys stay per origin. Only when an unsent draft exists, confirm
  `Your draft stays on <remote>.` with Switch / Cancel; the draft remains on return.
  Colab owns this draft confirmation and any message for an in-flight or uncertain
  send; the registry owns no send state. Drafts and keys never transfer across origins.
- **Static menu:** Remote uses the leaf's static entry and shared class names,
  without React. Its native popover and minimal script meet the BrowserMenu
  keyboard/focus contract: roving focus, Escape restores focus, outside-click
  dismissal and correct roles. A second static consumer moves this behavior into
  the leaf's static entry. B1 needs only the landing page Remotes list; the header
  chip/menu requirement binds at B2.

Look review at 1440 light / 390 dark covers the Remotes list (current connected
and foreign not checked), Add remote (door address and remote link), catalog
confirmation with a collision and nothing addable, remove confirmation, and the
Remote header's four chip states, menu and unreachable-page notice.

### Quick switcher

Colab's **Search pages** header action and Cmd/Ctrl+K open a modal
[BrowserDialog](browser-ui/README.md#dialog) over the current page. It uses the
same search editor and label/author filters as the page list; collections are not
part of v1. Pages remain inert title text, with muted author/labels and the
existing mark-plus-word Waiting state. The current page is excluded.

- Keep one search row above a scrollable, single-column result list, with no
  loading flash when a cached list is available. The input keeps focus while
  arrows move the active result. Filter suggestions get the keys first; otherwise
  Enter opens that result. Escape closes suggestions, then the dialog and returns
  focus. Successful navigation focuses the destination title. Editors retain
  their own shortcuts; repeated and composing keys never activate a result.
- The content-fitting surface is opaque, square, thin-edged and shadow-free,
  using existing focus/selection/scrim tokens. It is modal at every width, with
  safe-area margins, no body scroll lock and contained result scrolling. Close
  and the input remain reachable at compact or short viewport sizes.
- Show **Loading pages…** only without usable rows, **No pages yet.** for an empty
  space, **No other pages in this space.** after current-page exclusion, and
  **No matching pages.** for a filter with no results. A failed cold read says
  **Couldn't load pages. Close and try again.**; a failed background refresh keeps
  the cached list. No implicit navigation or write occurs on opening.

### Setup guide

The first-run and not-installed screen, built from Card, Status mark and Button:
one row per step (install tmt, install the extension, start it, open it), each
with its state and, when it needs the terminal, one copyable command. The
browser never runs a command for the user; the page checks state again and ticks
each step by itself. An open step carries the waiting mark and word.
Not shipped. Basis: own.

### Button

- Variants: **primary** (flat sheet; one emphasized decision per card), **text**
  (light, default for actions), **destructive** (text in the `blocked` role,
  always last in its group).
- States follow the leaf
  [Action contract](browser-ui/README.md#controlled-input-contracts): selection
  colors on enabled hover/focus, visible focus, opaque muted disabled controls
  and a retained label with a caller-supplied mark while busy. Busy is not
  disabled: see [Transient states](#transient-states). Hosts supply disabled
  reasons.
- Basis: own.

### Toggle

A light text button with a square indicator before a fixed label; the indicator
alone shows on or off, so the width never changes. `aria-pressed`, Space and
Enter. Shipped as the page list's `Show archived` (#1728). Basis: own.

### Tabs

`BrowserTabs` is a controlled leaf with a horizontal named tablist and persistent
panels. Left/Right wrap, Home/End move to the ends, and arrow navigation activates immediately;
Enter/Space use native activation. Only the selected tab enters the Tab order.
Panels have no tab stop by default; the host can opt in. The host retains selection, drafts, stable tab/panel IDs and optional mark/state
words. The accessible name includes the mark and state word. A trailing status
slot belongs to the host and wraps below the strip below 480px of component width.

The strip scrolls horizontally without wrapping or animation; selection and resize
keep the selected tab visible without moving the page. A label wider than the strip
truncates while retaining its full accessible name. Use the existing state roles,
a 1px flat square panel rule and a 2px neutral text-color rule above the selected tab,
which uses strong neutral text and joins the panel. State marks retain their own roles.
The leaf's [Tabs contract](browser-ui/README.md#controlled-input-contracts)
owns React/static inputs and panel lifetimes. Basis: own.

### Card

A square, opaque, shadow-free sheet with the shared thin edge.

- **Notice card:** the body of every state screen (opening, failed, ended,
  deleted, choose). Mark, eyebrow, title, one or two lines of text, actions
  inside the card. The mark carries the state role. Shipped as
  `NoticeCard` (`notice-card.tsx`).
- **Item card:** a page in the page list: icon, title, short ID, audience tag,
  expiry, actions row.
- Basis: own.

### Chip

A one-line label whose content is always vertically centered (fixed height,
inline flex). Kinds: **status** (mark plus word), **driver** (`claude`,
`codex` in the driver color), **filter** (a toggle button, accent when on) and
**count** (`◆ 2 open`, a button that opens its list). Static chips are square
outlines; interactive ones are buttons with a focus ring. Height 20 px, filter
24 px. Basis: own.

### BrowserChips

A removable list uses shared BrowserChips/BrowserChip: square 1px chips with a
centered one-line label and an explicit remove control. Wrap the list at narrow
widths; keep the full accessible name when a long label ellipsizes. A trailing
`＋` opens a search combobox/listbox. Arrow keys move, Enter adds, Escape returns
focus, Tab closes, and Backspace in an empty non-composing field removes the last
chip, when enabled. Recipients use `Remove @name` and `Add recipient`; selected agents are not
offered again. The host owns values, filtering, limits, status and trusted effects.
At its cap Add is disabled with a visible reason; empty/loading/error copy stays
host-owned. Page-list label tokens use the same item inside their existing editor.
The [leaf contract](browser-ui/README.md#chips) owns the API, focus and static entries.

### Tag

A square outline label in mono type for audience and kind (`Private`,
`Archived · writes frozen`). Not interactive. Basis: own.

### Listbox

Every choice from a list: share audience, roles, history, `@` autocomplete,
settings values.

- Arrow keys move, Enter picks, Esc closes and returns focus to the trigger,
  typing filters when it has an input. `role="listbox"`.
- Opens below its trigger and flips upward only when it would overflow.
- Shipped in `components/listbox.tsx`. Basis: Radix (Select / Popover with
  combobox).

### Menu

A list of actions behind a trigger: the header's More menu and a message's `⋯`.

- Items are text, destructive last; Esc closes; arrow navigation; `role="menu"`.
- Right edges align with the trigger. Prefer below, flip above when it has more
  room, and constrain scrolling to the visual viewport when neither side fits.
- Shared owner: [BrowserMenu](browser-ui/README.md#drawer-and-menu). Basis: Radix (Dropdown Menu).

### Tooltip

Short help for an icon or a status item, such as the Remote chip.

The leaf's [IconAction contract](browser-ui/README.md#controlled-input-contracts)
owns tooltip behavior: real pointer movement or keyboard focus-visible opens
help; layout-only entry does not. Display-only help defaults to the button's
accessible name; shortcut help may differ while the button keeps a concise name
and `aria-keyshortcuts`. It receives no pointer events and is never the only place
for required information. Activation dismisses it. Escape hides it synchronously
and still reaches the host's active menu, drawer or dialog.

### Overlay

Comments, Chat, Source, Share and settings panels.

- Opens over the page without moving it, closes with its own `×`, Esc or a backdrop press. Focus moves into it and returns to the opener.
- One overlay at a time; opening another replaces it.
- Basis: Radix (Dialog). Shared owner: [BrowserDrawer](browser-ui/README.md#drawer-and-menu), with native modal containment at every width.
- Colab shared page drawers (Chat, Source, Agents, Files, Comments, Export and About) leave an 8 px gutter below the header and at the right and bottom edges; through 480 px use 4 px. Keep the header visible, a square opaque surface and one 1 px rule on every edge. Source may be wider; other drawers are at most 440 px. Compact drawers fill the available width between gutters. The scrim covers the visible header and author iframe; the first press only dismisses, with no click-through, and the second press acts. The drawer body contains overscroll; there is no body scroll lock. Neither changes the author frame width.

### BrowserNotifications

Transient reply cards below the header at the right, separate from persistent
discussion state. Shipped Colab destinations are **Chat**, **Thread**, **Answer** and **Proposal**; the
visible text tag names the destination, with the quoted passage for Thread.

- Opaque square cards, one thin edge, no shadow or motion. Use an 8 px header/right
  gutter (4 px through 480 px), safe-area insets and at most 360 px width.
- Three cards visible; additional replies wait in FIFO order with a waiting count.
  Each gets 6,000 ms when first displayed; waiting does not consume that time.
  Hover and keyboard focus independently pause the remaining time. Dismissal,
  activation and expiry promote the oldest waiting reply.
- Hold the queue while the document is hidden or a modal or anchored Thread window
  is open. Resume rechecks current host eligibility before display and announcement.
- Arrivals never take focus. Use natural Tab order and visible focus; focused plain
  Escape dismisses only that card. Escape/Dismiss returns focus to its connected,
  visible prior element; Open leaves destination focus to the host. The surface
  has no catching backdrop; controls require trusted activation.
- Answer/Proposal Open reveals and focuses the existing block. Its arrival outline
  uses the focus token and clears on blur or the next pointer/key interaction.
- One persistent polite status announces each newly displayed reply once. Queue
  counts and timer changes do not reannounce cards. Clamp display text to two lines
  while retaining the full accessible text.
- Expiry and dismissal leave persistent discussion state intact. Open activates
  the host's existing destination; opening a canonical thread clears its New
  agent-resolution attention without resolving it. All three leave the header's
  open-thread total unchanged. A conversation's New messages attention follows
  its history scroll position. Chat has no separate header unread counter.

Shared owner: [BrowserNotifications](browser-ui/README.md#notifications).
Basis: own.

### Page Chat

- Shared Drawer header: Chat, `○ Everyone with access` caption and Close. Reuse the shared
  agent rail; identities come from verified metadata, with muted time at the right.
  Space separates messages; day rules use admitted timestamps.
- Center the empty title and teach `@` only where admitted. Keep the composer
  pinned below one thin rule: editor, then Attach/recipient at left and Send at
  right. Show `Posts as a comment.` only without a recipient. Failed delivery
  uses `✗ Not delivered · Retry` with the existing recovery action.

### Popover composer

The small input at a selection or an item: annotation, follow-up, quick reply.

- Colab shows Annotate beside the selection's focus end only after the pointer is
  released or keyboard selection finishes. C opens the same annotation view; ignore
  text-entry targets, modifiers, repeat and IME confirmation. Alt+Enter remains available.
  Opening captures the quote without publishing or sending. The dark pill has a
  speech-bubble icon and `Annotate`, with a short thin leader at the focus end.
  Shortcut help is accessible through `aria-keyshortcuts`, without visible hotkey copy.
- Annotation ranges use flat neutral grey token paint, no underline, rounding or
  shadow, preserving authored text colour and weight. Round dark badges with white
  numbers sit at the final text rect; collision lanes keep each target distinct.
  Creation-order numbering includes every non-deleted thread before view filters.
  Deletion renumbers; resolved threads retain list numbers without page markers.
- Annotation and thread windows are borderless, square grey panels with an uppercase
  monospace label and a large borderless composer. Wide windows sit beside the
  selection with a leader; mobile retains bottom placement without one. The footer
  has muted `Attached to this sentence` and square arrow-only Send, with a complete
  recipient-bearing accessible name. The existing creator-notification preference
  remains a borderless mark-and-label toggle beside it. Resolved windows start collapsed with counts,
  state/Reopen and an explicit Show conversation action; expansion retains drafts.

- The recipient is independent of the message. An admitted stable prior
  reply can supply the default; creation defaults require a reliable canonical binding.
  Unknown creation and ambiguity require explicit selection, never a name or sole-agent guess.
  Choose/Change recipient selects without mutating message text. There is no mandatory
  `@` prefix. Optional mention completion preserves the surrounding
  text and caret; changing or removing the token does not change the selected recipient.
- Enter submits the parent's current message action; Shift+Enter adds a line, Esc
  closes the innermost candidate list before the composer. Typed text and the selected
  recipient are kept on close and restored on the same anchor with `Draft kept`.
- Plain comments have an explicit Post action and remain available under current
  content-write admission when agent discovery fails. Asking an agent is an explicit
  action; choosing a recipient alone never prepares or sends a message.
- Outside press closes it, but a page click never hides a typed draft; nothing
  closes it while a send is in flight.
- `components/message-composer.tsx` is the shared plaintext editing boundary for
  Chat, annotation, thread reply and comment edit. Its plaintext/history
  extensions preserve undo, IME and exact message bytes; the source editor stays separate.
- Candidate lists use available viewport space and the browser popover layer, including
  inside modal Chat. The field grows to a bounded height, then scrolls. Square corners
  and existing ink/accent tokens apply. Basis: own parent chrome with Lexical editing.

Mention chips share an icon and named roles in editors, sent messages and lists:
agent chips are neutral grey with bold ink; person chips use pale blue and
blue-grey text. Unknown identity kinds default to agent. Current directory data
has no person kind: the person variant is tokens and explicit presentation fixtures,
not person routing. Editing tokens retain recipient UUIDs; styling never admits a
recipient or sends. Source plaintext, undo and IME remain owned by the shared editor.

### Conversation turn

Chat and annotation windows use the same borderless composer, with a thin rule
above it and 16px side padding. Recipient chips precede the text; labeled Attach,
the existing creator preference and the square arrow Send share its bottom row,
aligned to the text edges. The composer retains one editor and draft owner.

Annotation anchors and thread headers share one 22px circle, 12px semibold system
font and grid-centered number. Header items align vertically with an 8px gap.
An anchored window adds no decorative connector line beside the circle.

One message in Chat or a thread: author line (`name AGENT · time` for agents, `name · time` for people), body,
state (mark plus word), and a `⋯` menu for the author's own Edit or Delete.

- Chat and threads share `ConversationTurn` structure and history ownership.
  In annotation/thread windows, human messages sit directly on the grey panel;
  agent messages use a whole-message sheet block without borders or bubbles.
  Every author line includes a person/agent icon, name and time; agent messages
  carry one small outlined `AGENT` tag, 6px after the name and before the time.
  Chat retains its existing rows and trusted driver rail treatment.
- In Chat, only the current verified `runningDriver`, matched by machine and agent,
  selects driver treatment: `claude` uses `review`, `codex` uses `link`. Present
  metadata adds a square filled Bot avatar, bold name and lowercase driver tag;
  the rail uses the same role. No shadow.
- Loading, failed, absent or other values use neutral agent treatment, without a
  driver tag or explanation. Names, body content and remembered `driver` cannot
  select a treatment. Observation refreshes update treatment with stable turn
  keys, preserving the mounted turn and focus; no driver history is persisted.
- Literal markup in a body always renders as text. Basis: own.

### Message attachments

Annotation/thread attachments use flat metadata chips with a type mark, filename,
type and size; previews sit below. Uploads show progress, refusals show `✗` with
reason and Remove. Pending annotation/thread chips scroll within a viewport-bound
area so Send stays reachable. Trusted Send starts staged uploads and stays busy until done.
Chat pending attachments use one horizontally scrollable row of small raster
thumbnails or filenames inside the input below its text, each with a named ×
remove action. They share the text's left edge, with no cards or progress bar.
The message composer has a visible `Attach` action; Files keeps `Attach files`.
Dragging files over Chat or an annotation/thread window shows that panel's
inset 2 px dashed frame, naming `⤓ Drop to attach to Chat` or `to this thread`.
A refused batch shows `✗` and its reason and adds nothing. Crossing child
controls keeps the overlay; leave, Escape or drop clears it without moving
focus. Drop alone never uploads or sends.

Raster images use aspect-preserving thumbnails capped at 200 × 120 px. Two or
more images use a three-column square grid: up to six show in full; larger groups
show five thumbnails and a `+N` tile, where N is total minus five. The tile opens
image six. Non-images use compact filename/type/size chips with Download.

A modal viewer shows one image with Previous/Next (no wrapping), Download and
Close; arrows navigate, Escape closes, and closing restores the opening tile's
focus. Thumbnails, grids and cards share the message text's left edge without
an extra inset, in both Chat and anchored threads. Basis: own.

### Status mark

The mark-plus-word unit used by every component above. Sizes match the
surrounding text; the live dot is 8 px. Basis: own.

### Band

An inline panel that expands under a row to show more without leaving the list
(`e` on the board; expanded details in a list). Same border as its container,
full inner width, its own collapse action as the last line. One open at a time.
Basis: own.

### Proposal item

Colab renders authenticated proposals as parent-owned cards: title, body,
proposing agent and Approve / Decline / Follow up / Resolve actions. Approve is
the emphasized decision; Resolve is muted. Follow up opens a composer inside
the card. Resolved proposals fold to a plain one-line row with a resolved mark,
retained-history disclosure and Reopen. Comments offers `Proposals only`; its
header count combines unresolved ordinary threads and undecided unresolved
proposals.

The [Colab proposal contract](../extensions/tmt-colab/contracts/colab-v1.md#own-stream-discussion-records-1427)
owns decision states, placement, admission and failure behavior. Page HTML
cannot supply these actions or grant authority. Shared component metrics remain
owned by the [browser leaf](browser-ui/README.md).

## Interaction rules

- **Keyboard:** every action is reachable by Tab; focus is always visible;
  Esc closes the innermost active interaction (list, menu, composer, overlay).
  Display-only icon help hides on the same event without consuming it.
- **Confirmation:** no modal confirm. A risky action states its consequence
  inline and asks for a second press or `--yes` on the CLI side; the message
  names its own consequence, never a shared generic one.
- **Errors:** say what failed and the next step (`Run tmt colab serve`), in the
  component that failed. A failed send keeps the text.
- **Empty states:** one sentence of what will appear and how to make it appear.
- **Waiting and held:** `◆` with the reason (`held · waiting for approval on
your machine`) and a way to recheck.
- **Time:** relative and lowercase in running text (`5m ago`, `expires in 6
days`); exact time on hover.
- **Counts:** the header carries one pending count per surface; it disappears at
  zero.
- **Motion:** none except the LoaderCircle spin, which stops under
  `prefers-reduced-motion`.

## Transient states

A send or save never flashes an in-between state. The
[Action contract](browser-ui/README.md#controlled-input-contracts) implements
the busy mark and busy state for every Action and IconAction.

- **Delayed busy mark:** nothing changes for the first 400 ms of busy. After
  that the action's reserved busy mark shows, and once shown it stays at least
  400 ms. Errors show at once. No label swaps (`Saving…`) and no inserted
  status paragraphs.
- **Busy is not disabled:** a busy control keeps its look, focus and readable
  text; it is `aria-disabled` with an activation guard. Only an unavailable
  control uses the muted disabled style.
- **Echo in place:** a sent Chat message, reply or answer appears at once in its
  final position and layout, and the composer clears in the same frame. The
  echo is display only: its pending mark follows the same 400 ms delay, the
  verified record replaces it in place under the same key, and a failure turns
  it into the error row with Retry while the draft is kept. An echo is never
  counted in Needs-you or other counts and never exported.
- **One busy signal:** when an echo exists it is the only busy signal; Send
  shows no Action busy mark. The Action busy mark is for actions without an
  echo: save, confirm, remove, check.
- **No remount:** a composer clears through its editor; it is not remounted, so
  height, editor state and focus stay.
- **Reserved status line:** a multi-step card keeps one status line of fixed
  height instead of inserting and removing paragraphs.
- **Page open:** navigation is immediate. A load longer than 150 ms shows the
  page frame, kept at least 300 ms: the header, the title when it is already
  known, and four muted skeleton lines. No shimmer or other motion.
- **Local updates:** an update re-renders only what changed, not the whole
  page.

## Accessibility

Text meets 4.5:1 on its background, marks and borders 3:1, in both themes and on
the selection background. Every control has an accessible name; icons alone are
`aria-hidden` next to a visible word. Pointer targets are at least 24 px.

## Enforcement

Colab's browser suite checks one window scrollbar, the sticky header metrics on
every screen and the absence of native selects and checkboxes on the page list.
A shared package should move these checks next to its components so every
surface runs them.
