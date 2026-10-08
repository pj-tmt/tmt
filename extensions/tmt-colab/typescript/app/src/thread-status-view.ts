import { discussionRefKey } from './thread-status.js';
import type { DiscussionScope, ThreadView } from './thread-records.js';

/** Presentation only: callers pass the authenticated fold. A Chat thread never
 * contributes to status controls or the page's unresolved count. */
export function isStatusThread(thread: ThreadView) {
  return !thread.deleted && !(thread.anchor === null && thread.ref.id === thread.ref.writer);
}
export function openThreadCount(threads: readonly ThreadView[]) {
  return threads.filter((thread) => isStatusThread(thread) && !thread.resolved).length;
}

/** Browser-local attention, not authority or a synced read receipt. Only an
 * explicit parent open action marks a winning agent resolution seen. Rendering,
 * marker geometry, closing a window and a page reload never acknowledge it. */
export class ThreadStatusSeen {
  #opened = new Map<string, string>();
  constructor(
    private scope: DiscussionScope,
    private deviceId: string,
    private storage: Pick<Storage, 'getItem' | 'setItem'>,
  ) {}
  #key(thread: ThreadView) {
    return `tmt-colab:thread-status-seen:${this.scope.spaceId}:${this.scope.pageId}:${this.scope.epoch}:${this.deviceId}:${discussionRefKey(thread.ref)}`;
  }
  unseen(thread: ThreadView) {
    const status = thread.status;
    if (!isStatusThread(thread) || !status?.resolved || status.actor !== 'agent') return false;
    if (this.#opened.get(this.#key(thread)) === discussionRefKey(status.ref)) return false;
    try {
      return this.storage.getItem(this.#key(thread)) !== discussionRefKey(status.ref);
    } catch {
      return true;
    }
  }
  opened(thread: ThreadView) {
    const status = thread.status;
    if (!isStatusThread(thread) || !status?.resolved || status.actor !== 'agent') return;
    this.#opened.set(this.#key(thread), discussionRefKey(status.ref));
    try {
      this.storage.setItem(this.#key(thread), discussionRefKey(status.ref));
    } catch {
      // Unavailable browser storage affects attention only, never thread status.
    }
  }
}

/** Shared metadata for Comments rows and the thread-window header. Copy and
 * icons remain owned by those surfaces; labels and clocks stay in parent UI. */
export function projectThreadStatus(
  thread: ThreadView,
  seen?: ThreadStatusSeen,
  /** The local device has owner-member provenance, as the contract requires to
   * resolve or reopen. Cosmetic: publication re-checks current admission. */
  ownerDevice = false,
) {
  const status = thread.status;
  return {
    resolved: thread.resolved,
    controllable: ownerDevice && isStatusThread(thread),
    unseen: seen?.unseen(thread) ?? false,
    actor: status?.actor,
    actorName: status?.actor === 'agent' ? status.agentName : status?.deviceName,
    writer: status?.ref.writer,
    at: status?.at,
  };
}

export type ThreadStatusPresentation = ReturnType<typeof projectThreadStatus>;
