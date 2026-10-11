# Answer blocks

When authorized, write inert question HTML in the page:

```html
<tmt-question data-id="UUID" data-kind="single" data-agent="Your agent name" data-other>
  <p data-question>Which option?</p>
  <span data-choice>First</span><span data-choice>Second</span>
</tmt-question>
```

Use a unique canonical UUID. Kinds: single/multiple with choices and optional
`data-other`; short/long with no choices or Other. Limits: 50 blocks/page,
20 distinct choices/block, 200 characters per question/choice. Colab provides
trusted controls; Send names the recipient resolved from its directory. Answers
are ordinary annotation comments mentioning the asking agent; read with
`tmt colab threads PAGE --json`. Changing an answer adds a comment; the latest
verified device answer in the canonical thread wins. Colab prefers an answered
thread, using writer/thread identity order for ties. Reply through the normal receipt in the same thread.
HTML names grant no routing authority. Removing the question keeps its thread.

Run `tmt colab page check FILE` before publishing: a malformed block is otherwise ignored
without notice, and the check names the reason code and line.
