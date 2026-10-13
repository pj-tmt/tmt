import { expect, it } from 'vite-plus/test';
import { conflictAt, saveMessage, saveResult, SaveConflict } from '../src/save.js';
import { text } from '../src/strings.js';

const frame = (fields: Record<string, unknown>) => ({
  version: 1,
  type: 'saveresult',
  space: 's',
  page: 'p',
  epoch: '1',
  operationId: 'o',
  ...fields,
});
const DIGEST = 'A'.repeat(43);

it('a merged commit names the source it holds, and only a commit may', () => {
  const merged = saveResult(
    frame({ state: 'committed', revision: 'v1:abc', merged: true, sourceSha256: DIGEST }),
    'o',
  );
  expect(merged).toMatchObject({ state: 'committed', merged: true, sourceSha256: DIGEST });
  expect(saveResult(frame({ state: 'committed', revision: 'v1:abc' }), 'o').merged).toBeUndefined();
  for (const broken of [
    { state: 'committed', revision: 'v1:abc', merged: true },
    { state: 'committed', revision: 'v1:abc', merged: false, sourceSha256: DIGEST },
    { state: 'committed', revision: 'v1:abc', merged: true, sourceSha256: 'short' },
    { state: 'committed', revision: 'v1:abc', sourceSha256: DIGEST },
    { state: 'unchanged', revision: 'v1:abc', merged: true, sourceSha256: DIGEST },
  ])
    expect(() => saveResult(frame(broken), 'o'), JSON.stringify(broken)).toThrow();
});

it('the conflicting hunks come only with a patch conflict, as ascending distinct indices', () => {
  const rejected = { state: 'rejected', code: 'COLAB_PATCH_CONFLICT', message: 'x' };
  expect(saveResult(frame({ ...rejected, conflicts: [0, 3] }), 'o').conflicts).toEqual([0, 3]);
  for (const conflicts of [[], [3, 0], [1, 1], [-1], [64], [0.5], 'a', [null]])
    expect(
      () => saveResult(frame({ ...rejected, conflicts }), 'o'),
      JSON.stringify(conflicts),
    ).toThrow();
  expect(() =>
    saveResult(frame({ state: 'rejected', code: 'COLAB_STALE_BASE', conflicts: [0] }), 'o'),
  ).toThrow();
  expect(() => saveResult(frame({ state: 'absent', conflicts: [0] }), 'o')).toThrow();
});

it('a conflict is named by the first changed line of its region that holds text, as inert text', () => {
  // The context above the change is not what changed: the quote is the line the editor edited.
  expect(
    conflictAt({
      oldStart: 10,
      old: 'a\nb\nc\nTotal: 4\ne\n',
      new: 'a\nb\nc\nTotal: 5\ne\n',
    }),
  ).toEqual({ line: 13, excerpt: 'Total: 4' });
  // An insertion is named by the line it goes before; one at the end by the last line.
  expect(conflictAt({ oldStart: 3, old: 'a\nb\n', new: 'a\nnew\nb\n' })).toEqual({
    line: 4,
    excerpt: 'b',
  });
  expect(conflictAt({ oldStart: 3, old: 'a\nb\n', new: 'a\nb\nnew\n' })).toEqual({
    line: 4,
    excerpt: 'b',
  });
  // Blank lines do not name a region: the next line with text does, with its own line number.
  expect(
    conflictAt({
      oldStart: 10,
      old: 'ctx\n\n  \n\t<p>Hi   there</p>\n',
      new: 'ctx\nZ\n  \n\t<p>Hi   there</p>\n',
    }),
  ).toEqual({ line: 13, excerpt: '<p>Hi there</p>' });
  // Control and bidi characters are removed, so a quote cannot reorder or hide the text around it.
  expect(
    conflictAt({ oldStart: 1, old: 'a\u202eb\u2066c\u200fd\u0007e\u061cf\n', new: '' }).excerpt,
  ).toBe('abcdef');
  // Clipped to 60 characters (not code units) with an ellipsis.
  const long = '🐈'.repeat(61);
  expect(conflictAt({ oldStart: 1, old: `${long}\n`, new: '' }).excerpt).toBe(
    `${'🐈'.repeat(60)}…`,
  );
  expect(conflictAt({ oldStart: 1, old: `${'x'.repeat(60)}\n`, new: '' }).excerpt).toBe(
    'x'.repeat(60),
  );
  // Nothing but blank lines: the line number alone.
  expect(conflictAt({ oldStart: 4, old: '\n\n', new: '' })).toEqual({ line: 4, excerpt: '' });
});

it('the conflict sentence names one edit or several, quoting the first', () => {
  const one = saveMessage(new SaveConflict(1, { line: 12, excerpt: 'Total: 4' }));
  expect(one).toBe(
    'Nothing was saved. Your edit near “Total: 4” (line 12) conflicts with a change made elsewhere. Your draft is kept. Copy it, reload for the latest source, then apply that part again.',
  );
  expect(saveMessage(new SaveConflict(3, { line: 12, excerpt: 'Total: 4' }))).toBe(
    'Nothing was saved. 3 of your edits conflict with changes made elsewhere, the first near “Total: 4” (line 12). Your draft is kept. Copy it, reload for the latest source, then apply those parts again.',
  );
  expect(saveMessage(new SaveConflict(1, { line: 4, excerpt: '' }))).toContain(
    'Your edit at line 4 conflicts',
  );
  expect(text.saveMerged).toBe('Saved. Your edit was merged with a change made elsewhere.');
});
