import { expect, it } from 'vite-plus/test';
import { shortPageId, shortPageUrl, validPagePrefix } from '../src/short-links.js';
import { FrozenAsk } from '../src/ask-intent.js';
import { destination, selection } from './ask-fixtures.js';
it('a historical Ask link becomes a chooser prefix after catalog growth', () => {
  const page = '12345678-0000-4000-8000-000000000001';
  const ids = [page];
  const historical = shortPageId(page, ids);
  expect(historical).toBe('12345678');
  ids.push('12345678-1000-4000-8000-000000000002');
  expect(ids.filter((id) => id.startsWith(historical))).toEqual(ids);
  expect(shortPageId(page, ids)).toBe('12345678-0');
  expect(shortPageId(page, [])).toBe(page);
  expect(shortPageUrl('http://127.0.0.1:1234', page, historical)).toBe(
    'http://127.0.0.1:1234/p/12345678',
  );
});
it('rejects malformed, foreign-page or oversized prefixes', () => {
  for (const bad of [
    '1234567',
    '123456780',
    'FFFFFFFF',
    '12345678/',
    '12345678-0000-4000-8000-0000000000010',
  ])
    expect(validPagePrefix(bad)).toBe(false);
  expect(() => shortPageUrl('https://example.test', selection().page, '12345678')).toThrow();
});
it('freezes the short public owner link while preserving full scope IDs and secret rejection', () => {
  const admitted = { ...selection(), shortId: '00000000' };
  const frozen = FrozenAsk.capture(admitted, destination(), {
    operationId: '00000000-0000-4000-8000-000000000009',
    issuedAt: 1700000000000,
  });
  expect(frozen.view.message).toContain('Link: https://example.test/p/00000000\n');
  expect(frozen.view.message).not.toContain('#space=');
  expect(() =>
    FrozenAsk.capture(
      { ...admitted, url: 'https://example.test/x/colab/read#seed=secret' },
      destination(),
      { operationId: '00000000-0000-4000-8000-000000000009', issuedAt: 1700000000000 },
    ),
  ).toThrow();
});
