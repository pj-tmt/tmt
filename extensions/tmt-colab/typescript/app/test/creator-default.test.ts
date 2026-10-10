import { afterEach, expect, it, vi } from 'vite-plus/test';
import { readCreatorDefault, writeCreatorDefault } from '../src/creator-default.js';
afterEach(() => vi.unstubAllGlobals());
it('defaults on, remembers only an explicit off, and scopes each page/viewer independently', () => {
  const records = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => records.get(key),
    setItem: (key: string, value: string) => records.set(key, value),
  });
  expect(readCreatorDefault('page-device')).toBe(true);
  writeCreatorDefault('page-device', false);
  expect(readCreatorDefault('page-device')).toBe(false);
  expect(readCreatorDefault('other-page-device')).toBe(true);
  writeCreatorDefault('page-device', true);
  expect(readCreatorDefault('page-device')).toBe(true);
});
it('storage refusal keeps the safe on default and a write never throws', () => {
  vi.stubGlobal('localStorage', {
    getItem() {
      throw new Error('Refused');
    },
    setItem() {
      throw new Error('Refused');
    },
  });
  expect(readCreatorDefault('page-device')).toBe(true);
  expect(() => writeCreatorDefault('page-device', false)).not.toThrow();
});
