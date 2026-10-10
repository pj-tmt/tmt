import { afterEach, expect, it, vi } from 'vite-plus/test';
import { entryMount, entryThread, publicEntry } from '../src/entry.js';
import { readerUrl, parseReaderFragment } from '../src/reader-link.js';
afterEach(() => vi.unstubAllGlobals());
it('accepts exactly the public display-entry grammar', () => {
  for (const path of ['/colab', '/colab/', '/p/12345678', '/read/abcd-ef'])
    expect(publicEntry(path)).toBe(true);
  for (const path of ['/', '/p/abc', '/p/abcd/', '/read/abcd?seed=x', '/r/abcd', '/p/abcd/extra'])
    expect(publicEntry(path)).toBe(false);
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
it('serializes one reader capability with matching path and fragment identity', () => {
  const link = '22222222-2222-4222-8222-222222222222';
  const url = new URL(
    readerUrl('https://door.test', {
      space: 'a'.repeat(32),
      page: '11111111-1111-4111-8111-111111111111',
      link,
      revision: '2',
      statement: 'A'.repeat(43),
      seed: 'A'.repeat(43),
    }),
  );
  expect(url.pathname).toBe(`/read/${link}`);
  expect(url.search).toBe('');
  expect(parseReaderFragment(url.hash).link).toBe(link);
  expect(() =>
    readerUrl(url.origin, {
      space: 'invalid',
      page: link,
      link,
      revision: '2',
      statement: 'A'.repeat(43),
      seed: 'A'.repeat(43),
    }),
  ).toThrow();
});

it('allows only one canonical thread display target at startup and during history navigation', () => {
  const thread = '11111111-1111-4111-8111-111111111111';
  expect(entryThread('')).toBeNull();
  expect(entryThread(`#t=${thread}`)).toBe(thread);
  for (const hash of [
    '#space=foreign',
    '#seed=secret',
    '#t=',
    '#t=bad',
    `#t=${thread}&t=${thread}`,
    `#t=${thread}&path=other`,
  ])
    expect(() => entryThread(hash)).toThrow();
});
