import { describe, expect, it } from 'vite-plus/test';
import {
  hintFragment,
  hintSearch,
  ownerPageHint,
  ownerReadingHint,
  openSearch,
  ownerReadingHref,
  pageSearch,
  splitPageHint,
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

it('carries one task of the page as a third cosmetic hint (#4263)', () => {
  expect(pageSearch({ task: 'T-4' })).toEqual({ task: 'T-4' });
  expect(pageSearch({ task: 'T-1234567' })).toEqual({ task: 'T-1234567' });
  // Never with another hint, and only the work module's task-id shape.
  expect(pageSearch({ task: 'T-4', read })).toEqual({});
  expect(pageSearch({ task: 'T-4', anchor: 'decisions' })).toEqual({});
  expect(pageSearch({ task: 'T-4', read, anchor: 'decisions' })).toEqual({});
  for (const task of [
    '',
    'T-',
    'T-0',
    'T-01',
    't-1',
    'T-1x',
    'T-12345678',
    ' T-1',
    4,
    null,
    undefined,
  ])
    expect(pageSearch({ task })).toEqual({});
  const href = ownerReadingHref(`/pages/${page}?task=T-4`, '/ext/colab/', space);
  expect(href).toBe(`${owner.replace('https://example.test', '')}&task=T-4`);
  const url = new URL(`https://example.test${href}`);
  expect(ownerPageHint(url, space, page)).toEqual({ task: 'T-4' });
  // Only the exact owner page, and the reading reader never reports a task.
  expect(ownerPageHint(url, space, space)).toBeUndefined();
  expect(ownerReadingHint(url, space, page)).toBeUndefined();
  const short = ownerReadingHref('/short/10000000?task=T-4', '/ext/colab/', space);
  expect(ownerPageHint(new URL(`https://example.test${short}`), space)).toEqual({ task: 'T-4' });
  expect(splitPageHint(`/pages/${page}?task=T-4`)).toEqual({
    route: `/pages/${page}`,
    hint: { task: 'T-4' },
  });
  expect(splitPageHint('/short/10000000?task=T-4')).toEqual({
    route: '/short/10000000',
    hint: { task: 'T-4' },
  });
  // A bad task keeps its query inside the opaque path, uninterpreted.
  for (const bad of ['T-0', 'T-1x', 'x'])
    expect(splitPageHint(`/pages/${page}?task=${bad}`)).toEqual({
      route: `/pages/${page}?task=${bad}`,
    });
  expect(splitPageHint(`/pages/${page}?task=T-4&read=${read}`).hint).toBeUndefined();
});

it('refuses a task hint with extra, duplicate or mixed parameters', () => {
  for (const link of [
    owner + '&task=T-4&task=T-4',
    owner + '&task=T-4&secret=value',
    owner + `&task=T-4&read=${read}`,
    owner + '&task=T-4&anchor=decisions',
    owner + '&task=T-0',
    owner + '&task=',
    owner.replace('/ext/colab/', '/ext/colab/read') + '&task=T-4',
  ])
    expect(ownerPageHint(new URL(link), space, page)).toBeUndefined();
});

it('names a hint as one search or fragment parameter for every location form', () => {
  expect(hintSearch({ read })).toBe(`?read=${read}`);
  expect(hintSearch({ anchor: 'decisions' })).toBe('?anchor=decisions');
  expect(hintSearch({ task: 'T-4' })).toBe('?task=T-4');
  expect(hintFragment({ read })).toBe(`#read=${read}`);
  expect(hintFragment({ anchor: 'decisions' })).toBe('#anchor=decisions');
  expect(hintFragment({ task: 'T-4' })).toBe('#task=T-4');
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
