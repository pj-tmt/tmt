import { describe, expect, it, vi } from 'vite-plus/test';
import {
  collectionDirectory,
  collectionDetail,
  CollectionClient,
  CollectionError,
} from '../src/collection-client.js';
const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const collection = {
  id: id(1),
  name: 'Reports',
  maintainer: 'PM',
  archived: false,
  definition: { kind: 'manual', sort: 'manual' },
};
const item = { id: id(2), pageId: id(3), anchor: 'part:2', note: 'Read it', audience: 'Ben' };
const detail = { collection, items: [item], seen: [{ id: id(2), atMs: 1234 }] };
describe('strict owner collection replies', () => {
  it('accepts bounded metadata without turning malformed replies into empty lists', () => {
    expect(collectionDirectory({ kind: 'directory', collections: [collection] })).toEqual([
      collection,
    ]);
    expect(collectionDetail({ kind: 'detail', detail, changed: false }, id(1))).toEqual({
      detail,
      changed: false,
    });
    for (const value of [
      null,
      { kind: 'directory', collections: [collection, collection] },
      { kind: 'directory', collections: [], next: 'secret' },
      {
        kind: 'directory',
        collections: [{ ...collection, definition: { kind: 'manual', sort: 'random' } }],
      },
    ])
      expect(() => collectionDirectory(value)).toThrow();
  });
  it('fences the requested identity, targets, safe clock and the definition shape', () => {
    for (const altered of [
      { ...detail, collection: { ...collection, id: id(4) } },
      { ...detail, items: [item, item] },
      { ...detail, items: [{ ...item, anchor: '[id=part]' }] },
      { ...detail, seen: [{ id: id(4), atMs: 1234 }] },
      { ...detail, seen: [{ id: id(2), atMs: NaN }] },
      { ...detail, seen: [{ id: id(2), atMs: 1234, extra: 'no' }] },
      {
        ...detail,
        collection: { ...collection, definition: { kind: 'smart', search: { q: ' leading' } } },
      },
      {
        ...detail,
        collection: { ...collection, definition: { kind: 'smart', search: {}, needsYou: false } },
      },
    ])
      expect(() =>
        collectionDetail({ kind: 'detail', detail: altered, changed: false }, id(1)),
      ).toThrow();
  });
  it('does not depend on JSON field order and never treats a generic anchor as semantic authority', () => {
    const reordered = {
      definition: { sort: 'manual', kind: 'manual' },
      archived: false,
      maintainer: 'PM',
      name: 'Reports',
      id: id(1),
    };
    expect(collectionDirectory({ collections: [reordered], kind: 'directory' })).toEqual([
      collection,
    ]);
    expect(
      collectionDetail(
        {
          kind: 'detail',
          detail: { ...detail, items: [{ ...item, anchor: id(4) }] },
          changed: false,
        },
        id(1),
      ).detail.items[0].anchor,
    ).toBe(id(4));
  });
});
describe('one exchange for collection reads and writes', () => {
  it('forwards the signal and bounds the selected detail path', async () => {
    const fetcher = vi.fn<typeof fetch>(
      async () => new Response(JSON.stringify({ kind: 'detail', detail, changed: false })),
    );
    const client = new CollectionClient(new URL('http://localhost/mount/'), fetcher);
    const signal = new AbortController().signal;
    await expect(client.read(id(1), signal)).resolves.toEqual(detail);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(String(fetcher.mock.calls[0][0])).toBe(
      `http://localhost/mount/api/collections/${id(1)}`,
    );
    expect(fetcher.mock.calls[0][1]?.signal).toBe(signal);
    await expect(client.read('../other', signal)).rejects.toThrow();
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
  it('keeps a lost mutation acknowledgment unknown, with no retry', async () => {
    const fetcher = vi.fn(async () => {
      throw new Error('private payload');
    });
    const client = new CollectionClient(new URL('http://localhost/'), fetcher);
    await expect(client.change({ op: 'rename', id: id(1), name: 'New' })).rejects.toMatchObject({
      code: 'COLAB_OUTCOME_UNKNOWN',
    });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
  it('preserves an explicit owner refusal but never arbitrary error text', async () => {
    const fetcher = vi.fn(
      async () =>
        new Response(JSON.stringify({ code: 'DENIED', message: 'private data' }), { status: 403 }),
    );
    const client = new CollectionClient(new URL('http://localhost/'), fetcher);
    await expect(client.directory()).rejects.toEqual(new CollectionError('DENIED'));
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
});
