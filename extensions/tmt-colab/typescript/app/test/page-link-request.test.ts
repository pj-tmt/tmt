import { describe, expect, it } from 'vite-plus/test';
import { classifyPageLink, pageLinkRequest } from '../src/page-link-request.js';

const space = '10000000-0000-4000-8000-000000000001';
const page = '20000000-0000-4000-8000-000000000001';
const base = `https://owner.example/r/abcdefghijklmnop/x/colab/#space=${space}`;

describe('untrusted page link admission', () => {
  it('requires the closed shape, exact render and bounded UTF-8 href', () => {
    const value = {
      type: 'colab.render.link',
      renderId: 'current',
      href: '/p/20000000',
      newTab: false,
    };
    expect(pageLinkRequest(value, 'current')).toBe(true);
    for (const invalid of [
      null,
      [],
      { ...value, extra: true },
      { ...value, renderId: 'old' },
      { ...value, newTab: 1 },
      { ...value, href: 'x'.repeat(8193) },
      { ...value, href: '字'.repeat(3000) },
    ])
      expect(pageLinkRequest(invalid, 'current')).toBe(false);
  });
  it('admits same-space short links and cosmetic fragments without a page-supplied base', () => {
    expect(classifyPageLink('/p/20000000#details', base, space)).toMatchObject({
      kind: 'internal',
      route: '/short/20000000',
      anchor: 'details',
      label: '/p/20000000#details',
    });
    expect(classifyPageLink('/p/20000000', base, space)).toMatchObject({
      kind: 'internal',
      route: '/short/20000000',
    });
    expect(
      classifyPageLink(
        `${base}&path=${encodeURIComponent(`/pages/${page}`)}&anchor=details`,
        base,
        space,
      ),
    ).toMatchObject({ kind: 'internal', route: `/pages/${page}`, anchor: 'details' });
  });
  it.each([
    '/p/short',
    '/p/20000000?seed=secret',
    '/p/20000000#space=other',
    `${base}&path=%2Fpages%2F${page}&seed=secret`,
    `${base}&path=%2Fpages%2F${page}&space=${space}`,
    `${base.replace(space, page)}&path=%2Fpages%2F${page}`,
    'javascript:alert(1)',
    'data:text/html,hello',
    'file:///tmp/secret',
    'colab-attachment:20000000',
    'https://user:password@example.com/',
    'https://example.com/\nsecret',
    'https:\\example.com',
    'http://elsewhere.example/',
    'http://2130706433/',
    'http://127.1/',
    'http://[0:0::1]/',
  ])('refuses %s without navigation', (href) => {
    expect(classifyPageLink(href, base, space)).toEqual({ kind: 'refused' });
  });
  it.each([
    'https://outside.example/path?q=complete#fragment',
    'http://localhost:1234/path',
    'http://127.0.0.1/path',
    'http://[::1]/path',
  ])('external allowed URLs always remain external: %s', (href) => {
    expect(classifyPageLink(href, base, space)).toEqual({
      kind: 'external',
      href: new URL(href).href,
    });
  });
  it('a reader without same-space app authority never receives an internal navigation target', () => {
    expect(classifyPageLink('/p/20000000', base)).toMatchObject({ kind: 'external' });
  });
});
