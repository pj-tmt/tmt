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
`browserUiClasses` and presentation types without React; `@tmt/browser-ui/shell` exports the
[Shell](../gui-style.md#shell) model (`shellRail`, `describedMenu`, `shellCount`), its reader
`loadShellAnswer` and the framework-free `mountShell` renderer, with Lucide icon data
(`src/shell-icons.ts`, ISC, `src/LICENSE.lucide`); `@tmt/browser-ui/chart` exports the framework-free chart painters and their pure models ([Charts](../gui-style.md#charts)), including opt-in monotone areas and the six-colour `tmt-ui-chart-seats` palette; `@tmt/browser-ui/react`
exports `BrandMark`, `BrandLockup`, `BrowserShell` (with `useShellBar`, the header slot, and `useShellTools`, the bar's search and chip slots), `BrowserHeader` (`inShell` draws no brand and keeps the product in its accessible name; `titleVisible={false}` draws no heading, for a page whose hero is its heading, and keeps `<product> · <title>` as the landmark's name), `BrowserNotice`, `BrowserField`, `BrowserAction`, `BrowserIconAction`,
`BrowserToggle`, `BrowserCheckbox`, `BrowserList`, `BrowserListRow`, `BrowserCommand`, `BrowserDrawer`, `BrowserDialog`, `BrowserPanel`, `BrowserMenu`, `BrowserTabs`, `BrowserNotifications`, `BrowserChips`, `BrowserChip` and their prop types. The static entry also exports `BrowserTab` and `BrowserTabSelectReason` types. The optional peers are React 19.2.8 and
lucide-react 1.52.0. React components use built-in icons for controls such as Drawer
and Notifications dismissal; IconAction also accepts a caller-supplied icon.
Static serving requires neither peer.

`@tmt/browser-ui/shell-host` exports the app router and route slots, `@tmt/browser-ui/theme` the
stored theme choice. Products import the leaf only through these entries, never by a path into
`design/browser-ui`; the test-only `@tmt/browser-ui/test-fixtures/*` (files under
`test/fixtures/`, imported with their extension) and `@tmt/browser-ui/gallery/frames` serve
product tests and stay out of every production `src`. The TypeScript boundary test
(`typescript/test/tooling/browser-ui-boundary.test.ts`) enforces both; reading a generated file's
bytes (`new URL(…)`) is not an import.

The browser color, surface and shared metric roles live in the `browser` group
of `../tokens/tokens.json`. Legacy terminal/soft roles remain separately owned.
Fonts and header metrics use the existing authoritative groups. One owner
projects these values and the ordered CSS fragments:

```sh
node design/browser-ui/scripts/generate-static-css.mjs --write
node design/browser-ui/scripts/generate-static-css.mjs --check
```

The generator assembles the authored fragments and minifies component rules with the pinned
Vite CSS compiler (Lightning CSS, with modern CSS retained). The token projection keeps
its original declarations and spelling. Only that generated distribution
is excluded from formatting; authored CSS remains formatted. Source-level policy tests inspect
the assembled source, byte checks inspect the distribution, and browser comparisons verify
that compilation preserves computed paint and layout. The check compares expected bytes without writing. `test/byte-budget.test.ts` holds the gzip
size (level 9) of `generated/static.css` plus `generated/ui.css` to +15% of the 18,824-byte pre-v2 baseline (21,647 bytes). The GUI lead approved this
budget for the scoped v2 panel, dialog/confirm, ghost/icon action, tinted Notice and shell
metrics work; the original 10% predates that scope. Class encapsulation is retained, and any
further increase returns to the lead; each pull request that changes them also reports the raw sizes. Consumers embed checked CSS;
Cargo and installed products never run the generator. Select light/system-dark
on the document or scope `data-theme="light"`/`data-theme="dark"` on an island.
A dark island on a light page needs dark foregrounds as well as dark surfaces.
Every shared component is opaque and takes the radius scale (`radius-s`, `radius`, `radius-l`, `radius-pill`); only a floating layer (dialog, drawer, menu, popover, tooltip, autocomplete list, notification) takes the one overlay shadow, and resting surfaces stay shadow-free; the product-owned annotation badge/pill exceptions are defined in [browser style](../gui-style.md#popover-composer). Semantic marks retain words.
Links, essential edges, selection indicators and keyboard focus are separate.
Control frames use `--tmt-ui-surface-rule`: action, chip/add/popup, field,
outer toggle, command/copy and icon tooltip frames share the light rule. A control
drawn by its border alone (the toggle indicator, an unchecked checkbox or radio) uses
`--tmt-ui-color-edge`, which keeps 3:1 against the surface behind it (WCAG 1.4.11);
text fields and select triggers keep the soft rule by decision (see
[browser style](../gui-style.md), Fields). Selected indicators and keyboard focus
retain their stronger roles.

A mark's paint servers are prefixed by an id unique per document: `BrandMark` takes it from
`useId`, and `mountShell` asks the live document (`freshMarkId`) so a second shell, such as a
fixture or a hidden earlier mount, never makes the visible rail mark resolve to a hidden copy.

`scripts/generate-brand.mjs` writes the static brand assets from the mark's one definition
(`src/brand-mark-svg.ts`), `src/brand-mark.css` and the mark-only `brand.mark` roles in
`tokens.json`; `pnpm check:generated` fails when they are stale. `generated/favicon.svg` is the
TMT tab icon: the small form of the 04D-1 mark on its dark rounded-square tile, the same in light
and dark browser chrome. Site, Colab and Remote HTML embed data-URI copies; their tests decode
those copies and compare the exact SVG bytes with this file. Updating the icon requires refreshing
these literals; no icon request or runtime generator is involved. `generated/app-icon-light.svg`
and `generated/app-icon-dark.svg` are the 512 px app icons: the full mark on the same dark tile,
in each theme's mark roles. Colab's design-geometry browser test rasterises the favicon at 16, 24
and 32 px and the full mark at 256 px.

## Host metric inputs

The leaf preserves differing product metrics through explicit custom properties;
it does not guess a product profile or replace them with fallback values. The
host defines every property used by its chosen components. Header sizes are
62px/56px below the 760px threshold. Page positioning, body offset and full attribution disclosure remain host-owned; Drawer and Menu own their top-layer geometry and dismissal.

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

The shell and heading metrics follow the concept website: 232px rail, 45px module
rows, 33px sub-items and a 45px brand mark. Body and display text use
`Inter, -apple-system, system-ui, "Segoe UI", sans-serif`, without bundled fonts or
external font requests. Rail sub-item text starts 8px beyond the parent's icon
left edge (Ben direct); its smaller type and separate current-page marker retain hierarchy.

The optional static `page` class (`tmt-ui-page`) applies shared content insets:
33px/36px block/inline, 24px/18px through 760px. Hosts opt in on their content
wrapper and retain width limits, layout and scrolling. Page titles are 37px/720
(29px compact), and card titles are 19px/660; these come from browser metric tokens.

## Controlled input contracts

Header has one h1. Optional `leading`, `headingPrefix` and `headingSuffix` slots place
host-owned attribution before the brand, a separator before the title and a crumb or control
after it; none supplies state or changes the default layout. A suffix turns the heading into
one row in the header's title font: the title shrinks (ellipsis) before the suffix does, and a
`BrowserMenu` with `variant="crumb"` there is a borderless text trigger whose value truncates
with an ellipsis while its chevron stays visible. A menu item may carry `separated` to draw a rule above it. A supplied caption requires a stable `captionId`, retaining
full DOM copy and title association; the host supplies a visible full-caption
read path. `brandLink` preserves a caller's native anchor. `disclosure` selects a
caller-owned data attribute; the leaf does not choose or operate a menu.

The brand is the 04D-1 "Beam / Window" mark (`BrandMark`, `src/brand-mark.tsx` and
`.css`): a dark terminal panel with a `>_` prompt opening into an aurora beam. Every surface
shows this one mark. Its stops carry aurora-light; dark themes switch to the brighter aurora-dark
stops and give the panel a faint edge. Print swaps in a mono beam.
Below 28 px the simplified form drops the perspective and the underscore. `BrandLockup`
adds the `tmt` / `terminal team` words for marketing surfaces.

The header brand renders the small form at the header's mark size, `aria-hidden="true"`.
Mark size and brand gap derive from the wordmark-size token using the handbook's 33/19 and
12/19 ratios (about 24.32px and 8.84px with a 14px label); they stay equal at compact and wide
widths, within the 56px/62px header. The product label and brand link use the normal text
colour. The screen title follows the brand as plain text, without a divider.
`generated/brand-mark-static.svg` is the static-host copy of that header mark; static hosts embed it
(with their own extra classes and gradient ids) and the shared stylesheet styles it. The
header test compares React's output with that copy, ignoring only formatting whitespace and
gradient ids.

Notice tone, visible state label, mark and announcement are explicit. No state,
health, permission or recovery inference occurs. Marks are decorative because
visible state words carry their meaning.
Optional `variant="tinted"` puts waiting, working, review and blocked notices on their existing
soft tone surfaces; muted stays on the sheet. Static hosts set the data-tinted presence attribute
only for tinted notices and omit it for neutral. The default `neutral` keeps the existing markup
and sheet surface. Both retain the full neutral rule and radius: there is no dedicated tone-edge
token. Title, announcement, mark and state words remain independent inputs. Hosts compose
`BrowserCommand` in the body or actions slot and retain copying, feedback and command lifetime.
Tinted captions and state words keep their size and weight but use primary ink for contrast
on every soft surface; decorative marks retain their tone colour.
The gallery's `?frame=notice&theme=light|dark` renders the same Notice examples in isolation
for command focus and contrast checks, without unrelated overlay-demo iframe focus owners.

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

Action takes one `variant`: `primary` (the accent fill, a group's one main action: at most
one enabled primary per group of actions), `secondary` (the neutral bordered button),
`text` (underlined), `ghost` (plain) or `destructive`. Ghost has no resting fill or edge.
Ghost uses secondary ink at rest, primary ink on hover or keyboard focus, the token hover fill
and one inset keyboard-focus frame. It uses an explicit native button type, retained visible label and optional
busy mark. Optional `leadingIcon: ReactNode` uses that same decorative, aria-hidden
1em mark slot. A supplied `busyMark` replaces it on the existing delay/hold timing,
with no extra slot or width shift; without a busyMark the leading icon stays.
IconAction retains its separate required `icon` input and four variants.
Keep the same optional `busyMark` supplied in ready and busy states;
its leading slot remains in flow while visibility changes, with an equal trailing
reservation that centers the label in both states and preserves button geometry
without changing host padding or typography. Without a mark, both
states retain label-only geometry. The mark is delay-gated: busy shorter than
400 ms shows nothing, and a mark once shown stays at least 400 ms after busy ends
(`busy-mark.ts`; `useBusyMark` and its two constants are exported for hosts that show
their own pending mark on the same timing). A page without React uses the React-free
`createBusyMarkGate` (`@tmt/browser-ui/busy-mark`) and `createActionMark` (`@tmt/browser-ui/static`),
which builds the reserved mark slot an Action carries. Busy keeps the control's appearance, focus and readable label:
it sets `aria-busy` and `aria-disabled`, not native `disabled`, and the click
handler fences activation and cancels a busy submit's form submission. `disabled`
keeps the native disabled styling and wins over busy, without a mark.
Every Action variant keeps its idle fill and ink while busy under hover, press or
keyboard focus; the independent focus frame stays visible. Busy does not introduce
native disable or an appearance prop.
Disabled/busy blocks callback delivery. A supplied disabled reason
requires its stable ID and remains visible. `onActivate` receives the original
React MouseEvent from native button click, including keyboard-generated click;
there is no extra key handler or command DTO. Trusted-event admission remains
with the caller. Host links remain host links. By default `label` names both the
visible action and its accessible name. A host may supply `visibleLabel` for
shorter action text; the full `label` must begin with that visible text and remains
the accessible name.
Text actions have a transparent background in ready and disabled states;
disabled text is muted. Enabled hover and keyboard focus keep the text colour on the lighter `surface.hover` background; selection colors mark only the selected or pressed state.
Field focus recolours the same 1px edge in the focus color (`--tmt-ui-focus-width`, the one
focus width every control uses), entirely inside the border box. Native fields and contenteditable controls
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

Checkbox is one drawing, `tmt-ui-checkbox` (`browserUiClasses.checkbox`), on a real
`<input type="checkbox">`: a 1em box with an `edge` border, filled with the accent and a
currentColor tick when checked, a short bar when indeterminate, the shared focus edge (the text
colour's edge on a filled box) and the disabled roles.
Static hosts put the class on their own input. `BrowserCheckbox` renders that input with
native props (`checked`/`defaultChecked`, `onChange`, `disabled`, `name`) plus
`indeterminate`; name it with a `<label>` or `aria-label`. The UI layer's Radix `Checkbox`
uses the same class keyed on its `data-state`, so it needs `static.css` loaded.

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
and a 1px text-colored edge; disabled styling and the busy/disabled event fences take priority.
Busy follows Action's delay-gated mark and focusable `aria-disabled` state.
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

`BrowserListRow` takes an optional `leading` control (a selection checkbox) in its own column before
the title; the compact layout keeps it beside both the title and the metadata.

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
The strip is a segmented control: a 1px rule and the control radius around the tabs, the
selected tab on the selection fill with strong text in the neutral text role. State marks retain
their own roles. `variant="flush"` (default `boxed`) drops the frame and the panel padding for
views of a list that sits on the page: the strip keeps its rule underneath and the selected tab
carries a straight 2px accent underline instead of the fill. `variant="flat"` drops the strip's
frame altogether: every tab is its own pill with a transparent 1px edge, and the selected one takes
the selection fill with strong accent ink and a transparent edge that reserves the focus frame,
for a row of states on a page's header; its opt-in `wrap` (`data-wrap` on the root for static hosts) wraps the strip into
rows instead of scrolling sideways, with the arrow-key order staying the DOM order, and scroll stays
the default. A tab's optional `count` (`99+` above 99) is a small muted number after its label,
drawn in every variant; a selected flat count uses neutral text for contrast on the selection fill.
The accessible name carries it as `Label, 6` before any state word.
No product state, routing or persistence belongs to Tabs.
Static hosts use `tabs`, `tabsHeader`, `tabList`, `tab`, `tabMark`, `tabLabel`, `tabCount`,
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
within 1px, the 1px edge recoloured to the focus role, and restoration on blur.
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

## Gallery

`gallery/` is a dev-only living gallery of the real tokens and components. Run it with
`pnpm --filter @tmt/browser-ui --fail-if-no-match gallery` from `typescript/` (Vite dev server on
`http://127.0.0.1:5183/`, polling watcher; CSS updates in place, a source edit reloads the page). It imports `generated/static.css`,
`generated/ui.css`, `src/react.ts`, `src/shell.ts`, `src/ui` and `../tokens/tokens.json` exactly
as products do; nothing is redrawn. It shows each demo in a light and a dark column scoped with
`data-theme`, and pins hover/focus by mirroring the leaf's own `:hover`/`:focus-visible` rules
onto gallery attributes (`gallery/force-states.ts`). The shell, dialog, drawer, open menu and
notifications render in iframes of the gallery itself (`?frame=<name>&theme=<theme>`), because
they need a viewport or the top layer; the phone shell is a 390px frame. `?frame=charts`
shows the real chart demos without competing overlay frames for keyboard-focus checks. The gallery uses the website's system font stack without font requests and supplies Colab's host metrics.

When mounted as a local preview route, a side outline (wide screens) or a header dropdown
lists the sections and follows the scroll; the section's name is the page heading's suffix.

`vite.gallery.config.ts` serves the standalone gallery. The workspace export
`@tmt/browser-ui/gallery/frames` exposes its frame renderer for local preview hosts; it does not
add a product route. `gallery/covered.ts` maps every value export of
`src/react.ts` to its section; `tsc` and `test/gallery.test.ts` fail when an export is missing
from that map or is not rendered or called in the gallery source.

## Dialog

`BrowserDialog` is a controlled, content-fitting native modal at every width.
It shares its private show/close/dismiss/focus-return lifetime with Drawer and
renders in place, without a portal or body scroll lock. The opaque surface has one 1px
edge, `radius-l` corners and the overlay shadow, and enters and leaves with the base motion
(see Motion). `size="regular"` (default) caps width at 590px; `size="wide"` caps it at
800px. Both retain 16px viewport/safe-area margins; height is
capped by the available viewport. The body scrolls with contained overscroll and
stacks its children with the `space-l` gap (a bare action, icon action or menu keeps
its own width). Optional `actions` render an end-aligned footer row with the
`space-s` gap; Drawer takes the same `actions` prop.

`presentation="confirm"` centers a decorative warning mark, the string title and
body and centers its footer actions. The host supplies a secondary Cancel and
primary archive action; delete-type confirmation uses the existing destructive
action. It does not change admission, command ownership or
the one-overlay rule; the separate `./ui Dialog` API is unchanged.

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

## Non-modal panel

`BrowserPanel` is a controlled `aside` labelled by its string heading. It shares
Drawer's heading, optional position caption and Close control. The host selects
`mode="column" | "card" | "sheet"`; the panel does not infer a product breakpoint.
Column mode leaves grid, reflow, dividers and sticky positioning to the host.
The floating card is 330px wide, 12px from the right and bottom, and starts at the
header token plus 13px. The sheet has 7px side/bottom insets and a 75dvh maximum
height. Floating modes use the radius scale and single overlay shadow.

Opening focuses the heading. This region creates no scrim, focus trap, top-layer
modal or page inertness; keyboard and pointer navigation outside remain available.
An unconsumed, trusted, unmodified, non-repeated Escape **inside** the region or
Close requests `onDismiss('escape' | 'close', event)`. The host admits dismissal by
changing `open`; refusal leaves content and focus intact. Admitted closure or
unmount returns focus to the connected opener, or `focusFallback`, only when
focus is still inside the panel or has fallen to the document body/null. A page
control already holding focus keeps it during a host-driven close. Changing mode
or other props while open does not refocus the heading. Drafts, selection,
breakpoints and coordination with modal drawers remain host-owned.

## Drawer and Menu

Value-menu triggers use a centered ChevronDown icon in the shared icon box. A menu without a value
can pass `triggerText` for a text trigger (the text and a chevron, accessible name `label`) in place
of the icon action: a command that opens a choice rather than a field that holds one. A host
shortcut opens a menu by passing it a new `openRequest` number (a value it mounts with opens
nothing); the menu then behaves as if its trigger had been pressed.

Drawer and Menu render in place; native top-layer promotion keeps ancestor overflow and transforms from clipping them. Keep the mount point visible. Drawer uses native modal containment at every width, without body scroll locking.
Its independently scrolling body uses `overscroll-behavior: contain`. It leaves
8px below the shared header and at the right/bottom edges, 4px through 480px,
including safe-area insets. Regular width is `min(430px, 100vw - 18px)` and through 640px uses
`100vw - 12px`, with safe-area space retained. Wide uses `min(680px, 100vw - 18px)`
at every width, retaining the safe-area allowance. The opaque surface has one 1px edge, `radius-l` corners and the
overlay shadow, and slides from its edge with the base motion (see Motion). Its scrim covers the header and iframe: a first outside
press dismisses only, a second press acts on the uncovered control. Browser-only
`scrim` is `#12332942` in light and `#030A11AE` in dark; modal drawers
and BrowserDialog also blur the page behind by 3px.

Optional `headerActions` renders host-owned controls immediately before Close in DOM
and keyboard order, with shared spacing. Omitted, it adds no element or layout change;
footer `actions` is separate. Header actions retain their own labels, busy/admission
state and effects. The leaf does not infer saving or change dismissal/focus ownership.

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
reasons and menuitem/radio/checkbox semantics. An optional `description` is a non-interactive
first block of the open surface (12px text in the secondary colour, a rule below it) and the menu's accessible description
(`aria-describedby` on the `role="menu"` element). It is not an item and not a focus stop: opening
focuses the first enabled item (the last when the trigger was opened with ArrowUp), or the surface
when none is enabled, and arrows, Home/End and typeahead walk only the enabled items. A menu with
a description but no enabled item still opens; with neither, its trigger is disabled.
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
Other owners retain Escape. Opaque cards use shared roles, a 1px edge, the control radius and
the overlay shadow, and enter and leave with the base motion (see Motion). Visual line clamps retain the complete text in the
accessibility tree. A separate persistent polite status announces each newly
displayed event once in FIFO order and holds only currently eligible text: an
active withdrawal drops its item (clearing an emptied status) and never
re-announces surviving cards. Count and timer changes do not reannounce
the interactive stack. Wiring hosts must use one reply announcer, replacing any
existing modal live-region relocation with this suspension contract.

The isolated fake-source fixture is `test/fixtures/notifications.html`. Its
headless Chromium conformance spec uses the pinned Colab browser runner; no
product sources or navigation are wired by the fixture.

## Chips

`BrowserChips` presents a controlled removable list and an Add search popup.
Each chip's remove control is a square, full-height segment flush with the chip's inner
top, right and bottom edges (a rule on its left); its hover/focus fill meets those edges
and focus recolours its own border. The UI layer's `Chip` draws the same segment.
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

Chips have rounded 1px edges, one-line labels and explicit remove controls. The list
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

## Motion

Two durations (`fast` 120ms, `base` 180ms), two easings (`ease-out` entering, `ease-in` leaving),
the pop `scale` (0.96) and the `rise` distance (6px) are the `browser.motion` tokens; under
`prefers-reduced-motion` every duration is 0ms, so a change is instant. A native overlay
(dialog, drawer, menu, chips popup, notification) stays mounted with `data-state="closed"`
until its exit has played: `usePresence(open, ref)` (`src/presence.ts`) holds the node through
the exit transition or animation and releases it when `getAnimations()` settles. Native
dialogs and drawers animate with `transition ... allow-discrete` and `@starting-style`; the UI
layer's Radix layers use the named `animate-pop-*`, `animate-fade-*` and `animate-slide-*`
classes over the `tmt-foundation-*` keyframes, with the animated content as the portal's direct
child so Radix Presence can play the exit. A leaving layer (`data-state="closed"`) takes no pointer input, and the UI layer's `admitLayerDismiss` hands its Escape to the open hosted layer under it (the registry in `useLayerController`), so a second Escape inside the exit reaches the dialog at once; a modal Radix layer keeps Radix's body pointer lock until its exit has played. Hover and press on actions and tabs, the menu row tint
and the rail slide use the same tokens. `design/gui-style.md` owns the rule.

## Shared pieces

Each piece has static classes in `browserUiClasses`, a gallery section and, where it carries
state, a React component.

- **Count chip.** `BrowserCountChip` is a button with a label, an optional boxed count (`99+`
  above 99) and an optional leading icon; `tone="attention"` is for work that waits on the
  reader. `BrowserMenu` with `variant="count"` (plus `triggerText`, `count`, `tone`, `icon`) is
  the same chip as a menu trigger with a chevron; the count joins its accessible name. The
  shell's rail counts use the same count box, and the collapsed rail shows it as a corner badge.
- **Avatar.** `BrowserAvatar` draws the initials of a name or handle (`avatarInitials`) in a
  round mark, size `m` or `s`; it is an image with the name as its label unless `decorative`.
  `kind="person"` (the default) is the neutral fill; `kind="agent"` is the mint selection fill
  with the strong accent ink (`data-kind`), so the two read apart without a glyph.
- **Choice rows.** `BrowserChoice` is a radio or check row with a label, an optional sublabel
  and an optional `hint`; the whole row is the target and the control is a real input
  (`tmt-ui-radio`, `tmt-ui-checkbox`). The hint is a short key at the end of the row (the digit
  that picks it), drawn as a key cap in a `kbd` and hidden from assistive technology. The hint
  sits inside the label, so a row with a hint names its input by the label text
  (`aria-label`) and the key never joins the name. `BrowserChoiceGroup` is the fieldset with its
  legend, `stack` or `inline`.
- **Pager.** `BrowserPager` is `‹ 1/3 ›`: the host owns the 1-based position; the arrows stop at
  the ends; the position is a polite status named `<label> 1 of 3`.
- **Status tile.** `BrowserStatusTiles` is a description list of `BrowserStatusTile` terms: a
  small label over a dot and a value. The dot's tone (`working`, `info`, `waiting`, `review`,
  `blocked`, `muted`) is a hint; the value text carries the meaning. `numeric` sets a count
  large in the display face (about 24px, `data-numeric`), muted at `0`, so a row of tiles keeps its
  shape. Static hosts set `data-numeric` (`count` or `zero`) on the value.
- **State mark.** `tmt-ui-state-mark` is a mark drawn in one 12px box, chosen by `data-shape`
  (`dot`, `ring`, `dashed`, `clock`, `diamond`, `cross`, `unknown`), in the colour of the text around it
  (`data-tone` of `working`, `waiting`, `blocked` or `muted` sets it where that text is not the state's). Every
  mark has the same footprint whatever font a glyph would fall back to; the state word beside it
  carries the meaning, so the mark is `aria-hidden`.
- **Bare status tiles.** `data-bare` on `tmt-ui-status-tiles` drops each tile's border, fill and
  padding for use inside a card: a label over a value, no box in a box. The dot appears only when a
  tile carries a tone. `data-size='hero'` on a numeric value sets it at the page's first-read size
  (about 40px).
- **Card.** `tmt-ui-card` is a flat sheet that groups one subject: `tmt-ui-card-head` (the
  `tmt-ui-card-title` and an optional `tmt-ui-card-meta`) over the body. Static only.
- **Meter.** `tmt-ui-meter` is a read-only measure; its fill's width is `--tmt-ui-meter-fill`
  (0-100) and its colour comes from a chart series class (`tmt-ui-chart-sN`). Give it `role="img"`
  and a label that states the value.
- **Glyph.** `createIcon(name)` (from `@tmt/browser-ui/glyph`, outside the static graph because it reads the
  shell's icon data) returns a `tmt-ui-glyph`: a line icon from the shell's icon set,
  drawn in the colour of its text and hidden from assistive tech.
- **Page head.** `tmt-ui-page-head` holds a `tmt-ui-page-title` (the display face, the page's
  first read) over a `tmt-ui-page-subtitle`. Static only.
- **Stats strip.** `tmt-ui-stats` is one bordered strip of `tmt-ui-stat` cells: a mark or glyph, a
  `tmt-ui-stat-value` (a `tmt-ui-stat-number` in the display face, then its word) and a
  `tmt-ui-stat-caption`. A cell's `data-tone` (`working`, `waiting`, `blocked`, `muted`) colours its
  mark. Cells grow to fill their row, so a wrapped strip has no empty cell.
- **Banner.** `tmt-ui-banner` (`data-tone='waiting'` for what needs the reader) is a tinted band: a
  `tmt-ui-banner-head` (glyph, title, one quiet action) over one `tmt-ui-banner-row` button per item
  (mark, name, state word, detail, note, chevron). Under the compact width a row keeps the mark, name
  and state, with the detail on a second line. Static only.
- **Avatar stack.** `tmt-ui-avatar-stack` overlaps `tmt-ui-avatar`s in one row, for the people of a
  group; decorative, so `aria-hidden` with the names written beside it.
- **Glyph tile.** `tmt-ui-glyph-tile` holds one glyph in a small square tile, the mark of a card with
  one idea.
- **Read-only badge.** A `tmt-ui-chip` whose first child is a `tmt-ui-glyph` (a lock) draws the glyph
  before its label, for a passive badge such as Read only.
- **Recorded value.** `tmt-ui-table-mono` sets a table cell in the mono face, unwrapped, for a value
  shown as typed, such as a cron schedule; `tmt-ui-table-quiet` mutes an identifier beside it.
- **Copy with an icon.** A `tmt-ui-glyph` inside `tmt-ui-command-copy` sits before its label.
- **Search field.** `tmt-ui-search-field` wraps a `tmt-ui-field-control` (`type=search`) with a
  magnifier glyph at its start. Static only.
- **Facts.** `tmt-ui-facts` is a description list of short terms over values, for the properties of
  one selected thing. Static only.
- **Grouped, selectable table.** Inside a `tmt-ui-table-scroll` (the table scrolls there, never the
  page), `data-roomy` on the table gives it a minimum width and roomy cells; `tmt-ui-table-group`
  rows hold a `tmt-ui-table-group-toggle` button (`aria-expanded`, the chevron turns) and a
  `tmt-ui-table-group-count`; `data-selected` on a row draws a tint and a full rounded frame in the selection tone (a roomy table
  keeps separate borders so its cells can round the frame's corners); `tmt-ui-table-select` is the button
  in a row's first cell that selects it (`aria-pressed`). A card with `data-flush` runs the table
  edge to edge. Under the compact width a roomy table has no minimum width: a `data-compact-hide`
  header or cell is dropped and a `tmt-ui-table-compact-meta` line under the row's main cell
  carries its facts, so the table never scrolls sideways. A state chip is a `tmt-ui-chip` with `data-tone` (`working`, `waiting`, `blocked`,
  `review`, or none for neutral): a mark and its word on the state's soft surface.
- **Passive chip.** A `tmt-ui-chip` whose label is its only child carries no remove control and is
  padded the same on both sides: a label for a state that is not an action.

## Isolated shadcn foundation (Slice1)

`@tmt/browser-ui/ui` is a separate React presentation entry; import its checked
`@tmt/browser-ui/ui.css` explicitly. `cn`, `UiTheme` and `useUiPortalContainer`
provide its foundation; the allocated primitive exports are described below.
Layers exports Dialog, Sheet, DropdownMenu, Popover and stock Tooltip adapters; selection exports the Command family, Combobox, Chip and Chips (see [Controlled selection](#controlled-selection-slice4)). No consumer migration or candidate-5
page replica is included. The existing `./react`, `./static` and `./static.css` entries stay
unchanged; static never loads React, Radix, cmdk or the new UI graph.

The [browser component contracts](../gui-components.md) retain generation and
consumer ownership. Component qualification does not grant product authority.

### Provenance and dependencies

[`src/ui/provenance.json`](src/ui/provenance.json) records every selected source's
upstream path and SHA-256 at shadcn/ui commit
`8720dec73f5aebed9f649ea58636f54599fdedf1`, plus exact direct dependency versions,
licenses and repositories. Primitives adapt nine selected sources; layers adapt
the five pinned Dialog, Sheet, DropdownMenu, Tooltip and Popover sources; selection adapts the pinned Command source, and the Select pin stays excluded pending #3057. The helper is adapted from that revision's
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
admitted. [Controlled selection](#controlled-selection-slice4) owns their focus,
Tab-commit and composition behavior. React and
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
nonprefixed literal classes, stock rounded, shadow and ring utilities. A radius is
one of the named token utilities (`rounded-control`, `rounded-small`,
`rounded-surface`, `rounded-pill`, from `browser.metric.radius*`); the one shadow
is `shadow-overlay` (`browser.shadow.overlay`) and belongs on floating layers only
(dialog, drawer, menu, popover, tooltip, autocomplete list), never on a resting
surface. Layer enter and exit use token-backed `animate-[tmt-foundation-…]`
utilities over the generated `tmt-foundation-*` keyframes, which are the one
global registration the output may carry; durations are the `browser.motion` tokens
(0ms under `prefers-reduced-motion`). Focus uses `outline` and the token-backed
focus width/color, rather than a box-shadow ring.

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
There is no second palette. Resting UI surfaces stay shadow-free; only a floating
layer takes `shadow-overlay`. Later
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
The floating contents (DropdownMenu and its submenu, Popover, Tooltip) default
`collisionPadding` to that gutter, 8 px (`layerCollisionPadding`), so no layer touches the
viewport edge; a caller may pass its own `collisionPadding`.
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
an opener from whichever unrelated control happened to have focus. The
`focusFallback` takes focus only when it is lost: a nested layer that outlasts its parent's
close finds focus already returned to the parent's target and leaves it there. Radix owns
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
Screenshots are per-run evidence, not a replacement for observable assertions.

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

After an admitted removal, Chips focuses the first candidate that actually takes
focus: the next, then previous, enabled surviving remove control, then its input,
then the host `focusFallback`. A candidate that the same host update disabled, hid
or made inert is skipped, and native `focus()` keeps its normal scrolling.

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

## Browser audits

From `typescript/`, install the frozen leaf tools with
`pnpm --filter @tmt/browser-ui... install --frozen-lockfile --ignore-scripts`, then run
`pnpm --filter @tmt/browser-ui test:browser --workers=2`. The Playwright config starts
only the private gallery, on port 5184 by default (`BROWSER_UI_TEST_PORT` overrides it).
It never attaches to an existing server or opens a product/host fixture.

The Chromium suite measures rounded borders, text inset, control radii, focus frames,
border-only control contrast and the named soft-edge field exception, plus overlay exit
and reduced motion. Seeded controls prove each audit detects a violation and accepts its
repair. CI selection and gate ownership live in the
[CI guide](../../.agents/skills/tmt-release/references/ci-selection.md#worker-gates).
