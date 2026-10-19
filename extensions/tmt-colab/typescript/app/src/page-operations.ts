import {
  managementFailure,
  type Acknowledgment,
  type ManagementError,
  type ManagementPort,
  type Pending,
} from './management.js';

/** The two signed owner-log statements the page list can send for one page. */
export type PageOperation = 'page.archive' | 'page.delete';

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
