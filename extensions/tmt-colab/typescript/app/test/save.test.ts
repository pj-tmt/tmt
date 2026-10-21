import { expect, it } from 'vite-plus/test';
import {
  conflictAt,
  saveMessage,
  saveResult,
  SaveConflict,
  SaveNotApplied,
  SaveRefused,
  settledAgainst,
  workResult,
} from '../src/save.js';
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

it('a page that kept moving while its hunks were evaluated is still the stale-base sentence', () => {
  expect(saveMessage(new SaveRefused('COLAB_STALE_BASE', 'Moved.'))).toBe(text.saveStale);
  expect(saveMessage(new SaveRefused('COLAB_PATCH_CONFLICT', 'No match.'))).toBe('No match.');
});

it('a refusal that names hunks which no longer fit is the conflict the first one anchors', () => {
  const hunks = [
    { oldStart: 3, old: 'a\n', new: 'b\n' },
    { oldStart: 9, old: 'Total: 4\n', new: 'Total: 5\n' },
  ];
  const refused = (conflicts?: number[]) =>
    saveResult(
      frame({
        state: 'rejected',
        code: conflicts ? 'COLAB_PATCH_CONFLICT' : 'COLAB_STALE_BASE',
        message: 'The page changed.',
        ...(conflicts ? { conflicts } : {}),
      }),
      'o',
    );
  expect(() => settledAgainst(refused([1]), hunks)).toThrow(SaveConflict);
  try {
    settledAgainst(refused([1]), hunks);
  } catch (error) {
    expect((error as SaveConflict).count).toBe(1);
    expect((error as SaveConflict).first).toEqual(conflictAt(hunks[1]));
  }
  // Without conflicts named, or when the save sent no hunks, the refusal is the refusal.
  expect(() => settledAgainst(refused(), hunks)).toThrow(SaveRefused);
  expect(() => settledAgainst(refused([1]))).toThrow(SaveRefused);
  // A commit passes through untouched.
  const committed = saveResult(frame({ state: 'committed', revision: '7' }), 'o');
  expect(settledAgainst(committed, hunks)).toBe(committed);
});

it('a work reply is a save reply plus the ID a committed task.add made', () => {
  const reply = (fields: Record<string, unknown>) => ({ ...frame(fields), type: 'workresult' });
  expect(
    workResult(reply({ state: 'committed', revision: 'v1:abc', taskId: 'T-12' }), 'o'),
  ).toMatchObject({ state: 'committed', revision: 'v1:abc', taskId: 'T-12' });
  expect(workResult(reply({ state: 'unchanged', revision: 'v1:abc' }), 'o').taskId).toBeUndefined();
  expect(
    workResult(
      reply({ state: 'rejected', code: 'COLAB_WIP_LIMIT', message: 'Atlas would hold T-1' }),
      'o',
    ),
  ).toMatchObject({ state: 'rejected', code: 'COLAB_WIP_LIMIT' });
  for (const broken of [
    { state: 'committed', revision: 'v1:abc', taskId: 'x' },
    { state: 'committed', revision: 'v1:abc', taskId: 12 },
    { state: 'unchanged', revision: 'v1:abc', taskId: 'T-1' },
    { state: 'rejected', code: 'COLAB_INPUT_INVALID', taskId: 'T-1' },
    { state: 'committed', revision: 'v1:abc', extra: true },
  ])
    expect(() => workResult(reply(broken), 'o'), JSON.stringify(broken)).toThrow();
  expect(() => workResult(reply({ state: 'committed', revision: 'v1:abc' }), 'other')).toThrow();
});

it('a convert names the page it made in every final outcome, and nowhere else', () => {
  const page = '20000000-0000-4000-8000-000000000009';
  const reply = (fields: Record<string, unknown>) => ({ ...frame(fields), type: 'workresult' });
  expect(
    workResult(reply({ state: 'committed', revision: 'v1:abc', childPage: page }), 'o'),
  ).toMatchObject({ state: 'committed', childPage: page });
  // A made page whose link did not land: rejected with the page, or absent with the page.
  const refused = workResult(
    reply({ state: 'rejected', code: 'COLAB_LINK_REFUSED', childPage: page }),
    'o',
  );
  expect(() => settledAgainst(refused)).toThrow(SaveRefused);
  try {
    settledAgainst(refused);
  } catch (error) {
    expect((error as SaveRefused).childPage).toBe(page);
  }
  const absent = saveResult(frame({ state: 'absent', childPage: page }), 'o');
  try {
    settledAgainst(absent);
    expect.unreachable();
  } catch (error) {
    expect(error).toBeInstanceOf(SaveNotApplied);
    expect((error as SaveNotApplied).childPage).toBe(page);
  }
  for (const broken of [
    { state: 'unchanged', revision: 'v1:abc', childPage: page },
    { state: 'pending', childPage: page },
    { state: 'committed', revision: 'v1:abc', childPage: 'not-an-id' },
    { state: 'committed', revision: 'v1:abc', childPage: 7 },
    { state: 'committed', revision: 'v1:abc', childPage: page.replace('-4000-', '-1000-') },
  ])
    expect(() => saveResult(frame(broken), 'o'), JSON.stringify(broken)).toThrow();
});
