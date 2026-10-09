# Browser component contracts

The [browser style](gui-style.md) owns visual and interaction rules. This document
records the current consumers, extraction boundaries and adoption evidence for the
shared browser components. It does not approve a dependency, add tokens, or claim
that a package or consumer migration has shipped.

## Consumers and delivery boundary

| Consumer                                | Current rendering                                  | Adoption scope                                                                                                  |
| --------------------------------------- | -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Colab app and reader                    | React, app-owned components and styles             | Shared chrome, cards, controls, choice/action menus, conversation presentation, and overlay/composer mechanics. |
| Colab native guidance and recovery      | Rust HTML, embedded styles, a small recovery entry | The same token projection, header, notice and action styles, including when no app build is available.          |
| Remote pairing, landing and error pages | Rust-served static HTML/CSS and browser SDK        | Shared tokens, header, state card, fields and actions; no React runtime is required.                            |
| Remote settings and devices             | Static browser management page                     | Shared chrome, controls and inline command treatment; authority and store remain Remote-owned.                  |
| Office                                  | Frozen; independent browser implementation         | Static command/reference and inline-code presentation only; no revival or other UI migration.                   |

Colab and Remote are separate released products. Consumer adoption requires
the owning lead's code review and UX review. An
internal package is embedded into those products; it is not a separately published
npm product. Completion of the normalization milestone requires actual consumer
adoption and publication evidence, not only this guide or a package build.

The package owns presentation, focus, keyboard navigation, disclosure and disposable
placement. Consumers supply admitted content, labels and action callbacks. They
retain routing, page selection, persistence, source/renderer capabilities, device
sessions, operation IDs, Ask dispatch, status folding and notification ledgers.
No package import may reach a product router, store, SDK operation helper or author
frame. UI mounting, closing or theme changes cannot send, resolve, revoke or retry.

## Current inventory

Paths below are relative to the repository root. "Shipped" describes current main,
not adoption of a shared package. Approved targets remain distinct from observed
behavior and extraction work.

| Component                        | Shipped source and callers                                                                                    | Targets or gaps                                                                                                                                             | Extraction decision                                                                                                                                                                                 |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tokens                           | `design/tokens/tokens.json`; Vite `tokens-plugin.ts`; Colab native `chrome.rs`; Remote `assets/pages.css`     | Remote manually projects a subset; browser and native projection owners differ. Proposed type/space/size/border/shadow/layer/motion groups are not present. | Keep JSON authoritative. Share deterministic CSS projection and embedded component CSS; never maintain another palette.                                                                             |
| Header                           | Colab `colab-header.tsx`/`.css`, router and reader; native `socket.rs`; Remote page templates and `pages.css` | Same metrics, separate markup/classes; the Colab component imports product strings; native title tooltips remain.                                           | Share geometry and slots with React and static HTML contracts. Product label, navigation, status, overflow and trusted actions are supplied by consumers.                                           |
| Notice card                      | Colab `notice-card.tsx`/`.css`, router/reader/mounted app; native guidance; Remote `.sheet`                   | Mark implementation, heading typography and responsive margins differ.                                                                                      | Reuse current structure. Explicit state and content slots; common CSS works without React. Review visual differences rather than treating either implementation as an accidental universal default. |
| Button, toggle and field         | Colab `style.css`, header/card actions, page-list archive toggle; Remote pairing form                         | App-specific selectors; busy/disabled reasons and custom tooltips are not one shared implementation.                                                        | Shared presentation and accessible primitives. Native text fields remain allowed; no action or settings writer enters the package.                                                                  |
| Chip and tag                     | Colab header status, audience, expiry and archive labels                                                      | Shared chip sizing and count semantics are design targets; thread status (#1763) and Remote chip (#1770) are active product seams.                          | Common mark/label shell; callers own counts, status and driver provenance.                                                                                                                          |
| List rows                        | `design/browser-ui/src/list.tsx`/`.css`; Colab page index                                                     | Remote device lists remain future consumer work.                                                                                                            | The leaf owns row geometry and slots; hosts supply navigation, update metadata, state words and actions. See [the package contract](browser-ui/README.md).                                          |
| Listbox                          | Colab `components/listbox.tsx`/`.css`; share dialog and annotation input                                      | Current focus stays on the trigger; input-trigger list is portalled and tracks viewport/scroll ancestors. Radix replacement is proposed.                    | Extract current keyboard/placement mechanics after contract review; preserve controlled selection and product-owned options.                                                                        |
| Action menu                      | Colab `components/action-menu.tsx`/`.css`; thread comment actions                                             | Current menu depends on the owning row's padding box and clips against its scroll ancestor.                                                                 | Reuse current menu with an explicit anchor contract; don't copy a second navigation/placement implementation.                                                                                       |
| Conversation turn                | Colab `components/conversation-turn.tsx`; Ask and thread panels                                               | Imports Colab strings and relative-time formatting. Driver-colored treatment is tracked separately in #1789.                                                | Move presentation only; inject localized byline/time/meta/actions/delivery content and keep literal bodies as text. Preserve #1789 parentage.                                                       |
| Overlay and composer             | Colab router, annotation input, Chat/Ask/thread/share panels                                                  | Draft/anchor retention, live updates (#1774), small annotation window (#1761), and status actions (#1763) have product ownership.                           | Extract only overlay/placement/focus mechanics after these seams are reviewed. Drafts, sending and status remain consumer-owned.                                                                    |
| Tooltip                          | Native `title` attributes and selected product-local hints                                                    | The approved custom hover/focus/Esc tooltip is not a single shared component.                                                                               | One accessible presentation primitive for current icon and status callers; required information must also be visible or described outside the tooltip.                                              |
| Product switcher and setup guide | No shared implementation                                                                                      | Approved targets, not current shared consumers.                                                                                                             | Record separate product work; do not invent installation or discovery capabilities in the package.                                                                                                  |
| Proposal item                    | Colab feature #1773, not current main                                                                         | Its trusted action row and renderer containment belong to Colab.                                                                                            | Share basic controls when available; proposal storage and admission are not reusable UI logic.                                                                                                      |
| Inline band                      | Terminal board has its own TUI implementation                                                                 | Browser band is a target with no shared consumer yet.                                                                                                       | No browser extraction based solely on the terminal painter.                                                                                                                                         |

## Token and static-output contract

Use the existing `color`, `surface`, `font`, `header` and `breakpoint` groups.
Retain light/dark defaults and explicit theme override behavior. Preserve the
Vite plugin's terminal `--t-*` projection for handbook consumers. Proposed groups
stay labeled until values and consumers are reviewed; existing CSS constants must
be inventoried before adding a token rather than copied into a second schema.

The proposed package has a dependency-free CSS/static entry and optional React
exports. React and icon libraries are peer/consumer dependencies, not prerequisites
for serving Remote HTML. Reuse the existing pinned implementations first; selecting
Radix needs a separate dependency decision, with a concrete benefit and behavioral
comparison, before installation or replacement.

The `design/browser-ui/` home contains a private presentation leaf owned by
tmt-ux. Products may depend on the leaf; it must not import product code. Core and
the CLI must neither depend on nor embed it. The initial #1797 source implements
checked CSS, static class names and isolated Header, Notice, Field, Action and
Toggle. List/Row presentation is shared and used by the Colab page index. The leaf also provides an icon action with hover/focus tooltip and controlled
pressed state, and Field supports one native or contenteditable text control;
[the package contract](browser-ui/README.md) owns their accessible associations,
static markup, entries, generation
commands and host metric inputs. Consumer adoption is tracked separately from package availability. The package and its shared workspace,
lockfile, quality and static COPY integration are in place. Workspace,
lockfile and dependency changes receive Core review, and packaging changes receive
Colab/Remote review at the actual implementation head before delivery/adoption.

The static entry must provide checked, deterministic token/component CSS that the
native products embed at compile time. A Node build may generate assets in developer
or release preparation; an installed product never runs Node, React or a generator
to serve a page. Reuse the existing same-origin stylesheet routes and preserve CSP,
content types, secret-free requests and font fallbacks. Static pages do not fetch
fonts or assets from external origins. A generated file has one source, regeneration
command and equality check; consumers cannot hand-edit their own copy.

Component CSS is namespaced. Product page layouts, author frames and retained
scroll containers remain outside it. Changing imports must not impose a new global
reset, fixed content height, scroll region, theme store or resource lifetime.
Header safe-area offsets and the one window scrollbar remain observable gates.

## Commands and inline code

The command primitive is implemented in `browser-ui/src/command.css` and
`command.tsx`. `BrowserCommand` renders literal, selectable text and accepts
host-owned action and feedback slots. It never reads the clipboard or starts work.
Copy success uses visible **Copied.** feedback; denial leaves the exact value
available for manual selection. Hosts retain their async lifetime and permission rules.

Static hosts use `tmt-ui-command` with `tmt-ui-command-text`, an optional native
`tmt-ui-command-copy` button, and `tmt-ui-command-feedback` with `role="status"`.
An existing read-only input may use the text class to preserve native selection;
its value scrolls inside the input. Labels and values have separate gaps; input focus outlines stay inside the value field. The React text block wraps long values within
the available width. Copy actions stay at the end of the text row, centred vertically. Both use a
square, flat neutral surface and 1px edge. `tmt-ui-code` supplies the lighter
inline treatment in running prose. The `browser.surface.command` and `code`
tokens define both themes; the shared mono stack and command metrics define type
and spacing. Products own surrounding layout, labels and copied bytes.

Remote entry/error/settings, Colab trusted chrome and native guidance, and Office
reference/guidance views use these classes. Office imports the checked static CSS
asset directly; it adds no React or runtime dependency on the package. Author HTML,
the handbook Cmd component and terminal/TUI presentation retain their own owners.

## Component inputs and lifecycle

### Header and notice

A header accepts a product label, screen title, optional brand link rendering,
status/action slots and a DOM ref. The package supplies semantic `header`/one `h1`,
geometry, type and icon presentation. A host supplies already-resolved links and
handles routing; the package never enumerates installed products. Static markup
uses the same documented class/slot structure and escapes product-supplied text.

A notice accepts a presentation role, eyebrow, heading, body and action slots.
The state role and its visible label are separate inputs: the package cannot infer
health from an arbitrary status string. Opening/waiting/inactive uses status
semantics; an actionable failure uses alert semantics without announcing the same
message twice. A state mark is decorative next to its visible word. Mounting or
refreshing a notice never invokes recovery. Loading and failed notices retain the
same header and action ownership as ready pages.

### Controls, chips and fields

Controls accept labels, visible descriptions, disabled/busy state and host actions.
Busy presentation retains the label and size. Disabled reasons remain available to
keyboard users; a blocked action does not disappear. A destructive variant does
not perform confirmation or mutate anything. Consumers supply consequence wording,
explicit confirmation intent, and persistence/retry behavior.

Toggle and icon-action pressed state are controlled by their caller. Field accepts
a label, description, validation message and `renderControl` for one focusable text
control; the host retains value, ref, caret/IME and editor lifetime. The package owns
accessible association and visual feedback, not validation authority or settings writes.
Chips/tags accept visible text, an optional mark and a role; count and state are
never read from an inbox or inferred from a color. An interactive chip is a control,
not an arbitrary clickable span. Driver color requires consumer-verified metadata;
a name or display label alone cannot choose provenance.

### Choice and action menus

Choice options have stable keys, labels and disabled state, with one controlled
selected key. Opening or moving the active option never selects or mutates data.
Empty/all-disabled options cannot produce a callback. Keyboard navigation skips
disabled entries and preserves the existing trigger/input focus model. Esc closes
only the innermost popup and returns focus to the same trigger if it still exists.
Pointer dismissal does not dispatch a choice or erase the host's draft.

An action menu accepts stable action keys, labels, disabled state, an explicit
anchor and host callbacks. It owns roving focus and placement relative to that
anchor's scroll/viewport bounds. It owns no edit/delete/request permission. The
consumer migration must preserve existing trusted-input admission: moving a
callback cannot make synthetic activation authorize a product action. If a UI event
is required by that boundary, retain it in the reviewed callback contract rather
than stripping it during extraction.

Placement is disposable. Resize/scroll observers, document listeners, timers and
portals are released on close/unmount. A changed or disconnected anchor closes or
repositions presentation without targeting a different row. Long labels fit at
320 px and within a narrow overlay; a clipped ancestor must not hide the menu.

### Conversation, overlay and composer

A turn receives role/layout, display author, formatted timestamp/description,
metadata, body, action and delivery slots. It does not fetch identity metadata,
resolve a driver, fold a discussion, dispatch a request, or decide editing rights.
Bodies remain literal text; custom rich content is supplied only by trusted chrome,
never promoted from author HTML. Current Chat and thread layout distinctions are
preserved and tested separately, including agent attribution after reload.

An overlay receives controlled open state, an opener/anchor and content. It owns
focus/placement/disclosure mechanics; the host chooses whether it is modal and
which existing panel is active. It cannot create its own application-wide panel
manager. Esc affects the innermost presentation only, and the opener may disappear
without transferring focus to an unrelated control.

A composer primitive receives controlled text, disabled/busy/error presentation,
a label and host submit/close callbacks. It never owns a draft store, operation ID,
connection/session, frozen quote, recipient or retry policy. Enter/Shift+Enter and
composition events are verified against the host's existing send contract. No close,
mount, refresh, source replacement or outside press may automatically submit.

Before extraction, review #1761's retained small-window/anchor API and #1763's
status/notification coordinator together. Keep one owner for shared router/thread/
annotation integration. Package APIs must not force either product feature to
transfer persistence or ledger responsibility, and neither extraction nor a status
chip may destroy an unsent draft or bypass a held/uncertain operation.

## Verification and adoption evidence

| Scope                      | Required observable evidence                                                                                                                                                                                                                      |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Common CSS and chrome      | Light/dark at 1440 and 390, plus 320 px fit; safe-area header metrics, title/action overflow, one window scrollbar, font fallback and 4.5:1 text/3:1 mark contrast. Compare React and served static examples.                                     |
| Controls and popups        | Keyboard open/navigation/select/Esc/focus return, empty/all-disabled data, disabled reason, long labels, scroll/resize/replaced-anchor cleanup, and accessible names/relationships. Synthetic activation cannot bypass consumer action admission. |
| Conversation and composers | Literal markup, user/agent attribution, draft/caret/focus preservation, Unicode/composition input, frozen quote/anchor, busy/failed send, close/reopen and unmount cleanup.                                                                       |
| Colab native embed         | Real CSP guidance/recovery with no app build; no inline-style relaxation; native archive embeds the shared sources and the installed verifier checks actual assets.                                                                               |
| Remote native embed        | Real-socket landing/pair/error assets keep exact policy/content types; fragment erasure precedes SDK load; token and static/component CSS agree; archive includes/embed proofs use the released product.                                          |
| Remote settings (#1769)    | Owning Remote contract/tests prove view/change/device actions and its authority rule. Shared component tests never substitute for allowed/refused/held-path tests.                                                                                |
| Release                    | The published Colab and Remote tags contain their adoption commits; matching-host archive/installer verification and product upgrade/smoke pass. Failed or held cuts remain incomplete.                                                           |

Existing evidence owners remain Colab's app unit/browser and real-door acceptance,
Remote's pages/pairing/native suites, and the release verification pipeline. Move
component assertions next to their exports when responsibility actually moves;
retain product integration assertions for drafts, trusted actions, transport,
persistence and cleanup. Green package tests do not certify those boundaries.

Each adoption issue records its affected surfaces, reviewed full SHA, behavioral
and visual differences, exact gates, resulting main commit and published product
tag. #1792 remains open until the required current consumers are adopted and their
publication evidence is verified. Future feature targets are recorded as planned;
no unpublished package or unimplemented settings page is presented as current.
