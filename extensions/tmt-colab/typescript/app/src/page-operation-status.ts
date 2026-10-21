import { pageTitle } from './page-index.js';
import type { BatchOutcome, PageOperation } from './page-operations.js';
import { text } from './strings.js';

const t = text.pageList;

/** What the list's status line says about a finished batch: acknowledged is not verified, and a
 * stop names the page it stopped at and how far the batch got. */
export function operationStatus(
  batch: BatchOutcome,
  operation: PageOperation,
  total: number,
): { text: string; failed: boolean } {
  if (batch.failed) {
    const code = batch.failed.failure.code as keyof typeof t.operationFailures;
    const reason = t.operationFailures[code] ?? t.operationFailures.UNKNOWN;
    return {
      text:
        total === 1
          ? reason
          : t.partial(batch.done.length, total, pageTitle(batch.failed.page), reason),
      failed: true,
    };
  }
  const only = batch.done[0];
  if (batch.awaiting.length)
    return {
      text:
        total === 1 && only ? t.awaiting(pageTitle(only)) : t.awaitingMany(batch.awaiting.length),
      failed: false,
    };
  const messages = {
    'page.archive': { one: t.archived, many: t.archivedMany },
    'page.unarchive': { one: t.restored, many: t.restoredMany },
    'page.delete': { one: t.deleted, many: t.deletedMany },
  }[operation];
  return {
    text: total === 1 && only ? messages.one(pageTitle(only)) : messages.many(total),
    failed: false,
  };
}
