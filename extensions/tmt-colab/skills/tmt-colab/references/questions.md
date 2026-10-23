# Answer blocks

When authorized, place each decision beside the passage it concerns. Colab renders
compact question lines that the person can open; no tabs or layout HTML are needed.
Use a unique canonical UUID for every question and group.

```html
<div>
  <p>The first passage has one decision.</p>
  <tmt-question
    data-id="10000000-0000-4000-8000-000000000001"
    data-group="20000000-0000-4000-8000-000000000001"
    data-kind="single"
    data-agent="Your agent name"
    data-other
  >
    <p data-question>Which option fits this passage?</p>
    <span data-choice>First</span><span data-choice>Second</span>
  </tmt-question>
</div>
<p>A separate passage can have another question with that same data-group.</p>
<tmt-question
  data-id="10000000-0000-4000-8000-000000000002"
  data-group="20000000-0000-4000-8000-000000000001"
  data-kind="short"
  data-agent="Your agent name"
>
  <p data-question>What should this passage say?</p>
</tmt-question>
<tmt-questions data-id="20000000-0000-4000-8000-000000000002">
  <p data-title>Two decisions about the next passage</p>
  <tmt-question
    data-id="10000000-0000-4000-8000-000000000003"
    data-kind="short"
    data-agent="Your agent name"
  >
    <p data-question>Which heading?</p>
  </tmt-question>
  <tmt-question
    data-id="10000000-0000-4000-8000-000000000004"
    data-kind="long"
    data-agent="Your agent name"
  >
    <p data-question>Why does it fit?</p>
  </tmt-question>
</tmt-questions>
```

`data-group` links standalone questions across placements; do not put it on a
question inside a `tmt-questions` wrapper. A group has at most 12 questions and
counts once in Needs you. Question, group, review and reading IDs share one
page-wide uniqueness namespace. A split question's Send sends only that question.
A wrapper keeps one batched Send for its completed unsent answers. Neither sends
another passage's draft. Defaults come from the asking agents in that physical
block, deduplicated; the person can edit the visible recipient chips.

Kinds: single/multiple with choices and optional `data-other`; short/long with no
choices or Other. Limits: 50 questions/page, 20 distinct choices/question, 200 code
points per question/choice. Put who asks in `data-agent`, and explain why the
answer matters. Put elements inside a `div`, `li` or `td`, not a `p` that their
child paragraphs would end. Do not gather unrelated decisions at the page's end.

For an option that needs context, put one immediate plain-text description inside
its choice:

```html
<span data-choice
  >Click a marker
  <span data-choice-description>Open the discussion when a user clicks an inline marker.</span>
</span>
```

Descriptions allow up to 200 Unicode code points after whitespace normalization.
Use no nested markup, comments or control characters. An invalid description is
dropped with the `question-choice-description-invalid` warning; the choice stays
admitted when its label passes the existing rules. Marked description children
never count toward the label, even when invalid. Descriptions cannot distinguish
duplicate labels or replace an empty label. Colab stacks described choices and
announces each description with its control; the label alone is recorded in the
answer. Without descriptions the existing inline choices stay unchanged.

Questions start closed, except an arrival from Needs you or a notification. The
person's expanded/collapsed state is local and never changes the answer. Waiting
and Answered states come from the existing verified answer path; collapse does
not dismiss or acknowledge a question. HTML names grant no routing authority.

Answers are ordinary annotation comments; read with `tmt colab threads PAGE --json`. A short,
long or Other answer can carry images: they are the answer comment's attachments, which `threads`
lists, and `tmt colab attachment read PAGE ID` writes one out.
Changing an answer adds a normal follow-up; the latest verified device answer in
its canonical thread wins. Each answer has its own comment/thread and each
recipient receives one Ask for the recorded batch. Untouched and failed drafts
remain for later. A failed recipient retries its same recorded batch without
republishing comments. Removing markup keeps its conversations. Grammar, counts,
limits and publication/retry rules live in the
[answer-block contract](https://github.com/pj-tmt/tmt/blob/main/extensions/tmt-colab/contracts/colab-v1.md#answer-blocks).

Run `tmt colab page check FILE` before publishing. Malformed controls are ignored;
the check reports reason codes and lines. A structurally skipped wrapper also
reports `questionsSkipped`.
