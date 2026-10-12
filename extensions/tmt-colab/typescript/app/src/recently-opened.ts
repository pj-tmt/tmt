import { generatedId, spaceId } from '@tmt/colab-client';
import { record } from './storage.js';

/** Pages remembered per space. */
export const RECENTLY_OPENED = 100;

/**
 * Which pages this device opened, and when, so a quick switcher can offer them first. It is a
 * device-local convenience: it lives in this browser's IndexedDB under one key per space, is never
 * synced, exported or sent anywhere, and an absent, damaged or unavailable record is simply empty.
 */
const key = (space: string) => `recent-pages:${space}`;

/** The newest `RECENTLY_OPENED` entries; equal times keep the smaller ID. */
export function newestOpened(entries: Iterable<[string, number]>, limit = RECENTLY_OPENED) {
  return new Map(
    [...entries]
      .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))
      .slice(0, limit),
  );
}

/** Closed shape: a plain object of page ID to a non-negative whole millisecond time; anything else
 * is ignored entry by entry, never trusted and never thrown. */
function parse(value: unknown): Map<string, number> {
  const entries: [string, number][] = [];
  if (value !== null && typeof value === 'object' && !Array.isArray(value))
    for (const [id, at] of Object.entries(value)) {
      try {
        generatedId(id);
      } catch {
        continue;
      }
      if (Number.isSafeInteger(at) && (at as number) >= 0) entries.push([id, at as number]);
    }
  return newestOpened(entries);
}
const valid = (space: string | undefined): space is string => {
  try {
    spaceId(space as string);
    return true;
  } catch {
    return false;
  }
};

export async function readRecentlyOpened(
  space: string | undefined,
): Promise<ReadonlyMap<string, number>> {
  if (!valid(space)) return new Map();
  try {
    return parse(await record<unknown>(key(space)));
  } catch {
    return new Map();
  }
}

/** Records that `pageId` was opened now. Never throws: a device that cannot store it just forgets. */
export async function noteOpened(
  space: string | undefined,
  pageId: string,
  now = Date.now(),
): Promise<void> {
  if (!valid(space) || !Number.isSafeInteger(now) || now < 0) return;
  try {
    generatedId(pageId);
    const write = async () => {
      const entries = parse(await record<unknown>(key(space)));
      entries.set(pageId, now);
      await record(key(space), Object.fromEntries(newestOpened(entries)));
    };
    // Two tabs read, change and write one record: the lock keeps either's open from being lost.
    if (typeof navigator !== 'undefined' && navigator.locks)
      await navigator.locks.request(key(space), write);
    else await write();
  } catch {
    // A convenience record: nothing depends on it.
  }
}
