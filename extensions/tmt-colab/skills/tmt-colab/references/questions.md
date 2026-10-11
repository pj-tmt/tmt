# Answer blocks

When authorized, write inert question HTML in the page:

```html
<tmt-question data-id="UUID" data-kind="single" data-agent="Your agent name" data-other>
  <p data-question>Which option?</p>
  <span data-choice>First</span><span data-choice>Second</span>
</tmt-question>
```

For related questions, wrap up to 12 direct question children in
`<tmt-questions data-id="UUID"><p data-title>Review heading</p>…</tmt-questions>`.
Question and group IDs share one page-wide uniqueness namespace. Optional
`data-label` names a tab (24 normalized code points, no control characters);
missing, empty or invalid labels fall back to `Q<n>` among admitted questions.
Empty groups are skipped. A lone question or one-member group has no tab strip.

Use a unique canonical UUID. Kinds: single/multiple with choices and optional
`data-other`; short/long with no choices or Other. Limits: 50 blocks/page,
20 distinct choices/block, 200 characters per question/choice. Colab provides
trusted controls; Send names the recipient resolved from its directory. Answers
are ordinary annotation comments mentioning the asking agent; read with
`tmt colab threads PAGE --json`. Changing an answer adds a comment; the latest
verified device answer in the canonical thread wins. Colab prefers an answered
thread, using writer/thread identity order for ties. Reply through the normal receipt in the same thread.
HTML names grant no routing authority. Removing the question keeps its thread.
Tabs retain unsent drafts and show verified answer counts. Send & next advances
only after verification; the last unanswered question says Send answer. Answers
and agent replies remain visible when you return to an answered tab. Tab changes
and reloads retain the normal quote context of inactive questions.

Run `tmt colab page check FILE` before publishing: a malformed block is otherwise ignored
without notice, and the check names the reason code and line. A skipped group's finding adds
`questionsSkipped`, how many questions it took with it.
