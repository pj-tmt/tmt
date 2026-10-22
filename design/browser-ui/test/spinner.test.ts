import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { browserUiClasses, busyDots } from '../src/static';
import { BusyMark, busyFrames } from '../src/spinner';

const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
/** The declarations of the first rule whose selector list contains `selector`, outside any @media. */
const rule = (source: string, selector: string) =>
  new RegExp(`(?:^|\\n)([^{}@]*${selector.replace(/[.[\]'>]/g, '\\$&')}[^{}]*)\\{([^}]*)\\}`).exec(
    source,
  );

it('ships the a2 dot snake busy mark: six SVG dots, 0.9 s, eased, offset round the ring', () => {
  expect(browserUiClasses.busyMark).toBe('tmt-ui-busy-mark');
  const markup = renderToStaticMarkup(createElement(BusyMark));
  expect(markup).toContain('aria-hidden="true"');
  expect(markup).toContain('<svg viewBox="0 0 16 16">');
  expect(markup.match(/<circle class="tmt-ui-busy-dot"/g)).toHaveLength(6);
  // No inline style: the mark works under a strict style policy, and the host's text is unchanged.
  expect(markup).not.toContain('style=');
  expect(markup.replace(/<[^>]+>/g, '')).toBe('');
  expect(busyDots).toEqual([
    [5, 3],
    [11, 3],
    [11, 8],
    [11, 13],
    [5, 13],
    [5, 8],
  ]);
  const dot = rule(css, '.tmt-ui-busy-dot')!;
  expect(dot[2]).toContain(
    'animation: tmt-ui-busy var(--tmt-ui-spin-duration) ease-in-out infinite;',
  );
  expect(css).toContain('--tmt-ui-spin-duration: 0.9s;');
  // Each dot's lap starts a sixth of a lap after the one before it in the ring.
  for (let n = 1; n <= 6; n++)
    expect(css).toMatch(
      new RegExp(
        `\\.tmt-ui-busy-dot:nth-child\\(${n}\\) \\{\\s*animation-delay: calc\\(var\\(--tmt-ui-spin-duration\\) \\* -`,
      ),
    );
});

it('keeps the braille snake as the text and TUI parity of the mark', () => {
  expect(busyFrames).toEqual(['⠙', '⠸', '⠴', '⠦', '⠇', '⠋']);
  // Each frame lights three neighbouring dots of the 2x3 cell (braille dots 1-6, no 7 or 8).
  for (const frame of busyFrames) {
    const dots = frame.codePointAt(0)! - 0x2800;
    expect(dots & 0xc0, frame).toBe(0);
    expect(dots.toString(2).replace(/0/g, ''), frame).toHaveLength(3);
  }
});

it('holds one still frame under reduced motion and keeps the mark visible', () => {
  const media = [...css.matchAll(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?)\n\}/g)]
    .map((match) => match[1]!)
    .find((body) => body.includes('.tmt-ui-busy-dot'))!;
  expect(media).toMatch(/animation:\s*none;/);
  // The still frame lights the top-left three dots; nothing hides the mark.
  expect(media).toMatch(
    /nth-child\(1\),\s*\.tmt-ui-busy-dot:nth-child\(5\),\s*\.tmt-ui-busy-dot:nth-child\(6\)\s*\{\s*opacity:\s*1;/,
  );
  expect(media).not.toMatch(/display:\s*none|visibility:\s*hidden/);
});
