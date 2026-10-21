import { describe, expect, it } from 'vite-plus/test';
import {
  ownerPageHint,
  ownerReadingHint,
  openSearch,
  ownerReadingHref,
  pageSearch,
} from '../src/reading-link.js';
const space = '10000000-0000-4000-8000-000000000001';
const page = '10000000-0000-4000-8000-000000000002';
const read = '10000000-0000-4000-8000-000000000401';
const owner = `https://example.test/ext/colab/#space=${space}&path=${encodeURIComponent(`/pages/${page}`)}`;
it('reads only a bounded cosmetic reading hint from the exact owner page', () => {
  const link = `${owner}&read=${read}`;
  expect(ownerReadingHint(new URL(link), space, page)).toBe(read);
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
  }
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

it('carries one exact element anchor through a full or short page route, never with a reading', () => {
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
  const short = ownerReadingHref('/short/10000000?anchor=decisions', '/ext/colab/', space);
  expect(ownerPageHint(new URL(`https://example.test${short}`), space)).toEqual({
    anchor: 'decisions',
  });
  // A bad anchor keeps its query inside the opaque path, uninterpreted.
  for (const path of [`/pages/${page}?anchor=a b`]) {
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

describe('openSearch', () => {
  const id = '10000000-0000-4000-8000-0000000000a1';
  it('keeps one page ID and drops anything else', () => {
    expect(openSearch({ open: id })).toEqual({ open: id });
    expect(openSearch({ open: 'plan' })).toEqual({});
    expect(openSearch({ open: [id] })).toEqual({});
    expect(openSearch({ open: `${id}&x=1` })).toEqual({});
    expect(openSearch({})).toEqual({});
  });
});
