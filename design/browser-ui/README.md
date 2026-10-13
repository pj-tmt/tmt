# Private browser presentation leaf

`@tmt/browser-ui` is a private tmt-ux-owned leaf. Products may depend on it; it
must not import product code. Core and the CLI must neither depend on nor embed
it. The all-file native input guard proves direct literal/manifest-relative inputs
and local single-arm literal-forwarding wrappers at every literal call site;
unproved macro scope, forwarding, aliases or expressions fail closed. The package, its workspace and lockfile entries, shared quality
checks and static COPY integration are in place. Product adoption and product
release readiness are separate consumer work.

## Entries and generation

There is no root entry. `@tmt/browser-ui/static.css` exports checked
`generated/static.css`; `@tmt/browser-ui/static` exports the frozen
`browserUiClasses` and presentation types without React; `@tmt/browser-ui/react`
exports `BrowserHeader`, `BrowserNotice`, `BrowserField`, `BrowserAction`, `BrowserIconAction`,
`BrowserToggle`, `BrowserList`, `BrowserListRow`, `BrowserCommand`, `BrowserDrawer`, `BrowserMenu`, `BrowserTabs`, `BrowserNotifications`, `BrowserChips`, `BrowserChip` and their prop types. The static entry also exports `BrowserTab` and `BrowserTabSelectReason` types. The optional peers are React 19.2.8 and
lucide-react 1.52.0. React components use built-in icons for controls such as Drawer
and Notifications dismissal; IconAction also accepts a caller-supplied icon.
Static serving requires neither peer.

The browser color, surface and shared metric roles live in the `browser` group
of `../tokens/tokens.json`. Legacy terminal/soft roles remain separately owned.
Fonts and header metrics use the existing authoritative groups. One owner
projects these values and the ordered CSS fragments:

```sh
node design/browser-ui/scripts/generate-static-css.mjs --write
node design/browser-ui/scripts/generate-static-css.mjs --check
```

The check compares expected bytes without writing. Consumers embed checked CSS;
Cargo and installed products never run the generator. Select light/system-dark
on the document or scope `data-theme="light"`/`data-theme="dark"` on an island.
A dark island on a light page needs dark foregrounds as well as dark surfaces.
Every component is opaque, square and shadow-free. Semantic marks retain words.
Links, essential edges, selection indicators and keyboard focus are separate.

## Host metric inputs

The leaf preserves differing product metrics through explicit custom properties;
it does not guess a product profile or replace them with fallback values. The
host defines every property used by its chosen components. Header sizes are
56px/48px below the existing 479px threshold. Page positioning, body offset and full attribution disclosure remain host-owned; Drawer and Menu own their top-layer geometry and dismissal.

| Host property suffix after `--tmt-ui-host-`                                             | Existing product values retained for adoption                                                           |
| --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `header-action-gap`                                                                     | Colab 6px                                                                                               |
| `focus-offset`                                                                          | Colab 3px; Remote 4px                                                                                   |
| `toggle-focus-offset`                                                                   | Colab 2px                                                                                               |
| `notice-mark-size`                                                                      | Colab 18px; Remote 30px                                                                                 |
| `eyebrow-line-height`                                                                   | Host's existing line height                                                                             |
| `notice-heading-size`, `notice-heading-weight`, `notice-heading-tracking`               | Colab clamp(26px, 4vw, 36px), 700, -0.03em; Remote clamp(28px, 4vw, 36px), 650, -0.025em; settings 25px |
| `notice-body-size`, `notice-body-line-height`                                           | Colab 16px / 1.55; Remote 17px / 1.6                                                                    |
| `notice-action-gap`, `notice-action-margin`                                             | Remote 12px and 24px; form placement remains host-owned                                                 |
| `action-padding`, `action-min-height`                                                   | Colab 7px 12px with existing height; Remote 10px 22px and 44px                                          |
| `action-size`, `action-weight`, `action-line-height`, `action-font`                     | Colab existing inherited type; Remote 13px / 600 mono with existing line height                         |
| `action-gap`, `action-underline-offset`                                                 | Host's existing gap; Colab underline offset 4px                                                         |
| `icon-action-size`                                                                      | Explicit square icon target; at least 24px. The fixture derives 34px from existing icon/gap tokens.     |
| `field-padding`, `field-size`, `field-line-height`                                      | Colab 8px / 14px / 1.5; Remote 12px 14px / 17px with existing line height                               |
| `field-label-size`, `field-label-weight`, `field-label-line-height`, `field-label-font` | Host's existing label; Remote 12px / 600 mono with existing line height                                 |

Thin notice/action borders use the accepted 1px browser metric; field/header
borders remain 1px. This supersedes earlier shadow and opacity proposals only.
Input autocomplete remains a host-owned listbox. No native select or new font loading enters this leaf.

## Controlled input contracts

Header has one h1. Optional `leading` and `headingPrefix` slots place host-owned
attribution before the brand and a separator before the title; neither supplies state
or changes default layout. A supplied caption requires a stable `captionId`, retaining
full DOM copy and title association; the host supplies a visible full-caption
read path. `brandLink` preserves a caller's native anchor. `disclosure` selects a
caller-owned data attribute; the leaf does not choose or operate a menu.

The header brand uses the TMT mark (a knocked-out rounded square, one even-odd
path) mirrored in `site/src/home/assets/v9-0.svg`, rendered inline with the `0 0 96 96`
viewBox, `aria-hidden="true"` and `fill="currentColor"`. The leaf neither imports
the site nor fetches the mark. The product label and brand link use the normal
text colour. Mark size and brand gap derive from the wordmark-size token using
the handbook's 33/19 and 12/19 ratios (about 24.32px and 8.84px with a 14px label);
they stay equal at compact and wide widths, within the 48px/56px header. The
screen title follows the brand as plain text, without a divider.
Static hosts supply their own inline SVG with the same class, viewBox and
decorative attributes; the shared stylesheet does not replace a host's text mark.

Notice tone, visible state label, mark and announcement are explicit. No state,
health, permission or recovery inference occurs. Marks are decorative because
visible state words carry their meaning.

Field requires stable `controlId`, distinct IDs for rendered description/error,
and an explicit `renderControl` that spreads exactly the supplied ID, class and
ARIA props onto one focusable text control: a native input/textarea, or a
contenteditable root with `role="textbox"` (plus `aria-multiline="true"` when multiline),
or `role="combobox"` when that root owns a popup such as mention completion
(with `aria-expanded`/`aria-controls` supplied by the host).
The supplied `aria-labelledby` points to the visible label's `${controlId}-label`
ID; that ID must also be distinct from control/description/error IDs. A host
supplies focusability, such as `tabIndex={0}`, for a non-native control.
Host IDs precede local description
and current error IDs, with ordered deduplication. Absent content contributes no
ID; error copy does not imply invalid. The host retains value, change handler,
ref, read-only/disabled/access decisions and original control identity. Update
attributes on that same control when access changes; no remount or draft reset.

Action uses an explicit native button type, retained visible label and optional
busy mark. Keep the same optional `busyMark` supplied in ready and busy states;
its leading slot remains in flow while visibility changes, with an equal trailing
reservation that centers the label in both states and preserves button geometry
without changing host padding or typography. Without a mark, both
states retain label-only geometry. Disabled/busy blocks callback delivery. A supplied disabled reason
requires its stable ID and remains visible. `onActivate` receives the original
React MouseEvent from native button click, including keyboard-generated click;
there is no extra key handler or command DTO. Trusted-event admission remains
with the caller. Host links remain host links. By default `label` names both the
visible action and its accessible name. A host may supply `visibleLabel` for
shorter action text; the full `label` must begin with that visible text and remains
the accessible name.
Text actions have a transparent background in ready and disabled states;
disabled text is muted. Enabled hover and keyboard focus use selection colors.
Field focus strengthens the same 1px edge to a contiguous 2px rule in the focus
color, entirely inside the border box. Native fields and contenteditable controls
use the same `:focus-visible` treatment, without an outward ring, shadow or size
change. The host's `focus-offset` applies to other controls, not field focus paint;
field label spacing retains its existing host metrics.

Toggle is controlled: fixed label, `aria-pressed`, independent visible checked
indicator and original activation event. It never changes its own pressed value.
The leaf owns the symmetric 8px toggle inset through `toggle-gap`; hosts only
supply the focus offset. Disabled styling cannot inherit hover/selected styling. Static hosts perform the
same native semantics and ID associations themselves; CSS owns no effects.

List uses a named native `ul`; each ListRow is a native `li` with title, updated
metadata, state and optional trailing actions slots. The host owns title links,
metadata formatting, visible state words and menu behavior. Row padding is 10px
vertically and 14px horizontally, from the leaf's list padding tokens. One 1px
rule separates rows. At the header's compact threshold, metadata moves beneath
the title while actions remain at the end. Row insets and action geometry stay
constant across widths. Titles precede actions in DOM and keyboard order.

Colab supplies links for active titles and plain text for archived titles, with
an actions disclosure in both cases. Unknown or out-of-range update times use
`Update time unknown` without a fabricated date or `time` element. The host
retains window scrolling; the list does not establish a scroll container.

IconAction accepts Action's props plus a decorative `icon` node and optional
controlled `pressed`. The nonempty `label` is the button's `aria-label` and
the default visual tooltip copy; icon and tooltip are `aria-hidden`, so the name is
announced once. `pressed` is omitted when absent; supplied false/true becomes
`aria-pressed`, and activation never changes it. Pressed uses selection colors
and a 1px text-colored edge; disabled/busy styling and event fences take priority.
The icon and busy mark share one fixed square target without changing geometry.
Disclosure actions instead supply controlled `expanded` and optional `controls`,
forwarded as `aria-expanded`/`aria-controls` on the same button. Hosts may supply
`buttonRef`, `hasPopup` and `onKeyDown` for their own menu focus, popup semantics
and keyboard handling; the leaf does not own the menu. The type contract
requires `expanded` with `controls` and excludes `pressed` from disclosure actions.
Expanded uses the same selection treatment as pressed. While expanded, the tooltip
stays hidden and installs no Escape listener; the host owns menu dismissal.

An optional `tooltipLabel` adds shortcut help while `label` remains the required,
independently sufficient accessible name. `ariaKeyShortcuts` exposes the same
shortcut to assistive technology. Omitted tooltip text defaults to `label`;
help is display-only and never replaces the button name.

The tooltip is a manual native popover bounded by the visual viewport. It opens
below the button, flips above when its measured height does not fit below and
there is more room above, and clamps to that side when neither side fits. Pointer movement over the action and keyboard `:focus-visible` show it
immediately; layout-only pointer entry does not. The display-only
tooltip never receives pointer events. Activation dismisses it synchronously; a
fresh pointer movement after leaving restores hover help. It closes when neither
pointer hover nor keyboard focus remains;
Escape dismisses it without consuming the host's key event, so a containing menu or dialog closes in the same activation.
Focus restored by that Escape does not open a replacement tooltip; a one-frame
guard ends early on the next non-Escape key. No transition or polling is used. Placement observers and
resize/scroll/Escape listeners exist only while shown and are removed on close/unmount.
Long copy wraps within the viewport and is bounded by the remaining vertical space.
The host's visible disabled reason remains outside the tooltip.

Static hosts use `iconAction` around an `action iconActionControl` native button,
`aria-label`, optional controlled `aria-pressed`, and an `iconActionIcon` decorated
with `aria-hidden`. A sibling `iconActionTooltip` with `popover="manual"` and
`aria-hidden` contains `iconActionTooltipLabel`. Hosts retain native activation,
disabled reason association and disposable hover/focus/Escape/placement behavior;
the fixture demonstrates that contract without React. Do not add a native `title`
or describe the same label again through `aria-describedby`.

Command uses `command`, `commandText`, `commandCopy`, `commandFeedback` and `code`
static names. `BrowserCommand` accepts literal `text`, an optional host-owned
`action`, and optional `feedback` announced once as status. The host supplies a
native button with the copy class and retains clipboard/error/lifetime behavior.
Read-only input hosts keep their own labels/refs with the same text class; long
input values scroll internally, while React command text wraps. There is no
clipboard helper, command parser or page-wide code reset in the leaf. See the
[command contract](../gui-components.md#commands-and-inline-code).

Tabs has a named horizontal tablist, native tab buttons and persistent panels.
The host supplies ordered `tabs` (stable `id`, `tabId`, `panelId`, label and optional
state), `selected`, `onSelect(id, reason, originalEvent)` and a panel renderer.
Exactly one supplied ID is selected. Left/Right wrap; Home/End select the first/last
tab automatically and move focus. Enter/Space use native activation. The callback
reason is `keyboard` or `activation`; the host retains trusted-event admission.
Tab enters the selected tab, then the active panel's first control. Panels have
no tab stop by default; the host can opt in with `panelTabIndex={0}` (or use `-1`
for programmatic focus). Inactive panels use `hidden`
but remain mounted, preserving host-owned drafts and control identity. Renderer
source text has its separate product-owned visibility and quote contract.

Optional state takes a host-supplied mark, word and existing tone. The accessible
name includes mark, label and state word; colors alone never convey the state.
`status` is a trailing host-owned slot, without an automatic announcement. The
status wraps below the strip when the component is narrower than 480px. The
strip scrolls horizontally, never wraps or animates; selection and resize keep
the selected tab in view without moving the host page. A label wider than the
available strip truncates visually while its full accessible name remains.
The square panel uses a 1px rule; the selected tab joins it and carries a 2px
top rule in the link color role. No product state, routing or persistence belongs to Tabs.
Static hosts use `tabs`, `tabsHeader`, `tabList`, `tab`, `tabMark`, `tabLabel`,
`tabsStatus`, `tabPanel` classes and supply the same IDs, roles, selected state,
roving tab stop, native activation, panel visibility and disposable resize behavior.

## Fixtures

`/test/fixtures/tabs.html` exercises twelve controlled tabs and retained panel drafts.

From `typescript/`, serve the development fixtures with
`pnpm --filter @tmt/browser-ui --fail-if-no-match exec vp dev --host 127.0.0.1`.
`/test/fixtures/static.html` shows Field/native/contenteditable, toggle and icon states
using checked CSS and a small fixture-only static host. The hover/focus examples
are labeled CSS demonstrations; all icons also support actual hover/keyboard focus.
`/test/fixtures/react.html` runs observable editor/action/tooltip lifecycle assertions
with **Run lifecycle checks**, then retains interactive React controls.
`/test/fixtures/overlays.html` exercises Drawer/Menu focus, retained drafts, iframe/header scrims and bottom-anchor placement.

**Run field focus checks** in the static fixture and the React lifecycle checks
both exercise native and textbox fields: contained focus paint, unchanged geometry
within 1px, a visible contiguous 1px-to-2px edge change, and restoration on blur.
The React checks also measure linked/plain header brands against the same handbook
mark/gap ratios at every viewport, their containment and neutral colour, and the
absence of a title divider.
Use the document's `data-theme` to capture light/dark at 390/1440 and inspect
320px fit. Fixtures use local font fallbacks and no product runtime or remote assets.

Package-local verification after owning workspace integration:

```sh
pnpm --filter @tmt/browser-ui --fail-if-no-match check
pnpm --filter @tmt/browser-ui --fail-if-no-match test
```

## Drawer and Menu

Drawer and Menu render in place; native top-layer promotion keeps ancestor overflow and transforms from clipping them. Keep the mount point visible. Drawer uses native modal containment at every width, without body scroll locking.
Its independently scrolling body uses `overscroll-behavior: contain`. It leaves
8px below the shared header and at the right/bottom edges, 4px through 480px,
including safe-area insets. Regular width is at most 440px; wide is 660px; compact
fills the available width. The opaque square surface has one 1px edge and no
shadow or animation. Its scrim covers the header and iframe: a first outside
press dismisses only, a second press acts on the uncovered control. Browser-only
`scrim` starts at black 12% light / 32% dark.

The host supplies `open`, title/caption, children and `onDismiss(reason, event)`.
It owns busy admission, drafts, attempts and commands. An optional `notice` slot
presents host-owned status and recovery actions below the header. Standard surface
DOM handlers and an optional `overlay` slot let the host present a full-surface drag
target without a second owner inside the drawer body. `keepMounted` retains
children while closed. Escape respects a child's `defaultPrevented`; repeated,
modified or composing keys do not dismiss. Close, backdrop and Escape requests
carry their original events. Focus returns to the connected visible opener, or
the host's supplied fallback when the opener disappears.

Menu uses a native top-layer surface with a transparent catching backdrop and
one open menu per document. It closes before invoking the host command, returning
focus before another dialog opens. Arrows, Home/End and typeahead move among
enabled items; Tab closes. The host owns checked state, command keys, disabled
reasons and menuitem/radio/checkbox semantics. Description content is informational.
Placement uses the visual viewport: below, then above if more room, then constrained
scrolling when neither side fits; right edges align and horizontal placement clamps.
A menu inside a drawer dismisses before that drawer on Escape. Input autocomplete
and anchored annotation windows retain their existing owners.

Optional `onOpenChange(open)` reports actual Menu visibility transitions, without
an initial false. Selection, Tab, Escape, backdrop, native close/cancel,
replacement, anchor removal and unmount close the report once. StrictMode effect
replay does not report a spurious close. This is a presentation signal for host
suspension; it does not transfer command ownership.

## Notifications

`BrowserNotifications` accepts literal host-admitted `events` with stable `id`,
`destination` (`Chat`, `Thread`, `Answer`, `Proposal`), `firstLine`, optional
Thread `quote` and `onActivate(originalEvent)`. It invokes activation once, only
from a trusted native button event. The host owns admission, viewer permissions,
baseline/replay suppression, destination state, navigation and persistent counts.
Changing `pageKey` resets this presentation-only queue and duplicate memory.

FIFO shows three entries, with `N more waiting` for overflow. Each newly visible
entry receives 6000 ms; waiting entries do not age. Hover and focus independently
pause the remaining budget. Dismissal, activation and expiry promote the oldest
waiting event. Unmount cancels timers and listeners.

The host supplies `suspended` for every modal layer (Drawer, Menu, Manage and
attachment dialogs) and `isEligible(event)`. A hidden document uses the same
suspension path. Suspension hides the manual native popover, pauses budgets and
queues arrivals silently. Resume synchronously rechecks **every** retained and
pending entry before any display or announcement; withdrawn entries drop
silently. Hosts must combine all active modal states; closing one of two modals
does not resume presentation. Header counts remain independent.

The top layer sits below the shared header at the right, without a catching
backdrop or autofocus. Arrival preserves focus. Natural Tab reaches controls;
plain Escape in a focused card dismisses that card and consumes its event.
Escape or Dismiss restores the connected, visible focus origin from before entry into the surface; Open leaves focus to the host.
Other owners retain Escape. Opaque square cards use shared roles, a 1px edge,
no shadow and no animation. Visual line clamps retain the complete text in the
accessibility tree. A separate persistent polite status announces each newly
displayed event once in FIFO order; count and timer changes do not reannounce
the interactive stack. Wiring hosts must use one reply announcer, replacing any
existing modal live-region relocation with this suspension contract.

The isolated fake-source fixture is `test/fixtures/notifications.html`. Its
headless Chromium conformance spec uses the pinned Colab browser runner; no
product sources or navigation are wired by the fixture.

## Chips

`BrowserChips` presents a controlled removable list and an Add search popup.
The host supplies `label`, `items` (`id`, `label`, `removeLabel`, optional `disabled`),
filtered `options` (`id`, `label`, optional `description`/`disabled`), `query` and
`onQueryChange`, `onAdd(id, event)` and `onRemove(id, event)`. It also supplies
`addLabel`, `searchLabel`, `emptyText`, optional `maxItems`/`limitText`, `statusText`,
`disabled`, `addDisabled`, and a `focusFallback` when its opener can disappear.
Values, filtering, directory loading, product limits and effects stay host-owned.
Callbacks retain the original event; the host admits trusted actions.
`BrowserChip` is the same item primitive for a host-owned rich editor, with
`label`, `removeLabel`, `disabled`, `buttonRef` and `onRemove(event)`; that host
restores editor focus and owns removal. Enter/Space on the remove control belongs
to that button, not the surrounding editor. Static classes and item/option types live
in the React-free static entry; checked CSS comes from the generator.

Chips have square 1px edges, one-line labels and explicit remove controls. The list
wraps; long names ellipsize without shortening accessible names. Add is disabled
at the host's cap, with its supplied reason. Empty, loading and failure copy is
host-supplied. Colab page-list label tokens use BrowserChip; their Lexical model,
search application and autocomplete retain their owners.

Enter/Space or Up/Down on Add opens a native top-layer surface with a combobox/listbox,
not a menu. Focus
stays in its search field while Up/Down moves the active enabled option. Enter
adds one, closes and returns focus; Escape closes without adding; Tab closes and
continues normal focus order. Empty-field Backspace removes the last chip, when enabled;
text deletion and composition keep normal input behavior. Selected IDs are excluded,
disabled options cannot be added, and modified/default-prevented keys do not act.
A remove action focuses the next/previous surviving remove control, then Add or
the supplied fallback. Menu and Chips share replacement and visual-viewport
placement/cleanup; outside presses dismiss only. Optional `onOpenChange` reports
real transitions only, including native close, replacement, anchor removal and
unmount; StrictMode replay owes no false transition. `test/fixtures/chips.html`
provides isolated keyboard, cap, empty, status and long-name examples.
