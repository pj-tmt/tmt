import { decimal, generatedId, spaceId } from '@tmt/colab-client';
import { keys, nonempty, requireValue, byteLength } from './closed.js';
import type { OwnState } from './fold-protocol.js';
import type { DiscussionScope } from './thread-records.js';
import type { NeedsYouCandidate } from './needs-you.js';

/** A viewer acknowledges attention, never an answer, review close or proposal decision. */
export type NeedsYouTarget = {
  kind: 'answer' | 'review' | 'proposal';
  id: string;
  version: string;
};
export type NeedsYouAcknowledgement = DiscussionScope & {
  version: 1;
  kind: 'needs-you-handled';
  senderDevice: string;
  actionId: string;
  at: string;
  items: readonly NeedsYouTarget[];
};

export function needsYouKey(
  item: Pick<NeedsYouCandidate, 'kind' | 'id' | 'version' | 'title' | 'agents'>,
) {
  return JSON.stringify([
    item.kind,
    item.id,
    item.version ?? JSON.stringify([item.title, item.agents]),
  ]);
}
export function needsYouTarget(item: NeedsYouCandidate): NeedsYouTarget {
  requireValue(item.kind !== 'reading');
  return {
    kind: item.kind,
    id: item.id,
    version: item.version ?? JSON.stringify([item.title, item.agents]),
  };
}
export function validateNeedsYouAcknowledgement(
  key: string,
  value: unknown,
): asserts value is NeedsYouAcknowledgement {
  const record = keys(value, [
    'version',
    'kind',
    'spaceId',
    'pageId',
    'epoch',
    'senderDevice',
    'actionId',
    'at',
    'items',
  ]);
  requireValue(record.version === 1 && record.kind === 'needs-you-handled');
  spaceId(record.spaceId as string);
  generatedId(record.pageId as string);
  decimal(record.epoch as string);
  generatedId(record.senderDevice as string);
  generatedId(record.actionId as string);
  decimal(record.at as string);
  requireValue(key === `${record.actionId}:needs-you-handled`);
  requireValue(
    Array.isArray(record.items) && record.items.length > 0 && record.items.length <= 1000,
  );
  for (const raw of record.items) {
    const item = keys(raw, ['kind', 'id', 'version']);
    requireValue(['answer', 'review', 'proposal'].includes(item.kind as string));
    nonempty(item.id, 512);
    nonempty(item.version, 64 * 1024);
  }
  requireValue(byteLength(JSON.stringify(record)) <= 256 * 1024);
}

/** `own` is already admitted. Outer writer, page and epoch still bind every acknowledgement. */
export function readNeedsYouAcknowledgements(
  own: OwnState,
  scope: DiscussionScope,
  viewer: string,
): ReadonlySet<string> {
  const handled = new Set<string>();
  for (const [key, value] of Object.entries(own[viewer]?.messages ?? {})) {
    if (
      !value ||
      typeof value !== 'object' ||
      Array.isArray(value) ||
      value.kind !== 'needs-you-handled'
    )
      continue;
    try {
      validateNeedsYouAcknowledgement(key, value);
      requireValue(
        value.senderDevice === viewer &&
          value.spaceId === scope.spaceId &&
          value.pageId === scope.pageId &&
          value.epoch === scope.epoch,
      );
      for (const item of value.items)
        handled.add(JSON.stringify([item.kind, item.id, item.version]));
    } catch {
      // Unrelated or malformed records provide no handled state.
    }
  }
  return handled;
}
