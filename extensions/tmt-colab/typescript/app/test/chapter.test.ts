import { expect, it } from 'vite-plus/test';
import { chapterMode, chapterOf } from '../src/chapter.js';

it('reads a chapter from history state only when it is well formed', () => {
  expect(chapterOf({ chapter: { scroll: 982 } })).toEqual({ scroll: 982 });
  expect(chapterOf({ chapter: { scroll: 0, full: true } })).toEqual({ scroll: 0, full: true });
  // A hand-edited or foreign entry is not a chapter.
  for (const chapter of [
    undefined,
    null,
    'x',
    {},
    { scroll: -1 },
    { scroll: Number.NaN },
    { scroll: Infinity },
    { scroll: '5' },
  ])
    expect(chapterOf({ chapter })).toBeUndefined();
  expect(chapterOf({})).toBeUndefined();
  // Only `true` means the full page; anything else is dropped.
  expect(chapterOf({ chapter: { scroll: 1, full: 'yes' } })).toEqual({ scroll: 1 });
});

it('is a panel only on a wide viewport and only until the full page is chosen', () => {
  expect(chapterMode({ scroll: 5 }, true)).toBe('panel');
  expect(chapterMode({ scroll: 5 }, false)).toBe('full');
  expect(chapterMode({ scroll: 5, full: true }, true)).toBe('full');
  expect(chapterMode(undefined, true)).toBeUndefined();
});
