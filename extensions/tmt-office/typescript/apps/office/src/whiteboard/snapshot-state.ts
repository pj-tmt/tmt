import { decodeWhiteboardDocument } from './document-contract.js';
import type { WhiteboardDocument } from './document-contract.js';
import { decodeWhiteboardCapture } from './snapshot-contract.js';
import type {
  WhiteboardCapture,
  WhiteboardSnapshot,
  WhiteboardSnapshotPort,
} from './snapshot-contract.js';

interface SnapshotPreview {
  busy: boolean;
  pending: boolean;
  intent?: WhiteboardCapture;
  captured?: WhiteboardSnapshot;
  image?: Blob;
  error?: string;
}
interface Pending {
  documentId: string;
  intent: WhiteboardCapture;
  captured?: WhiteboardSnapshot;
  image?: Blob;
}

/** Capture and image admission have separate receipts; uncertain retries reuse both. */
export function createSnapshotState(
  port: WhiteboardSnapshotPort,
  render: (snapshot: WhiteboardSnapshot) => Promise<Blob>,
  operationId: () => string = () => crypto.randomUUID()
) {
  let snapshot: SnapshotPreview = { busy: false, pending: false };
  let pending: Pending | undefined;
  let disposed = false;
  const lifetime = new AbortController();
  const listeners = new Set<() => void>();
  function publish(patch: Partial<SnapshotPreview>) {
    if (disposed) return;
    snapshot = { ...snapshot, ...patch };
    for (const listener of listeners) listener();
  }
  async function run() {
    if (disposed || snapshot.busy || !pending) return;
    const original = pending;
    publish({ busy: true, error: undefined });
    try {
      original.captured ??= await port.capture(
        original.documentId,
        original.intent,
        lifetime.signal
      );
      if (disposed) return;
      publish({ captured: original.captured });
      original.image ??= await render(original.captured);
      if (disposed) return;
      const image = await port.attachImage(original.captured.id, original.image, lifetime.signal);
      if (disposed) return;
      pending = undefined;
      publish({ image, pending: false });
    } catch {
      publish({
        error:
          'Snapshot was not confirmed. Retry keeps the same version, selection and image. If the source changed before capture, start a new preview after reloading the board.',
      });
    } finally {
      publish({ busy: false });
    }
  }
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    async capture(document: WhiteboardDocument, selectedElementIds: string[], annotation: string) {
      if (disposed || snapshot.busy || pending || snapshot.captured) return;
      try {
        const source = decodeWhiteboardDocument(document);
        const intent = decodeWhiteboardCapture({
          expectedRevision: source.revision,
          operationId: operationId(),
          selectedElementIds,
          annotation,
        });
        const ids = new Set(source.scene.elements.map((element) => element.id));
        if (intent.selectedElementIds.some((id) => !ids.has(id)))
          throw new Error('Invalid selection.');
        pending = { documentId: source.id, intent };
        publish({ pending: true, intent, error: undefined });
      } catch {
        publish({
          error: 'Save the board first and use a valid selection and annotation (up to 16 KiB).',
        });
        return;
      }
      await run();
    },
    retry: run,
    reset() {
      if (disposed || snapshot.busy) return;
      pending = undefined;
      publish({
        intent: undefined,
        captured: undefined,
        image: undefined,
        pending: false,
        error: undefined,
      });
    },
    dispose() {
      disposed = true;
      lifetime.abort();
      pending = undefined;
      listeners.clear();
    },
  };
}
export type SnapshotState = ReturnType<typeof createSnapshotState>;
