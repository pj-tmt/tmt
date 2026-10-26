import { readFileSync } from 'node:fs';
import { expect, test } from 'vite-plus/test';

const guide = readFileSync(
  new URL('../../../skills/tmt-colab/references/components.md', import.meta.url),
  'utf8',
);
const names = JSON.parse(
  readFileSync(
    new URL('../../../rust/tmt-colab/src/page_check/page_ui.json', import.meta.url),
    'utf8',
  ),
) as { classes: string[]; icons: string[] };
const section = guide.slice(guide.indexOf('## Design system'), guide.indexOf('## Text'));

test('every class and icon the design-system guide shows is one the frame and `page check` know', () => {
  const used = new Set(section.match(/\btmt-ui-[a-z0-9-]+/g));
  expect(used.size).toBeGreaterThan(10);
  expect([...used].filter((name) => !names.classes.includes(name))).toEqual([]);
  const icons = [...section.matchAll(/<tmt-icon name="([^"]+)"/g)].map((match) => match[1]!);
  expect(icons.length).toBeGreaterThan(0);
  expect(icons.filter((name) => !names.icons.includes(name))).toEqual([]);
});
