import * as Y from 'yjs';
import { digest, equal, frame, text } from '@tmt/colab-client';
import {
  BASELINE_UPDATE_BYTES,
  STATE_BYTES,
  UPDATE_BYTES,
  validateProjection,
  type FoldCommand,
} from './fold-protocol.js';

// One document per dedicated Worker; only this module imports/decodes Yjs.
let committed = new Y.Doc();
let initialized = false;
function declare(doc: Y.Doc) {
  doc.getText('html');
  doc.getMap('meta');
}
declare(committed);
function project(doc: Y.Doc, complete = true) {
  const html = doc.getText('html'),
    meta = doc.getMap('meta');
  if (
    [...doc.share.keys()].some((key) => key !== 'html' && key !== 'meta') ||
    html._map.size !== 0 ||
    meta._start !== null ||
    (complete && (doc.store.pendingStructs !== null || doc.store.pendingDs !== null)) ||
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
self.onmessage = async (event: MessageEvent<{ id: number; command: FoldCommand }>) => {
  const { id, command } = event.data;
  const candidate = new Y.Doc();
  declare(candidate);
  try {
    const state = Y.encodeStateAsUpdate(committed);
    Y.applyUpdate(candidate, state);
    let update = new Uint8Array();
    if (command.type === 'baseline') {
      if (initialized || command.update.length > BASELINE_UPDATE_BYTES)
        throw new Error('Invalid baseline state or capacity');
      Y.applyUpdate(candidate, command.update);
    } else if (command.type === 'checkpoint') {
      if (command.update.length > STATE_BYTES) throw new Error('Decoder checkpoint capacity');
      Y.applyUpdate(candidate, command.update);
    } else if (command.type === 'apply' || command.type === 'check') {
      if (
        command.updates.length > 200 ||
        command.updates.reduce((n, item) => n + item.length, 0) > UPDATE_BYTES
      )
        throw new Error('Decoder input capacity');
      for (const item of command.updates) Y.applyUpdate(candidate, item);
    } else if (command.type === 'prepare') {
      validateProjection({ source: command.source, title: '' });
      if (command.base !== undefined && command.base !== committed.getText('html').toString())
        throw new Error('Source changed before preparing the edit');
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
    // A writer checkpoint may depend on another writer's checkpoint. Final tail
    // admission requires complete resolution before the parent publishes anything.
    const projection = project(candidate, command.type !== 'checkpoint');
    if (
      command.type === 'baseline' &&
      (projection.title !== command.title ||
        !equal(await digest(text(projection.source)), command.sourceDigest) ||
        !equal(
          await digest(
            frame(
              text('tmt-colab-baseline-v1'),
              text('1'),
              text(projection.source),
              command.update,
            ),
          ),
          command.commitment,
        ))
    )
      throw new Error('Baseline commitment or projection mismatch');
    if (Y.encodeStateAsUpdate(candidate).length > STATE_BYTES)
      throw new Error('Decoder state capacity');
    // Prepared local bytes have no durable receipt yet. Keep their projection out
    // of committed state so later foreign updates cannot publish an unsaved draft.
    if (command.type === 'apply' || command.type === 'baseline' || command.type === 'checkpoint') {
      initialized = true;
      committed.destroy();
      committed = candidate;
    } else candidate.destroy();
    self.postMessage({ id, ...projection, update });
  } catch {
    candidate.destroy();
    self.postMessage({ id, error: 'Rejected content update' });
  }
};
