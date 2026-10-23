import { expect, it } from 'vite-plus/test';
import { brandMarkSvg, freshMarkId } from '../src/brand-mark-svg.js';

const defined = (markup: string) =>
  [...markup.matchAll(/\bid="([^"]+)"/g)].map((match) => match[1]);
const referenced = (markup: string) =>
  [...markup.matchAll(/url\(#([^)]+)\)/g)].map((match) => match[1]);

it('takes the base prefix while it is free and counts up past the ones in use', () => {
  expect(freshMarkId('tmt-rail', () => false)).toBe('tmt-rail');
  const used = new Set(['tmt-rail', 'tmt-rail-1']);
  expect(freshMarkId('tmt-rail', (prefix) => used.has(prefix))).toBe('tmt-rail-2');
});

it('keeps every paint server reference of a mark inside that mark, whatever the prefix', () => {
  const first = brandMarkSvg({ size: 32, id: 'tmt-rail' });
  const second = brandMarkSvg({ size: 32, id: 'tmt-rail-1' });
  for (const mark of [first, second]) {
    expect(referenced(mark).length).toBeGreaterThan(0);
    for (const id of referenced(mark)) expect(defined(mark)).toContain(id);
  }
  // Two marks on one page share no id, so no reference can resolve into the other one.
  expect(defined(first).filter((id) => defined(second).includes(id))).toEqual([]);
});
