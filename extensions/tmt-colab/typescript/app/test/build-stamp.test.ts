import { expect, it } from 'vite-plus/test';
import { entryOf, isNewerBuild, pick } from '../src/build-stamp.js';

const base = new URL('https://door.example/r/abc/x/colab/');
const script = (src: string) => `<script type="module" crossorigin src="${src}"></script>`;
const page = (...sources: string[]) =>
  `<!doctype html><html><head>${sources.map(script).join('')}</head></html>`;

it('reads the hashed entry module of a served page', () => {
  expect(entryOf(page('./assets/index-AAA.js'), base)).toBe('/r/abc/x/colab/assets/index-AAA.js');
  expect(entryOf(page('./assets/index-AbC123xY.js'), base)).toBe(
    '/r/abc/x/colab/assets/index-AbC123xY.js',
  );
});

it('finds no entry in a page without a module script', () => {
  expect(entryOf('<p>no script</p>', base)).toBeNull();
  expect(entryOf('<script src="x.js"></script>', base)).toBeNull();
  expect(entryOf('', base)).toBeNull();
  // A script reference past the read limit is not an entry.
  expect(entryOf(`${' '.repeat(64 * 1024)}${page('./assets/index-AbC123xY.js')}`, base)).toBeNull();
});

it('takes the hashed bundle when a page references two module entries', () => {
  // The dev server's client comes first; a build has the hashed bundle.
  expect(entryOf(page('/@vite/client', './assets/index-AbC123xY.js'), base)).toBe(
    '/r/abc/x/colab/assets/index-AbC123xY.js',
  );
  // Two hashed bundles: the first is the entry.
  expect(entryOf(page('./assets/index-AAAAAAAA.js', './assets/other-BBBBBBBB.js'), base)).toBe(
    '/r/abc/x/colab/assets/index-AAAAAAAA.js',
  );
  // None hashed: the first one stands, and it names no build.
  expect(entryOf(page('/@vite/client', './assets/recovery.js'), base)).toBe('/@vite/client');
  expect(pick(['/a/recovery.js', '/a/index-AAAAAAAA.js'])).toBe('/a/index-AAAAAAAA.js');
  expect(pick([])).toBeNull();
});

it('skips a malformed reference instead of failing the read', () => {
  expect(entryOf(page('http://[bad', './assets/index-AbC123xY.js'), base)).toBe(
    '/r/abc/x/colab/assets/index-AbC123xY.js',
  );
  expect(entryOf(page('http://[bad'), base)).toBeNull();
});

it('tells an upgrade from a different kind of page', () => {
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/index-BBBBBBBB.js')).toBe(true);
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/index-AAAAAAAA.js')).toBe(false);
  // Pairing guidance (unhashed recovery.js) or the reader entry is not a newer app build.
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/assets/recovery.js')).toBe(false);
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/assets/reader.js')).toBe(false);
  expect(isNewerBuild('/src/main.tsx', '/src/main.tsx')).toBe(false);
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/other-BBBBBBBB.js')).toBe(false);
});

it('relies on the 8-character content hash, so another length is silent', () => {
  expect(isNewerBuild('/a/index-AAAAAAA.js', '/a/index-BBBBBBB.js')).toBe(false);
  expect(isNewerBuild('/a/index-AAAAAAAAA.js', '/a/index-BBBBBBBBB.js')).toBe(false);
});

it('compares builds by name and hash wherever the entry is served from', () => {
  // The module's stamp names the entry under `/assets/`; the served read names it under the mount.
  expect(isNewerBuild('/assets/index-OLDOLD12.js', '/r/abc/x/colab/assets/index-NEWNEW34.js')).toBe(
    true,
  );
  expect(isNewerBuild('/assets/index-SAME1234.js', '/r/abc/x/colab/assets/index-SAME1234.js')).toBe(
    false,
  );
});
