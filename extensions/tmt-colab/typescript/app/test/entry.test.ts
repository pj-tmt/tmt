import { afterEach, expect, it, vi } from 'vite-plus/test';
import {
  entryHash,
  entryMount,
  entryOpenTarget,
  entryTarget,
  mountOfModule,
  publicEntry,
} from '../src/entry.js';
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
it('names the paired mount a module script was loaded from, and nothing else', () => {
  const door = 'https://door.test';
  const script = `${door}/r/abcdefghijklmnop/x/colab/assets/colab-module.js`;
  expect(mountOfModule(script, door)?.href).toBe(`${door}/r/abcdefghijklmnop/x/colab/`);
  expect(mountOfModule(new URL(script), door)?.href).toBe(`${door}/r/abcdefghijklmnop/x/colab/`);
  for (const other of [
    // The dev server, another module's script, another origin, a script outside any mount.
    `${door}/src/colab-module.tsx`,
    `${door}/r/abcdefghijklmnop/x/ops/assets/colab-module.js`,
    `https://other.test/r/abcdefghijklmnop/x/colab/assets/colab-module.js`,
    `${door}/assets/colab-module.js`,
    `${door}/r/abcdefghijklmnop/x/colab/assets/deep/colab-module.js`,
  ])
    expect(mountOfModule(other, door), other).toBeNull();
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
    // The task-era hint is gone (#5825).
    '#task=T-4',
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

it('opening a public link ignores a fragment that names no display target and never an owner key', () => {
  const thread = '11111111-1111-4111-8111-111111111111';
  const reading = '22222222-2222-4222-8222-222222222222';
  expect(entryOpenTarget('')).toBeNull();
  expect(entryOpenTarget(`#t=${thread}`)).toEqual({ thread });
  expect(entryOpenTarget(`#read=${reading}`)).toEqual({ read: reading });
  expect(entryOpenTarget('#anchor=decisions')).toEqual({ anchor: 'decisions' });
  // A plain #id is the web's own form; the strict parser still takes only the canonical ones.
  expect(entryOpenTarget('#decisions')).toEqual({ anchor: 'decisions' });
  expect(entryOpenTarget('#section_1.2:a')).toEqual({ anchor: 'section_1.2:a' });
  expect(entryOpenTarget('#seed')).toEqual({ anchor: 'seed' });
  expect(() => entryTarget('#decisions')).toThrow();
  for (const hash of [
    '#%%%garbage',
    '#a=1&b=2',
    '#-leading',
    `#${'a'.repeat(129)}`,
    '#t=',
    '#t=bad',
    '#read=bad',
    '#anchor=',
    // An old task link opens the page at the top.
    '#task=T-4',
    `#t=${thread}&t=${thread}`,
    `#t=${thread}&read=${reading}`,
    `#read=${reading}&read=${reading}`,
  ])
    expect(entryOpenTarget(hash)).toBeNull();
  // The owner's own location and a reader's capability are never taken from a public link.
  for (const hash of [
    '#space=foreign',
    '#seed=secret',
    '#path=%2Fshort%2F12345678',
    `#t=${thread}&path=other`,
    '#v=1&space=a&seed=b',
  ])
    expect(() => entryOpenTarget(hash)).toThrow();
});

it('the location keeps the canonical fragment of its display target only', () => {
  const thread = '11111111-1111-4111-8111-111111111111';
  expect(entryHash(null)).toBe('');
  expect(entryHash({ thread })).toBe(`#t=${thread}`);
  expect(entryHash({ read: thread })).toBe(`#read=${thread}`);
  expect(entryHash(entryOpenTarget('#decisions'))).toBe('#anchor=decisions');
  expect(entryHash(entryOpenTarget('#a=1&b=2'))).toBe('');
});
