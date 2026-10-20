import { expect, it } from 'vite-plus/test';
import { shareView } from '../src/share-view.js';

const page = () => ({
  source: 'x'.repeat(1000),
  title: 'T',
  threads: [
    { id: 'a', comments: [{ body: 'one' }] },
    { id: 'b', comments: [{ body: 'two' }] },
  ],
  own: { w: { threads: { k: new Uint8Array([1, 2, 3]) } } },
  nothing: undefined,
});

it('returns the previous object when nothing changed, so a fresh copy costs no render', () => {
  const previous = page();
  expect(shareView(previous, page())).toBe(previous);
});

it('keeps every unchanged part and replaces only the path to the changed one', () => {
  const previous = page();
  const next = page();
  next.threads[1]!.comments.push({ body: 'a reply' });
  const shared = shareView(previous, next);
  expect(shared).not.toBe(previous);
  expect(shared.threads).not.toBe(previous.threads);
  expect(shared.threads[0]).toBe(previous.threads[0]);
  expect(shared.threads[1]).not.toBe(previous.threads[1]);
  expect(shared.threads[1]!.comments[0]).toBe(previous.threads[1]!.comments[0]);
  expect(shared.threads[1]!.comments).toHaveLength(2);
  expect(shared.own).toBe(previous.own);
  expect(shared.source).toBe(previous.source);
  expect(shared).toEqual(next);
});

it('handles added and removed keys, shorter arrays and unrelated shapes without sharing wrongly', () => {
  const previous = page();
  const next = { ...page(), extra: 1 } as ReturnType<typeof page> & { extra: number };
  expect(shareView(previous, next)).toEqual(next);
  expect(shareView(previous, next).threads).toBe(previous.threads);
  const fewer = page();
  fewer.threads.pop();
  const shared = shareView(previous, fewer);
  expect(shared.threads).toHaveLength(1);
  expect(shared.threads[0]).toBe(previous.threads[0]);
  expect(shareView(5, 6)).toBe(6);
  expect(shareView({ a: 1 }, [1])).toEqual([1]);
  const bytes = new Uint8Array([1, 2, 3]);
  expect(shareView(bytes, new Uint8Array([1, 2, 3]))).toBe(bytes);
  const changed = new Uint8Array([1, 2, 4]);
  expect(shareView(bytes, changed)).toBe(changed);
});
