# Page chrome (#1586)

`colab-header.tsx` owns the shared fixed, one-row header for the page list,
owner page, router errors, mounted lifecycle notices and reader states. Screens
supply their title and actions; `colab-header.css` owns geometry, brand/title
hierarchy and flat actions. Header dimensions and typography come from
`design/tokens/tokens.json`, including the shared compact viewport rule. Lucide
icons use currentColor, square caps and miter joins.

Native pairing/build guidance stays static. `chrome.rs` projects the same tokens
and includes the same header CSS; `/assets/chrome.css` serves those immutable
bytes even without an app build. `socket.rs` uses matching header slots and keeps
the parent CSP free of inline style/script exemptions.

`router.tsx` retains the active Source, Comments, Chat or Export overlay. At narrow
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

`page-drawer.tsx` portals chrome outside the header/menu. Desktop panels float on
the right with a hard shadow; mobile uses a full-screen native modal sheet. Neither
changes renderer width or content layout. The panel body scrolls independently.
Close/Escape restores focus, and media listeners/dialogs clean up on close or
unmount. Chat initializes on first opening; closed Source, Comments and Chat panels retain drafts and admitted history;
closing never dispatches, abandons or retries. Export closes its preparation and
revokes download Blob URLs. Manage also portals outside the menu. Safety details
remain available from Page information and blocked views; visibility never
substitutes for writer admission.

`e2e/chrome.spec.ts` compares header dimensions, font metrics and window-scroll
containment across every screen at 1440/390 in light/dark, including responses
from the native no-app socket fixture. Short-screen midpoint checks add an inert
scroll probe; their top captures show the natural notice layout.
`e2e/layout.spec.ts` covers window-scrolled 1440/390 light/dark short/long captures,
one-row/menu layout, overlays that preserve page geometry, and focus/Escape.
`e2e/renderer-scroll.spec.ts` covers owner and reader resizing, local anchors,
viewport-growth fallback and malformed/stale/over-limit claims. Existing Ask,
discussion, management and export scenarios verify the publication controls;
run the [app/browser and real-binary acceptance gates](development.md).
