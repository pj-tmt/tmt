import { expect, it } from 'vite-plus/test';
import { ownerSectionHint, ownerSectionHref, sectionSearch } from '../src/section-link.js';
const space = '10000000-0000-4000-8000-000000000001';
const page = '10000000-0000-4000-8000-000000000002';
it('carries an exact cosmetic section on owner links, keeping collection routes intact', () => {
  const href = ownerSectionHref(`/pages/${page}?section=part:2`, '/ext/colab/', space);
  expect(ownerSectionHint(new URL(`https://example.test${href}`), space)).toBe('part:2');
  expect(ownerSectionHref('/collections', '/ext/colab/', space)).toBe(
    `/ext/colab/#space=${space}&path=${encodeURIComponent('/collections')}`,
  );
  expect(sectionSearch({ section: '[id=x]' })).toEqual({});
});
it('rejects missing, duplicate or capability-bearing hints, never admitting a reader link', () => {
  const href = `https://example.test${ownerSectionHref(`/pages/${page}?section=part:2`, '/ext/colab/', space)}`;
  for (const url of [
    href + '&section=x',
    href + '&secret=x',
    href.replace('/ext/colab/', '/ext/colab/read'),
    href.replace('#', '?secret=x#'),
    href.replace('section=part%3A2', 'section=%5Bid%3Dx%5D'),
  ])
    expect(ownerSectionHint(new URL(url), space)).toBeUndefined();
  expect(ownerSectionHint(new URL(href), 'other')).toBeUndefined();
});
