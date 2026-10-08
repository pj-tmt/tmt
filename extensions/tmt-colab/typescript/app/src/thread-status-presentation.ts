import type { DiscussionRef, ThreadView } from './thread-records.js';
import type { PageAsk } from './ask-panel.js';
import { projectThreadStatus, type ThreadStatusSeen } from './thread-status-view.js';
import { projectStatusNotifications } from './thread-status-notification.js';

/** Detached presentation inputs, computed once by the authenticated parent.
 * UI receives no storage and never repeats status/Ask/failure association. The
 * controllable shape is cosmetic; current parent admission authorizes actions. */
export function projectThreadPresentation(
  thread: ThreadView,
  asks: readonly PageAsk[],
  seen?: ThreadStatusSeen,
  ownerDevice = false,
) {
  return {
    thread: { ...thread.ref },
    status: projectThreadStatus(thread, seen, ownerDevice),
    notificationOutcomes: projectStatusNotifications(thread, asks),
  };
}
export type ThreadPresentation = ReturnType<typeof projectThreadPresentation>;

/** What a window needs to report after a status change; warnings name mentions
 * the parent could not turn into a recipient. */
export interface ThreadStatusOutcome {
  changed: boolean;
  warnings?: readonly { name: string }[];
}
export function presentationOf(
  presentations: readonly ThreadPresentation[] | undefined,
  ref: DiscussionRef,
) {
  return presentations?.find(
    (value) => value.thread.writer === ref.writer && value.thread.id === ref.id,
  );
}
