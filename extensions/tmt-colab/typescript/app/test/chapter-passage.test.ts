import { describe, expect, it } from 'vite-plus/test';
import { chapterTargets, selectChapterTarget } from '../src/chapter-passage.js';
import type { NeedsYouCandidate } from '../src/needs-you.js';

const review = (
  id: string,
  state: 'waiting' | 'changes' | 'closed' = 'waiting',
): NeedsYouCandidate => ({ id, title: id, agent: 'Agent', kind: 'review', state });
const answer = (id: string): NeedsYouCandidate => ({
  id,
  title: id,
  agent: 'Agent',
  kind: 'answer',
  state: 'unanswered',
  targetId: `${id}-question`,
});
const reading = (id: string): Extract<NeedsYouCandidate, { kind: 'reading' }> => ({
  id,
  title: id,
  agent: 'Agent',
  kind: 'reading',
  state: 'unread',
});
const positions = (...ids: string[]) => ids.map((id, order) => ({ id, order }));

describe('Today chapter passage selection', () => {
  it('prefers a waiting review, then an unanswered group, then an unread reading', () => {
    const input = [reading('reading'), answer('group'), review('review')];
    const slots = positions('reading', 'group', 'review');
    const targets = chapterTargets(input, slots);
    expect(selectChapterTarget(targets)?.id).toBe('review');
    expect(selectChapterTarget(chapterTargets(input.slice(0, 2), slots))?.id).toBe('group');
    expect(selectChapterTarget(chapterTargets(input.slice(0, 1), slots))?.id).toBe('reading');
    expect(targets.find((target) => target.id === 'group')?.targetId).toBe('group-question');
  });

  it('uses authored order within a kind and an ID tie-break, not projection order', () => {
    const targets = chapterTargets(
      [review('z'), review('b'), review('a')],
      [
        { id: 'z', order: 200 },
        { id: 'b', order: 10 },
        { id: 'a', order: 10 },
      ],
    );
    expect(targets.map((target) => target.id)).toEqual(['a', 'b', 'z']);
  });

  it('does not select a change request awaiting the agent or a completed target initially', () => {
    const targets = chapterTargets(
      [review('closed', 'closed'), review('changes', 'changes'), reading('reading')],
      positions('closed', 'changes', 'reading'),
    );
    expect(selectChapterTarget(targets)?.id).toBe('reading');
    expect(
      selectChapterTarget(targets.filter((target) => target.kind === 'review')),
    ).toBeUndefined();
  });

  it('retains the same done passage until explicit Next, including when a higher-priority item appears', () => {
    const current = { id: 'reading', kind: 'reading' } as const;
    const targets = chapterTargets(
      [{ ...reading('reading'), state: 'read' }, review('review'), answer('group')],
      positions('reading', 'group', 'review'),
    );
    expect(selectChapterTarget(targets, current)).toMatchObject({ id: 'reading', state: 'read' });
    expect(selectChapterTarget(targets, current, true)?.id).toBe('review');
    const onlyDone = targets.filter((target) => target.id === 'reading');
    expect(selectChapterTarget(onlyDone, current, true)?.id).toBe('reading');
  });

  it('does not offer proposal actions, duplicate IDs, or targets absent from the admitted source order', () => {
    const proposal: NeedsYouCandidate = {
      id: 'proposal',
      kind: 'proposal',
      state: 'waiting',
      title: 'Proposal',
      agent: 'Agent',
    };
    const targets = chapterTargets(
      [proposal, answer('group'), answer('group'), review('missing'), reading('invalid')],
      [...positions('proposal', 'group'), { id: 'invalid', order: NaN }],
    );
    expect(targets.map((target) => target.id)).toEqual(['group']);
  });

  it('retires a selection removed from the admitted render instead of exposing its old controls', () => {
    const targets = chapterTargets([reading('new')], positions('new'));
    expect(selectChapterTarget(targets, { id: 'gone', kind: 'review' })?.id).toBe('new');
    expect(selectChapterTarget([], { id: 'gone', kind: 'review' })).toBeUndefined();
  });
});
