# Answer blocks

When authorized, place each decision beside the passage it concerns. Colab renders
compact question lines that the person can open; no tabs or layout HTML are needed.
Use a unique canonical UUID for every question and group. Register each question before writing
its block (groups themselves are not registrations):

```sh
tmt colab page create --title 'Decision' --layout column --creator NAME
tmt colab question add PAGE --id 10000000-0000-4000-8000-000000000001 --agent NAME --title 'Which option fits this passage?'
tmt colab page read PAGE --lines 1:1 --json
tmt colab page patch PAGE --lines 1:1 --range-sha256 HASH --file page.html
```

Replace PAGE with the created ID, NAME with the resolved identity, and HASH with the
rangeSha256 returned by the read. Use 1:1 for an empty page; otherwise choose the intended range. Register every other
question ID in the examples before writing them. `page create --file` containing live questions
refuses and prints this sequence. Built-in `--template` creation fills the creator and registers
its compiled questions in the same commit. User-authored templates do not get that exception.
`page create` uses its invoking stable identity when available and otherwise creates with no
creator. Anonymous review/compare templates register unassigned questions: answers remain on
the page without Ask. Omit `--agent` to register an unassigned question; it follows the creator
for new drafts once one is set. Explicit assignments keep their target across creator changes.
Change future drafts with `tmt colab page creator PAGE NAME`; this never rewrites sent messages.
Unavailable lookup is unverified, not unknown. New names without lookup need a canonical UUID
or a retry; never guess their IDs.

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
does not combine registry counts: native Needs you counts each registered open question (browser adoption follows #5146). Question, group, review and reading IDs share one
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
[answer-block contract](../../../contracts/colab-v1.md#answer-blocks).

Inspect state with `tmt colab question ls PAGE --json`. A verified question-bound answer
projects answered independently of Ask delivery. `question handle PAGE ID` marks
it handled; `question reopen PAGE ID` reopens it. Repeat `question add --title` to update a
registered title; an unchanged registration is a no-op. A handled entry stays handled until
explicitly reopened, and deleting its last verified answer otherwise returns it to open.

Run `tmt colab page check FILE` before publishing; it is markup-only and offline. Add `--page
PAGE` to compare against that page's registry and creator. Native write/patch/owner Save compare new occurrences and changed ID/agent pairs; unchanged
legacy occurrences in the admitted base remain allowed. Creation and explicit page checks
compare the whole source. New or changed blocks refuse
`COLAB_QUESTION_UNREGISTERED`, `COLAB_QUESTION_AGENT_MISMATCH`,
`COLAB_QUESTION_AGENT_UNKNOWN` with a line and a command repair, without a partial source write.
An explicit `data-agent` must match the registered assignment, or the creator for an unassigned
question. Empty or omitted `data-agent` uses the assigned registration, or the creator for an unassigned
question. Without either target, answers stay on the page and no Ask is sent.
Anonymous creation never fabricates an identity. An identity lookup failure alone is never
proof that an identity is unknown. Malformed controls are ignored; the markup check reports
reason codes and lines. A structurally skipped wrapper also reports `questionsSkipped`.
