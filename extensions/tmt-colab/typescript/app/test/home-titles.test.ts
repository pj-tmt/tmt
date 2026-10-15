import { expect, it, vi } from 'vite-plus/test';
import { HOME_TITLE_BYTES, HOME_TITLE_PAGES, HomeTitleHints } from '../src/home-titles.js';
import type { TitleReadBudget } from '../src/transport.js';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

it('reads visible missing rows one at a time and publishes their verified titles', async () => {
  const first = deferred<string>(),
    second = deferred<string>();
  const read = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const publish = vi.fn(),
    queue = new HomeTitleHints(read, publish);
  queue.enqueue('first', () => true);
  queue.enqueue('second', () => true);
  expect(read).toHaveBeenCalledTimes(1);
  first.resolve('First title');
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  expect(publish).toHaveBeenCalledExactlyOnceWith('first', 'First title');
  second.resolve('Second title');
  await vi.waitFor(() => expect(publish).toHaveBeenCalledTimes(2));
  await queue.close();
});

it('rechecks delayed visibility, deduplicates rows and never retries a failed row in the generation', async () => {
  const gate = deferred<string>();
  const read = vi.fn().mockReturnValueOnce(gate.promise).mockRejectedValue(new Error('Unreadable'));
  const queue = new HomeTitleHints(read, vi.fn());
  let visible = true;
  queue.enqueue('first', () => true);
  queue.enqueue('scrolled-away', () => visible);
  queue.enqueue('failed', () => true);
  queue.enqueue('failed', () => true);
  visible = false;
  gate.resolve('First');
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  expect(read.mock.calls.map(([id]) => id)).toEqual(['first', 'failed']);
  queue.enqueue('failed', () => true);
  await queue.close();
  expect(read).toHaveBeenCalledTimes(2);
});

it('caps attempted pages even when every read fails', async () => {
  const read = vi.fn(async () => {
    throw new Error('No title');
  });
  const queue = new HomeTitleHints(read, vi.fn());
  for (let id = 0; id < HOME_TITLE_PAGES + 10; id++) queue.enqueue(`${id}`, () => true);
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(HOME_TITLE_PAGES));
  await queue.close();
  expect(read).toHaveBeenCalledTimes(HOME_TITLE_PAGES);
});

it('shares the byte budget across failures and admits no further page when it is exhausted', async () => {
  const read = vi.fn(async (_id: string, budget: TitleReadBudget) => {
    expect(budget.remainingBytes).toBe(HOME_TITLE_BYTES);
    budget.remainingBytes = 0;
    throw new Error('Limit reached');
  });
  const queue = new HomeTitleHints(read, vi.fn());
  queue.enqueue('large', () => true);
  queue.enqueue('next', () => true);
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
  await queue.close();
  expect(read).toHaveBeenCalledTimes(1);
});

it('cancels and joins the in-flight read without late publication or a successor', async () => {
  const gate = deferred<string>(),
    publish = vi.fn();
  const read = vi.fn((_id: string, _budget: TitleReadBudget, signal: AbortSignal) => {
    expect(signal.aborted).toBe(false);
    return gate.promise;
  });
  const queue = new HomeTitleHints(read, publish);
  queue.enqueue('first', () => true);
  queue.enqueue('next', () => true);
  let joined = false;
  const closing = queue.close().then(() => {
    joined = true;
  });
  expect(read.mock.calls[0][2].aborted).toBe(true);
  await Promise.resolve();
  expect(joined).toBe(false);
  gate.resolve('Late title');
  await closing;
  expect(publish).not.toHaveBeenCalled();
  expect(read).toHaveBeenCalledTimes(1);
  queue.enqueue('after-close', () => true);
  expect(read).toHaveBeenCalledTimes(1);
});

it('starts a row added while an empty drain is settling', async () => {
  const read = vi.fn(async () => 'Title'),
    publish = vi.fn();
  const queue = new HomeTitleHints(read, publish);
  queue.enqueue('hidden', () => false);
  queue.enqueue('visible', () => true);
  await vi.waitFor(() => expect(publish).toHaveBeenCalledExactlyOnceWith('visible', 'Title'));
  await queue.close();
});
