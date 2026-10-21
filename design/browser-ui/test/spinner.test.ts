import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { browserUiClasses } from '../src/static';

const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
/** The declarations of the first rule whose selector list contains `selector`, outside any @media. */
const rule = (source: string, selector: string) =>
  new RegExp(`(?:^|\\n)([^{}@]*${selector.replace(/[.[\]'>]/g, '\\$&')}[^{}]*)\\{([^}]*)\\}`).exec(
    source,
  );

it('ships one spinner: 0.9 s, linear, infinite, for the class and for a shown busy mark', () => {
  expect(browserUiClasses.spin).toBe('tmt-ui-spin');
  const main = rule(css, '.tmt-ui-spin')!;
  expect(main[1]).toContain(".tmt-ui-action-mark[data-busy='true'] > svg");
  expect(main[2].trim()).toBe('animation: tmt-ui-spin 0.9s linear infinite;');
  expect(
    css.match(/@keyframes tmt-ui-spin\s*\{\s*to\s*\{\s*transform:\s*rotate\(360deg\);/),
  ).not.toBe(null);
  // One keyframes rule: no surface brings its own.
  expect(css.match(/@keyframes/g)).toHaveLength(1);
});

it('stops under reduced motion and keeps the mark visible', () => {
  const media = /@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?)\n\}/.exec(css)!;
  expect(media[1]).toContain('.tmt-ui-spin');
  expect(media[1]).toContain(".tmt-ui-action-mark[data-busy='true'] > svg");
  expect(media[1]).toMatch(/animation:\s*none;/);
  expect(media[1]).not.toMatch(/display:\s*none|visibility:\s*hidden|opacity/);
});
