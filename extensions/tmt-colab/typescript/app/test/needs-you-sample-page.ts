/** The markup of the owner's "QA sample: needs-you components" page (page 2f7f7eef): one reading,
 * a question with a chip-form side question, a review that closes by confirming and a review whose
 * question closes it. Real needs-you components with the author's own styles, so layout specs
 * run against the page the owner actually opens. */
export const needsYouSamplePage = `<style>
  :root {
    --ink: #343434;
    --muted: #626262;
    --paper: #fafafa;
    --sheet: #ffffff;
    --edge: #d0d0d0;
    --body: system-ui, -apple-system, 'Segoe UI', sans-serif;
    --mono: ui-monospace, SFMono-Regular, Menlo, monospace;
    --size: 14px;
    --heading: 28px;
    --rule: 1px;
    --gap: 16px;
    --small-gap: 8px;
    color-scheme: light dark;
  }
  @media (prefers-color-scheme: dark) {
    :root:not([data-theme='light']) {
      --ink: #b0b0b0;
      --muted: #909090;
      --paper: #080808;
      --sheet: #101010;
      --edge: #303030;
    }
  }
  :root[data-theme='light'] {
    --ink: #343434;
    --muted: #626262;
    --paper: #fafafa;
    --sheet: #ffffff;
    --edge: #d0d0d0;
    color-scheme: light;
  }
  :root[data-theme='dark'] {
    --ink: #b0b0b0;
    --muted: #909090;
    --paper: #080808;
    --sheet: #101010;
    --edge: #303030;
    color-scheme: dark;
  }
  * {
    box-sizing: border-box;
  }
  body {
    max-width: 1280px;
    margin: 0 auto;
    padding: var(--gap);
    background: var(--paper);
    color: var(--ink);
    font: var(--size)/1.5 var(--body);
    overflow-wrap: anywhere;
  }
  h1,
  h2,
  h3 {
    margin: 0 0 var(--small-gap);
    line-height: 1.2;
  }
  h1 {
    font-size: var(--heading);
  }
  h2 {
    font-size: 22px;
  }
  h3 {
    font-size: 18px;
  }
  p,
  ul,
  ol,
  table,
  pre {
    margin: 0 0 var(--gap);
  }
  ul,
  ol {
    padding-left: calc(var(--gap) * 2);
  }
  a {
    color: inherit;
    text-decoration: underline;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    padding: var(--small-gap);
    border-bottom: var(--rule) solid var(--edge);
    text-align: left;
    vertical-align: top;
  }
  code,
  pre {
    font-family: var(--mono);
  }
  pre {
    padding: var(--gap);
    background: var(--sheet);
    border: var(--rule) solid var(--edge);
    white-space: pre-wrap;
  }
  section {
    margin: 0 0 var(--gap);
    padding: var(--gap);
    background: var(--sheet);
    border: var(--rule) solid var(--edge);
  }
  .muted {
    color: var(--muted);
  }
</style>
<h1>QA sample: needs-you components</h1>
<p class="muted">Sample page for the owner checks on #3778 (needs-you components) and #3779 (review sections). Every block below is a real component; acting on it is the check.</p>

<tmt-read data-id="7a3e1c20-4b1d-4c8e-9f20-000000003778" data-agent="tmt-feat-lead" data-why="Check 2 on #3778: mark this read, reload, open its link">
  <h2 data-title>Recommended reading: what the release keeps</h2>
  <p>A page that is kept forever on this machine stays here even when its cloud copy expires. Only the shared link stops working after the cloud date.</p>
</tmt-read>

<section>
  <h2>Pick a default</h2>
  <p>New pages need a default project scope.</p>
  <tmt-question data-id="7a3e1c20-4b1d-4c8e-9f20-000000003781" data-kind="single" data-agent="tmt-feat-lead" data-other>
    <p data-question>Which default scope should a new page get? (Check 3 on #3778: answer, send, Esc, reload)</p>
    <span data-choice>The last chosen project</span><span data-choice>No project</span>
  </tmt-question>
  <div>Side decision inside running text: the compact chip sits here
    <tmt-question data-id="7a3e1c20-4b1d-4c8e-9f20-000000003782" data-kind="single" data-agent="tmt-feat-lead" data-form="chip" data-other>
      <p data-question>Show archived pages in search by default?</p>
      <span data-choice>Yes</span><span data-choice>No</span>
    </tmt-question>
    and the sentence continues after it.</div>
</section>

<tmt-review data-id="7a3e1c20-4b1d-4c8e-9f20-000000003779" data-agent="tmt-feat-lead">
  <h2 data-title>Review: cloud expiry wording</h2>
  <p>Proposed line for a locally forever page with a confirmed cloud copy: "Kept forever here · cloud copy until 2026-11-18". Confirm and close, or ask for a change (#3779 checks 1–3).</p>
</tmt-review>

<tmt-review data-id="7a3e1c20-4b1d-4c8e-9f20-000000003783" data-agent="tmt-feat-lead">
  <h2 data-title>Review: answering closes the section</h2>
  <p>Answering the question below closes this section (#3779 check 4).</p>
  <tmt-question data-id="7a3e1c20-4b1d-4c8e-9f20-000000003784" data-kind="single" data-agent="tmt-feat-lead">
    <p data-question>Is the near-expiry warning wording clear?</p>
    <span data-choice>Clear</span><span data-choice>Unclear</span>
  </tmt-question>
</tmt-review>

<p class="muted">Header: "Needs you" should count the waiting reading, questions and sections and drop after each action (#3778 check 4). Check 5 on #3779 (page HTML cannot close a section) is covered by the contract tests; there is no button to try.</p>`;
