import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';

const read = (name: string) =>
  readFileSync(new URL(`../generated/${name}.css`, import.meta.url), 'utf8');
const editorial = read('editorial');

/** The top-level selectors of a stylesheet whose rules nest under them. */
function topLevel(css: string): string[] {
  const selectors: string[] = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < css.length; index += 1) {
    const char = css[index];
    if (char === '{') {
      if (depth === 0) selectors.push(css.slice(start, index).trim());
      depth += 1;
    } else if (char === '}') {
      depth -= 1;
      if (depth === 0) start = index + 1;
    }
  }
  return selectors;
}

it('scopes every prose rule under the one zero-specificity container a host can rescope', () => {
  const css = editorial.replace(/\/\*[\s\S]*?\*\//g, '');
  expect(topLevel(css)).toEqual([
    ':where(.tmt-ui-prose)',
    '.tmt-ui-tag',
    'a.tmt-ui-tag:hover',
    ':where(.tmt-ui-prose)',
  ]);
  expect(css.split('.tmt-ui-prose')).toHaveLength(3);
});

it('ships the prose table from the table rules, not a second table style', () => {
  expect(editorial).toMatch(/& table\{[^}]*border-collapse:collapse/);
  expect(editorial).toContain('& table[data-numeric]');
  expect(editorial).not.toMatch(/\.tmt-ui-table/);
});

it('reads only ramp parts that the shared tokens define', () => {
  const statics = read('static');
  const used = new Set(editorial.match(/--tmt-ui-type-[a-z0-9-]+/g));
  expect(used.size).toBeGreaterThan(10);
  for (const name of used) expect(statics, name).toContain(`${name}:`);
  // The h2 the Colab starter used to hard-code is now the ramp's own size.
  expect(statics).toContain('--tmt-ui-type-h2-size: 22px;');
});
