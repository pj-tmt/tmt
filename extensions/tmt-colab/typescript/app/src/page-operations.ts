import type { PageSummary } from './transport.js';
import {
  managementFailure,
  type Acknowledgment,
  type ManagementError,
  type ManagementPort,
  type Pending,
} from './management.js';

/** Signed owner-log lifecycle statements available to the page list. */
export type PageOperation = 'page.archive' | 'page.unarchive' | 'page.delete';

export type PageOutcome =
  /** The owner log shows the statement. */
  | { state: 'verified' }
  /** The owner acknowledged it, but its log could not be read back yet: not a claimed success. */
  | { state: 'awaiting' }
  | { state: 'failed'; failure: ManagementError };

/** Read the page's verified management view, sign and send one statement, then verify it. The
 * dialog runs the same four steps with a review between them; the list has no use for a review
 * beyond its own confirmation. */
export async function runPageOperation(
  port: ManagementPort,
  pageId: string,
  operation: PageOperation,
  signal?: AbortSignal,
): Promise<PageOutcome> {
  let pending: Pending | undefined;
  let acknowledgment: Acknowledgment;
  try {
    const view = await port.read(pageId, signal);
    pending = await port.prepare(view, { operation, value: { pageId } });
    acknowledgment = await port.send(pending);
  } catch (error) {
    return { state: 'failed', failure: managementFailure(error, pending !== undefined) };
  }
  try {
    await port.verify(pending, acknowledgment, signal);
    return { state: 'verified' };
  } catch {
    return { state: 'awaiting' };
  }
}

export interface BatchOutcome {
  /** The pages whose statement the owner acknowledged, in order. */
  done: PageSummary[];
  /** The part of `done` whose log could not be read back yet. */
  awaiting: PageSummary[];
  /** The page that stopped the batch, with why; the pages after it were not tried. */
  failed?: { page: PageSummary; failure: ManagementError };
}

/** One signed statement at a time, each through its own read, prepare, send and verify: the owner
 * log admits them in order, and the first refusal stops the batch so nothing later is attempted. */
export async function runPageOperations(
  port: ManagementPort,
  pages: readonly PageSummary[],
  operation: PageOperation,
  signal?: AbortSignal,
): Promise<BatchOutcome> {
  const outcome: BatchOutcome = { done: [], awaiting: [] };
  for (const page of pages) {
    const result = await runPageOperation(port, page.id, operation, signal);
    if (result.state === 'failed') return { ...outcome, failed: { page, failure: result.failure } };
    outcome.done.push(page);
    if (result.state === 'awaiting') outcome.awaiting.push(page);
  }
  return outcome;
}
