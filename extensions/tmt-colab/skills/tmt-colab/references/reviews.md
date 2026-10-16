# Review sections

When authorized, wrap each part of the page the reader must confirm:

```html
<tmt-review data-id="UUID" data-agent="Your agent name">
  <h2 data-title>Section heading</h2>
  …the content to confirm; answer blocks may sit inside…
</tmt-review>
```

Use a unique canonical UUID (one namespace with question and group IDs) and exactly one direct
`data-title` child (at most 200 characters, no control characters). `data-agent` names who is
told; it must match one directory entry. At most 50 per page. Sections do not nest and do not
sit inside a question or group. Run `tmt colab page check` before publishing.

Colab draws the controls under the section; page HTML cannot confirm, close or count anything.
`✓ Confirm and close` resolves the section's thread and tells you once. `✎ Needs change…` adds a
comment, asks you, and leaves the thread open. A closed section fades but stays expanded, and the
page head shows `n / m closed`. Answering every question inside a section closes it too.

Read results with `tmt colab threads PAGE --json`. The section's thread is anchored on the whole
heading; its comments start `Confirmed “…”`, `Needs change to “…”: …` or `Answered “…”`. Only a
person's resolve closes a section: your own `tmt colab threads resolve` leaves it waiting. After
you edit for a change request, answer with `tmt reply`; the section goes back to waiting for the
reader. To ask again after a close, run `tmt colab threads reopen PAGE THREAD`.

Grammar, limits and rules live in
[the review-section contract](https://github.com/pj-tmt/tmt/blob/main/extensions/tmt-colab/contracts/colab-v1.md#review-sections).
