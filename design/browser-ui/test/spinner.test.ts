import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { browserUiClasses } from '../src/static';
import { BusyMark, busyFrames } from '../src/spinner';

const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
/** The declarations of the first rule whose selector list contains `selector`, outside any @media. */
const rule = (source: string, selector: string) =>
  new RegExp(`(?:^|\\n)([^{}@]*${selector.replace(/[.[\]'>]/g, '\\$&')}[^{}]*)\\{([^}]*)\\}`).exec(
    source,
  );

it('ships the braille busy mark: ten frames, 0.9 s, stepped, infinite', () => {
  expect(browserUiClasses.busyMark).toBe('tmt-ui-busy-mark');
  expect(busyFrames).toEqual(['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏']);
  const markup = renderToStaticMarkup(createElement(BusyMark));
  expect(markup).toContain('aria-hidden="true"');
  expect(markup.match(/data-frame="[⠀-⣿]"/g)).toHaveLength(10);
  // The frames are drawn by CSS, so the host's text is unchanged.
  expect(markup.replace(/<[^>]+>/g, '')).toBe('');
  const frames = rule(css, '.tmt-ui-busy-frames')!;
  expect(frames[2]).toContain(
    'animation: tmt-ui-busy var(--tmt-ui-spin-duration) steps(10) infinite;',
  );
  expect(css).toContain('--tmt-ui-spin-duration: 0.9s;');
  expect(
    css.match(/@keyframes tmt-ui-busy\s*\{\s*to\s*\{\s*transform:\s*translateY\(-10em\);/),
  ).not.toBe(null);
});

it('holds the first frame under reduced motion and keeps the mark visible', () => {
  const media = [...css.matchAll(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?)\n\}/g)]
    .map((match) => match[1]!)
    .find((body) => body.includes('.tmt-ui-busy-frames'))!;
  expect(media).toMatch(/animation:\s*none;/);
  expect(media).not.toMatch(/display:\s*none|visibility:\s*hidden|opacity/);
});
