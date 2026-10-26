import {
  coreId,
  decimal,
  exactKeys,
  generatedId,
  requireValue,
  spaceId,
  text,
} from '@tmt/colab-client';
import { validateSelector, type DiscussionScope, type QuoteSelector } from './thread-records.js';
import type { OwnState } from './fold-protocol.js';
export const EDIT_LOCKS_PER_WRITER = 8;
export const EDIT_LOCK_HOLDERS_PER_WRITER = 1000;
export type EditLockTarget = { whole: true } | QuoteSelector;
export type LockPresence = 'active' | 'offline' | 'ended' | 'unknown';
export interface EditLockRecord extends DiscussionScope {
  version: 1;
  kind: 'edit-lock';
  senderDevice: string;
  revision: string;
  lockId: string;
  holder: { id: string; name: string };
  target: EditLockTarget;
  released: boolean;
}
export const editLockKey = (record: EditLockRecord) => `${record.holder.id}:edit-lock`;
export function editLockTarget(value: unknown): asserts value is EditLockTarget {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  if (Object.hasOwn(value, 'whole')) {
    exactKeys(value, ['whole']);
    requireValue(value.whole === true);
  } else validateSelector(value);
}
/** One current value per identity/writer. Names and identity IDs are publisher assertions. */
export function editLock(value: unknown, root: string, key: string): EditLockRecord {
  exactKeys(value, [
    'version',
    'kind',
    'spaceId',
    'pageId',
    'epoch',
    'senderDevice',
    'revision',
    'lockId',
    'holder',
    'target',
    'released',
  ]);
  requireValue(
    root === 'messages' &&
      value.version === 1 &&
      value.kind === 'edit-lock' &&
      typeof value.released === 'boolean',
  );
  spaceId(value.spaceId as string);
  generatedId(value.pageId as string);
  decimal(value.epoch as string);
  generatedId(value.senderDevice as string);
  generatedId(value.lockId as string);
  requireValue(decimal(value.revision as string) <= BigInt(Number.MAX_SAFE_INTEGER));
  exactKeys(value.holder, ['id', 'name']);
  coreId(value.holder.id as string);
  requireValue(
    typeof value.holder.name === 'string' &&
      value.holder.name.length > 0 &&
      text(value.holder.name).length <= 128 &&
      !/\p{Cc}/u.test(value.holder.name),
  );
  requireValue(key === `${value.holder.id}:edit-lock`);
  editLockTarget(value.target);
  return structuredClone(value) as unknown as EditLockRecord;
}
/** Only typed lock cells may advance in place. Other own records remain immutable. */
export function advancesEditLock(
  previous: unknown,
  next: unknown,
  root: string,
  key: string,
): boolean {
  try {
    const old = editLock(previous, root, key),
      value = editLock(next, root, key);
    return (
      old.spaceId === value.spaceId &&
      old.pageId === value.pageId &&
      old.epoch === value.epoch &&
      old.senderDevice === value.senderDevice &&
      old.holder.id === value.holder.id &&
      decimal(value.revision) > decimal(old.revision)
    );
  } catch {
    return false;
  }
}
/** The publishing owner increments the greatest observed revision. Concurrent writes
 * use writer IDs, never wall clocks. Superseded Yjs values can be compacted. */
export function foldEditLocks(records: readonly EditLockRecord[]): EditLockRecord[] {
  const winners = new Map<string, EditLockRecord>();
  for (const record of records) {
    const old = winners.get(record.holder.id);
    const order = (value: EditLockRecord) => value.senderDevice;
    if (
      !old ||
      decimal(record.revision) > decimal(old.revision) ||
      (record.revision === old.revision && order(record) > order(old))
    )
      winners.set(record.holder.id, record);
  }
  return [...winners.values()].sort((a, b) => (a.holder.id < b.holder.id ? -1 : 1));
}
export function readEditLocks(
  own: OwnState,
  scope: DiscussionScope,
  ownerWriter: (writer: string) => boolean,
): EditLockRecord[] {
  const records: EditLockRecord[] = [];
  for (const [writer, roots] of Object.entries(own)) {
    if (!ownerWriter(writer)) continue;
    for (const [key, value] of Object.entries(roots.messages)) {
      if (!value || typeof value !== 'object' || Array.isArray(value) || value.kind !== 'edit-lock')
        continue;
      const record = editLock(value, 'messages', key);
      if (
        record.senderDevice === writer &&
        record.spaceId === scope.spaceId &&
        record.pageId === scope.pageId &&
        record.epoch === scope.epoch
      )
        records.push(record);
    }
  }
  return foldEditLocks(records);
}
export function activeEditLocks(
  locks: readonly EditLockRecord[],
  presence: (holder: string) => LockPresence,
): EditLockRecord[] {
  return locks.filter(
    (lock) => !lock.released && !['offline', 'ended'].includes(presence(lock.holder.id)),
  );
}
/** Bound current cells, including release tombstones. Edit count does not grow this map. */
export function validateEditLockLimits(own: OwnState) {
  for (const roots of Object.values(own)) {
    let count = 0;
    for (const [root, records] of Object.entries(roots))
      for (const [key, value] of Object.entries(records))
        if (
          value &&
          typeof value === 'object' &&
          !Array.isArray(value) &&
          value.kind === 'edit-lock'
        ) {
          editLock(value, root, key);
          count++;
        }
    requireValue(count <= EDIT_LOCK_HOLDERS_PER_WRITER);
  }
}
