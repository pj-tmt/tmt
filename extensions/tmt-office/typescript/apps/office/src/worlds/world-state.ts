import type { OfficeSession } from '../auth/session.js';
import type { World, WorldDraft, WorldPort } from './world-contract.js';
import { validWorldName } from './world-contract.js';

export interface WorldSnapshot {
  admission: 'signed-out' | 'checking' | 'waiting' | 'approved' | 'error';
  world: World | null;
  loading: boolean;
  busy: boolean;
  error: string | null;
  draft: WorldDraft | null;
}

/** One session-scoped owner for admission and the selected world's subscriptions. */
export function createWorldState(session: OfficeSession, port: WorldPort) {
  let snapshot: WorldSnapshot = {
    admission: 'signed-out',
    world: null,
    loading: false,
    busy: false,
    error: null,
    draft: null,
  };
  let uid: string | undefined;
  let selected: string | undefined;
  let generation = 0;
  let admissionGeneration = 0;
  let worldGeneration = 0;
  let disposed = false;
  let stopAdmission = () => {};
  let stopWorld = () => {};
  const listeners = new Set<() => void>();
  function publish(next: Partial<WorldSnapshot>) {
    if (disposed) return;
    snapshot = { ...snapshot, ...next };
    for (const listener of listeners) listener();
  }
  function clearWorld() {
    worldGeneration++;
    stopWorld();
    stopWorld = () => {};
    publish({ world: null, loading: false });
  }
  function observeWorld() {
    clearWorld();
    if (!uid || !selected || snapshot.admission !== 'approved') return;
    const current = worldGeneration;
    publish({ loading: true, error: null });
    stopWorld = port.watch(
      selected,
      (world) => {
        if (disposed || current !== worldGeneration) return;
        if (world && world.ownerUid === uid) publish({ world, loading: false });
        else publish({ world: null, loading: false, error: 'World unavailable.' });
      },
      () => {
        if (disposed || current !== worldGeneration) return;
        publish({ world: null, loading: false, error: 'World unavailable.' });
      }
    );
  }
  function observeAdmission() {
    if (disposed) return;
    generation++;
    const current = ++admissionGeneration;
    stopAdmission();
    stopAdmission = () => {};
    clearWorld();
    publish({ admission: uid ? 'checking' : 'signed-out', draft: null, busy: false, error: null });
    if (!uid) return;
    stopAdmission = port.watchAdmission(
      uid,
      (enabled) => {
        if (disposed || current !== admissionGeneration) return;
        if (snapshot.admission === (enabled ? 'approved' : 'waiting')) return;
        publish({ admission: enabled ? 'approved' : 'waiting', error: null });
        if (!enabled) {
          generation++;
          publish({ busy: false, draft: null });
          clearWorld();
        } else observeWorld();
      },
      () => {
        if (disposed || current !== admissionGeneration) return;
        generation++;
        clearWorld();
        publish({
          admission: 'error',
          busy: false,
          draft: null,
          error: 'Could not verify access. Please retry.',
        });
      }
    );
  }
  function changedSession() {
    const state = session.getSnapshot();
    const next = state.pending ? undefined : state.user?.uid;
    if (next === uid) return;
    uid = next;
    observeAdmission();
  }
  const unsubscribe = session.subscribe(changedSession);
  changedSession();
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      if (disposed) return () => {};
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    select(id?: string) {
      if (disposed) return;
      if (selected === id) return;
      selected = id;
      observeWorld();
    },
    retryAccess: observeAdmission,
    async create(name: string): Promise<string | null> {
      if (disposed || !uid || snapshot.admission !== 'approved' || snapshot.busy) return null;
      const current = generation;
      if (!snapshot.draft && !validWorldName(name)) {
        publish({
          error: 'Use a nonblank name of at most 80 characters without control characters.',
        });
        return null;
      }
      try {
        const draft = snapshot.draft ?? port.draft(name);
        publish({ draft, busy: true, error: null });
        const id = await port.create(draft, uid);
        if (disposed || current !== generation) return null;
        publish({ draft: null });
        return id;
      } catch {
        if (current === generation)
          publish({
            error: 'World creation could not be confirmed. Retry the same creation when connected.',
          });
        return null;
      } finally {
        if (current === generation) publish({ busy: false });
      }
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      generation++;
      worldGeneration++;
      admissionGeneration++;
      unsubscribe();
      stopAdmission();
      stopWorld();
      listeners.clear();
      snapshot = {
        admission: 'signed-out',
        world: null,
        loading: false,
        busy: false,
        error: null,
        draft: null,
      };
    },
  };
}

export type WorldState = ReturnType<typeof createWorldState>;
