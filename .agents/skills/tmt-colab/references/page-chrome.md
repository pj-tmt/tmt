# Page chrome (#1586)

`colab-header.tsx` and `notice-card.tsx` adapt Colab content to the private
`@tmt/browser-ui/react` Header/Notice exports. Router chrome uses the shared Action/Toggle
and static classes; Theme and page overflow/close use `BrowserIconAction`.
The overflow trigger supplies controlled disclosure state and its menu ID; the
host owns menu dismissal and focus return. Original trusted activation events remain Colab-owned. App and reader
entry points import `/static.css`. Header/card geometry, opaque square surfaces, mark plus
visible state word and shadow-free presentation belong to that leaf; its
[package contract](../../../../design/browser-ui/README.md) owns generation and host inputs.
`colab-header.css` supplies Colab safe areas, fixed viewport placement, body offset and host
metrics; `notice-card.css` retains product page positioning and inherited state-label type. Legacy product color/font
variable names alias the shared browser roles rather than projecting another palette.

Native pairing/build guidance stays static. `chrome.rs` compile-time embeds the same checked
CSS and Colab host/reader/notice layout styles; `/assets/chrome.css` serves those immutable
bytes even without an app build. `socket.rs` uses shared static header/notice slots, escapes
product text, and keeps the parent CSP free of inline style/script exemptions. Guidance
loads the admitted build's recovery entry only when present; otherwise its details remain
visible. Neither Cargo nor installed serving runs Node or fetches external assets.

`router.tsx` retains the active Source, Comments, Chat, Agents or Export overlay. At narrow
widths the page actions move into an overflow menu. The existing display-only
`local · <name>` label and sharing metadata move inside that menu on mobile.
Page-list cards show their name (or `Untitled page`), a short mono ID and the
shared expiry hint; Details and Manage stay inside the card. Full IDs remain in
Details. Neither titles nor retention hints grant access.

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
Author backgrounds and widths belong to the page; parent theme tokens style
chrome. The sandbox attributes, source limits and CSP remain unchanged.

Source revisions replace only the author renderer, not parent chrome. `router.tsx`
keeps the mounted annotation input and frozen quote/rectangle during loading;
page-ID changes reset page-local selection, panels and unsent drafts. Existing
thread highlights are projected again when the new render is ready. `renderer.ts`
checks the frozen composer quote through the same bounded cosmetic resolver: the
reserved empty anchor ID resolves without painting a highlight or creating a
marker/action. Only an admitted resolution response marks the check complete;
pending checks do not show the stale-quote hint. Annotation/Chat submits and recipient selection use `BrowserAction`: the existing
Enter intent selects the one primary submit, while other submits and recipient selection
remain text actions. `MessageComposer` renders its retained Lexical editable combobox through
`BrowserField`, with host-owned popup ARIA, draft, ref, caret/IME and editor history.
`PageDrawer` retains dialog modality and focus; Chat supplies its shared
`ConversationWindow` header and close action instead of a duplicate drawer header.
Close uses `BrowserIconAction`; its tooltip and focus presentation belong to the leaf. Annotation surfaces and drawers are shadow-free.
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
The [UI contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#own-stream-discussion-records-1427)
owns draft survival and stale-quote behavior.

The annotation window uses the same `ThreadWindow` as Comments from first
selection through the committed conversation. Its one shared composer stays
mounted on first send; Close, Escape and outside press collapse without sending
or resolving. Page-owned drafts keep the exact edit and recipient for reopening
the known thread. Current renderer anchor positions locate marker-opened windows;
viewport dimensions clamp their square, shadow-free surface independently of
document height. The shell fits its content without reserved history height and
grows away from the same selection edge through first Send. At the viewport cap,
only history scrolls; the field and Send remain visible. Its shared icon actions
retain hover/keyboard tooltips; a long status title truncates before the fixed
right-aligned controls; attachment metadata stays on that same header row.
Header/composer stay in place while messages scroll, including
an associated delayed reply to an earlier turn. Unrelated live publications keep
the reader's message-history position. This changes no renderer messages,
subscription, publication admission or reply association. Comments remains the
explicit full-history/details view.

`page-drawer.tsx` portals chrome outside the header/menu. Desktop panels float on
the right on an opaque square surface; mobile uses a full-screen native modal sheet. Neither
changes renderer width or content layout. The panel body scrolls independently.
Crossing the mobile breakpoint changes native modality in place, preserving a connected
focused descendant and its editing selection; it does not restart a draft or run initial-open focus.
Close/Escape restores focus, and media listeners/dialogs clean up on close or
unmount. Chat initializes on first opening; closed Source, Comments and Chat panels retain drafts and admitted history;
closing never dispatches, abandons or retries. Export closes its preparation and
revokes download Blob URLs. Manage also portals outside the menu. Safety details
remain available from Page information and blocked views; visibility never
substitutes for writer admission.

`agent-status-panel.tsx` presents the canonical read-only `LiveAsk` observation
inside the Agents drawer. It reads only while opened or after explicit Recheck,
retains transient directory failures as a labelled stale snapshot, and drops rows
on ended/evicted/scope-denied or changed page/client admission. Session/directory
faults never become offline presence. Pending results are fenced on close and
binding replacement; this presentation has no message or recovery capability.
The read owner and typed failure boundary are defined in [Ask agent](ask-agent.md).

`e2e/chrome.spec.ts` compares header and state-card dimensions, font metrics, state words/colors and window-scroll
containment across every screen at 1440/390/320 in light/dark, including responses
from the native no-app socket fixture. Short-screen midpoint checks add an inert
scroll probe; their top captures show the natural notice layout.
`e2e/layout.spec.ts` covers window-scrolled 1440/390 light/dark short/long captures,
one-row/menu layout, overlays that preserve page geometry, and focus/Escape.
`e2e/live-update.spec.ts` covers annotation/thread/Chat draft survival, frozen-quote
sends, highlight reprojection, window offsets and page-change resets at 1440/390.
`e2e/renderer-scroll.spec.ts` covers owner and reader resizing, local anchors,
viewport-growth fallback and malformed/stale/over-limit claims. Existing Ask,
discussion, management and export scenarios verify the publication controls;
run the [app/browser and real-binary acceptance gates](development.md).
