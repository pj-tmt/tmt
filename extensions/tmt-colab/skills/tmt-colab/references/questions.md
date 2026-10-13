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
trusted controls and visible recipient chips. Defaults are distinct uniquely resolved
asking agents in tab order; readers can add or remove them. Empty recipients keep
answers on the page. Answers are ordinary annotation comments; read with
`tmt colab threads PAGE --json`. Changing an answer adds a normal follow-up;
the latest verified device answer in its canonical thread wins. Colab prefers an
answered thread, using writer/thread identity order for ties. HTML names grant no
routing authority. Removing the question keeps its thread.

In-tab drafts remain mounted while switching questions. The group's shared footer
sends complete unsent drafts in one explicit action, including a partial group;
untouched drafts stay for later. Each answer gets its own comment/thread, while
each recipient receives one Ask listing the recorded answers. Sending keeps the
selected tab. A one-question group uses Send answer. The canonical first-recorded
question holds the reply; other participating tabs use Open reply. Failed comment
drafts remain for the next Send, which publishes them again; a failed recipient
retries the same recorded batch. Change answer sends only that question's follow-up.
Grammar, counts, limits and publication/retry rules live in
[the answer-block contract](https://github.com/pj-tmt/tmt/blob/main/extensions/tmt-colab/contracts/colab-v1.md#answer-blocks).

Run `tmt colab page check FILE` before publishing: a malformed block is otherwise ignored
without notice, and the check names the reason code and line. A structurally skipped group's finding adds
`questionsSkipped`, how many questions it took with it.
