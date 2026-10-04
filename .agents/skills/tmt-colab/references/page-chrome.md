# Page chrome (#1586)

`router.tsx` owns one sticky page bar and the active Source, Comments, Ask or
Export drawer. At narrow widths the secondary actions move into a single-row
bar's overflow menu. A plain `local · <name>` label uses the mounted session's
existing device-context name (display only), or `local` when no name is known. The renderer fills the remaining viewport; source/title and
page scripts remain inside the same opaque sandbox. Author backgrounds and
scrolling belong to the page, while light/dark tokens style only parent chrome.
The read-only reader uses the same frameless viewport without publication controls.

`page-drawer.tsx` portals parent chrome outside the bar/menu. Desktop drawers are
nonmodal so the author page stays selectable; mobile uses a native modal dialog
as a full-screen sheet. Close/Escape restores focus, and media listeners/dialogs
clean up on close or unmount. Closed Source, Comments and Ask drawers stay mounted
to retain drafts and frozen attempts. Closing does not dispatch, abandon or retry.
Export closes its existing preparation/download lifecycle and revokes Blob URLs.

The renderer bootstrap, selection/highlight channel, sandbox attributes and CSP
are unchanged. Safety details remain available from Page information and blocked
views. Drawer visibility never substitutes for connection/writer admission.

`e2e/layout.spec.ts` covers exact viewport geometry, retained sandbox attributes,
one-row/menu layout, focus/Escape and 1440/390 light/dark captures of short and
mid-scroll long pages. Its source is a read-only test fixture. Existing Ask,
discussion, management and export scenarios exercise controls through the drawers;
run the [app/browser and real-binary acceptance gates](development.md).
