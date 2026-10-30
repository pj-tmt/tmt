import { expect, it } from 'vite-plus/test';
import { readPersonDrafts, writePersonDraft } from '../src/person-drafts.js';

it('keeps unsent names scoped to the Remote and removes restored drafts', () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
    removeItem: (key: string) => void values.delete(key),
  };
  writePersonDraft(storage, 'remote-a', 'member', { baseline: 'Guest', value: 'Draft' });
  expect(readPersonDrafts(storage, 'remote-a').get('member')).toEqual({
    baseline: 'Guest',
    value: 'Draft',
  });
  expect(readPersonDrafts(storage, 'remote-b').size).toBe(0);
  writePersonDraft(storage, 'remote-a', 'member', undefined);
  expect(readPersonDrafts(storage, 'remote-a').size).toBe(0);
});
