import type {
  HistoryCursor,
  HistoryDetail,
  HistoryPage,
  HistoryScope,
  RequestHistoryPort,
} from './request-history-contract.js';

export const CONVERSATION_POLL_MS = 3000;
export const CONVERSATION_OBSERVATION_MS = 15 * 60 * 1000;
export const CONVERSATION_PAGE_SIZE = 10;
interface ConversationSnapshot {
  page?: HistoryPage;
  before?: HistoryCursor;
  details: Record<string, HistoryDetail>;
  detailErrors: Record<string, string>;
  loading: boolean;
  paused: boolean;
  error?: string;
}

/** A bounded chat window over canonical exchanges, never a second message store. */
export function createConversationState(port: RequestHistoryPort, scope: HistoryScope) {
  let snapshot: ConversationSnapshot = {
    details: {},
    detailErrors: {},
    loading: false,
    paused: false,
  };
  const listeners = new Set<() => void>();
  let active = false;
  let disposed = false;
  let deadline = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let reading: AbortController | undefined;
  function publish(next: ConversationSnapshot) {
    if (disposed) return;
    snapshot = next;
    for (const listener of listeners) listener();
  }
  function stopReads() {
    clearTimeout(timer);
    timer = undefined;
    reading?.abort();
    reading = undefined;
  }
  async function refresh(forceDetail = false) {
    if (!active || disposed || reading) return;
    if (Date.now() >= deadline) {
      publish({ ...snapshot, paused: true });
      return;
    }
    clearTimeout(timer);
    const controller = new AbortController();
    reading = controller;
    publish({ ...snapshot, loading: true, error: undefined });
    let succeeded = false;
    try {
      const page = await port.list(
        {
          ...scope,
          limit: CONVERSATION_PAGE_SIZE,
          ...(snapshot.before ? { before: snapshot.before } : {}),
        },
        controller.signal
      );
      if (controller.signal.aborted || disposed) return;
      const previous = new Map(
        snapshot.page?.items.map((item) => [item.requestId, JSON.stringify(item)])
      );
      const details: ConversationSnapshot['details'] = {};
      const detailErrors: ConversationSnapshot['detailErrors'] = {};
      const pending: string[] = [];
      for (const item of page.items) {
        const id = item.requestId;
        const unchanged = previous.get(id) === JSON.stringify(item);
        if (unchanged && snapshot.details[id]) details[id] = snapshot.details[id];
        if (unchanged && !forceDetail && snapshot.detailErrors[id])
          detailErrors[id] = snapshot.detailErrors[id];
        if (forceDetail || (!details[id] && !detailErrors[id])) pending.push(id);
      }
      // Drop displaced/expired bodies immediately; a page holds at most ten exchanges.
      publish({ ...snapshot, page, details, detailErrors });
      let index = 0;
      const readNext = async () => {
        while (!controller.signal.aborted && !disposed && index < pending.length) {
          const id = pending[index++]!;
          try {
            const detail = await port.show(id, controller.signal);
            if (controller.signal.aborted || disposed) return;
            if (
              detail.requestId !== id ||
              (scope.recipientId && detail.recipientId !== scope.recipientId) ||
              (scope.roomId && detail.roomId !== scope.roomId)
            )
              throw new Error('Unexpected conversation scope.');
            publish({ ...snapshot, details: { ...snapshot.details, [id]: detail } });
          } catch {
            if (controller.signal.aborted || disposed) return;
            publish({
              ...snapshot,
              detailErrors: {
                ...snapshot.detailErrors,
                [id]: 'This message could not be read. Refresh to retry.',
              },
            });
          }
        }
      };
      // Bound full-body reads rather than launching one connection per message.
      await Promise.all([readNext(), readNext()]);
      succeeded = true;
      if (!controller.signal.aborted && !disposed) publish({ ...snapshot, loading: false });
    } catch {
      if (!controller.signal.aborted && !disposed)
        publish({
          ...snapshot,
          loading: false,
          error: 'Could not update messages. Existing results are kept; refresh to retry.',
        });
    } finally {
      if (reading === controller) reading = undefined;
      if (succeeded && !controller.signal.aborted && active && !disposed) {
        if (Date.now() >= deadline) publish({ ...snapshot, paused: true });
        else
          timer = setTimeout(
            () => void refresh(),
            Math.min(CONVERSATION_POLL_MS, deadline - Date.now())
          );
      }
    }
  }
  function restart() {
    stopReads();
    deadline = Date.now() + CONVERSATION_OBSERVATION_MS;
    publish({ ...snapshot, paused: false, loading: false });
    void refresh(true);
  }
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    setActive(value: boolean) {
      if (disposed || active === value) return;
      active = value;
      if (active) restart();
      else {
        stopReads();
        publish({ ...snapshot, loading: false });
      }
    },
    refresh: restart,
    older() {
      if (!snapshot.page?.nextBefore || disposed) return;
      publish({ ...snapshot, before: snapshot.page.nextBefore });
      restart();
    },
    latest() {
      if (disposed) return;
      publish({ ...snapshot, before: undefined });
      restart();
    },
    dispose() {
      disposed = true;
      active = false;
      stopReads();
      listeners.clear();
    },
  };
}
export type ConversationState = ReturnType<typeof createConversationState>;
