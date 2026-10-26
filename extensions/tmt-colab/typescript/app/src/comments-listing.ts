import { isChatThread, type ThreadView } from './thread-records.js';

/** The key a thread has in the thread numbers and as the panel's selected thread. */
export const threadKey = (thread: ThreadView) => `${thread.ref.writer}:${thread.threadId}`;

/**
 * The threads the Comments panel lists and its Discussion arrows walk, in the order of their numbers:
 * the open ones, or every one while Show resolved is on. The thread being viewed stays while it is
 * resolved (a link, a marker, or an agent resolved it under the viewer), so the position never loses
 * the thread it names.
 */
export function listedThreads(
  threads: readonly ThreadView[],
  numbers: ReadonlyMap<string, number>,
  showResolved: boolean,
  viewed: string | null,
) {
  return threads
    .filter(
      (thread) =>
        !thread.deleted &&
        !isChatThread(thread) &&
        (showResolved || !thread.resolved || threadKey(thread) === viewed),
    )
    .sort(
      (a, b) => (numbers.get(threadKey(a)) ?? Infinity) - (numbers.get(threadKey(b)) ?? Infinity),
    );
}

/**
 * Where the panel goes when the thread being viewed is resolved and resolved threads are hidden: the
 * next open thread by number, else the nearest one before it, else none (the list's empty state).
 */
export function afterResolving(
  threads: readonly ThreadView[],
  numbers: ReadonlyMap<string, number>,
  resolved: string,
) {
  const mine = numbers.get(resolved) ?? 0;
  const open = listedThreads(threads, numbers, false, null).filter(
    (thread) => threadKey(thread) !== resolved,
  );
  return (
    open.find((thread) => (numbers.get(threadKey(thread)) ?? 0) > mine) ??
    open.filter((thread) => (numbers.get(threadKey(thread)) ?? 0) < mine).pop()
  );
}
