import { afterEach, expect, it, vi } from 'vite-plus/test';
import { entryMount, entryTarget, publicEntry } from '../src/entry.js';
import { deriveSpaceId, encodeBinary } from '@tmt/colab-client';
import { readerUrl, parseReaderFragment } from '../src/reader-link.js';
afterEach(() => vi.unstubAllGlobals());
it('accepts exactly the public display-entry grammar on a Remote door only', () => {
  const door = 'https://door.test';
  for (const path of ['/colab', '/colab/', '/p/12345678', '/read/abcd-ef'])
    expect(publicEntry(path, door)).toBe(true);
  for (const path of ['/', '/p/abc', '/p/abcd/', '/read/abcd?seed=x', '/r/abcd', '/p/abcd/extra'])
    expect(publicEntry(path, door)).toBe(false);
  // The hosted static origin has no Remote mount to name.
  expect(publicEntry('/read/abcd-ef', 'https://demo-colab-reader.web.app')).toBe(false);
});
it('uses only the Remote-supplied same-origin canonical mount at public entries', () => {
  vi.stubGlobal('location', { pathname: '/p/12345678', origin: 'https://door.test' });
  let content: string | undefined = '/r/abcdefghijklmnop/x/colab/';
  vi.stubGlobal('document', { querySelector: () => (content === undefined ? null : { content }) });
  expect(entryMount().href).toBe('https://door.test/r/abcdefghijklmnop/x/colab/');
  for (content of [
    undefined,
    '/colab/',
    'https://other.test/r/abcdefghijklmnop/x/colab/',
    '/r/abcdefghijklmnop/x/other/',
    '/r/abcdefghijklmnop/x/colab/?x=1',
  ])
    expect(() => entryMount()).toThrow();
});
it('serializes one reader capability with matching path and fragment identity', async () => {
  const link = '22222222-2222-4222-8222-222222222222';
  const owner = new Uint8Array(32).fill(9);
  const capability = {
    space: await deriveSpaceId(owner),
    page: '11111111-1111-4111-8111-111111111111',
    link,
    revision: '2',
    statement: 'A'.repeat(43),
    seed: 'A'.repeat(43),
    root: encodeBinary(owner),
    role: 'viewer' as const,
  };
  const url = new URL(readerUrl('https://door.test', capability));
  expect(url.pathname).toBe(`/read/${link}`);
  expect(url.search).toBe('');
  expect((await parseReaderFragment(url.hash)).link).toBe(link);
  expect(() => readerUrl(url.origin, { ...capability, space: 'invalid' })).toThrow();
  expect(() => readerUrl(url.origin, { ...capability, root: 'short' })).toThrow();
});

it('allows one canonical display target at startup and during history navigation', () => {
  const thread = '11111111-1111-4111-8111-111111111111';
  const reading = '22222222-2222-4222-8222-222222222222';
  expect(entryTarget('')).toBeNull();
  expect(entryTarget(`#t=${thread}`)).toEqual({ thread });
  expect(entryTarget(`#read=${reading}`)).toEqual({ read: reading });
  for (const hash of [
    '#space=foreign',
    '#seed=secret',
    '#t=',
    '#t=bad',
    '#read=bad',
    '#anchor=',
    `#t=${thread}&t=${thread}`,
    `#t=${thread}&path=other`,
    `#t=${thread}&read=${reading}`,
    `#read=${reading}&read=${reading}`,
  ])
    expect(() => entryTarget(hash)).toThrow();
});
