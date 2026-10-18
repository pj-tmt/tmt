import { expect, it } from 'vite-plus/test';
import {
  ownerPageHint,
  ownerReadingHint,
  ownerReadingHref,
  pageSearch,
  readingPageUrl,
} from '../src/reading-link.js';
const space = '10000000-0000-4000-8000-000000000001';
const page = '10000000-0000-4000-8000-000000000002';
const read = '10000000-0000-4000-8000-000000000401';
const owner = `https://example.test/ext/colab/#space=${space}&path=${encodeURIComponent(`/pages/${page}`)}`;
it('adds only a bounded cosmetic reading hint to the exact owner page', () => {
  const link = readingPageUrl(owner, space, page, read)!;
  expect(link).toBe(`${owner}&read=${read}`);
  expect(ownerReadingHint(new URL(link), space, page)).toBe(read);
  expect(readingPageUrl(link, space, page, read)).toBe(link);
  expect(ownerReadingHint(new URL(link), space, space)).toBeUndefined();
});
it('does not absorb capabilities, extra parameters, duplicate keys or invalid targets', () => {
  for (const link of [
    owner + '&read=bad',
    owner + `&read=${read}&read=${read}`,
    owner + `&read=${read}&secret=value`,
    owner.replace('/ext/colab/', '/ext/colab/read') + `&read=${read}`,
    owner.replace('#', '?secret=value#') + `&read=${read}`,
  ]) {
    expect(ownerReadingHint(new URL(link), space, page)).toBeUndefined();
    expect(readingPageUrl(link, space, page, read)).toBeUndefined();
  }
  expect(readingPageUrl(owner, space, page, 'x'.repeat(10000))).toBeUndefined();
  expect(readingPageUrl(owner.replace('https:', 'javascript:'), space, page, read)).toBeUndefined();
});

it('keeps an admitted cosmetic hint through full and short internal routes without mixing it into the page path', () => {
  expect(ownerReadingHref(`/pages/${page}?read=${read}`, '/ext/colab/', space)).toBe(
    `${owner.replace('https://example.test', '')}&read=${read}`,
  );
  const short = ownerReadingHref(`/short/10000000?read=${read}`, '/ext/colab/', space);
  expect(ownerReadingHint(new URL(`https://example.test${short}`), space)).toBe(read);
  expect(ownerReadingHref(`/pages/${page}`, '/ext/colab/', space)).toBe(
    owner.replace('https://example.test', ''),
  );
});

it('leaves unrelated internal navigation byte-for-byte unchanged', () => {
  expect(ownerReadingHref('/?filter=archived', '/ext/colab/', space)).toBe(
    `/ext/colab/#space=${space}&path=${encodeURIComponent('/?filter=archived')}`,
  );
});

it('carries one exact shell anchor as the other cosmetic page hint, never with a reading or on a short link', () => {
  expect(pageSearch({ anchor: 'decisions' })).toEqual({ anchor: 'decisions' });
  expect(pageSearch({ read, anchor: 'decisions' })).toEqual({});
  for (const anchor of ['', '-start', 'a b', 'a/b', 'x'.repeat(129), 7, undefined])
    expect(pageSearch({ anchor })).toEqual({});
  const href = ownerReadingHref(`/pages/${page}?anchor=decisions`, '/ext/colab/', space);
  expect(href).toBe(`${owner.replace('https://example.test', '')}&anchor=decisions`);
  const url = new URL(`https://example.test${href}`);
  expect(ownerPageHint(url, space, page)).toEqual({ anchor: 'decisions' });
  // The reading hint never reports an anchor, so ask-intent's reading checks are unchanged.
  expect(ownerReadingHint(url, space, page)).toBeUndefined();
  expect(ownerPageHint(url, space, space)).toBeUndefined();
  // A short link or a bad anchor keeps its query inside the opaque path, uninterpreted.
  for (const path of [`/short/10000000?anchor=decisions`, `/pages/${page}?anchor=a b`]) {
    expect(
      ownerPageHint(
        new URL(`https://example.test${ownerReadingHref(path, '/ext/colab/', space)}`),
        space,
      ),
    ).toBeUndefined();
  }
});

it('refuses an anchor hint with extra, duplicate or mixed parameters', () => {
  for (const link of [
    owner + '&anchor=decisions&anchor=decisions',
    owner + '&anchor=decisions&secret=value',
    owner + `&anchor=decisions&read=${read}`,
    owner + '&anchor=-bad',
    owner.replace('/ext/colab/', '/ext/colab/read') + '&anchor=decisions',
  ])
    expect(ownerPageHint(new URL(link), space, page)).toBeUndefined();
});
