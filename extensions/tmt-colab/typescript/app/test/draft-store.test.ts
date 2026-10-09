import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';
import type { ComposerEdit } from '../src/components/message-composer-edit.js';
import { DRAFTS_PER_PAGE, DraftSession, LocalDraftStore, persistable } from '../src/draft-store.js';
import { draftKey, titleKey } from '../src/keyring.js';
import { COMMENT_BYTES } from '../src/thread-records.js';

const stored = new Map<string, unknown>();
let unavailable = false;
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (unavailable) throw new Error('Storage unavailable');
    if (values.length) stored.set(key, structuredClone(values[0]));
    else return structuredClone(stored.get(key));
  },
}));
beforeEach(() => {
  stored.clear();
  unavailable = false;
  vi.stubGlobal('navigator', {
    locks: { request: async (_name: string, action: () => unknown) => action() },
  });
});
afterEach(() => vi.useRealTimers());

const RECORD = 'drafts:space:device:page';
const store = (space = 'space', device = 'device') => new LocalDraftStore(space, device);
const edit = (value: string, extra: Partial<ComposerEdit> = {}): ComposerEdit => ({
  value,
  ...extra,
});

it('encrypts drafts with a key kept apart from title hints and restores exact bytes', async () => {
  const body = 'Line one\nLine 🐈 <script>two</script>';
  const mention = {
    key: { machine: 'machine-1', agent: 'agent-1' },
    range: { start: 0, end: 6 },
  };
  expect(
    await store().apply('page', new Map([['{"exact":"q"}', edit(body, { mentions: [mention] })]])),
  ).toBe(true);
  const sealed = stored.get(RECORD);
  expect(sealed).toEqual({ nonce: expect.any(String), ciphertext: expect.any(String) });
  expect(JSON.stringify(sealed)).not.toContain('Line');
  const loaded = await store().load('page');
  expect(loaded?.get('{"exact":"q"}')).toEqual(edit(body, { mentions: [mention] }));
  // Separate purposes keep separate non-extractable key records.
  const draft = await draftKey('device');
  expect(draft.extractable).toBe(false);
  expect(stored.has('draft-key:device')).toBe(true);
  expect(stored.has('title-key:device')).toBe(false);
  await titleKey('device');
  expect(stored.get('title-key:device')).not.toBe(stored.get('draft-key:device'));
});

it('binds a record to space, device and page even if it is copied', async () => {
  await store().apply('page', new Map([['chat', edit('Scoped')]]));
  const value = stored.get(RECORD);
  stored.set('draft-key:other', structuredClone(stored.get('draft-key:device')));
  for (const [space, device, page] of [
    ['other', 'device', 'page'],
    ['space', 'other', 'page'],
    ['space', 'device', 'other'],
  ]) {
    stored.set(`drafts:${space}:${device}:${page}`, structuredClone(value));
    expect((await store(space, device).load(page))?.size).toBe(0);
  }
  expect((await store().load('page'))?.get('chat')).toEqual(edit('Scoped'));
});

it('ignores malformed, tampered or future-version records and replaces them on the next write', async () => {
  await store().apply('page', new Map([['chat', edit('Good')]]));
  const good = stored.get(RECORD) as { nonce: string; ciphertext: string };
  for (const bad of [
    null,
    7,
    { ...good, extra: true },
    { ...good, nonce: 'AA' },
    { ...good, ciphertext: 'AAAA' },
  ]) {
    stored.set(RECORD, bad);
    expect((await store().load('page'))?.size).toBe(0);
  }
  stored.set(RECORD, good);
  expect((await store().load('page'))?.get('chat')).toEqual(edit('Good'));
  // A readable record with a wrong version or entry shape is not partly trusted.
  const key = await draftKey('device');
  const { seal } = await import('../src/local-seal.js');
  const scope = ['tmt-colab-drafts-v1', 'space', 'device', 'page'];
  const plain = (value: unknown) => new TextEncoder().encode(JSON.stringify(value));
  for (const value of [
    { v: 2, drafts: [] },
    { v: 1, drafts: [{ target: 'a', edit: { value: 1 } }] },
    { v: 1, drafts: [{ target: 'a', edit: { value: 'x', extra: 1 } }] },
    {
      v: 1,
      drafts: [
        {
          target: 'a',
          edit: {
            value: 'x',
            mentions: [{ key: { machine: 'm', agent: 'a' }, range: { start: 0, end: 9 } }],
          },
        },
      ],
    },
    {
      v: 1,
      drafts: [
        { target: 'a', edit: { value: 'x' } },
        { target: 'a', edit: { value: 'y' } },
      ],
    },
    {
      v: 1,
      drafts: Array.from({ length: DRAFTS_PER_PAGE + 1 }, (_, i) => ({
        target: `t${i}`,
        edit: { value: 'x' },
      })),
    },
  ]) {
    stored.set(RECORD, await seal(key, scope, plain(value)));
    expect((await store().load('page'))?.size).toBe(0);
  }
  expect(await store().apply('page', new Map([['chat', edit('Fresh')]]))).toBe(true);
  expect([...(await store().load('page'))!.keys()]).toEqual(['chat']);
});

it('reports unavailable storage honestly and keeps the last good record', async () => {
  await store().apply('page', new Map([['chat', edit('Kept')]]));
  unavailable = true;
  expect(await store().load('page')).toBeUndefined();
  expect(await store().apply('page', new Map([['chat', edit('Lost')]]))).toBe(false);
  unavailable = false;
  expect((await store().load('page'))?.get('chat')).toEqual(edit('Kept'));
});

it('bounds drafts per page and keeps the newest', async () => {
  const many = new Map<string, ComposerEdit | null>();
  for (let i = 0; i < DRAFTS_PER_PAGE + 3; i++) many.set(`t${i}`, edit(`draft ${i}`));
  expect(await store().apply('page', many)).toBe(true);
  const loaded = await store().load('page');
  expect(loaded?.size).toBe(DRAFTS_PER_PAGE);
  expect(loaded?.has('t0')).toBe(false);
  expect(loaded?.has(`t${DRAFTS_PER_PAGE + 2}`)).toBe(true);
  // Updating an old draft makes it the newest, so a later overflow drops another one.
  await store().apply('page', new Map([['t3', edit('updated')]]));
  await store().apply('page', new Map([['new', edit('new')]]));
  const after = await store().load('page');
  expect(after?.get('t3')?.value).toBe('updated');
  expect(after?.has('t4')).toBe(false);
  await store().apply('page', new Map([['t3', null]]));
  expect((await store().load('page'))?.has('t3')).toBe(false);
});

it('refuses an oversized or malformed draft without touching stored ones', async () => {
  await store().apply('page', new Map([['chat', edit('Safe')]]));
  const before = structuredClone(stored.get(RECORD));
  expect(await store().apply('page', new Map([['big', edit('x'.repeat(COMMENT_BYTES + 1))]]))).toBe(
    false,
  );
  expect(
    await store().apply(
      'page',
      new Map([
        [
          'm',
          edit('abc', {
            mentions: [{ key: { machine: 'm', agent: 'a' }, range: { start: 2, end: 9 } }],
          }),
        ],
      ]),
    ),
  ).toBe(false);
  expect(stored.get(RECORD)).toEqual(before);
});

it('persists only deliberate nonblank drafts', () => {
  expect(persistable(edit(''))).toBe(false);
  expect(persistable(edit('  \n'))).toBe(false);
  expect(persistable(edit('@Agent ', { edited: false }))).toBe(false);
  expect(persistable(edit('@Agent hi', { edited: true }))).toBe(true);
  expect(persistable(edit('plain'))).toBe(true);
});

it('coalesces writes, skips clearing what was never saved, and retries after a failure', async () => {
  vi.useFakeTimers();
  const apply = vi.fn(
    async (_page: string, _changes: ReadonlyMap<string, ComposerEdit | null>) => true,
  );
  const failed = vi.fn();
  const session = new DraftSession({ load: async () => new Map(), apply }, 'page', failed);
  session.set('a', edit('one'));
  session.set('a', edit('two'));
  session.set('never-saved', edit('  '));
  expect(apply).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(500);
  expect(apply).toHaveBeenCalledTimes(1);
  expect([...apply.mock.calls[0][1]]).toEqual([['a', edit('two')]]);
  expect(failed).toHaveBeenLastCalledWith(false);

  apply.mockResolvedValueOnce(false);
  session.set('a', null);
  await vi.advanceTimersByTimeAsync(500);
  expect(failed).toHaveBeenLastCalledWith(true);
  session.set('b', edit('newer'));
  await session.flush();
  expect(failed).toHaveBeenLastCalledWith(false);
  // The failed removal is retried together with the newer change.
  expect([...apply.mock.calls.at(-1)![1]].sort()).toEqual([
    ['a', null],
    ['b', edit('newer')],
  ]);
  await session.close();
  session.set('c', edit('after close'));
  await vi.advanceTimersByTimeAsync(500);
  expect(apply.mock.calls.flatMap((call) => [...call[1].keys()])).not.toContain('c');
});

it('learns what is already stored so clearing it is written', async () => {
  vi.useFakeTimers();
  const apply = vi.fn(async () => true);
  const session = new DraftSession(
    { load: async () => new Map([['old', edit('stored')]]), apply },
    'page',
    () => {},
  );
  await session.load();
  session.set('old', edit(''));
  await session.flush();
  expect(apply).toHaveBeenCalledTimes(1);
});
