# TMT browser interface style

This document owns how TMT's browser surfaces look and behave: Colab, the Remote
pages, Office and any later TMT page in a browser. It defines the principles,
the shared components and the interaction rules. Terminal output and the Squad
board follow the [CLI style](cli-style.md). Colors, type, sizes and breakpoints
come from [`tokens/tokens.json`](tokens/tokens.json). Use shared roles and the leaf
contract's explicit host metric inputs; do not invent a product palette.
Component extraction and consumer ownership are recorded in the [browser component contracts](gui-components.md).

**Adoption status:** the [browser component contracts](gui-components.md#current-consumers-and-ownership)
own current consumers and integration boundaries. Shipped Colab components are
named below; explicitly labelled design targets remain unshipped.

## Principles

1. **Square and flat.** Corners are square everywhere. The browser leaf and Colab
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
browser surface: Colab, Remote, Settings (core) and Office, each with its state
as mark plus word (`● running`, `○ not installed`) and one action (`Open`,
`Set up`). The current product is selected. Products that aren't installed
stay listed and open the setup guide. Same position on every product; compact
width shows `tmt ▾`. Not shipped. Basis: Radix (Dropdown Menu).

### Remote switcher

Design target for [#2463](https://github.com/pj-tmt/tmt/issues/2463); not shipped.
Remote owns the registry, chip, menu, registration and Remotes settings; Colab owns
page-follow behavior. Use **remote** throughout UI, CLI help and handbook copy,
**Local** / **Firebase** for kinds and **this machine** for the current host.

- **Header:** immediately after the logo, before the separate Product switcher:
  `[● studio ▾] [Colab ▾] <page title>`. Show the display name with a state mark:
  `● connected`, `◌ connecting`, `○ unreachable`, `✗ access ended` (revoked/expired).
  Compact width keeps the mark and a truncated name; the tooltip and accessible
  name retain the full name and state (`Remote: studio, connected`). State words
  remain available; colour alone never carries state.
- **Menu:** Local rows first, then Firebase rows with their project ID. Every
  registered remote stays listed with mark plus state word; the selected one has
  a check. Attention rows cannot be selected until fixed and expose the reason
  and one action: `Unreachable · Retry`, `Access ended · Pair again`,
  `Sign-in needed · Sign in`. Footer actions are `Add remote…` and
  `Manage remotes`, opening Remote settings' Remotes section. Mark the configured
  default `default`; choosing a row changes browser selection, while changing the
  per-home default happens only in Manage remotes. Never silently fall back.
- **Add remote:** a dialog offers Local host or Firebase project. Local host shows
  `tmt remote pair` to run on that host, the existing pairing words flow, and
  `--talk` for allowing sending; completion adds the host's name. Firebase uses a
  **remote link** printed by `tmt remote deploy firestore` (or a terminal QR),
  re-printable by a read-only command. It carries public configuration only;
  access still requires sign-in, admission and deployed Rules. Opening or pasting
  it in `Paste remote link` imports that configuration and asks
  `Add Firebase remote <project ID>, deployed by <host>?` with Add / Cancel, then
  starts that remote's sign-in. No free-form JSON field. Tenant registration uses
  the same link and says `You'll join as a tenant of <host>'s project` before Add;
  the link grants no space access.
- **Per-remote states:** menu rows and Remotes settings show `Not paired` →
  `Pairing…` → `Connected · read only` / `Connected · can send`. Firebase adds
  `Sign-in needed` → `Signing in…` → `Signed in as <account>`. Show
  `Unreachable · last seen <age>`, `Access ended · Pair again`, or
  `Over budget today · resets at <time>` only from observed state/budget refusal;
  never invent a usage percentage. With only the current host registered, show
  `● this machine`, its single menu row and Add remote, without another banner.
- **Page states:** an unreachable selected remote keeps the last verified page
  read-only and its draft, with `<remote> is unreachable. Changes can't be saved.`
  and Retry / Switch remote. Access ended hides page content and says
  `Access to <remote> ended. Pair again or switch remote.` Colab shows
  `This page is not on <remote>` when its space is not published there, with Publish
  for the owner or Back to pages. Drafts and device keys stay per remote; warn
  `Your draft stays on <remote>` before leaving an unsent draft, which remains on
  return. Do not transfer drafts or keys across origins.

**Open point:** the switch's reload/navigation target, origin and registry handoff
await Remote's [#2461 registry plan](https://github.com/pj-tmt/tmt/issues/2461).

Implementation look review covers the four chip states, a menu with three remotes,
both Add remote choices and the unreachable-page notice at 1440 light / 390 dark.

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
  and a retained label with a caller-supplied mark while busy. Hosts supply
  disabled reasons.
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
a 1px flat square panel rule and a 2px link-color rule above the selected tab, which
joins the panel. The leaf's [Tabs contract](browser-ui/README.md#controlled-input-contracts)
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
  Opening captures the quote without publishing or sending.

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

### Conversation turn

One message in Chat or a thread: author line (`name · agent · time`), body,
state (mark plus word), and a `⋯` menu for the author's own Edit or Delete.

- Chat and threads share the flat `ConversationTurn`: user and agent rows span
  the history, separated by a 1 px rule; agent replies have a 3 px rail.
- Only the current verified `runningDriver`, matched by machine and agent,
  selects driver treatment: `claude` uses `review`, `codex` uses `link`. Present
  metadata adds a square filled Bot avatar, bold name and lowercase driver tag;
  the rail uses the same role. No shadow.
- Loading, failed, absent or other values use neutral agent treatment, without a
  driver tag or explanation. Names, body content and remembered `driver` cannot
  select a treatment. Observation refreshes update treatment with stable turn
  keys, preserving the mounted turn and focus; no driver history is persisted.
- Literal markup in a body always renders as text. Basis: own.

### Message attachments

Pending attachments use one horizontally scrollable row of small raster
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
image six. Non-images use compact filename/type/size cards with Download.

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

## Accessibility

Text meets 4.5:1 on its background, marks and borders 3:1, in both themes and on
the selection background. Every control has an accessible name; icons alone are
`aria-hidden` next to a visible word. Pointer targets are at least 24 px.

## Enforcement

Colab's browser suite checks one window scrollbar, the sticky header metrics on
every screen and the absence of native selects and checkboxes on the page list.
A shared package should move these checks next to its components so every
surface runs them.
