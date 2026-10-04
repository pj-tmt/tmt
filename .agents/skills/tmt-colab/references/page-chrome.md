# Page chrome (#1586)

`router.tsx` owns one fixed page bar and the active Source, Comments, Ask or
Export overlay. At narrow widths the actions move into an overflow menu. A plain
`local · <name>` label uses the mounted session's existing device-context name
(display only), or `local` when no name is known; mobile places it and sharing
metadata inside the menu. Toolbar actions and menu rows are flat, and the theme
menu row labels its current value. The read-only reader uses the same sizing/anchor
mechanism without publication controls; its separate presentation is reader-owned.

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
unmount. Closed Source, Comments and Ask panels retain drafts and frozen attempts;
closing never dispatches, abandons or retries. Export closes its preparation and
revokes download Blob URLs. Manage also portals outside the menu. Safety details
remain available from Page information and blocked views; visibility never
substitutes for writer admission.

`e2e/layout.spec.ts` covers window-scrolled 1440/390 light/dark short/long captures,
one-row/menu layout, overlays that preserve page geometry, and focus/Escape.
`e2e/renderer-scroll.spec.ts` covers owner and reader resizing, local anchors,
viewport-growth fallback and malformed/stale/over-limit claims. Existing Ask,
discussion, management and export scenarios verify the publication controls;
run the [app/browser and real-binary acceptance gates](development.md).
