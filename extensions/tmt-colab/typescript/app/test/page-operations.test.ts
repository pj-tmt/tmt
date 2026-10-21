import { describe, expect, it } from 'vite-plus/test';
import { ManagementError, type ManagementPort } from '../src/management.js';
import { operationStatus } from '../src/page-operation-status.js';
import { runPageOperation, runPageOperations } from '../src/page-operations.js';
import type { PageSummary } from '../src/transport.js';

const page = (n: number, title = `Page ${n}`) =>
  ({
    id: `00000000-0000-4000-8000-00000000000${n}`,
    title,
    sharing: 'private',
    archived: false,
    lastUpdateAtMs: 0,
    retentionDays: 30,
    expiresAtMs: null,
    warnings: [],
  }) as PageSummary;

/** A port whose steps run in order and record what they were given. */
function port(options: { refuse?: string; unverified?: string } = {}) {
  const sent: { operation: string; pageId: string }[] = [];
  const steps: string[] = [];
  const fake = {
    read: async (pageId: string) => {
      steps.push(`read ${pageId}`);
      return { pageId };
    },
    prepare: async (
      _view: unknown,
      selection: { operation: string; value: { pageId: string } },
    ) => {
      steps.push(`prepare ${selection.value.pageId}`);
      return selection;
    },
    send: async (pending: { operation: string; value: { pageId: string } }) => {
      steps.push(`send ${pending.value.pageId}`);
      if (pending.value.pageId === options.refuse) throw new ManagementError('DENIED');
      sent.push({ operation: pending.operation, pageId: pending.value.pageId });
      return {};
    },
    verify: async (pending: { value: { pageId: string } }) => {
      steps.push(`verify ${pending.value.pageId}`);
      if (pending.value.pageId === options.unverified) throw new Error('log unreadable');
      return {};
    },
  } as unknown as ManagementPort;
  return { fake, sent, steps };
}

describe('one page', () => {
  it.each(['page.archive', 'page.unarchive', 'page.delete'] as const)(
    'reads, prepares, sends and verifies %s',
    async (operation) => {
      const { fake, sent, steps } = port();
      const [a] = [page(1)];
      expect(await runPageOperation(fake, a.id, operation)).toEqual({ state: 'verified' });
      expect(steps).toEqual([`read ${a.id}`, `prepare ${a.id}`, `send ${a.id}`, `verify ${a.id}`]);
      expect(sent).toEqual([{ operation, pageId: a.id }]);
    },
  );
  it('is awaiting, not done, when the owner acknowledged but its log cannot be read back', async () => {
    const { fake } = port({ unverified: page(1).id });
    expect(await runPageOperation(fake, page(1).id, 'page.delete')).toEqual({ state: 'awaiting' });
  });
  it('fails with the owner code when the statement is refused', async () => {
    const { fake } = port({ refuse: page(1).id });
    const outcome = await runPageOperation(fake, page(1).id, 'page.delete');
    expect(outcome.state).toBe('failed');
    expect(outcome.state === 'failed' && outcome.failure.code).toBe('DENIED');
  });
});

describe('a batch', () => {
  it('sends one statement at a time, in order', async () => {
    const { fake, steps } = port();
    const pages = [page(1), page(2), page(3)];
    const outcome = await runPageOperations(fake, pages, 'page.archive');
    expect(outcome.done).toEqual(pages);
    expect(outcome.failed).toBeUndefined();
    expect(steps.filter((step) => step.startsWith('send'))).toEqual(
      pages.map(({ id }) => `send ${id}`),
    );
    // The next page's read waits for the previous page's verification.
    expect(steps.indexOf(`verify ${pages[0].id}`)).toBeLessThan(
      steps.indexOf(`read ${pages[1].id}`),
    );
  });
  it('stops at the first refusal and never tries the pages after it', async () => {
    const pages = [page(1), page(2), page(3)];
    const { fake, sent, steps } = port({ refuse: pages[1].id });
    const outcome = await runPageOperations(fake, pages, 'page.delete');
    expect(outcome.done).toEqual([pages[0]]);
    expect(outcome.failed?.page).toBe(pages[1]);
    expect(sent).toEqual([{ operation: 'page.delete', pageId: pages[0].id }]);
    expect(steps.some((step) => step.includes(pages[2].id))).toBe(false);
  });
  it('carries on past an acknowledged statement it could not verify, and says which', async () => {
    const pages = [page(1), page(2)];
    const { fake } = port({ unverified: pages[0].id });
    const outcome = await runPageOperations(fake, pages, 'page.archive');
    expect(outcome.done).toEqual(pages);
    expect(outcome.awaiting).toEqual([pages[0]]);
  });
});

describe('what the status line says', () => {
  const done = (pages: PageSummary[], rest: object = {}) => ({
    done: pages,
    awaiting: [],
    ...rest,
  });
  it('names a single page and counts a batch', () => {
    expect(operationStatus(done([page(1, 'Plan')]), 'page.archive', 1)).toEqual({
      text: 'Archived “Plan”.',
      failed: false,
    });
    expect(operationStatus(done([page(1, 'Plan')]), 'page.unarchive', 1).text).toBe(
      'Restored “Plan”.',
    );
    expect(operationStatus(done([page(1, 'Plan')]), 'page.delete', 1).text).toBe('Deleted “Plan”.');
    expect(operationStatus(done([page(1), page(2)]), 'page.archive', 2).text).toBe(
      'Archived 2 pages.',
    );
    expect(operationStatus(done([page(1), page(2)]), 'page.delete', 2).text).toBe(
      'Deleted 2 pages.',
    );
  });
  it('never calls an unverified acknowledgment done', () => {
    const single = operationStatus(
      done([page(1, 'Plan')], { awaiting: [page(1, 'Plan')] }),
      'page.archive',
      1,
    );
    expect(single.text).toContain('not verified yet');
    expect(single.text).not.toMatch(/^Archived/);
    const many = operationStatus(
      done([page(1), page(2)], { awaiting: [page(2)] }),
      'page.delete',
      2,
    );
    expect(many.text).toContain('not verified yet');
  });
  it('reports how far a stopped batch got and which page stopped it', () => {
    const stopped = operationStatus(
      done([page(1)], {
        failed: { page: page(2, 'Beta'), failure: new ManagementError('DENIED') },
      }),
      'page.delete',
      3,
    );
    expect(stopped.failed).toBe(true);
    expect(stopped.text).toBe(
      'Updated 1 of 3 pages. “Beta” was not changed: This device is not allowed to manage pages. Reopen your paired session.',
    );
  });
  it('says only the reason when the one page was refused', () => {
    const refused = operationStatus(
      {
        done: [],
        awaiting: [],
        failed: { page: page(1), failure: new ManagementError('CAPACITY') },
      },
      'page.archive',
      1,
    );
    expect(refused).toEqual({
      text: 'The owner reached a capacity limit. Nothing was changed.',
      failed: true,
    });
  });
  it('falls back to the unknown-result text for a code it has no wording for', () => {
    const odd = operationStatus(
      { done: [], awaiting: [], failed: { page: page(1), failure: new ManagementError('ODD') } },
      'page.archive',
      1,
    );
    expect(odd.text).toContain('result is unknown');
  });
});
