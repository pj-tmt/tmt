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
`BrowserToggle`, `BrowserList`, `BrowserListRow`, `BrowserCommand`, `BrowserDrawer`, `BrowserDialog`, `BrowserMenu`, `BrowserTabs`, `BrowserNotifications`, `BrowserChips`, `BrowserChip` and their prop types. The static entry also exports `BrowserTab` and `BrowserTabSelectReason` types. The optional peers are React 19.2.8 and
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
Every shared component is opaque, square and shadow-free; the product-owned annotation badge/pill exceptions are defined in [browser style](../gui-style.md#popover-composer). Semantic marks retain words.
Links, essential edges, selection indicators and keyboard focus are separate.
Control frames use `--tmt-ui-surface-rule`: action, chip/add/popup, field,
outer toggle, command/copy and icon tooltip frames share the light rule. The
toggle indicator keeps `--tmt-ui-color-edge`; selected indicators and keyboard
focus retain their stronger roles. These frame choices do not change token values.

`src/favicon.svg` owns the light/dark TMT tab icon. Site, Colab and Remote HTML
embed data-URI copies; their tests decode those copies and compare the exact SVG
bytes with this source. Updating the icon requires refreshing these literals;
no icon request or runtime generator is involved.

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
The label is text or a host node; an optional `ariaLabel` is the accessible name when the
visible label is a node or abbreviates a longer name.
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
Keyboard-only `:focus-visible` uses a flush 1px focus-token ring; pointer activation
adds no focus ring. Header action groups and a nested navigation landmark share
the host action gap, leaving space between neighboring hover backgrounds.
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

Only one icon tooltip is visible per document: opening help dismisses the prior
help synchronously, including when keyboard focus remains on its action. This
ephemeral replacement owner is independent of menu/dialog ownership.
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
top rule and strong text in the neutral text role. State marks retain their own roles.
No product state, routing or persistence belongs to Tabs.
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

## Dialog

`BrowserDialog` is a controlled, content-fitting native modal at every width.
It shares its private show/close/dismiss/focus-return lifetime with Drawer and
renders in place, without a portal or body scroll lock. The opaque square surface
has one 1px edge and no shadow or animation. Its width is at most 600px, with
16px viewport/safe-area margins, reduced to 4px through 480px; its height is
capped by the available viewport. The body scrolls with contained overscroll.

The host supplies `open`, title, children and `onDismiss(reason, event)`.
Children may consume Escape for a suggestion level before the dialog sees it.
Unconsumed, unmodified, non-repeated Escape, native close, the Close action and a
backdrop press synchronize the host's controlled open state. The scrim catches
header/iframe presses without click-through. Optional `initialFocus` selects a
control inside the dialog; the connected opener or `focusFallback` receives focus
on dismissal. Navigation may set `restoreFocus={false}` and explicitly focus its
new destination instead. `keepMounted` retains host content while closed; it is
false by default. Values, admission, loading, listbox semantics and navigation
remain host-owned.

## Drawer and Menu

Value-menu triggers use a centered ChevronDown icon in the shared icon box.

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

A chip or option may carry an optional `content` node that the host draws instead of the
label text, such as an agent mark and name. The required string `label` stays the fallback
text when `content` is absent and the accessible name of an option (`label`, plus its
description when it has one); a chip's remove control keeps its own `removeLabel`.
`content` never changes ordering, filtering, caps or events; the host owns what it draws.

## Isolated shadcn foundation (Slice1)

`@tmt/browser-ui/ui` is a separate React presentation entry; import its checked
`@tmt/browser-ui/ui.css` explicitly. `cn`, `UiTheme` and `useUiPortalContainer`
provide its foundation; the allocated primitive exports are described below.
Layers exports Dialog, Sheet, DropdownMenu, Popover and stock Tooltip adapters; the selection barrel remains empty. No consumer migration or candidate-5
page replica is included. The existing `./react`, `./static` and `./static.css` entries stay
unchanged; static never loads React, Radix, cmdk or the new UI graph.

The [browser component contracts](../gui-components.md) retain generation and
consumer ownership. Component qualification does not grant product authority.

### Provenance and dependencies

[`src/ui/provenance.json`](src/ui/provenance.json) records every selected source's
upstream path and SHA-256 at shadcn/ui commit
`8720dec73f5aebed9f649ea58636f54599fdedf1`, plus exact direct dependency versions,
licenses and repositories. Primitives adapt nine selected sources; layers adapt
the five pinned Dialog, Sheet, DropdownMenu, Tooltip and Popover sources. Selection sources remain pins for a later slice. The helper is adapted from that revision's
`apps/v4/registry/new-york-v4/lib/utils.ts` to configure `extendTailwindMerge` with
our `tmt` prefix. The upstream MIT notice is retained in
[`src/ui/LICENSE.shadcn`](src/ui/LICENSE.shadcn). This revision predates upstream's
separate `cn` dependency; adding that package is outside the accepted dependency
set. `class-variance-authority@0.7.1` is Apache-2.0, explicitly accepted for the
first slice needing variants. Button, Badge, Alert and TabsList use genuine CVA;
its exact pin, [retained Apache license](src/ui/LICENSE.cva) and dependency guard admission are reviewed with
the manifest and lockfile. npm tarball integrity and all transitive versions live in the sole workspace
`typescript/pnpm-lock.yaml`; checked provenance must agree with the manifest.

Combobox qualification uses the pinned Radix Popover + Command/cmdk composition.
Command imports Dialog; Layers imports Button. The current upstream Base UI
combobox registry is deliberately not selected; no `@base-ui/react` dependency is
admitted. Later slices must prove focus, Tab commit and IME with those real
implementations. This foundation does not make those claims. React and
lucide-react remain optional peers for the existing leaf entries. The Tailwind
compile API is a development dependency confined to this leaf; no CLI package is
required. No Cargo build or native
installation requires Node or generates UI CSS.

### Local theme, CSS and portal contract

Wrap each new UI island in `UiTheme theme="light" | "dark" | "system"` and load
UI CSS once. `data-tmt-ui` owns the namespace; `data-tmt-theme` is independent of
legacy `data-theme` and never changes document/body state. Each island resets its
own variables, so explicit nested themes and nested system preference are
independent of the surrounding island. Only complete `tmt:` utility literals are
compiled; dynamically constructed utilities are unsupported. `cn` merges the
same fixed Tailwind v4 prefix while preserving variants.

[`scripts/generate-ui-css.mjs`](scripts/generate-ui-css.mjs) compiles only utilities,
with no default Tailwind theme, preflight, global reset, global theme variables or
property registrations. The checked output uses native CSS nesting under the
island selector. Tailwind's internal non-inheriting defaults become namespaced
local element/pseudo-element resets inside islands; they cannot affect host or
legacy styles. The island wrapper is a namespace, so place utilities on its
children. Animation/interpolation of registered custom properties is not
qualified here. Source policy rejects raw colors, stock color palettes,
nonprefixed literal classes, rounded, shadow and ring utilities. Focus uses
`outline` and the token-backed focus width/color, rather than a box-shadow ring.

The sole theme source is `design/tokens/tokens.json`: semantic shadcn color names
alias existing `browser.color`/`browser.surface` roles. `layer-boundary`, generic
`border` and `input` project the light `browser.surface.rule` frame, including
outline/secondary Buttons and native fields. Overlay content uses the named layer
boundary; Tooltip retains stock background/text/arrow with the same thin frame.
`ring` remains `browser.color.focus`; the separate `indicator` alias preserves
`browser.color.edge` for the unchecked Checkbox frame and Switch track. Checked
and invalid state roles retain their own colors; light frames do not weaken
focus or state indicators. Focus width, rule width and radius alias current metrics. New spacing and layer/tooltip z-order
live only in `browser.ui`, which the old static generator does not enumerate.
There is no second palette. UI surfaces remain square and shadow-free. Later
components must use these roles for actual styles, not merely expose unused
variables. Token/CSS changes require UX and affected shared-head acceptance.

`useUiPortalContainer()` returns the originating island's dedicated descendant
container. Pass it to the real Radix `Portal.container` and render only after it
is non-null; never omit the container or fall back to `document.body`. Portals
then inherit that island's namespace, live theme and nesting without copying
variables or registering a global popup owner. Missing `UiTheme` throws; server
rendering has no body portal. Radix owns modal containment and its dismissable-layer stack. Host close admission
is carried by the layer adapters described below. A layer unable to use this container returns to the
plan rather than introducing a global theme synchronizer.

### Controlled layers (Slice3)

Dialog and Sheet use Radix Dialog for modality. Popover is nonmodal by default;
DropdownMenu retains Radix keyboard navigation, disabled items and submenus.
Tooltip retains stock Radix hover/focus timing and Escape behavior; it has no
busy-veto controller. All portals use their originating `UiTheme` container.
The scrim aliases the existing `browser.surface.scrim` token; layer dimensions
follow the visual viewport with a gutter derived from existing spacing.
No second popup stack or domain operation lives in this leaf.

Actionable hosts that require close veto must supply a controlled
`useLayerController({ open, onOpenChange, canClose })` as the root's `controller`.
Return `false` from `canClose` to hold a supported dismissal. The controller
passes a reason and the original event when available; all host programmatic
close paths must call `controller.requestClose()`. Raw external `open=false` or
unmount remains authoritative and bypasses this admission. Without a controller,
the roots retain stock Radix state behavior and make no close-veto promise.
Do not perform Ask, upload, request or other domain effects inside the adapters.

Dialog/Sheet initially focus their title, or a usable explicit `initialFocus`
target. `returnFocus` can name a persistent target; `focusFallback` must name a
usable surviving control if the opener may disappear. Radix retains focus
return to a menu trigger by default. Supply both `returnFocus` and `focusFallback`
on menu content when that trigger can disappear; pointer entry does not infer
an opener from whichever unrelated control happened to have focus. Radix owns
containment for modal surfaces. Host autofocus handlers run first and can consume
the event. Pinned Radix Popover is nonmodal by default: outside pointer or focus can leave
and dismiss it, but Tab/Shift+Tab loop within its open content. Escape closes
only that Popover and restores its trigger. In a menu submenu, ArrowLeft closes
only the submenu and returns to its parent item; Escape closes the whole menu
and returns to its trigger. These are the pinned stock semantics, not a second
focus navigator.

For menu-to-Dialog/Sheet transfer, create one `useMenuLayerHandoff({ menu, layer,
trigger })` with the controlled menu, destination controller and persistent menu
trigger getter. Connect `onSelect` to the menu item, `onCloseAutoFocus` to the
menu content, and `returnFocus` to the destination content. Keep the destination
root outside the transient menu content. A vetoed menu close does not open the
destination. An admitted selection closes the menu once, suppresses its competing
close autofocus and opens the destination; destination dismissal returns to the
persistent menu trigger, then its supplied fallback if unavailable. These hooks
carry host admission and focus targets; Radix owns surface lifetime and stacking.

### Slice file ownership and checks

| Owner after foundation merge | Exclusive source/barrel                                                                                          | Exclusive gallery fragment                                      |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| Slice2 Primitives            | `src/ui/{button,input,textarea,checkbox,switch,separator,badge,alert,tabs}.tsx`, `src/ui/primitives.ts`          | `test/fixtures/ui/primitives.tsx`                               |
| Slice3 Layers                | `src/ui/{dialog,sheet,dropdown-menu,tooltip,popover}.tsx`, `src/ui/{layer,layers}.ts`                            | `test/fixtures/ui/layers.tsx`                                   |
| Slice4 Selection             | `src/ui/{command,combobox,chips}.tsx`, `src/ui/selection.ts`                                                     | `test/fixtures/ui/selection.tsx`                                |
| Foundation/integrator        | `src/ui/index.ts`, `src/ui/{utils.ts,theme.tsx}`, provenance, generator/policy, package/lock, `generated/ui.css` | `test/fixtures/ui/foundation.tsx`, browser fixture wrapper/spec |

Do not edit sibling barrels or fixture fragments. One UX-designated integrator
publishes generated UI CSS and provenance updates after integrating reviewed
slice sources; members hand off their source/fixture changes and generation
results. Component implementation waits for this foundation's accepted merge.
Package/workspace/lock publication has one active issue owner and transfers only
through an explicitly accepted handoff naming its delivered baseline. UI-output
ownership likewise transfers at an accepted merged commit. Staffing is assigned
by the lead, not implied by this file map.

`pnpm --filter @tmt/browser-ui generate` regenerates both checked stylesheets;
`check:generated` verifies exact bytes. Run workspace commands from `typescript/`
(the leaf is a sibling workspace member). Leaf tests include baseline SHA/source
and static CSS hashes, prefix merging, theme/container admission and CSS/token
policy. The shared tooling boundary guard admits new dependencies only inside
`src/ui`, with no alias/static/legacy escape. Existing Colab Playwright config
serves `/test/browser-ui-foundation.html`; its infrastructure-only gallery checks
host/legacy computed styles before and after loading UI CSS, tokens, focus and
originating portals in light/system-dark and nested themes at 320/390/1440.
Screenshots are per-run evidence, not a replacement for observable assertions or
qualification of the unimplemented later components.

### Primitive contract (Slice2)

`primitives.ts` exports Button/buttonVariants, Input, Textarea, Checkbox, Switch,
Separator, Badge/badgeVariants, Alert/AlertTitle/AlertDescription,
Tabs/TabsList/TabsTrigger/TabsContent/tabsListVariants. Normal native and
Radix props, refs, default values and controlled callbacks remain with the host.
These components contain no product store, command or request state.

Button retains the upstream variants and sizes. Its native busy form requires
`busyLabel`, renders that visible state word, sets `aria-busy`/`aria-disabled`
and disables activation. Busy `asChild` is rejected; links retain their own owner.
Disabled `asChild` exposes `aria-disabled` and blocks ordinary click and activation
keys, including the slotted target's capture callbacks before Slot composition. Native fields retain readOnly/disabled/value and ARIA props; a busy host
uses readOnly plus `aria-busy` and a visible, associated state description.
Checkbox/Switch `readOnly` blocks value-changing activation while retaining focus.
Hosts supply visible names/reasons for disabled, busy and read-only states.

Select is excluded from Stage1 exports and its fixture. Its stock Popper and
item-aligned modes did not satisfy the required after-open visual viewport
containment contract. [Select qualification #3057](https://github.com/pj-tmt/tmt/issues/3057)
requires an owner-reviewed minimal adaptation design before implementation.
Existing product controls remain unchanged; no consumer adoption is included.
Parking Select is not an A5 pass, and no physical mobile keyboard animation is
qualified by this slice.

The allocated primitive gallery and existing-runner
`browser-ui-primitives.spec.ts` cover real values, activation, keyboard focus,
tokens, width containment and state captures. The separate foundation coexistence spec and legacy
source/static hash guards remain required. CSS adds aliases of existing
command-size/caption-size/eyebrow-weight metrics, never a second token palette.

### Controlled selection (Slice4)

`selection.ts` exports the pinned Command family, Combobox, Chip and Chips with
selection prop/event types. Combobox owns its `SelectionOption` primitive props;
Chips owns `SelectionItem` from its displayed/removable fields. Existing Browser
chip values remain structurally assignable, with no UI-to-static source import.
CommandItem retains cmdk's stock value-only
`onSelect(value)` callback; it does not promise an originating keyboard event.
The composed Combobox/Chips callbacks instead receive the actual React event
from the input key handler or item's click capture. Do not add a second host
mutation callback to the underlying cmdk item, cache a last event or synthesize
activation to recover an origin. Cmdk owns active-option navigation and
active-descendant identity; no mirrored active index is maintained.

The host controls open state through the delivered layer controller, options,
selected IDs, query, filtering and all domain caps/dispatch. Supply stable option
IDs and host-filtered options; the composed pattern disables duplicate cmdk
filtering. Busy/read-only/disabled states require a visible `stateLabel`; unknown
operation state is held by the host's busy/admission state with its own words.
The input remains mounted when its originating-island option portal closes.

Enter commits once with its original event after close admission. Combobox Tab
commits once on admitted close by default (`commitOnTab=true`), independently
of `outsidePress`. Chips owns `commitOnTab=false` and omits that prop from
`ChipsProps`, so its Tab closes without committing. Native Tab
traversal remains native; an admitted Tab close suppresses competing close
return-focus only for that transition. A newer admitted focus move also wins
over deferred popup close autofocus, including chip removal. A veto holds open state and makes no
selection callback. Composition/code229, modified/prevented/untrusted keys and
removed/disabled options never commit. Space in the editable query remains text.
No selection/removal key dispatches an enclosing product Send operation.

Combobox outside pointer dismissal allows the underlying action through. Chips
uses the UX-approved optional `PopoverContent outsidePress='dismiss-only'`
shield, while ordinary Combobox and Popover retain default `'through'`. The
transparent, nonfocusable, aria-hidden shield is owned by the layer, shares its
originating portal and lifetime, and always consumes the first pointer press,
even when host close admission vetoes it. The persistent external input remains
usable above the shield with token-backed stacking only while that popup is open;
closed anchors retain no overlay stacking. It is neither hidden nor
focus-trapped by a modal substitute. No document click interceptor or second
popup owner is involved. Escape follows the
accepted layer contract. The host supplies persistent return/fallback targets
when controls may disappear. Chip removal targets a surviving next/previous
enabled remove control, then its input or explicit fallback. Empty-query
Backspace removes only the last eligible supplied chip; nonempty editing is
native. Caps are supplied through eligible options and state words, never
inferred as a recipient/parser rule in this leaf.

While Chips is open, its persistent input owns one `visualViewport.resize`
listener. On the next animation frame, only an actually focused input outside
the real visual viewport is revealed with nearest-block/inline `scrollIntoView`.
Close/unmount removes the listener and cancels pending work. This event-driven
reveal neither changes popup placement nor mutates focus, query or close state;
ordinary Combobox does not add it.

The allocated selection gallery/unit/spec own selection proofs, including
original-event identity, exactly-once callbacks, blocked/veto cases, focus and
viewport/theme coverage. Only `browser-ui-selection.spec.ts` is allocated in
Colab; existing foundation HTML/runner remain its infrastructure. Added shard
wall time and exact-head Colab acceptance accompany delivery. No consumer
migration or Select revival is included.
