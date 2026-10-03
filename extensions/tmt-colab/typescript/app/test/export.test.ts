import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import { Downloads, prepareExport, type ExportView } from '../src/export.js';

const fixture = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/export-v1.json', import.meta.url), 'utf8'),
);
const input = (): ExportView => structuredClone(fixture.input);
const bytes = async (blob: Blob) => new Uint8Array(await blob.arrayBuffer());
const sha = (value: Uint8Array) => createHash('sha256').update(value).digest('hex');
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it('matches the shared native manifest byte for byte, including exact order and large decimals', async () => {
  const bundle = await prepareExport(input());
  const html = await bytes(bundle.blob('page.html'));
  const manifest = await bytes(bundle.blob('manifest.json'));
  expect(html).toEqual(new TextEncoder().encode(fixture.input.source));
  expect(manifest).toEqual(new TextEncoder().encode(fixture.manifestUtf8));
  expect(sha(manifest)).toBe(fixture.manifestSha256);
  expect(bundle.files).toEqual([
    { name: 'page.html', sizeBytes: html.length, sha256: sha(html) },
    { name: 'manifest.json', sizeBytes: manifest.length, sha256: sha(manifest) },
  ]);
  // Runtime key insertion order must not change the wire order.
  const reordered = input();
  reordered.membershipHead = {
    statementHash: reordered.membershipHead.statementHash,
    revision: reordered.membershipHead.revision,
  };
  expect(await bytes((await prepareExport(reordered)).blob('manifest.json'))).toEqual(manifest);
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
  const bundle = await pending;
  expect(await bytes(bundle.blob('manifest.json'))).toEqual(
    new TextEncoder().encode(fixture.manifestUtf8),
  );
  const copy = await bytes(bundle.blob('page.html'));
  copy.fill(0);
  expect(await bytes(bundle.blob('page.html'))).toEqual(
    new TextEncoder().encode(fixture.input.source),
  );
  expect(Object.isFrozen(bundle.files)).toBe(true);
  expect(bundle.files.every(Object.isFrozen)).toBe(true);
});

it('exports empty source/title without renderer HTML or field additions', async () => {
  const value = input();
  value.source = '';
  value.title = '';
  const bundle = await prepareExport(value);
  expect((await bytes(bundle.blob('page.html'))).length).toBe(0);
  const m = JSON.parse(await bundle.blob('manifest.json').text());
  expect(m.title).toBe('');
  expect(m.files).toEqual([{ name: 'page.html', sizeBytes: 0, sha256: sha(new Uint8Array()) }]);
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
  expect(m.discussions).toBe('not-included');
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
