---
name: tmt-ui-ux-design
description: Draw UI mockups and propose UX changes for TMT browser surfaces (Colab, Remote, the GUI shell) so they match the shipped product. Load before drawing a mockup or writing a UX proposal.
---

# UI mockups and UX proposals

A mockup is a proposal about the real product. A reader trusts what it shows, so it
must be either the product's own look or plainly marked as a placeholder. This skill is
the procedure; the rules it follows have their own owners:

- [Browser style](../../../design/gui-style.md): visual and interaction rules per component.
- [Browser component contracts](../../../design/gui-components.md) and the
  [shared browser leaf](../../../design/browser-ui/README.md): components, tokens and entries.
- The [Colab page skill](../tmt-colab/SKILL.md) (`tmt colab skill components`): the
  `tmt-ui-*` classes and icons a Colab page can use.

Do not restate those rules in a mockup page or in this skill; link to them.

## 1. Read the shipped surface first

Before drawing a surface, read the code that draws it today, not a memory of it:

- The shell, sidebar (rail) and module menu are in `design/browser-ui/src/shell.ts` and
  `shell.css`. Colab's sidebar menu is defined in
  `extensions/tmt-colab/typescript/app/src/colab-shell.tsx`.
- Colab's screens are in `extensions/tmt-colab/typescript/app/src/` (for example `home.tsx`
  for the page list and `project-dialogs.tsx` for the project drawer).
- Copy UI text exactly from the product's strings (`extensions/tmt-colab/typescript/app/src/strings.ts`,
  `extensions/tmt-remote/typescript/remote-client/src/`). The product's UI is English, so a
  mockup's UI text is English too, even when the explanation around it is in the reader's language.
- Read sizes and colours from the tokens in `design/browser-ui/generated/static.css`. Check what
  a token means before using it. For example, `--tmt-ui-color-working` is green, close to the accent,
  so it cannot stand for a second, different state beside the accent.

When a new screen resembles an existing one, reuse that one's pattern. For example, a manage
list follows the project drawer: one rounded card per item, a name and a small secondary line,
move-up and move-down icon actions, Edit and Delete, and a primary create action in the footer.

## 2. Draw only what changes, and mark it

- Draw the parts the proposal adds or changes with the real components. Mark them, for
  example with a dashed outline, and say what the mark means in one caption.
- Draw everything else as neutral skeleton blocks. They show where things sit, never how
  they look. Never hand-draw an approximation of an existing screen: an approximation reads as
  a proposed change to it.
- Use sample data that is plainly sample data. Name in the caption what is real and what is placeholder.

## 3. Build every frame from one set of parts

- Generate the frames from one script or template with one function per part: the rail,
  the page list, the page header, a dialog and a skeleton row. Every frame then changes together
  when one part changes, and the frames stay consistent.
- Show each frame as a whole screen: the sidebar plus the main area, with a dialog centred over a
  dimmed product. Do not show a dialog as a fragment floating in an empty box.
- Keep wide frames inside their own horizontal scroller, so the page itself never scrolls sideways.
- A Colab page can use the shell classes (`tmt-ui-rail`, `tmt-ui-rail-sub-item` and so on). They
  hide the rail below 900 px, so a mockup overrides that display rule only inside its own frame.

## 4. Check before you say it matches

- Run `tmt colab page check FILE --json` and fix every `unknown-ui-class` and `unknown-icon`;
  an unknown name draws nothing. The checker's list of classes and icons is
  `extensions/tmt-colab/rust/tmt-colab/src/page_check/page_ui.json`.
- Read the published page back and compare it with the file (see the Colab skill).
- Say what you verified. If you checked only markup and did not see it rendered, say so and ask
  the reader to point at what looks wrong.

## 5. Write the proposal around the mockup

- Start with what already ships and what is new, with ticket numbers, so the reader can tell
  the proposal from the product.
- For every new concept, say what it is, where it is created, where it is seen, how it is
  changed and where it is stored, including whether it syncs through Remote.
- Put each open decision on the page as a `tmt-question` block with a recommended choice. Record
  each answer on the page as decided, with its date, in the same change that applies it.
- When a mockup choice departs from a shipped value (a spacing, a colour, a word), name the
  shipped value and say that adopting the new one changes the shared component, not only this screen.
