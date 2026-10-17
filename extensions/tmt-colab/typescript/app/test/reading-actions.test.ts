import { expect, it, vi } from 'vite-plus/test';
import { ReadingActions, type ReadingOutcome } from '../src/reading-actions.js';
import { AdmittedView } from '../src/review-actions.js';
import type { Reading } from '../src/questions.js';
import type { CommentContext, ThreadBinding } from '../src/thread-store.js';
import type { ThreadStatusCoordinator } from '../src/thread-status-coordinator.js';
import type { ThreadView } from '../src/thread-records.js';

const reading: Reading = {
  id: '10000000-0000-4000-8000-000000000401',
  title: 'Hosting',
  agent: 'alpha',
  why: 'Please read',
};
function fixture() {
  const context: CommentContext = {
    thread: { writer: 'device', id: 'thread' },
    message: { writer: 'device', id: 'comment' },
    threadRevision: '1',
    messageRevision: '1',
  };
  let thread: ThreadView | undefined;
  let current = true;
  const outcomes: ReadingOutcome[] = [];
  const create = vi.fn(async (body: string) => {
    thread = {
      ref: context.thread,
      revision: '1',
      comments: [
        {
          ref: context.message,
          revision: '1',
          sequence: '1',
          deviceAuthor: true,
          deleted: false,
          body,
        },
      ],
      resolved: false,
      deleted: false,
    } as ThreadView;
    return context;
  });
  const binding = { create, reply: vi.fn() } as unknown as ThreadBinding;
  const change = vi.fn(async (_thread: ThreadView, resolved: boolean) => {
    thread = { ...thread!, resolved, status: { resolved, actor: 'person' } } as ThreadView;
    return { changed: true, status: thread.status, warnings: [] };
  });
  const working = vi.fn();
  const actions = new ReadingActions({
    working,
    thread: () => thread,
    selector: () => ({ type: 'TextQuoteSelector', exact: 'Hosting', prefix: '', suffix: '' }),
    threadHolding: () => thread,
    discussion: () => binding,
    status: () => ({ change }) as unknown as ThreadStatusCoordinator,
    current: () => current,
    title: () => 'Page',
    admitted: new AdmittedView(),
    changed: (_id, outcome) => outcomes.push(outcome),
  });
  return {
    actions,
    binding,
    create,
    change,
    outcomes,
    working,
    stale: () => {
      current = false;
    },
    thread: () => thread,
  };
}
it('coalesces double clicks, records then resolves with only the named agent, and never repeats a completed read', async () => {
  const f = fixture();
  await Promise.all([f.actions.mark(reading), f.actions.mark(reading)]);
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(f.create.mock.calls[0][0]).toContain(`Reading: ${reading.id}`);
  expect(f.change).toHaveBeenCalledTimes(1);
  expect(f.change.mock.calls[0]).toEqual([expect.anything(), true, { onlyAgentName: 'alpha' }]);
  expect(f.outcomes.at(-1)).toEqual({ state: 'settled', step: 'closing' });
  await f.actions.mark(reading);
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(f.change).toHaveBeenCalledTimes(1);
});
it('an explicit retry reuses the comment after a failed Resolve', async () => {
  const f = fixture();
  f.change.mockRejectedValueOnce(new Error('Refused'));
  await f.actions.mark(reading);
  expect(f.outcomes.at(-1)).toEqual({ state: 'failed', step: 'closing' });
  await f.actions.mark(reading);
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(f.change).toHaveBeenCalledTimes(2);
  expect(f.outcomes.at(-1)?.state).toBe('settled');
});
it('a page transition after the comment lands cannot resolve or notify in the old page', async () => {
  const f = fixture();
  const record = f.create.getMockImplementation()!;
  f.create.mockImplementationOnce(async (body) => {
    const context = await record(body);
    f.stale();
    return context;
  });
  await f.actions.mark(reading);
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(f.change).not.toHaveBeenCalled();
  expect(f.outcomes.some((outcome) => outcome.state === 'settled')).toBe(false);
});

it('joins action lifetime even when its final presentation is fenced by a page transition', async () => {
  const f = fixture();
  const record = f.create.getMockImplementation()!;
  f.create.mockImplementationOnce(async (body) => {
    const context = await record(body);
    f.stale();
    return context;
  });
  await f.actions.mark(reading);
  expect(f.working.mock.calls).toEqual([
    [reading.id, true],
    [reading.id, false],
  ]);
  expect(f.change).not.toHaveBeenCalled();
  expect(f.outcomes.some((outcome) => outcome.state === 'settled')).toBe(false);
});
