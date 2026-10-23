import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserAction } from '../src/action.js';
import { BrowserIconAction } from '../src/icon-action.js';
import type { BrowserActionVariant } from '../src/static.js';

const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
const block = (selector: string) => {
  const found = css.split('\n').findIndex((line) => line === `${selector} {`);
  expect(found, selector).toBeGreaterThanOrEqual(0);
  return css.slice(css.indexOf(`${selector} {`)).split('}')[0]!;
};

it('has four variants and renders each one as its own data-variant', () => {
  const variants: BrowserActionVariant[] = ['primary', 'secondary', 'text', 'destructive'];
  for (const variant of variants) {
    expect(
      renderToStaticMarkup(
        <BrowserAction type="button" label="Save" variant={variant} onActivate={() => {}} />,
      ),
    ).toContain(`data-variant="${variant}"`);
    expect(
      renderToStaticMarkup(
        <BrowserIconAction
          type="button"
          label="Add"
          variant={variant}
          icon={<svg />}
          onActivate={() => {}}
        />,
      ),
    ).toContain(`data-variant="${variant}"`);
  }
});

it('keeps the accent fill for primary alone: secondary is the neutral bordered button', () => {
  expect(block(".tmt-ui-action[data-variant='primary']")).toContain(
    'background: var(--tmt-ui-color-accent)',
  );
  // `secondary` has no rule of its own: it is the base button, a command surface with a rule edge.
  expect(css).not.toContain("[data-variant='secondary']");
  const base = block('.tmt-ui-action');
  expect(base).toContain('background: var(--tmt-ui-surface-command)');
  expect(base).toContain('solid var(--tmt-ui-surface-rule)');
  expect(base).not.toContain('accent');
});
