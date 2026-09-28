import type * as Y from 'yjs';
import { decodeWorldDocument, sameWorld } from './world-contract.js';
import type { WorldDocument } from './world-contract.js';
import { WorldYjsDocument } from './world-yjs-document.js';

const LOCAL_EDIT = Symbol('layout gesture');
const OBSERVATION = Symbol('confirmed native layout');
const HISTORY_BYTES = 16 * 1024 * 1024;
const FORMAT_CHANGE = 'layout-source-format-change';

/** Session-local selective history. Native JSON/CAS remains the durable owner;
 * no provider, browser database or untrusted binary-update endpoint is created.
 */
export function createWorldYjs(initial: WorldDocument) {
  let world = decodeWorldDocument(structuredClone(initial));
  let document: WorldYjsDocument;
  let undoManager: Y.UndoManager;
  let destroyed = true;
  let updateBytes = 0;
  let historyNotice: string | undefined;
  let captureGroup: string | undefined;

  function initialize() {
    document = new WorldYjsDocument(world, OBSERVATION);
    undoManager = document.createHistory(LOCAL_EDIT);
    updateBytes = document.encodedSize();
    document.onUpdate((update: Uint8Array) => {
      updateBytes += update.byteLength;
    });
    destroyed = false;
    captureGroup = undefined;
  }
  function destroy() {
    if (destroyed) return;
    undoManager.destroy();
    document.destroy();
    destroyed = true;
  }
  function resume() {
    if (destroyed) initialize();
  }
  function compactIfNeeded() {
    if (updateBytes < HISTORY_BYTES) return;
    destroy();
    initialize();
    historyNotice = 'Older undo history was cleared to keep this session within its memory budget.';
  }
  function update(value: WorldDocument, origin: symbol, group?: string) {
    const admitted = decodeWorldDocument(structuredClone(value));
    if (sameWorld(world, admitted)) return;
    resume();
    compactIfNeeded();
    // A format inverse can hide entities belonging to the newer representation.
    // Do not replay it across an external writer, even if both projections decode.
    if (
      origin === OBSERVATION &&
      [...undoManager.undoStack, ...undoManager.redoStack].some((item) =>
        item.meta.has(FORMAT_CHANGE)
      )
    ) {
      undoManager.clear();
      historyNotice = 'External layout changes cleared the earlier format-conversion history.';
    }
    const formatChanged = world.map.version !== admitted.map.version;
    if (!group || group !== captureGroup || origin !== LOCAL_EDIT) undoManager.stopCapturing();
    captureGroup = group;
    document.replace(admitted, origin);
    if (origin === LOCAL_EDIT && formatChanged)
      undoManager.undoStack.at(-1)?.meta.set(FORMAT_CHANGE, true);
    world = document.snapshot();
  }
  function travel(direction: 'undo' | 'redo') {
    resume();
    captureGroup = undefined;
    undoManager.stopCapturing();
    const item = undoManager[direction]();
    if (!item) return;
    if (item.meta.has(FORMAT_CHANGE))
      (direction === 'undo' ? undoManager.redoStack : undoManager.undoStack)
        .at(-1)
        ?.meta.set(FORMAT_CHANGE, true);
    try {
      world = document.snapshot();
    } catch {
      // A selective inverse across a topology change can be unrenderable.
      // Restore the operation and retain the last admitted projection instead.
      try {
        undoManager[direction === 'undo' ? 'redo' : 'undo']();
        if (!sameWorld(document.snapshot(), world)) throw new Error('Inverse drift');
      } catch {
        destroy();
        initialize();
        historyNotice = 'Undo history was reset after a conflicting topology change.';
      }
      throw new Error(
        'This history step conflicts with the current layout. Reload to start a new history.'
      );
    }
  }
  initialize();
  return {
    get world() {
      return structuredClone(world);
    },
    get canUndo() {
      return !destroyed && undoManager.canUndo();
    },
    get canRedo() {
      return !destroyed && undoManager.canRedo();
    },
    get historyNotice() {
      return historyNotice;
    },
    change: (value: WorldDocument, group?: string) => update(value, LOCAL_EDIT, group),
    stopCapturing() {
      captureGroup = undefined;
      undoManager.stopCapturing();
    },
    observe: (value: WorldDocument) => update(value, OBSERVATION),
    undo: () => travel('undo'),
    redo: () => travel('redo'),
    reset(value: WorldDocument) {
      const admitted = decodeWorldDocument(structuredClone(value));
      destroy();
      world = admitted;
      historyNotice = undefined;
      initialize();
    },
    resume,
    destroy,
  };
}
