import { isChatThread, type ThreadView } from './thread-records.js';
import { presentationOf, type ThreadPresentation } from './thread-status-presentation.js';

export const threadKey = (thread: ThreadView) => `${thread.ref.writer}:${thread.threadId}`;
export type ThreadFilter = 'open' | 'needs' | 'all';

/** Needs-you is the existing authenticated attention/proposal projection, never a new read. */
export function threadNeedsYou(thread: ThreadView, presentations?: readonly ThreadPresentation[]) {
  return (
    !thread.deleted &&
    !thread.resolved &&
    ((!!thread.proposal && (!thread.decision || thread.decision.deleted)) ||
      !!presentationOf(presentations, thread.ref)?.status.unseen)
  );
}

/** The selected thread stays until explicit navigation, even after resolution or acknowledgement. */
export function listedThreads(
  threads: readonly ThreadView[],
  numbers: ReadonlyMap<string, number>,
  filter: ThreadFilter,
  viewed: string | null,
  presentations?: readonly ThreadPresentation[],
) {
  return threads
    .filter(
      (thread) =>
        !thread.deleted &&
        !isChatThread(thread) &&
        (threadKey(thread) === viewed ||
          filter === 'all' ||
          (filter === 'needs' ? threadNeedsYou(thread, presentations) : !thread.resolved)),
    )
    .sort(
      (a, b) => (numbers.get(threadKey(a)) ?? Infinity) - (numbers.get(threadKey(b)) ?? Infinity),
    );
}

/** The dock opens the first unhandled thread, preferring one that needs the reader. */
export function firstThread(
  threads: readonly ThreadView[],
  presentations?: readonly ThreadPresentation[],
) {
  return threads.find((thread) => threadNeedsYou(thread, presentations)) ?? threads[0];
}
