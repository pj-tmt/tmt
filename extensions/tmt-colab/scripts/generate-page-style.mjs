import { readFile, writeFile } from 'node:fs/promises';

const mode = process.argv[2];
if (process.argv.length !== 3 || !['--write', '--check'].includes(mode)) {
  throw new Error('Use --write or --check');
}
const tokens = JSON.parse(
  await readFile(new URL('../../../design/tokens/tokens.json', import.meta.url), 'utf8')
);
const colours = {
  ink: tokens.browser.color.text,
  muted: tokens.browser.color.secondary,
  paper: tokens.browser.surface.paper,
  sheet: tokens.browser.surface.sheet,
  edge: tokens.browser.surface.rule,
};
const themed = (theme, indent) =>
  Object.entries(colours)
    .map(([name, value]) => `${indent}--${name}: ${value[theme].toLowerCase()};`)
    .join('\n');
// Author pages use local system fallbacks; the browser leaf's packaged fonts are not available.
const systemFont = (stack) => stack.replace(/^"[^"]+",\s*/, '').replaceAll('"', "'");
const starter = `<style>
  :root {
${themed('light', '    ')}
    --body: ${systemFont(tokens.font.body.stack)};
    --mono: ${systemFont(tokens.font.mono.stack)};
    --size: ${tokens.header['wordmark-size']};
    --heading: 28px;
    --rule: ${tokens.browser.metric['rule-width']};
    --gap: ${tokens.browser.metric['notice-heading-gap']};
    --small-gap: ${tokens.browser.metric['eyebrow-gap']};
    color-scheme: light dark;
  }
  @media (prefers-color-scheme: dark) {
    :root:not([data-theme='light']) {
${themed('dark', '      ')}
    }
  }
  :root[data-theme='light'] {
${themed('light', '    ')}
    color-scheme: light;
  }
  :root[data-theme='dark'] {
${themed('dark', '    ')}
    color-scheme: dark;
  }
  * {
    box-sizing: border-box;
  }
  body {
    margin: 0;
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
    line-height: ${tokens.header['title-line-height']};
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
`;
const begin = '<!-- BEGIN generated page style -->';
const end = '<!-- END generated page style -->';
const output = new URL('../skills/tmt-colab/SKILL.md', import.meta.url);
const skill = await readFile(output, 'utf8');
const start = skill.indexOf(begin);
const finish = skill.indexOf(end);
if (
  start < 0 ||
  finish < start ||
  skill.indexOf(begin, start + begin.length) !== -1 ||
  skill.indexOf(end, finish + end.length) !== -1
)
  throw new Error('Expected one ordered generated page style region in SKILL.md');
const generated = `${begin}\n\n\`\`\`html\n${starter}\`\`\`\n\n${end}`;
const updated = skill.slice(0, start) + generated + skill.slice(finish + end.length);
if (
  Buffer.byteLength(starter) > 3_000 ||
  Buffer.byteLength(updated) > 12_000 ||
  updated.split('\n').length - 1 > 500
) {
  throw new Error('Page style or SKILL.md exceeds its compactness budget');
}
if (mode === '--write') await writeFile(output, updated, 'utf8');
else if (skill !== updated)
  throw new Error('Generated page style is stale; run generate-page-style.mjs --write');
