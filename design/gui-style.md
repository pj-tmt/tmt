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

1. **Rounded and layered.** Controls and surfaces take the radius scale of the
   tokens: `radius-s` for chips and keycaps, `radius` for controls and rows'
   fills, `radius-l` for cards and overlays, `radius-pill` for pills and counts.
   Surfaces are opaque charcoal (or light) steps with thin rules. Only an overlay
   (menu, dialog, drawer, tooltip) takes the one overlay shadow; cards and rows
   stay shadow-free.
   **No one-sided border on a rounded box.** A rounded element (radius above 0)
   never takes `border-top`, `-right`, `-bottom` or `-left` alone: the lone edge
   curls at the corner into a hook. Use a border on all four sides, a fill or
   tint, or a separate straight indicator (an underline bar, a rail marker) with
   radius 0 that sits outside the rounded box instead of being its border. A row
   that needs a divider rule stays square. The leaf's CSS test fails a rule that
   sets a radius and a single-side border together.
2. **Edge to edge.** The page fills the window. There is exactly one window
   scrollbar; no region under the header scrolls on its own, except an overlay's
   body.
3. **One sticky row.** Only the one-row header is sticky. Everything else that
   appears on demand is an overlay that never reflows the page.
4. **Light actions.** Actions are light text buttons. A disabled action keeps its
   place and shows why. One primary (accent-filled) action per action group at most.
5. **Mark plus word.** A state is a mark and a word (`◆ waiting`, `✗ failed`).
   Color is never the only signal.
6. **Our controls only.** Use custom choices, toggles and icon help,
   without native selects, date pickers or confirm dialogs. A checkbox is the shared
   drawn checkbox (`tmt-ui-checkbox`, `BrowserCheckbox`) over the real input. Clipped
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
- **Spacing** steps are 4, 8, 12, 16, 24 and 32 px (`space-xs` to `space-2xl`);
  containers space their children with a token gap, never by margins alone.
- **Focus** is one 1px edge in the focus color (`focus-width`) on every control;
  fields and framed boxes recolour their own 1px rule instead of adding a ring.
- **Fields** (text fields, text areas and select triggers) keep the soft `rule`
  edge, an exception to the 3:1 non-text contrast the check-style controls meet
  (WCAG 1.4.11; the `edge` color on an unchecked checkbox or radio, a toggle's
  indicator). It holds on three conditions: every field has a visible label (a
  select shows its value and a chevron instead, a search field its placeholder
  beside a magnifier), its fill (`surface.field`)
  differs from the surface behind it, and its focus and invalid states keep the
  strong edge. The leaf's design audit (`controlBoundaries`) skips fields by name
  for the border check and fails a field that fills like its surface or has no
  visible label or value.
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
status remains visible as mark plus word. A page whose hero is its heading (the Colab page
list) draws no title in the header: the header keeps its accessible name, and the hero is the page's `h1`.

- Colab owner pages and space home are the exception to the leading mark: neutral
  `◉ local · <backend name>` (`local` when absent), Colab, `/`, title and muted
  `by <name>` (original author only) replace the default left group. Attribution is display-only, with no
  switcher or menu; full labels remain accessible. Hide author below 900 px, then
  attribution text below 640 px; keep the Remote mark, title and state chip. At
  every width, Discussion (count) and More remain; Agent status, Files (count),
  Source and Export live in More with their current authority and disabled
  states. Agent status is writer-only; its secondary Recheck uses a decorative refresh icon that yields to the busy mark. The title truncates within the shared-height row. Reader and native
  guidance headers retain the default mark and caption layout.
- Primitive: shared Header; host-owned optional attribution and title-prefix slots.
- Shipped as `ColabHeader` (`colab-header.tsx`). Basis: own.

### Shell

The one TMT web UI frame (#3937): one header row, then a global rail, an optional
module menu and the host's content. Every Colab and Remote page except the refusal
page renders in it.

- **Rail:** one fixed list, identical on every page: the collapse toggle in its own
  row, ruled off above Home, then Home,
  Colab, Remote, Ops, Digest, any other module Remote's
  [composed shell](../contracts/remote-channel-v1.md#module-descriptions-and-composed-shell)
  describes (in its order), and TMT settings at the bottom. A module the composed shell does not describe keeps its place, dimmed in
  the disabled text colour with its reason as the tooltip (not paired, or not
  installed); it is never removed. Every page paints the answer this browser read
  last (session storage), then refreshes it once.
  Each item is a Lucide icon plus its label; the current item has the selected
  fill, the accent icon and a straight marker (`indicator-width`) in the rail's gutter,
  outside the rounded row and never its border. The current module's pages nest under
  it as sub-items; the page you are on has the same fill and marker. A count is a
  small box after the label: a plain total in the hover fill, a needs-you count a
  pill in the waiting colour on its soft fill; `99+` above 99; a count of zero draws
  nothing. The number alone is
  drawn, and the item's accessible name says what it counts. Home sums needs-you
  counts only when every module reports one.
- **Collapse:** the toggle collapses the rail to 56 px of icons and expands it again,
  in one short slide (the `base` motion, none under reduced motion); icons keep
  their place. The choice is remembered on the device. Collapsed counts become
  a compact badge at the icon's top-right corner (a needs-you count in the waiting
  colour); hovering or focusing an item shows its label as a tooltip beside
  the rail, and labels stay accessible names.
- **Module menu:** the active module's own menu (Colab: All pages, then each project
  with its page count and needs-you count, then New project and Manage projects; Remote:
  each saved remote, Add remote, Remote settings). It replaces header dropdowns. On a
  wide screen its pages nest under the current rail item and share the rail's surface,
  rows and selection marker; on a phone they are a tab strip under the title bar. It has
  no title of its own.
- **Header:** one continuous sticky row: the host's header (moved into the shell's bar)
  shows the page title and the page's own actions, with one bottom rule. There is no
  module crumb and no brand in the bar: the rail's head carries the brand and the current
  rail item names the module. The header keeps the module in its accessible name
  (`Colab · <page>`), and the document title keeps it too. While a route loads the bar
  shows the module's name as its title and keeps its height and rule.
- **Bar cluster:** the shell owns the right end of the bar. A module fills the **search**
  slot (Colab: Search pages, an icon with a quiet `⌘K` key hint that a touch screen and a
  bar under 640 px do not draw; ⌘K opens it from anywhere in the module) and the **chip**
  slot (Colab: the Remote presence chip, `Remote · N`, not drawn under 640 px, where its
  count is in the page More menu). Search comes first. An empty slot takes no room, and the
  header leaves the room the shell measures for the cluster. The four-state connection chip
  of the [Remote switcher](#remote-switcher) is a design target and not shipped; the bar has
  no avatar slot until who the avatar is has been decided.
- **Module content:** the shell adds no padding or width to a module's main area;
  each module keeps its own layout. Shell page styles stay scoped to their own
  pages (Remote: `.tmt-page`), since every module shares the one document. While a
  route loads, its place shows the busy mark.
- **Phone (≤ 900 px):** the rail becomes a top-left menu button that opens it as a
  drawer; the module menu becomes tabs under the title bar.
- Primitive: `mountShell` in `design/browser-ui/src/shell.ts` (static hosts) and
  `BrowserShell` (React). Basis: own.

### Remote switcher

Design target for [#2463](https://github.com/pj-tmt/tmt/issues/2463) and the
[#2461 browser registry](https://github.com/pj-tmt/tmt/issues/2461); not shipped.
Remote owns the registry, registration, static header and Remotes view; Colab owns
the React header glue and page-follow behavior. Use **remote** throughout UI, CLI help and
handbook copy, **Local** / **Firebase** for kinds and **this machine** for the host.

- **Module menu:** inside the [Shell](#shell), Remote's module menu lists the
  remotes. Show the current origin's live state:
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
  Retry only rechecks the current origin with its own keys. The last entries are
  `Add remote…` and `Remote settings`, whose Remotes section keeps the copyable
  `tmt remote settings default-remote <ref|local>` command for the separate CLI/API
  default. No browser default mark. Never silently fall back.
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
- **Static menu:** Remote renders these rows through the leaf's shell renderer,
  without React; disabled rows keep their place and say why in their tooltip.

Look review at 1440 light / 390 dark covers the Remotes list (current connected
and foreign not checked), Add remote (door address and remote link), catalog
confirmation with a collision and nothing addable, remove confirmation, and the
Remote module menu's four row states and unreachable-page notice.

### Colab Pages

The space list follows the concept website `/#/colab/pages`: hero with New page,
pinned-page cards, state strip, project scope, search/sort and the page list. Numbered
concept 17 is the fallback only for states absent from that website. Actual pins keep owner order, wrap after three
on wide screens and stack on compact screens; absent pins leave no empty section.
Projects keep every membership and use a stable project-name mapping onto the existing blue, violet, amber and green role tints. Row stages use soft-filled role chips with dots; Needs you keeps its real count. Stage, creator, time and
progress show only available metadata; no missing summary becomes a false zero or stage.

Wide rows separate title, projects, stage, updated, author and access; compact rows stack
these without horizontal page overflow. Rows with divider rules remain square inside the
fully bordered rounded list frame. Task progress is compact count text, without a repeated bar or comments line. Home is the sole vertical viewport, with the shared page insets and a truthful count/access footer. Successful action feedback remains an unobtrusive live region, while errors stay visible. Selection, keyboard actions, incremental rendering and
management refusals retain their product owners. The shared flat state tabs show muted numeric
counts and wrap on compact screens; the hero owns the heading and the shell bar keeps only tools.

### Colab Projects

The space-level project manager follows the concept website's Manage projects drawer:
introductory copy, separate bordered project cards with name/count and Rename/Delete actions,
and New project below them. Manager-row Delete is ghost; the final confirmation stays destructive. New project uses a decorative leading Plus, not a text glyph. New project has the name field, a divider and an Initial pages
selection, with file marks and right-aligned Cancel/Create project actions. Archived choices
remain explicitly labeled; project mutations disclose their full affected scope.

Rename and delete use a centered shared dialog that replaces the manager and returns to its
card action on cancel. This preserves the one-overlay rule instead of the website's stacked
surfaces. The shared regular drawer owns geometry; products never override its internal width.

### Colab page dialogs

New page presents the literal machine command with its Copy action, the return instruction and
Close in the shared dialog footer. The host-owned copy contents include one decorative mark;
clipboard feedback describes copying, never execution. Lifecycle confirmations use the shared
footer and its divider, with a bordered Cancel beside the operation action. Titles, affected scope,
restore and destructive consequence words remain product-owned. Shared dialog geometry and
presentation stay in the browser leaf; products do not override its internals.

### Quick switcher

Colab's **Search pages** header action and Cmd/Ctrl+K open a modal
[BrowserDialog](browser-ui/README.md#dialog) over the current page. It uses the
same search editor and label/author filters as the page list; collections are not
part of v1. The website `/#/colab/pages` Search surface (concept38 fallback) supplies the
layout: query with a search mark, a folder/project picker and shortcut row, divider,
file-marked results with title and project/author metadata, then keyboard hints in the
shared footer. Results preserve native page links and show only real metadata, with the
mark-plus-word Needs-you chip at the right. The current page is excluded. Project choices
write the same search edit as the query chips; plural memberships wrap inside a result.
The shared dialog owns its header rule, width and scrim.

- Keep the query and project rows above a scrollable, single-column result list, with no
  loading flash when a cached list is available. The input keeps focus while
  arrows move the active result. Filter suggestions get the keys first; otherwise
  Enter opens that result. Escape closes suggestions, then the dialog and returns
  focus. Successful navigation focuses the destination title. Editors retain
  their own shortcuts; repeated and composing keys never activate a result.
- The content-fitting surface is opaque, thin-edged and rounded (`radius-l`), with the
  overlay shadow, using existing focus/selection/scrim tokens. It is modal at every width, with
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

- Variants: **primary** (the accent fill: the one main action of a group, at most one
  enabled per action group, which Colab's `e2e/primary-groups.spec.ts` checks; a panel
  that holds several forms and rows marks itself `data-action-group` and is one group),
  **secondary** (the neutral bordered button, for the other actions beside it),
  **text** (light, default for actions), **destructive** (text in the `blocked`
  role, always last in its group).
- States follow the leaf
  [Action contract](browser-ui/README.md#controlled-input-contracts): neutral
  hover/focus (same text colour on `surface.hover`, lighter than selection), visible focus, opaque muted disabled controls
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
a 1px strip rule with the control radius. The boxed selected tab uses the selection fill and
strong neutral text. Flush uses a straight 2px accent underline. Flat has no strip frame:
the selected pill uses selection fill, strong accent ink and a transparent edge reserving
the focus frame, optionally wrapping into rows.
A count is a small muted number after the label; selected flat counts use neutral text for contrast.
State marks retain their own roles.
The leaf's [Tabs contract](browser-ui/README.md#controlled-input-contracts)
owns React/static inputs and panel lifetimes. Basis: own.

### Card

An opaque, shadow-free sheet with the shared thin edge and the control radius.

- **Notice card:** the body of every state screen (opening, failed, ended,
  deleted, choose). Mark, eyebrow, title, one or two lines of text, actions
  inside the card. The mark carries the state role. Shipped as
  `NoticeCard` (`notice-card.tsx`), which owns its centered width and outer margins alongside the native guidance state-card host; that placement does not apply to inline shared notices.
- **Item card:** a page in the page list: icon, title, short ID, audience tag,
  expiry, actions row.
- **Group card:** `tmt-ui-card`, one subject per card (a Digest provider, a chart): a head
  (sentence-case title, optional muted note) over the body. Facts inside it are bare status tiles
  (label over value, no tile box, a dot only for a tone), never a box in a box; tile chrome is for a
  page-level row. A read-only measure is a `tmt-ui-meter` placed directly under the
  value it measures, coloured by its chart series.
- **Overview page:** a page of live state (Ops Agents) reads top to bottom as a page head
  (`tmt-ui-page-head`), one `tmt-ui-stats` strip of counts (a count in the display face, its
  word, a caption), a `tmt-ui-banner` for what waits on the reader (hidden at zero), then the
  flush-card table and a facts card for the selected row (one row is always selected: the reader's choice, else the first
  that needs them, else the first). Counts and states are always a mark and
  a word. What the reader sets (filter, search, collapsed groups, selection) stays across a refresh.
  A page that is read-only shows no row menu.
- Basis: own.

### Chip

A one-line label whose content is always vertically centered (fixed height,
inline flex). Kinds: **status** (mark plus word), **filter** (a toggle button, accent when on) and
**count** (a label and a boxed number such as `Needs you 5`: `BrowserCountChip`, or a
`BrowserMenu` count trigger with a chevron that opens its list; the attention tone is
for work that waits on the reader). Static chips are rounded outlines; interactive
ones are buttons with a focus edge. A **passive** chip (label only, no remove control) names a
state that is not an action, such as `Not connected`. Height 20 px, filter
24 px. Basis: own.

### BrowserChips

A removable list uses shared BrowserChips/BrowserChip: rounded 1px chips with a
centered one-line label and an explicit remove control. Wrap the list at narrow
widths; keep the full accessible name when a long label ellipsizes. A trailing
`＋` opens a search combobox/listbox. Arrow keys move, Enter adds, Escape returns
focus, Tab closes, and Backspace in an empty non-composing field removes the last
chip, when enabled. A host that lists recipients with it (the leaf fixture does; Colab's composer binds
mention chips from its text instead) uses `Remove @name` and `Add recipient`; selected agents are not
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
- Shared owner: [BrowserDrawer](browser-ui/README.md#drawer-and-menu), with native HTML dialog containment at every width.
- Source uses the shared header action slot for primary Save before Close, with a truthful draft explanation, paper-toned bordered monospace editor and lower draft-equality metadata. The owner and editor-link save contracts and real notice states remain unchanged. No equality state claims a confirmed save.
- Colab shared page drawers (Chat, Source, Agents, Files, Comments, Export and About) leave an 8 px gutter below the header and at the right and bottom edges; through 480 px use 4 px. Keep the header visible, an opaque rounded surface and one 1 px rule on every edge. Source may be wider; regular drawers are at most 430 px (viewport minus 18 px), and use viewport minus 12 px through 640 px. Compact drawers fill the available width between gutters. The scrim covers the visible header and author iframe; the first press only dismisses, with no click-through, and the second press acts. The drawer body contains overscroll; there is no body scroll lock. Neither changes the author frame width. The modal scrim dims and blurs the page using the shared scrim role and 3 px overlay blur.

### Non-modal side panel

Use [BrowserPanel](browser-ui/README.md#non-modal-panel) for a side region that must
leave the page interactive. It shares the Drawer header. The host selects column
(its own grid), card (761–1179 px) or sheet (through 760 px) mode. Opening focuses
the heading; Escape inside and Close request dismissal, then return focus to the
opener. There is no scrim, trap or inert page. The host coordinates it with modal
drawers and retains content, drafts and breakpoints.

### BrowserNotifications

Transient reply cards below the header at the right, separate from persistent
discussion state. Shipped Colab destinations are **Chat**, **Thread**, **Answer** and **Proposal**; the
visible text tag names the destination, with the quoted passage for Thread.

- Opaque cards with one thin edge, the control radius and the overlay shadow, entering
  and leaving with the base motion. Use an 8 px header/right
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

- Shared Drawer header: Chat, `○ Everyone with access` caption and Close. A turn is the
  website's: the shared avatar, the name (identities come from verified metadata) and the muted
  time at the end of that row, then the text in a filled bubble under the name. Space separates
  messages; day rules use admitted timestamps.
- Center the empty title and teach `@` only where admitted. Keep the composer pinned at the
  bottom as one bordered box: the editor, then Attach/recipient at the start of its row and a
  primary `Send` (the paper plane, then the word) at the end. Show
  `Posts as a comment.` only without a recipient. Failed delivery
  uses `✗ Not delivered · Retry` with the existing recovery action.

### Popover composer

The small input at a selection or an item: annotation, follow-up, quick reply.

- Colab shows Annotate beside the selection's focus end only after the pointer is
  released or keyboard selection finishes. C opens the same annotation view; ignore
  text-entry targets, modifiers, repeat and IME confirmation. Alt+Enter remains available.
  Opening captures the quote without publishing or sending. The dark pill has a
  speech-bubble icon and `Annotate`, with a short thin leader at the focus end.
  Shortcut help is accessible through `aria-keyshortcuts`, without visible hotkey copy.
- Annotation ranges use the mint `annotation-highlight` tint and a 1px
  `annotation-rule` underline, unbroken under descenders (skip-ink off on the highlight), with
  no rounding or shadow, preserving authored text
  colour and weight. While the pointer is over one it takes the `-hover` tint and a
  2px underline, with no outline box. The passage whose thread window is open takes the
  `-selected` tint and a solid 3px `annotation-rule-selected` underline (hover's is 2px), above
  hover, and its marker (or the `+n` pill that holds it) wears a 2px ring in that colour, 2px
  outside the circle inside the 24px hit area; both return to rest when the window closes. Round number markers stand in the page's right
  marker lane (28px, reserved on every page from its first paint, so the first thread
  moves no text), centred on the passage's last line: a 16px circle inside a 24px hit
  area that covers no text. On a column page the marker stands 4px after the column's
  end, beside its passage; on a page that spans the frame, at the frame's edge. Markers
  on lines closer than one hit area share one `+n` pill (16px tall, at least 16px wide,
  4px inside each end, same 24px hit area) that opens the thread list.
  Creation-order numbering includes every non-deleted thread before view filters.
  Deletion renumbers; resolved threads retain list numbers without page markers.
- Annotation and thread windows are borderless, square grey panels with an uppercase
  monospace label and a large borderless composer. Wide windows sit beside the
  selection with a leader; mobile retains bottom placement without one. The footer
  has muted `Attached to this sentence` and square arrow-only Send, with a complete
  recipient-bearing accessible name. The existing creator-notification preference
  remains a borderless mark-and-label toggle beside it. Resolved windows start collapsed with counts,
  state/Reopen and an explicit Show conversation action; expansion retains drafts.

- The text decides the recipients. There is no separate recipient control: bound
  mention chips and exactly typed `@name` tokens that match one current directory entry
  are the recipients, and removing or replacing a token or chip removes its binding.
  Prior replies and display names never choose a recipient. The one default is the
  page's admitted creator, seeded as a chip in each fresh message while the device-local
  creator toggle is on; removing that chip suppresses it for that message only. Unknown
  names stay plain text, ambiguous names block Send until chosen or removed, fuzzy
  matches never select, and mention completion preserves the surrounding text and caret.
  The [browser discussion contract](../extensions/tmt-colab/contracts/colab-v1.md#inline-annotation-conversations-1587)
  owns binding, the eight-recipient fan-out, offline handling and failure rules.
- Enter submits the parent's current message action; Shift+Enter adds a line, Esc
  closes the innermost candidate list before the composer. Typed text and the selected
  recipient are kept on close and restored on the same anchor with `Draft kept`.
- There is one Send. With no bound or exact mention it records a plain comment and
  asks no one (`Posts as a comment.`; with a resolved creator and typed text the hint
  adds `No agent will be asked.` and an `Ask @name` action that only inserts a chip);
  with mentions its name says who gets it (`Send · to <names>`). A plain comment stays
  available under current content-write admission when agent discovery fails. Choosing
  or removing a recipient never prepares or sends a message.
- Outside press closes it, but a page click never hides a typed draft; nothing
  closes it while a send is in flight.
- `components/message-composer.tsx` is the shared plaintext editing boundary for
  Chat, annotation, thread reply and comment edit. Its plaintext/history
  extensions preserve undo, IME and exact message bytes; the source editor stays separate.
- Candidate lists use available viewport space and the browser popover layer, including
  inside modal Chat. The field grows to a bounded height, then scrolls. Square corners
  and existing ink/accent tokens apply. Basis: own parent chrome with Lexical editing.

Mention chips share an icon and named roles in editors, sent messages and lists
(a bordered chip rounds at the small radius token, as the website's tags do):
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

Annotation anchors and thread headers share the annotation action fill, semibold
system font and grid-centered number: an anchor is a 16px circle with a 10px number
inside its 24px hit area, a thread header's number a 22px circle with a 12px number.
Header items align vertically with an 8px gap.
An anchored window adds no decorative connector line beside the circle.

One message in Chat or a thread: author line (`name AGENT · time` for agents, `name · time` for people), body,
state (mark plus word), and a `⋯` menu for the author's own Edit or Delete.

- Chat and threads share `ConversationTurn` structure and history ownership. A turn, a person's
  or an agent's, leads with the shared `BrowserAvatar` (decorative, size `s`, the author's
  initials) before the author line: the name in semibold sans, then the muted time, so an agent's
  name is the chip's text without its robot; a working agent keeps the robot with its dot. The
  text differs by surface. In Chat it is a bubble under the name, indented past the avatar: the
  `command` surface fill, a 1px rule edge and the radius token, square at the top start, the same
  whoever wrote it. In a thread it sits directly on its surface with no card fill (the Discussion
  view separates turns with a 1px rule). The thread's own `THREAD` and `OPEN` header may stay mono.
- An agent reply looks the same running or finished: no edge or colour per driver
  and no driver word. A running turn shows only the
  chip's own working state. The verified `runningDriver` never selects a
  treatment. No shadow.
- A body is a small safe Markdown subset: paragraphs, bullet and numbered lists, block quotes,
  fenced code, `code`, **strong**, _emphasis_ and links, with 8px between blocks. Anything else
  stays the text the author wrote, and raw markup always renders as text. Fenced code scrolls
  inside its own block and never widens the window. A link opens only a page of this space, in the app; an
  outside URL shows as words followed by the URL, and any other target stays literal text.
  Basis: own.

### Page files

Files shows the real page-file count, a 64 px decorative type mark beside each
filename and size, and a lower right action row: bordered Preview/Download and
ghost Remove, with leading icons. The actions wrap on phones. Only admitted
raster types have Preview. A visible multiple-file chooser stages local bytes;
staged rows use a full dashed border, a soft filled surface and a named close
button. Add to page is the one primary action, aligned left below the staged
rows. Choosing or removing a staged file does not upload or save.

Preview and removal confirmation replace the drawer rather than stacking over
it. Closing returns to the same file action, with staged bytes retained. Remove
confirms reference removal; it never claims to erase stored or local file data.
The real read, upload, refusal and stale states remain readable.

### Message attachments

Annotation/thread attachments use flat metadata chips with a type mark, filename,
type and size; previews sit below. Uploads show progress, refusals show `✗` with
reason and Remove. Pending annotation/thread chips scroll within a viewport-bound
area so Send stays reachable. Trusted Send starts staged uploads and stays busy until done.
Chat pending attachments use one horizontally scrollable row of small raster
thumbnails or filenames inside the input below its text, each with a named ×
remove action. They share the text's left edge, with no cards or progress bar.
The message composer has a visible `Attach` action.
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

### Charts

`@tmt/browser-ui/chart` paints three kinds of data view with plain SVG and HTML, no
dependency and no inline style: `createSeriesChart` (stacked areas, a 100 % share, columns or
lines on one time axis, with an optional dashed reference line and vertical guides),
`createMatrix` (who talks to whom) and `createStrips` (rows of fixed slots). Each has a pure
model (`seriesModel`, `matrixModel`, `stripsModel`) that carries every number, label and
intensity step; the painter only maps it to elements, so the model is what is unit-tested.

- Colour comes only from tokens. Series use the categorical roles `browser.color.series-1` to
  `series-7` (blue, orange, aqua, yellow, magenta, green, violet: the validated data-viz order,
  each stepped for light and dark), separated by a sheet-coloured rule. A chart keeps a few
  series and folds the rest into one `Other`, drawn in a neutral step of the text colour
  (`tmt-ui-chart-s7`). Default matrix intensity uses five steps of the text colour; its text
  meets 4.5:1 on every step in light and dark (a test computes it). Colour is never the only
  carrier: every series is named in a legend (a line chart also shows its value at the selected
  column; a stacked chart leaves values to the hover tip, since a last bucket reads like a total)
  and the data table is always there.
- The opt-in `tmt-ui-chart-seats` palette uses the website's six pastel `browser.color.seat-*` roles,
  with 75% area opacity. It scopes chart and legend together, leaving default provider hues intact.
  `curve: 'monotone'` smooths area edges through the sampled positions without overshoot.
- Hovering the plot (or focusing it and using the arrows) shows a tip beside the column with the
  time and every series' value; a visually hidden live line says the same to assistive technology.
- A missing observation is never a zero. A line breaks, a cell reads `—`, a slot stays blank;
  a known none is `·` and the faintest step. Self is a blank cell in the default matrix.
  `numeric: true` draws known zero as `0` and self as `—`; unavailable stays `—` with an explicit
  spoken label. Numeric matrices use borderless, sans-serif counts on quiet mint cells.
- Keyboard: the plot is one tab stop; Left, Right, Home and End move the readout. Every chart
  also carries a visually hidden data table for assistive technology. A matrix wider than its
  container scrolls inside its own named, keyboard-focusable region; the page never scrolls sideways.
  Sender labels stay opaque and sticky at the left. Matrix cells use separate, zero-spaced
  borders so each sticky sender keeps its own right and bottom rules while scrolling. A separate straight edge cue marks more
  columns to the right and disappears at the end or when the matrix fits. Digest keeps all
  seats in that region; it does not silently discard low-traffic participants.
- Text is repainted at the container's width (ResizeObserver), so type keeps its size at phone
  and desktop widths. Empty data shows one plain sentence, not an empty axis.
- Square strips use the first series hue. `layout: 'heatmap'` uses mint, short rectangular bars
  in the token radius; optional column `labels` identify their buckets. Missing slots remain
  hollow, and value tips and keyboard traversal keep the square strips' model.
- More series than a chart keeps (six on Digest) fold into one `Other` with `topLayers`.

### Status mark

The mark-plus-word unit used by every component above. Sizes match the
surrounding text; the live dot is 8 px. Basis: own.

### Band

An inline panel that expands under a row to show more without leaving the list
(`e` on the board; expanded details in a list). Same border as its container,
full inner width, its own collapse action as the last line. One open at a time.
Basis: own.

### Question item

A question is the concept website's question block (route `#/colab/page/ops-ui`, built from the
numbered concept 40 where the website lacks it): one band, the page sheet between two 1px rules with
no side rule and no radius. A line is a `?` glyph in a disc, an eyebrow `Q&A · <agent>` over the
title, the state (`◆` and `Needs your answer` in the waiting colour, or a check and `Answered` in the
secondary colour) and a chevron; an answered line shows the recorded answer under the title. The open
band's fields start in the heading's column: a kind line and an instruction over the choices in one
wrapped row, the `Other` field under the row once Other is picked, then a rule and a `Goes to` row
(recipient chips, add) that ends with the group's one primary action, `Send answer` / `Send answers`;
`Retry`, `Change answer` and `Open reply` stay text actions. A group of several questions is one band
with one heading, its rows separated by the same rule, and one Send. It differs from the website only
where the product does more or the tokens decide: recipient chips replace plain text, the type scale
and the opaque sheet follow the tokens, a multiple or text question has its own control, and the
primary reads `Send answer` because the answer is sent at once (the website's `Review answer` has no
shipped step).

A question card's choices are the shared `BrowserChoice` rows in a `BrowserChoiceGroup` (legend
`Choose one answer, or write your own.` or its multiple form, drawn under the kind line), the `Other` row
last, in one wrapped line. The digit keys 1 to 9 pick a row from anywhere in the card except a text
field, and the focus follows the pick when it starts on a choice; no row draws a key cap. Each control declares its key
(`aria-keyshortcuts`) and shows it in its hint, and is named by its label alone. A row carries its
own vertical spacing so the whole row is the tap target, with no gap between two rows. Shared row
metrics stay owned by the [browser leaf](browser-ui/README.md).

### Proposal item

Colab renders authenticated proposals as parent-owned cards: title, body,
proposing agent and Approve / Decline / Follow up / Resolve actions. Approve is
the emphasized decision; Resolve is muted. Follow up opens a composer inside
the card. Resolved proposals fold to a plain one-line row with a resolved mark,
retained-history disclosure and Reopen. Comments lists the page's threads in
views `Open`, `Needs you` and `Resolved` (and `Proposals` when the page has one),
a 1px rule under the view strip, as rows of number, title, quoted passage and
`N comments · state`, each closed by a 1px rule with 18px of vertical padding; the title stays at
14px (the compact body token) and the quote and meta take the caption size; the selected view tab uses the shared
selection tint. Each view has an
empty state; its header count combines unresolved ordinary threads and undecided unresolved
proposals. Selecting a row opens that thread's Discussion view in the drawer: a status line
(`Needs you`, `Open discussion` or `Resolved by X`) with `All comments`, the title, the quoted
passage with its number and a `Show in the page` arrow, the turns, the reply composer and a footer
rule with `Resolve` or `Reopen`. The reply composer is a four-line field flush with the content
column, with one row under it: the note (`Attached to this sentence · Posts as a comment.`) at the
start and the primary `Reply` (the paper plane, then the word) at the end.

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
- **Motion:** two durations, two easings and two distances, all tokens
  (`browser.motion`: `fast` 120 ms, `base` 180 ms, `ease-out` for entering,
  `ease-in` for leaving, `scale` 0.96, `rise` 6 px). Every overlay enters and
  leaves with the same motion and stays mounted (`data-state="closed"`) until the
  exit has played: a dialog fades and scales in the base duration, a drawer slides
  from its edge in the base duration, the scrim fades, a menu, popover, tooltip or
  autocomplete list fades and scales from its anchor in the fast duration. Hover
  and press on actions and tabs, the menu row tint and the rail collapse use the
  fast or base duration. Under `prefers-reduced-motion` every duration is 0 ms, so
  a change is instant. A leaving layer is inert: it takes no pointer input and
  hands its Escape to the layer under it. The LoaderCircle spin (see
  [Transient states](#transient-states)) stops under the same preference. The
  leaf's browser test (`design/browser-ui/e2e/overlay-motion.spec.ts`) asserts the
  mounted-as-closed exit and the instant reduced-motion path.

## Transient states

A send or save never flashes an in-between state. The
[Action contract](browser-ui/README.md#controlled-input-contracts) implements
the busy mark and busy state for every Action and IconAction.

- **Delayed busy mark:** nothing changes for the first 400 ms of busy. After
  that the action's reserved busy mark shows, and once shown it stays at least
  400 ms. Errors show at once. No label swaps (`Saving…`) and no inserted
  status paragraphs.
- **Spinner:** one turn per 0.9 s, linear, infinite, defined once in
  `browser-ui` (`browserUiClasses.spin`, `tmt-ui-spin`). Every shown
  LoaderCircle uses it: the busy mark of an Action or IconAction turns by
  itself, any other mark takes the class. A surface never brings its own
  keyframes or speed. Under `prefers-reduced-motion: reduce` the mark stays
  on screen and stands still.
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
every screen, the absence of native selects on the page list, and that its only
checkboxes are the shared drawn row selection.
A shared package should move these checks next to its components so every
surface runs them.
