import { expect, it } from 'vite-plus/test';
import { hunksOf, SAVE_HUNKS, type SaveHunk } from '../src/save-hunks.js';

/** The serve's evaluation, in short: each hunk's old text sits once at a line start of `page`. */
function apply(page: string, hunks: SaveHunk[]): string {
  const placed = hunks.map((hunk) => {
    const at: number[] = [];
    for (let i = page.indexOf(hunk.old); i >= 0; i = page.indexOf(hunk.old, i + 1))
      if (i === 0 || page[i - 1] === '\n') at.push(i);
    expect(at, `hunk at line ${hunk.oldStart}`).toHaveLength(1);
    return { at: at[0], hunk };
  });
  let out = page;
  for (const { at, hunk } of placed.sort((x, y) => y.at - x.at))
    out = out.slice(0, at) + hunk.new + out.slice(at + hunk.old.length);
  return out;
}
const numbered = (count: number, mark = '') =>
  Array.from({ length: count }, (_, i) => `line ${i + 1}${mark}\n`).join('');

it('nothing is anchored when there is no change or no base to anchor to', () => {
  expect(hunksOf('same\n', 'same\n')).toBeUndefined();
  expect(hunksOf('', 'new\n')).toBeUndefined();
});

it('one change carries three lines of context on each side and its line as opened', () => {
  const base = numbered(20);
  const draft = base.replace('line 10\n', 'LINE ten\n');
  expect(hunksOf(base, draft)).toEqual([
    {
      oldStart: 7,
      old: 'line 7\nline 8\nline 9\nline 10\nline 11\nline 12\nline 13\n',
      new: 'line 7\nline 8\nline 9\nLINE ten\nline 11\nline 12\nline 13\n',
    },
  ]);
});

it('context stops at the ends of the source, and an insertion or a deletion is anchored too', () => {
  expect(hunksOf('a\nb\nc\n', 'A\nb\nc\n')).toEqual([
    { oldStart: 1, old: 'a\nb\nc\n', new: 'A\nb\nc\n' },
  ]);
  expect(hunksOf('a\nb\n', 'a\nb\nc\n')).toEqual([
    { oldStart: 1, old: 'a\nb\n', new: 'a\nb\nc\n' },
  ]);
  expect(hunksOf('a\nb\nc\n', 'a\nc\n')).toEqual([
    { oldStart: 1, old: 'a\nb\nc\n', new: 'a\nc\n' },
  ]);
  // A last line without its LF is part of the change when the LF is added.
  expect(hunksOf('a\nb', 'a\nb\n')).toEqual([{ oldStart: 1, old: 'a\nb', new: 'a\nb\n' }]);
});

it('changes farther apart than their context are separate hunks and closer ones are one', () => {
  const base = numbered(40);
  const apart = base.replace('line 5\n', 'five\n').replace('line 30\n', 'thirty\n');
  const hunks = hunksOf(base, apart)!;
  expect(hunks.map((hunk) => hunk.oldStart)).toEqual([2, 27]);
  expect(apply(base, hunks)).toBe(apart);
  const near = base.replace('line 5\n', 'five\n').replace('line 11\n', 'eleven\n');
  expect(hunksOf(base, near)).toHaveLength(1);
  // Contexts that only touch (six unchanged lines between) are still one hunk; seven are two.
  const touching = base.replace('line 5\n', 'five\n').replace('line 12\n', 'twelve\n');
  expect(hunksOf(base, touching)).toHaveLength(1);
  const far = base.replace('line 5\n', 'five\n').replace('line 13\n', 'thirteen\n');
  expect(hunksOf(base, far)).toHaveLength(2);
});

it('applying the hunks to the base gives the draft, and to a page changed elsewhere gives both', () => {
  const base = numbered(60);
  let seed = 7;
  const random = (limit: number) => {
    seed = (seed * 1103515245 + 12345) % 2 ** 31;
    return seed % limit;
  };
  for (let round = 0; round < 200; round++) {
    const lines = base.split(/(?<=\n)/);
    for (let edit = 0; edit < 1 + random(4); edit++) {
      const at = random(lines.length);
      const kind = random(3);
      if (kind === 0) lines.splice(at, 1);
      else if (kind === 1) lines.splice(at, 0, `inserted ${round}.${edit}\n`);
      else lines[at] = `changed ${round}.${edit}\n`;
    }
    const draft = lines.join('');
    const hunks = hunksOf(base, draft);
    if (draft === base) {
      expect(hunks).toBeUndefined();
      continue;
    }
    expect(hunks).toBeDefined();
    expect(apply(base, hunks!)).toBe(draft);
  }
  // Someone else edited line 1 and appended: the editor's hunk away from both still lands.
  const draft = base.replace('line 30\n', 'mine\n');
  const elsewhere = base.replace('line 1\n', 'theirs\n') + 'tail\n';
  expect(apply(elsewhere, hunksOf(base, draft)!)).toBe(elsewhere.replace('line 30\n', 'mine\n'));
});

it('a change that is too large to send as hunks leaves the whole source to be saved', () => {
  const base = numbered(600);
  // A change every eighth line is more hunks than a Save may carry.
  const many = base
    .split(/(?<=\n)/)
    .map((line, i) => (i % 8 === 0 ? `changed ${i}\n` : line))
    .join('');
  expect(Math.ceil(600 / 8)).toBeGreaterThan(SAVE_HUNKS);
  expect(hunksOf(base, many)).toBeUndefined();
  // One huge changed line cannot travel in a frame, so it is saved whole.
  expect(
    hunksOf(`<p>${'a'.repeat(70_000)}</p>\n`, `<p>${'b'.repeat(70_000)}</p>\n`),
  ).toBeUndefined();
  // A source of other lines throughout is past the changed-line limit.
  expect(hunksOf(numbered(3000), numbered(3000, ' x'))).toBeUndefined();
});
