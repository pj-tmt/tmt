import * as Y from 'yjs';
import {
  STATE_BYTES,
  UPDATE_BYTES,
  validateProjection,
  type FoldCommand,
} from './fold-protocol.js';

// One document per dedicated Worker; only this module imports/decodes Yjs.
let committed = new Y.Doc();
function declare(doc: Y.Doc) {
  doc.getText('html');
  doc.getMap('meta');
}
declare(committed);
function project(doc: Y.Doc) {
  const html = doc.getText('html'),
    meta = doc.getMap('meta');
  if (
    [...doc.share.keys()].some((key) => key !== 'html' && key !== 'meta') ||
    html._map.size !== 0 ||
    meta._start !== null ||
    doc.store.pendingStructs !== null ||
    doc.store.pendingDs !== null ||
    html
      .toDelta()
      .some(
        (part: { insert: unknown; attributes?: unknown }) =>
          typeof part.insert !== 'string' || part.attributes,
      ) ||
    [...meta.keys()].some((key) => key !== 'title') ||
    (meta.has('title') && typeof meta.get('title') !== 'string')
  )
    throw new Error('Rejected content roots or unresolved dependencies');
  const projection = { source: html.toString(), title: (meta.get('title') ?? '') as string };
  validateProjection(projection);
  return projection;
}
self.onmessage = (event: MessageEvent<{ id: number; command: FoldCommand }>) => {
  const { id, command } = event.data;
  const candidate = new Y.Doc();
  declare(candidate);
  try {
    const state = Y.encodeStateAsUpdate(committed);
    Y.applyUpdate(candidate, state);
    let update = new Uint8Array();
    if (command.type === 'apply') {
      if (
        command.updates.length > 200 ||
        command.updates.reduce((n, item) => n + item.length, 0) > UPDATE_BYTES
      )
        throw new Error('Decoder input capacity');
      for (const item of command.updates) Y.applyUpdate(candidate, item);
    } else if (command.type === 'edit') {
      validateProjection({ source: command.source, title: '' });
      const html = candidate.getText('html'),
        old = html.toString(),
        next = command.source;
      let start = 0,
        end = 0;
      while (start < old.length && start < next.length && old[start] === next[start]) start++;
      // Never split a UTF-16 surrogate pair at the diff boundary.
      if (start && /[\uD800-\uDBFF]/.test(old[start - 1])) start--;
      while (
        end < old.length - start &&
        end < next.length - start &&
        old[old.length - end - 1] === next[next.length - end - 1]
      )
        end++;
      if (end && /[\uDC00-\uDFFF]/.test(old[old.length - end])) end--;
      const vector = Y.encodeStateVector(candidate);
      candidate.transact(() => {
        html.delete(start, old.length - start - end);
        html.insert(start, next.slice(start, next.length - end));
      });
      update = new Uint8Array(Y.encodeStateAsUpdate(candidate, vector));
      if (update.length > UPDATE_BYTES) throw new Error('Edit exceeds update capacity');
    } else throw new Error('Invalid decoder command');
    const projection = project(candidate);
    if (Y.encodeStateAsUpdate(candidate).length > STATE_BYTES)
      throw new Error('Decoder state capacity');
    committed.destroy();
    committed = candidate;
    self.postMessage({ id, ...projection, update });
  } catch {
    candidate.destroy();
    self.postMessage({ id, error: 'Rejected content update' });
  }
};
