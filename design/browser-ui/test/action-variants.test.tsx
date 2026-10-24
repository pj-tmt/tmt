import { staticCss } from './static-css.js';
import { expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserAction } from '../src/action.js';
import { BrowserIconAction } from '../src/icon-action.js';
import type { BrowserActionVariant } from '../src/static.js';

const css = staticCss;
const block = (selector: string) => {
  const found = css.split('\n').findIndex((line) => line === `${selector} {`);
  expect(found, selector).toBeGreaterThanOrEqual(0);
  return css.slice(css.indexOf(`${selector} {`)).split('}')[0]!;
};

it('has five variants and renders each one as its own data-variant', () => {
  const variants: BrowserActionVariant[] = ['primary', 'secondary', 'text', 'destructive', 'ghost'];
  for (const variant of variants) {
    expect(
      renderToStaticMarkup(
        <BrowserAction type="button" label="Save" variant={variant} onActivate={() => {}} />,
      ),
    ).toContain(`data-variant="${variant}"`);
    if (variant !== 'ghost')
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

it('owns one decorative leading slot without changing label or explicit disabled semantics', () => {
  const html = renderToStaticMarkup(
    <BrowserAction
      type="button"
      variant="ghost"
      label="New project"
      leadingIcon={
        <svg role="img">
          <title>Add mark</title>
        </svg>
      }
      busyMark={<svg data-busy-example="" />}
      disabled
      disabledReason="Read only"
      disabledReasonId="new-project-reason"
      onActivate={() => {}}
    />,
  );
  expect(html.match(/class="tmt-ui-action-mark"/g)).toHaveLength(1);
  expect(html).toContain('aria-hidden="true"');
  expect(html).toContain('<title>Add mark</title>');
  expect(html).not.toContain('data-busy-example');
  expect(html).toContain('disabled=""');
  expect(html).toContain('aria-describedby="new-project-reason"');
  expect(html).toContain('>New project</span>');
});
