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
`BrowserToggle`, `BrowserList`, `BrowserListRow`, `BrowserCommand` and their prop types. The optional peers are React 19.2.8 and
lucide-react 1.52.0. Icons are caller-supplied; static serving requires neither.

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
56px/48px below the existing 479px threshold. Page positioning, safe areas,
z-index, body offset, scrolling and full attribution disclosure remain host-owned.

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
No native select, listbox, page geometry or new font loading enters this leaf.

## Controlled input contracts

Header has one h1. A supplied caption requires a stable `captionId`, retaining
full DOM copy and title association; the host supplies a visible full-caption
read path. `brandLink` preserves a caller's native anchor. `disclosure` selects a
caller-owned data attribute; the leaf does not choose or operate a menu.

The header brand uses the six-blade aperture copied from
`site/src/home/assets/v9-0.svg`, rendered inline with the original `0 0 200 200`
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
with the caller. Host links remain host links.
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
the visual tooltip copy; icon and tooltip are `aria-hidden`, so the name is
announced once. `pressed` is omitted when absent; supplied false/true becomes
`aria-pressed`, and activation never changes it. Pressed uses selection colors
and a 1px text-colored edge; disabled/busy styling and event fences take priority.
The icon and busy mark share one fixed square target without changing geometry.
Disclosure actions instead supply controlled `expanded` and optional `controls`,
forwarded as `aria-expanded`/`aria-controls` on the same button. The type contract
requires `expanded` with `controls` and excludes `pressed` from disclosure actions.
Expanded uses the same selection treatment as pressed. While expanded, the tooltip
stays hidden and installs no Escape listener; the host owns menu dismissal.

The tooltip is a manual native popover, below the button and bounded by the visual
viewport. Hover and keyboard `:focus-visible` show it immediately. Its transparent
top padding bridges the button-to-label gap, so moving into the tooltip retains
it. Pointer/focus departure closes it; Escape dismisses it and stops propagation
only while it is open. No transition or polling is used. Placement observers and
resize/scroll/Escape listeners exist only while shown and are removed on close/unmount.
Long copy wraps within the viewport; remaining vertical space bounds scrolling.
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

## Fixtures

From `typescript/`, serve the development fixtures with
`pnpm --filter @tmt/browser-ui --fail-if-no-match exec vp dev --host 127.0.0.1`.
`/test/fixtures/static.html` shows Field/native/contenteditable, toggle and icon states
using checked CSS and a small fixture-only static host. The hover/focus examples
are labeled CSS demonstrations; all icons also support actual hover/keyboard focus.
`/test/fixtures/react.html` runs observable editor/action/tooltip lifecycle assertions
with **Run lifecycle checks**, then retains interactive React controls.
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
