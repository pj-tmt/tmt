import type { SpacePage, SpacePort } from './space-contract.js';

interface Snapshot extends SpacePage {
  busy: boolean;
  ready: boolean;
  error: string | null;
  observedAtMs: number;
}
/** One bounded page per mounted world. No polling or grant snapshot mirror. */
export function createSpaceState(port: SpacePort, worldId: string, ownerUid: string) {
  let snapshot: Snapshot = {
    entries: [],
    next: null,
    busy: false,
    ready: false,
    error: null,
    observedAtMs: 0,
  };
  let disposed = false;
  const listeners = new Set<() => void>();
  function publish(next: Partial<Snapshot>) {
    if (disposed) return;
    snapshot = { ...snapshot, ...next };
    for (const listener of listeners) listener();
  }
  async function load(after?: string) {
    if (disposed || snapshot.busy) return;
    publish({ busy: true, ready: false, entries: [], next: null, error: null });
    try {
      const page = await port.list(worldId, ownerUid, after);
      publish({ ...page, ready: true, observedAtMs: Date.now() });
    } catch {
      publish({ error: 'Agent spaces unavailable. Refresh to retry.' });
    } finally {
      publish({ busy: false });
    }
  }
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      if (disposed) return () => {};
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    refresh: () => load(),
    next: () => (snapshot.next ? load(snapshot.next) : Promise.resolve()),
    dispose() {
      disposed = true;
      listeners.clear();
      snapshot = {
        entries: [],
        next: null,
        busy: false,
        ready: false,
        error: null,
        observedAtMs: 0,
      };
    },
  };
}
export type SpaceState = ReturnType<typeof createSpaceState>;
