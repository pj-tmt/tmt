import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import { Downloads, prepareExport, type ExportView } from '../src/export.js';

const fixture = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/export-v1.json', import.meta.url), 'utf8'),
);
const input = (): ExportView => {
  const { signingKeys, ...rest } = structuredClone(fixture.input);
  return {
    ...rest,
    signingKeys: Object.fromEntries(
      Object.entries(signingKeys as Record<string, string>).map(([writer, key]) => [
        writer,
        Uint8Array.from(Buffer.from(key, 'hex')),
      ]),
    ),
  };
};
const utf8 = (value: string) => new TextEncoder().encode(value);
const bytes = async (blob: Blob) => new Uint8Array(await blob.arrayBuffer());
const sha = (value: Uint8Array) => createHash('sha256').update(value).digest('hex');
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it('matches the shared native bundle byte for byte, including exact order and large decimals', async () => {
  const bundle = await prepareExport(input());
  const html = await bytes(bundle.blob('page.html'));
  const json = await bytes(bundle.blob('conversations.json'));
  const markdown = await bytes(bundle.blob('conversations.md'));
  const manifest = await bytes(bundle.blob('manifest.json'));
  expect(html).toEqual(utf8(fixture.input.source));
  expect(json).toEqual(utf8(fixture.conversationsJson));
  expect(markdown).toEqual(utf8(fixture.conversationsMarkdown));
  expect(manifest).toEqual(utf8(fixture.manifestUtf8));
  expect(sha(manifest)).toBe(fixture.manifestSha256);
  expect(bundle.files).toEqual([
    { name: 'page.html', sizeBytes: html.length, sha256: sha(html) },
    { name: 'conversations.json', sizeBytes: json.length, sha256: sha(json) },
    { name: 'conversations.md', sizeBytes: markdown.length, sha256: sha(markdown) },
    { name: 'manifest.json', sizeBytes: manifest.length, sha256: sha(manifest) },
  ]);
  // Runtime key insertion order must not change the wire order.
  const reordered = input();
  reordered.membershipHead = {
    statementHash: reordered.membershipHead.statementHash,
    revision: reordered.membershipHead.revision,
  };
  reordered.own = Object.fromEntries(Object.entries(reordered.own).reverse());
  const other = await prepareExport(reordered);
  for (const name of ['conversations.json', 'conversations.md', 'manifest.json'] as const)
    expect(await bytes(other.blob(name))).toEqual(await bytes(bundle.blob(name)));
});

it('exports only verified, in-scope conversations and keeps display text from forging structure', async () => {
  const conversations = JSON.parse(fixture.conversationsJson);
  // The foreign-page thread, the keyless writer and the forged ask are absent.
  const ids = conversations.threads.map((thread: { id: string }) => thread.id.slice(-2));
  expect(ids).toEqual(['02', '17', '18']);
  expect(
    conversations.asks.map((ask: { operationId: string }) => ask.operationId.slice(-2)),
  ).toEqual(['09', '19']);
  for (const raw of ['\u0000', '\u202e', '\r'])
    expect(fixture.conversationsMarkdown).not.toContain(raw);
  // Without a historical key a writer's records disappear; a wrong key drops its signed ask.
  const keyless = input();
  delete keyless.signingKeys['00000000-0000-4000-8000-000000000014'];
  const withoutW2 = JSON.parse(
    new TextDecoder().decode(
      await bytes((await prepareExport(keyless)).blob('conversations.json')),
    ),
  );
  expect(withoutW2.threads.map((thread: { id: string }) => thread.id.slice(-2))).toEqual(['02']);
  expect(withoutW2.threads[0].comments).toHaveLength(1);
  expect(withoutW2.asks).toHaveLength(1);
  const swapped = input();
  const [first, second] = Object.keys(swapped.signingKeys);
  [swapped.signingKeys[first], swapped.signingKeys[second]] = [
    swapped.signingKeys[second],
    swapped.signingKeys[first],
  ];
  const wrongKeys = JSON.parse(
    new TextDecoder().decode(
      await bytes((await prepareExport(swapped)).blob('conversations.json')),
    ),
  );
  // Each ask verifies only under its own writer's key: swapped keys drop both real asks and
  // admit only the one that W1's key actually signed.
  expect(wrongKeys.asks.map((ask: { operationId: string }) => ask.operationId.slice(-2))).toEqual([
    '22',
  ]);
});

it('copies a committed view before hashing and exposes only immutable files', async () => {
  const value = input(),
    pending = prepareExport(value);
  value.source = 'new live source';
  value.title = 'new title';
  value.epoch = '7';
  value.membershipHead.revision = '9';
  value.membershipHead.statementHash = 'f'.repeat(64);
  value.exportedAtMs++;
  for (const roots of Object.values(value.own)) roots.threads = {};
  Object.values(value.signingKeys).forEach((key) => key.fill(0));
  const bundle = await pending;
  expect(await bytes(bundle.blob('manifest.json'))).toEqual(utf8(fixture.manifestUtf8));
  expect(await bytes(bundle.blob('conversations.json'))).toEqual(utf8(fixture.conversationsJson));
  const copy = await bytes(bundle.blob('page.html'));
  copy.fill(0);
  expect(await bytes(bundle.blob('page.html'))).toEqual(utf8(fixture.input.source));
  expect(Object.isFrozen(bundle.files)).toBe(true);
  expect(bundle.files.every(Object.isFrozen)).toBe(true);
});

it('exports empty source/title without renderer HTML or field additions', async () => {
  const value = input();
  value.source = '';
  value.title = '';
  value.own = {};
  value.signingKeys = {};
  const bundle = await prepareExport(value);
  expect((await bytes(bundle.blob('page.html'))).length).toBe(0);
  const m = JSON.parse(await bundle.blob('manifest.json').text());
  expect(m.title).toBe('');
  expect(m.files.map((file: { name: string }) => file.name)).toEqual([
    'page.html',
    'conversations.json',
    'conversations.md',
  ]);
  expect(m.files[0]).toEqual({ name: 'page.html', sizeBytes: 0, sha256: sha(new Uint8Array()) });
  expect(Object.keys(m)).toEqual([
    'format',
    'version',
    'spaceId',
    'pageId',
    'title',
    'exportedAtMs',
    'membershipHead',
    'epoch',
    'plaintext',
    'discussions',
    'files',
  ]);
  expect(m.plaintext).toBe(true);
  expect(m.discussions).toEqual({
    included: true,
    scope: 'current-epoch',
    format: 'tmt-colab-conversations',
    version: 1,
  });
});

it('rejects malformed scope/decimal/hash/Unicode/time and oversized source before downloads', async () => {
  for (const change of [
    { epoch: '01' },
    { pageId: '../page' },
    { spaceId: 'bad' },
    { exportedAtMs: -1 },
    { exportedAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { source: '\ud800' },
    { source: 'x'.repeat(2 * 1024 * 1024 + 1) },
    { membershipHead: { revision: '0', statementHash: 'a'.repeat(64) } },
    { membershipHead: { revision: '1', statementHash: 'A'.repeat(64) } },
    { own: { writer: 'not roots' } as never },
  ])
    await expect(prepareExport({ ...input(), ...change })).rejects.toThrow();
});

function browser(throwClick = false) {
  const anchor = {
    href: '',
    download: '',
    hidden: false,
    click: vi.fn(() => {
      if (throwClick) throw new Error('refused');
    }),
    remove: vi.fn(),
  };
  vi.stubGlobal('document', { createElement: vi.fn(() => anchor), body: { append: vi.fn() } });
  const create = vi
    .spyOn(URL, 'createObjectURL')
    .mockReturnValueOnce('blob:first')
    .mockReturnValueOnce('blob:second');
  const revoke = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
  return { anchor, create, revoke };
}
it('requests literal filenames and revokes every parent URL after handoff or close', async () => {
  vi.useFakeTimers();
  const b = browser(),
    downloads = new Downloads(await prepareExport(input()));
  downloads.request('page.html');
  expect(b.anchor.download).toBe('page.html');
  expect(b.anchor.href).toBe('blob:first');
  expect(b.anchor.remove).toHaveBeenCalledOnce();
  expect(b.revoke).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1000);
  expect(b.revoke).toHaveBeenCalledWith('blob:first');
  downloads.request('manifest.json');
  expect(b.anchor.download).toBe('manifest.json');
  downloads.close();
  downloads.close();
  expect(b.revoke).toHaveBeenCalledWith('blob:second');
  await vi.runAllTimersAsync();
  expect(b.revoke).toHaveBeenCalledTimes(2);
  expect(() => downloads.request('page.html')).toThrow();
});
it('failed download handoff revokes the URL and removes its anchor', async () => {
  const b = browser(true),
    downloads = new Downloads(await prepareExport(input()));
  expect(() => downloads.request('page.html')).toThrow('refused');
  expect(b.revoke).toHaveBeenCalledOnce();
  expect(b.anchor.remove).toHaveBeenCalledOnce();
  downloads.close();
  expect(b.revoke).toHaveBeenCalledOnce();
});
