# TMT browser interface style

This document owns how TMT's browser surfaces look and behave: Colab, the Remote
pages, Office and any later TMT page in a browser. It defines the principles,
the shared components and the interaction rules. Terminal output and the Squad
board follow the [CLI style](cli-style.md). Colors, type, sizes and breakpoints
come from [`tokens/tokens.json`](tokens/tokens.json). A surface never writes a
color or size that is not a token. Component extraction and consumer ownership
are recorded in the [browser component contracts](gui-components.md).

**Adoption status:** Colab's shipped implementations are named below under
`extensions/tmt-colab/typescript/app/src`; the other descriptions include
approved design targets that have not shipped yet. Remote's pairing and door
pages follow the same token, header and card rules in their own `pages.css`.
Office has not adopted this style. The private browser leaf and its shared
integration are in place; product adoption is separate consumer work. The implementation basis column still includes proposals beyond that
leaf. See the [package contract](browser-ui/README.md) for the bounded subset.

## Principles

1. **Square and flat.** Corners are square everywhere. The initial browser leaf
   uses opaque surfaces and no shadows. Existing product shadows await owning
   adoption; they are not a fallback for the new leaf.
2. **Edge to edge.** The page fills the window. There is exactly one window
   scrollbar; no region under the header scrolls on its own, except an overlay's
   body.
3. **One sticky row.** Only the one-row header is sticky. Everything else that
   appears on demand is an overlay that never reflows the page.
4. **Light actions.** Actions are light text buttons. A disabled action keeps its
   place and shows why. One primary filled button per card at most.
5. **Mark plus word.** A state is a mark and a word (`◆ waiting`, `✗ failed`).
   Color is never the only signal.
6. **Our controls only.** No native select, date picker, checkbox, tooltip
   (`title`) or confirm dialog. Every control is a component below.
7. **One name per surface.** The header names the product once (`Colab`,
   `Remote`) after the `tmt` mark.
8. **Say the consequence.** Copy that asks for a decision states what happens
   and how to undo it, in plain words. No internal terms (baseline, epoch,
   descriptor) in human text.

## Tokens

Use the `color`, `surface`, `font`, `header` and `breakpoint` groups of
`tokens.json` through the generated CSS variables. Light and dark themes come
from the same tokens; `prefers-color-scheme` picks the default and the header's
theme action overrides it.

State roles map to marks:

| Role      | Mark | Word examples                | Use                  |
| --------- | ---- | ---------------------------- | -------------------- |
| `waiting` | `◆`  | waiting, waits on you, held  | the user must act    |
| `blocked` | `✗`  | failed, unavailable, deleted | stopped; needs a fix |
| `working` | `●`  | running, live, replied       | healthy and active   |
| `review`  | `◐`  | opening, in review           | in progress          |
| `muted`   | `○`  | ended, archived, resolved    | finished or idle     |

In the browser the mark may be the matching Lucide icon (Diamond, X, Circle
filled, LoaderCircle, Circle), always followed by its word.

## Icons

Lucide (`lucide-react`), with square line caps and miter joins, 18 px at a
1.75 stroke in the header and 16 px inline, colored with `currentColor`. An icon
never stands alone where a word fits; an icon-only button needs an accessible
name and a visible tooltip.

## Layout

- **Widths:** compact below the header's `compact-max-width` (480 px), full
  above. Content columns center at up to 1080 px; the page canvas behind them is
  edge to edge.
- **Header height:** 56 px, 48 px when compact (`header` tokens).
- **Overlays** (drawers and panels) open over the page from the right at full
  height under the header, 400–440 px wide, and full width when compact.
- **Spacing** steps are 4, 8, 12, 16, 24 and 32 px.
- **Responsive:** every component works from 320 px up and sizes from its own
  container (CSS container queries), not only the viewport, because the same
  component appears full width and inside a 400 px overlay. Nothing overflows
  horizontally: rows wrap, long titles truncate with `…`, and mono text breaks
  anywhere. When the header lacks room it drops in a fixed order: actions into
  the More menu first, then the device and audience items, then the live
  status, leaving mark, product, title and More.

## Components

Each component lists its job, its required states and its rules. "Basis" is how
a shared package should build it (proposal): **own** means plain React and CSS
on tokens; **Radix** means a Radix primitive restyled to this document, in the
shadcn manner (code copied into our package, not a themed dependency).

### Header

The one sticky row on every screen: `tmt` mark · product word · rule · screen
title on the left; status group, then light actions on the right.

- States: actions overflow into the More menu below 1100 px; the status group
  shows connection, audience and live state as mark plus word.
- Rules: same height, type and order on every screen; a screen supplies only
  its title, status items and actions (`header` tokens).
- Shipped as `ColabHeader` (`colab-header.tsx`). Basis: own.

### Product switcher

The product word in the header is a menu trigger (`Colab ▾`) listing every TMT
browser surface: Colab, Remote, Settings (core) and Office, each with its state
as mark plus word (`● running`, `○ not installed`) and one action (`Open`,
`Set up`). The current product is selected. Products that aren't installed
stay listed and open the setup guide. Same position on every product; compact
width shows `tmt ▾`. Not shipped. Basis: Radix (Dropdown Menu).

### Setup guide

The first-run and not-installed screen, built from Card, Status mark and Button:
one row per step (install tmt, install the extension, start it, open it), each
with its state and, when it needs the terminal, one copyable command. The
browser never runs a command for the user; the page checks state again and ticks
each step by itself. The card's shadow takes the `waiting` role while a step is
open. Not shipped. Basis: own.

### Button

- Variants: **primary** (filled accent, hard shadow; one per card), **text**
  (light, default for actions), **destructive** (text in the `blocked` role,
  always last in its group).
- States: default, hover (underline), focus-visible (2 px accent ring),
  pressed, disabled (muted, keeps width, `aria-disabled` with a reason),
  busy (label kept, LoaderCircle before it).
- Basis: own.

### Toggle

A light text button with a square indicator before a fixed label; the indicator
alone shows on or off, so the width never changes. `aria-pressed`, Space and
Enter. Shipped as the page list's `Show archived` (#1728). Basis: own.

### Card

A square sheet with a 1 px ink border and a hard shadow.

- **Notice card:** the body of every state screen (opening, failed, ended,
  deleted, choose). Mark, eyebrow, title, one or two lines of text, actions
  inside the card. The shadow takes the state role's color. Shipped as
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
- Anchored below the trigger, right edges aligned; never covers the item it
  acts on.
- Shipped in `components/action-menu.tsx`. Basis: Radix (Dropdown Menu).

### Tooltip

Short help for an icon or a status item, such as the Remote chip.

- Opens on hover and on keyboard focus; Esc closes; the content is also the
  element's accessible description. Never the only place for required
  information.
- Basis: Radix (Tooltip / Hover Card for rich lists).

### Overlay

Comments, Chat, Source, Share and settings panels.

- Opens over the page without moving it, closes with its own `×`, Esc or the
  header action that opened it. Focus moves into it and returns to the opener.
- One overlay at a time; opening another replaces it.
- Basis: Radix (Dialog, non-modal for side panels).

### Popover composer

The small input at a selection or an item: annotation, follow-up, quick reply.

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

- Chat: user turns on the right, tinted with `accent-soft`. Agent turns on the
  left with a Bot mark. The approved target adds a square Bot avatar filled in
  the agent's driver color, a bold name, a driver tag (`claude`, `codex`) and a
  hard shadow in the driver color (the `review` role for Claude, `link` for
  Codex, as on the board).
- Threads: square User or Bot avatar and an agent-body rail. The approved target
  colors the Bot avatar and rail for the agent's driver.
- Literal markup in a body always renders as text.
- The shared `components/conversation-turn.tsx` ships for Chat and threads through
  `ask-panel.tsx` and `thread-panel.tsx` (#1772); the driver-colored treatment is
  not shipped. Basis: own.

### Status mark

The mark-plus-word unit used by every component above. Sizes match the
surrounding text; the live dot is 8 px. Basis: own.

### Band

An inline panel that expands under a row to show more without leaving the list
(`e` on the board; expanded details in a list). Same border as its container,
full inner width, its own collapse action as the last line. One open at a time.
Basis: own.

### Proposal item

A checklist item an agent writes into a page: title, body, proposing agent,
and a trusted action row (`Approve`, `Follow up`, `Decline`, resolve checkbox)
drawn by Colab, not by page HTML. Designed in #1773; not shipped.

## Token plan (proposal)

`tokens.json` already has `color`, `surface`, `font`, `header` and
`breakpoint`. These groups are proposed so components stop hard-coding values:

| Group    | Values                                                                                                                 |
| -------- | ---------------------------------------------------------------------------------------------------------------------- |
| `driver` | `claude` → the `review` color, `codex` → the `link` color                                                              |
| `type`   | sizes 11, 12, 13, 14, 16, 22 and 34 px with line heights and weights                                                   |
| `space`  | 4, 8, 12, 16, 24, 32 px                                                                                                |
| `size`   | controls 20 (chip), 24 (small control, minimum target), 32 (input, button); icons 16, 18; avatars 24, 28; status dot 8 |
| `border` | 1 px default, 2 px emphasis (focus ring, selected underline, rails); radius always 0                                   |
| `shadow` | hard offsets 2, 3, 4, 5 px (small button, chip and tooltip, menu and overlay, card), colored by role                   |
| `layer`  | z order: page, header, overlay, popover, tooltip                                                                       |
| `motion` | spinner 1 s; 0 under reduced motion                                                                                    |

## Interaction rules

- **Keyboard:** every action is reachable by Tab; focus is always visible;
  Esc closes the innermost open thing (list, menu, composer, overlay) and
  nothing else.
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
