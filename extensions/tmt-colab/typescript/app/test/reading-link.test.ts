import { expect, it } from 'vite-plus/test';
import { ownerReadingHint, ownerReadingHref, readingPageUrl } from '../src/reading-link.js';
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
