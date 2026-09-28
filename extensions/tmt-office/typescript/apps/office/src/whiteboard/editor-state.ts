import { commitScene, createHistory, currentScene, redoScene, undoScene } from './history.js';
import type { WhiteboardHistory } from './history.js';
import type { WhiteboardDocument, WhiteboardPort, WhiteboardSave } from './document-contract.js';
import type { WhiteboardScene } from './scene-contract.js';

export type WhiteboardLeaveState = 'ready' | 'draft' | 'pending';

interface EditorSnapshot {
  document?: WhiteboardDocument;
  history?: WhiteboardHistory;
  busy: boolean;
  dirty: boolean;
  unconfirmed: boolean;
  error?: string;
}

/** One mounted editor owns its draft, history and in-flight intent. No background writes. */
export function createWhiteboardState(
  port: WhiteboardPort,
  id: string,
  operationId: () => string = () => crypto.randomUUID()
) {
  let snapshot: EditorSnapshot = { busy: false, dirty: false, unconfirmed: false };
  let pending: WhiteboardSave | undefined;
  let disposed = false;
  const lifetime = new AbortController();
  const listeners = new Set<() => void>();
  function publish(patch: Partial<EditorSnapshot>) {
    if (disposed) return;
    snapshot = { ...snapshot, ...patch };
    for (const listener of listeners) listener();
  }
  function replaceHistory(history: WhiteboardHistory) {
    publish({
      history,
      dirty: JSON.stringify(currentScene(history)) !== JSON.stringify(snapshot.document?.scene),
      error: undefined,
    });
  }
  function canEdit() {
    return !disposed && !snapshot.busy && !pending && Boolean(snapshot.history);
  }
  const state = {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    async load() {
      if (disposed || snapshot.busy) return;
      publish({ busy: true, error: undefined });
      try {
        const document = await port.show(id, lifetime.signal);
        if (disposed) return;
        pending = undefined;
        publish({
          document,
          history: createHistory(document.scene),
          dirty: false,
          unconfirmed: false,
        });
      } catch {
        publish({ error: 'Whiteboard could not load. Your current draft has been kept.' });
      } finally {
        publish({ busy: false });
      }
    },
    edit(scene: WhiteboardScene) {
      if (!canEdit()) return;
      try {
        replaceHistory(commitScene(snapshot.history!, scene));
      } catch {
        publish({ error: 'This edit exceeds the whiteboard format or document limits.' });
      }
    },
    undo() {
      if (canEdit()) replaceHistory(undoScene(snapshot.history!));
    },
    redo() {
      if (canEdit()) replaceHistory(redoScene(snapshot.history!));
    },
    async save() {
      if (disposed || snapshot.busy || !snapshot.document || !snapshot.history) return;
      if (!pending && !snapshot.dirty && snapshot.document.revision !== 0) return;
      pending ??= {
        expectedRevision: snapshot.document.revision,
        operationId: operationId(),
        scene: currentScene(snapshot.history),
      };
      const intent = pending;
      publish({ busy: true, error: undefined });
      try {
        const receipt = await port.save(id, intent, lifetime.signal);
        if (disposed) return;
        pending = undefined;
        publish({
          document: {
            id,
            scene: intent.scene,
            revision: receipt.revision,
            updatedAtMs: receipt.updatedAtMs,
          },
          dirty: false,
          unconfirmed: false,
        });
      } catch {
        publish({
          unconfirmed: true,
          error:
            'Save was not confirmed or the document changed elsewhere. Your draft is kept. Retry the same save, or explicitly discard and reload.',
        });
      } finally {
        publish({ busy: false });
      }
    },
    dispose() {
      disposed = true;
      lifetime.abort();
      listeners.clear();
    },
  };
  return state;
}
export type WhiteboardState = ReturnType<typeof createWhiteboardState>;
