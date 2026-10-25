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

import { creatorTarget } from '../src/creator-default.js';
import type { PageCreator } from '../src/question-registry.js';
it('routing uses the editable creator ID and never falls back to the old creator or a display label', () => {
  const legacy = { agentId: 'old-agent', machineId: 'old-machine' };
  const creator = { agent: { id: 'new-agent', name: 'A name' } } as PageCreator;
  expect(creatorTarget({ creator, creationRecipient: legacy })).toEqual({ agentId: 'new-agent' });
  expect(creatorTarget({ creationRecipient: legacy })).toEqual(legacy);
  expect(creatorTarget({})).toBeUndefined();
});
