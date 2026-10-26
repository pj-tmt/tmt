import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import {
  editLock,
  editLockKey,
  advancesEditLock,
  readEditLocks,
  activeEditLocks,
  type EditLockRecord,
} from '../src/edit-locks.js';
import { validateOwn, type OwnState, type JsonValue } from '../src/fold-protocol.js';
const fixture = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/edit-lock-v1.json', import.meta.url), 'utf8'),
);
const base = fixture.record as EditLockRecord;
function own(...records: EditLockRecord[]): OwnState {
  const state: OwnState = {};
  for (const record of records) {
    state[record.senderDevice] ??= { threads: {}, messages: {}, intents: {}, replies: {} };
    state[record.senderDevice].messages[editLockKey(record)] = structuredClone(
      record,
    ) as unknown as JsonValue;
  }
  return state;
}
const read = (state: OwnState) => readEditLocks(state, fixture.scope, () => true);
it('admits only the closed versioned grammar and bounded annotation quote target', () => {
  expect(editLock(base, 'messages', editLockKey(base))).toEqual(base);
  const whole = { ...base, target: fixture.whole };
  expect(editLock(whole, 'messages', editLockKey(whole))).toEqual(whole);
  for (const record of [
    { ...base, extra: true },
    { ...base, revision: '0' },
    { ...base, revision: '01' },
    { ...base, revision: '9007199254740992' },
    { ...base, released: 'false' },
    { ...base, holder: { ...base.holder, id: 'not-an-id' } },
    { ...base, holder: { ...base.holder, name: 'bad\nname' } },
    { ...base, target: { whole: false } },
    { ...base, target: { whole: true, exact: 'x' } },
    { ...base, target: { exact: '', prefix: '', suffix: '' } },
    { ...base, target: { exact: 'x'.repeat(fixture.quoteBytes + 1), prefix: '', suffix: '' } },
    { ...base, target: { exact: 'x', prefix: '😀'.repeat(fixture.contextPoints + 1), suffix: '' } },
  ])
    expect(() => editLock(record, 'messages', editLockKey(base))).toThrow();
  expect(() => editLock(base, 'threads', editLockKey(base))).toThrow();
  const misplaced = own(base);
  misplaced[base.senderDevice].threads = misplaced[base.senderDevice].messages;
  misplaced[base.senderDevice].messages = {};
  expect(() => validateOwn(misplaced)).toThrow();
  expect(() => editLock(base, 'messages', 'substituted')).toThrow();
  expect(() =>
    validateOwn(own({ ...base, target: { whole: false } } as unknown as EditLockRecord)),
  ).toThrow();
});
it('keeps one revision winner per identity without wall clocks or growing action ancestry', () => {
  const replaced = {
    ...base,
    senderDevice: crypto.randomUUID(),
    revision: '2',
    lockId: crypto.randomUUID(),
    target: fixture.whole,
  };
  for (const records of [
    [base, replaced],
    [replaced, base],
  ])
    expect(read(own(...records))).toEqual([replaced]);
  const released = { ...replaced, revision: '3', released: true };
  expect(read(own(base, replaced, released))).toEqual([released]);
  expect(readEditLocks(own(base), fixture.scope, () => false)).toEqual([]);
  expect(readEditLocks(own(base), { ...fixture.scope, epoch: '2' }, () => true)).toEqual([]);
  const advanced = { ...base, revision: '2', released: true };
  expect(advancesEditLock(base, advanced, 'messages', editLockKey(base))).toBe(true);
  for (const invalid of [
    base,
    { ...advanced, revision: '1' },
    { ...advanced, pageId: crypto.randomUUID() },
    { ...advanced, senderDevice: crypto.randomUUID() },
    null,
  ])
    expect(advancesEditLock(base, invalid, 'messages', editLockKey(base))).toBe(false);
});
it('unknown keeps a lock; offline/ended or explicit release removes it, and a returning UUID may retain it', () => {
  const locks = read(own(base));
  for (const presence of ['active', 'unknown'] as const)
    expect(activeEditLocks(locks, () => presence)).toHaveLength(1);
  for (const presence of ['offline', 'ended'] as const)
    expect(activeEditLocks(locks, () => presence)).toEqual([]);
  expect(
    activeEditLocks(read(own({ ...base, revision: '2', released: true })), () => 'active'),
  ).toEqual([]);
  expect(activeEditLocks(locks, () => 'active')).toHaveLength(1);
});
it('repeated edits replace one holder cell; distinct retained holders remain bounded', () => {
  const state = own(base);
  for (let revision = 2; revision <= 2000; revision++)
    state[base.senderDevice].messages[editLockKey(base)] = {
      ...base,
      revision: String(revision),
      released: revision % 2 === 0,
    } as unknown as JsonValue;
  expect(Object.keys(state[base.senderDevice].messages)).toHaveLength(1);
  expect(() => validateOwn(state)).not.toThrow();
  const records = Array.from({ length: fixture.holdersPerWriter }, () => ({
    ...base,
    holder: { ...base.holder, id: crypto.randomUUID() },
  }));
  expect(() => validateOwn(own(...records))).not.toThrow();
  expect(() =>
    validateOwn(own(...records, { ...base, holder: { ...base.holder, id: crypto.randomUUID() } })),
  ).toThrow();
});
