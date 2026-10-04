import * as Y from 'yjs';
import { digest, equal, frame, text } from '@tmt/colab-client';
import {
  BASELINE_UPDATE_BYTES,
  STATE_BYTES,
  UPDATE_BYTES,
  validateProjection,
  validateOwn,
  type FoldCommand,
} from './fold-protocol.js';

// One shared content document and one own document per admitted writer; only this module imports/decodes Yjs.
let committed = new Y.Doc();
let initialized = false;
const own = new Map<string, Y.Doc>();
function ownDoc() {
  const doc = new Y.Doc();
  for (const name of ['threads', 'messages', 'intents', 'replies']) doc.getMap(name);
  return doc;
}
function clone(source: Y.Doc, target: Y.Doc) {
  Y.applyUpdate(target, Y.encodeStateAsUpdate(source));
  // encodeStateAsUpdate omits pending data. Preserve both pending v2 fragments
  // across unpublished checkpoint steps until the final dependency check.
  if (source.store.pendingStructs) Y.applyUpdateV2(target, source.store.pendingStructs.update);
  if (source.store.pendingDs) Y.applyUpdateV2(target, source.store.pendingDs);
}
function stateBytes(doc: Y.Doc) {
  return (
    Y.encodeStateAsUpdate(doc).length +
    (doc.store.pendingStructs?.update.length ?? 0) +
    (doc.store.pendingDs?.length ?? 0)
  );
}
function projectOwn(doc: Y.Doc, complete: boolean) {
  const names = ['threads', 'messages', 'intents', 'replies'];
  if (
    [...doc.share.keys()].some((key) => !names.includes(key)) ||
    (complete && (doc.store.pendingStructs || doc.store.pendingDs))
  )
    throw new Error('Rejected own roots or unresolved dependencies');
  return Object.fromEntries(
    names.map((name) => {
      const map = doc.getMap(name);
      if (map._length !== 0) throw new Error('Rejected own list mutation');
      return [
        name,
        Object.fromEntries(
          [...map.entries()].map(([key, value]) => {
            // Reject before JSON.stringify can call a shared type's toJSON method.
            if (value instanceof Y.AbstractType) throw new Error('Rejected shared own value');
            return [
              key,
              // Match yrs Any -> serde_json: undefined/nonfinite become null and byte
              // buffers become arrays. Shared Yjs values are never plain record data.
              JSON.parse(
                JSON.stringify(value, (_key, v: unknown) => {
                  if (v instanceof Y.AbstractType) throw new Error('Rejected shared own value');
                  if (v instanceof Uint8Array) return [...v];
                  if (typeof v === 'bigint') return Number(v);
                  return v === undefined ? null : v;
                }),
              ),
            ];
          }),
        ),
      ];
    }),
  );
}
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
  const nextOwn = new Map(own),
    changed = new Map<string, Y.Doc>();
  function writerDoc(writer: string) {
    let doc = changed.get(writer);
    if (!doc) {
      doc = ownDoc();
      const previous = own.get(writer);
      if (previous) clone(previous, doc);
      changed.set(writer, doc);
      nextOwn.set(writer, doc);
    }
    return doc;
  }
  try {
    clone(committed, candidate);
    let update = new Uint8Array();
    if (command.type === 'baseline') {
      if (initialized || command.update.length > BASELINE_UPDATE_BYTES)
        throw new Error('Invalid baseline state or capacity');
      Y.applyUpdate(candidate, command.update);
    } else if (command.type === 'checkpoint') {
      if (command.update.length > STATE_BYTES) throw new Error('Decoder checkpoint capacity');
      Y.applyUpdate(
        command.writer === undefined ? candidate : writerDoc(command.writer),
        command.update,
      );
    } else if (command.type === 'apply' || command.type === 'check') {
      if (
        command.updates.length + (command.own?.length ?? 0) > 200 ||
        command.updates.reduce((n, item) => n + item.length, 0) +
          (command.own ?? []).reduce((n, item) => n + item.update.length, 0) >
          UPDATE_BYTES
      )
        throw new Error('Decoder input capacity');
      for (const item of command.updates) Y.applyUpdate(candidate, item);
      for (const item of command.own ?? []) Y.applyUpdate(writerDoc(item.writer), item.update);
    } else if (command.type === 'prepare-own') {
      if (command.records.length < 1 || command.records.length > 32)
        throw new Error('Own batch capacity');
      const doc = writerDoc(command.writer);
      const vector = Y.encodeStateVector(doc);
      for (const record of command.records) {
        if (!['threads', 'messages', 'intents', 'replies'].includes(record.root))
          throw new Error('Invalid own root');
        const map = doc.getMap(record.root);
        const previous = map.get(record.key);
        if (previous !== undefined && JSON.stringify(previous) !== JSON.stringify(record.value))
          throw new Error('Own record is immutable');
        if (previous === undefined) map.set(record.key, record.value);
      }
      update = new Uint8Array(Y.encodeStateAsUpdate(doc, vector));
      if (update.length > UPDATE_BYTES) throw new Error('Own record exceeds update capacity');
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
    const ownProjection = Object.fromEntries(
      [...nextOwn].map(([writer, doc]) => [writer, projectOwn(doc, command.type !== 'checkpoint')]),
    );
    validateOwn(ownProjection);
    for (const [writer] of changed) {
      const previous = own.get(writer);
      if (!previous) continue;
      const roots = projectOwn(previous, false);
      for (const root of ['threads', 'messages'])
        for (const [key, value] of Object.entries(roots[root])) {
          if (value?.kind !== 'thread' && value?.kind !== 'comment') continue;
          if (JSON.stringify(value) !== JSON.stringify(ownProjection[writer][root][key]))
            throw new Error('Discussion record is immutable');
        }
    }

    if (
      stateBytes(candidate) + [...nextOwn.values()].reduce((n, doc) => n + stateBytes(doc), 0) >
        STATE_BYTES ||
      text(JSON.stringify({ ...projection, own: ownProjection })).length > STATE_BYTES
    )
      throw new Error('Decoder state capacity');
    // Prepared local bytes have no durable receipt yet. Keep their projection out
    // of committed state so later foreign updates cannot publish an unsaved draft.
    if (command.type === 'apply' || command.type === 'baseline' || command.type === 'checkpoint') {
      initialized = true;
      committed.destroy();
      committed = candidate;
      for (const [writer, doc] of changed) {
        own.get(writer)?.destroy();
        own.set(writer, doc);
      }
    } else {
      candidate.destroy();
      changed.forEach((doc) => doc.destroy());
    }
    self.postMessage({ id, ...projection, own: ownProjection, update });
  } catch {
    candidate.destroy();
    changed.forEach((doc) => doc.destroy());
    self.postMessage({ id, error: 'Rejected content update' });
  }
};
