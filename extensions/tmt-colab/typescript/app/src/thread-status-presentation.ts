import type { ThreadView } from './thread-records.js';
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
) {
  return {
    thread: { ...thread.ref },
    status: projectThreadStatus(thread, seen),
    notificationOutcomes: projectStatusNotifications(thread, asks),
  };
}
export type ThreadPresentation = ReturnType<typeof projectThreadPresentation>;
