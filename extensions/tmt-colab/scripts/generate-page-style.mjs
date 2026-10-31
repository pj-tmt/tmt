import { readFile, writeFile } from 'node:fs/promises';

const mode = process.argv[2];
if (process.argv.length !== 3 || !['--write', '--check'].includes(mode)) {
  throw new Error('Use --write or --check');
}
const tokens = JSON.parse(
  await readFile(new URL('../../../design/tokens/tokens.json', import.meta.url), 'utf8')
);
const metric = tokens.browser.metric;
// The design system (tokens, fonts, components) and the editorial layer (#5794: type ramp, prose rhythm,
// tables, figures, code, tags) are injected by the page frame (#5057); the starter is only the glue a page
// needs on top of them: the page column and section spacing. Its `tmt-starter` layer, which the frame's
// editorial rules share, also marks the page as a design-system page, so they apply; the design system's
// classes and an author's own CSS both win over it.
const starter = `<style>
@layer tmt-starter {
  * { box-sizing: border-box; }
  body {
    max-width: 1280px;
    margin: 0 auto;
    padding: ${metric['notice-heading-gap']};
  }
  section { margin: 0 0 ${metric['notice-heading-gap']}; }
  section > :last-child { margin-bottom: 0; }
  .muted { color: var(--tmt-ui-color-secondary); }
  .waiting { color: var(--tmt-ui-color-waiting); }
}
</style>
<!-- Examples to copy with fresh UUIDs (this comment is inert); see tmt colab skill, "Ask and flag"
<tmt-questions data-id="20000000-0000-4000-8000-000000000001">
  <p data-title>One decision</p>
  <tmt-question data-id="10000000-0000-4000-8000-000000000001" data-kind="single" data-agent="Your agent name">
    <p data-question>Which option?</p>
    <span data-choice>First</span><span data-choice>Second</span>
  </tmt-question>
</tmt-questions>
<tmt-read data-id="10000000-0000-4000-8000-000000000002" data-agent="Your agent name" data-why="Read before choosing">
  <h2 data-title>Requirements</h2>
  <p>The passage.</p>
</tmt-read>
-->
`;
const begin = '<!-- BEGIN generated page style -->';
const end = '<!-- END generated page style -->';
const output = new URL('../skills/tmt-colab/references/page-style.md', import.meta.url);
const skill = await readFile(output, 'utf8');
const start = skill.indexOf(begin);
const finish = skill.indexOf(end);
if (
  start < 0 ||
  finish < start ||
  skill.indexOf(begin, start + begin.length) !== -1 ||
  skill.indexOf(end, finish + end.length) !== -1
)
  throw new Error('Expected one ordered generated page style region in references/page-style.md');
const generated = `${begin}\n\n\`\`\`html\n${starter}\`\`\`\n\n${end}`;
const updated = skill.slice(0, start) + generated + skill.slice(finish + end.length);
if (
  Buffer.byteLength(starter.slice(0, starter.indexOf('<!-- Examples to copy'))) > 2_000 ||
  Buffer.byteLength(updated) > 12_000 ||
  updated.split('\n').length - 1 > 500
) {
  throw new Error('Page style reference exceeds its compactness budget');
}
if (mode === '--write') await writeFile(output, updated, 'utf8');
else if (skill !== updated)
  throw new Error('Generated page style is stale; run generate-page-style.mjs --write');
