import * as Y from 'yjs';
import { attachment, digest, equal, frame, text } from '@tmt/colab-client';
import { sortedJson } from './sorted-json.js';
import {
  BASELINE_UPDATE_BYTES,
  CONTENT_CHUNK_BYTES,
  sameContent,
  validateContentUpdates,
  READ_TAIL_UPDATES,
  STATE_BYTES,
  UPDATE_BYTES,
  WRITE_TAIL_BYTES,
  WRITE_TAIL_UPDATES,
  validateProjection,
  validateOwn,
  type DecoderCommand,
  type ContentPreparation,
  type Projection,
} from './fold-protocol.js';
const hex = (bytes: Uint8Array) => [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');

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
/** Whether `update` sets an own key that the writer's admitted document already holds with a
 * different value. Own records are immutable, but two tabs are two Yjs clients, so a write
 * prepared by a tab whose view lags could merge in and silently replace the admitted value (the
 * client with the higher ID wins, not the later write). The structs are read without integrating
 * them, so a key is judged against what the stream already holds. A byte-identical value is
 * already present. An update that cannot be decoded is judged by the ordinary checks. */
function ownConflict(doc: Y.Doc | undefined, update: Uint8Array) {
  if (!doc) return false;
  let structs: ReturnType<typeof Y.decodeUpdate>['structs'];
  try {
    structs = Y.decodeUpdate(update).structs;
  } catch {
    return false;
  }
  for (const struct of structs) {
    if (!(struct instanceof Y.Item) || typeof struct.parent !== 'string' || !struct.parentSub)
      continue;
    if (!['threads', 'messages', 'intents', 'replies'].includes(struct.parent)) continue;
    const held = doc.getMap(struct.parent).get(struct.parentSub);
    if (held === undefined) continue;
    const content = struct.content.getContent();
    if (content.length !== 1 || JSON.stringify(content[0]) !== JSON.stringify(held)) return true;
  }
  return false;
}
/** A device's checkpoint over its own stream is the state encoding of the one document that
 * holds exactly that writer's admitted updates (never the shared content document, which would
 * reattribute others' updates). The answer is empty, meaning "publish nothing", unless the
 * encoding decodes back to an identical document: same state vector, same bytes when encoded
 * again, same own projection, no unresolved dependencies, and it fits one update. */
function encodeOwn(doc: Y.Doc | undefined) {
  const none = new Uint8Array();
  if (!doc || doc.store.pendingStructs || doc.store.pendingDs) return none;
  const replay = ownDoc();
  try {
    const encoded = Y.encodeStateAsUpdate(doc);
    if (encoded.length === 0 || encoded.length > UPDATE_BYTES) return none;
    Y.applyUpdate(replay, encoded);
    return !replay.store.pendingStructs &&
      !replay.store.pendingDs &&
      equal(Y.encodeStateVector(replay), Y.encodeStateVector(doc)) &&
      equal(Y.encodeStateAsUpdate(replay), encoded) &&
      sortedJson(projectOwn(replay, true)) === sortedJson(projectOwn(doc, true))
      ? encoded
      : none;
  } catch {
    return none;
  } finally {
    replay.destroy();
  }
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
    [...meta.keys()].some(
      (key) =>
        key !== 'title' &&
        key !== 'publisherAgent' &&
        key !== 'originalAuthor' &&
        key !== 'layout' &&
        key !== 'layoutReason' &&
        key !== 'creationRecipient' &&
        key !== 'pageType' &&
        key !== 'shell' &&
        key !== 'attachments',
    ) ||
    (meta.has('title') && typeof meta.get('title') !== 'string')
  )
    throw new Error('Rejected content roots or unresolved dependencies');
  const projection = {
    source: html.toString(),
    title: (meta.get('title') ?? '') as string,
    ...(meta.has('originalAuthor') ? { originalAuthor: meta.get('originalAuthor') as string } : {}),
    ...(meta.has('layout') ? { layout: meta.get('layout') } : {}),
    ...(meta.has('layoutReason') ? { layoutReason: meta.get('layoutReason') } : {}),
    ...(meta.has('publisherAgent') ? { publisherAgent: meta.get('publisherAgent') as string } : {}),
    ...(meta.has('creationRecipient') ? { creationRecipient: meta.get('creationRecipient') } : {}),
    ...(meta.has('attachments') ? { attachments: meta.get('attachments') } : {}),
    ...(meta.has('pageType') ? { pageType: meta.get('pageType') } : {}),
    ...(meta.has('shell') ? { shell: meta.get('shell') } : {}),
  };
  validateProjection(projection);
  return projection;
}
/** Bound UTF-8 bytes while retaining Y.Text's UTF-16 indices. */
function* contentPieces(source: string) {
  let start = 0;
  while (start < source.length) {
    let end = start,
      bytes = 0;
    while (end < source.length) {
      const point = source.codePointAt(end)!;
      const size = point <= 0x7f ? 1 : point <= 0x7ff ? 2 : point <= 0xffff ? 3 : 4;
      if (bytes + size > CONTENT_CHUNK_BYTES) break;
      bytes += size;
      end += point > 0xffff ? 2 : 1;
    }
    yield source.slice(start, end);
    start = end;
  }
}
function contentDiff(old: string, next: string) {
  let start = 0,
    end = 0;
  while (start < old.length && start < next.length && old[start] === next[start]) start++;
  if (start && /[\uD800-\uDBFF]/.test(old[start - 1])) start--;
  while (
    end < old.length - start &&
    end < next.length - start &&
    old[old.length - end - 1] === next[next.length - end - 1]
  )
    end++;
  if (end && /[\uDC00-\uDFFF]/.test(old[old.length - end])) end--;
  return { start, remove: old.length - start - end, insert: next.slice(start, next.length - end) };
}
/** Worker-owned replay guard; the authority-holding parent never parses these bytes. */
export function replayContent(base: Y.Doc, updates: Uint8Array[], expected: Projection) {
  const replay = new Y.Doc();
  declare(replay);
  try {
    clone(base, replay);
    for (const bytes of updates) {
      Y.applyUpdate(replay, bytes);
      project(replay); // Each causal prefix must resolve against the exact admitted base.
    }
    if (!sameContent({ ...project(replay), own: {} }, { ...expected, own: {} }))
      throw new Error('Content replay mismatch');
  } finally {
    replay.destroy();
  }
}
self.onmessage = async (event: MessageEvent<{ id: number; command: DecoderCommand }>) => {
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
    let prepared: ContentPreparation | undefined;
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
        command.updates.length + (command.own?.length ?? 0) >
          (command.type === 'apply' ? READ_TAIL_UPDATES : WRITE_TAIL_UPDATES) ||
        command.updates.reduce((n, item) => n + item.length, 0) +
          (command.own ?? []).reduce((n, item) => n + item.update.length, 0) >
          (command.type === 'apply' ? STATE_BYTES : WRITE_TAIL_BYTES)
      )
        throw new Error('Decoder input capacity');
      for (const item of command.updates) Y.applyUpdate(candidate, item);
      for (const item of command.own ?? []) Y.applyUpdate(writerDoc(item.writer), item.update);
    } else if (command.type === 'encode-own') {
      update = new Uint8Array(encodeOwn(own.get(command.writer)));
    } else if (command.type === 'own-conflict') {
      // The writer is named, so its (possibly still empty) document is part of the answer.
      update = ownConflict(writerDoc(command.writer), command.update)
        ? new Uint8Array([1])
        : new Uint8Array();
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
    } else if (command.type === 'prepare-content') {
      validateProjection({ source: command.source, title: '' });
      validateProjection(command.base);
      if (command.base.pageType === 'shell')
        throw new Error('HTML source operations cannot replace a shell page');
      validateOwn(command.base.own);
      const admittedOwn = Object.fromEntries(
        [...own].map(([writer, doc]) => [writer, projectOwn(doc, true)]),
      );
      validateOwn(admittedOwn);
      const before = { ...project(committed), own: admittedOwn };
      if (!sameContent(before, command.base)) throw new Error('Stale content preparation base');
      const diff = contentDiff(before.source, command.source);
      // The change is checked here against the source being written, never taken on trust.
      const change =
        command.attachments === undefined
          ? undefined
          : attachment.documentChange(command.attachments);
      const next =
        change && !attachment.emptyDocumentChange(change)
          ? attachment.applyDocumentChange(
              before.attachments ?? [],
              change,
              hex(await digest(text(command.source))),
            )
          : undefined;
      const updates: Uint8Array[] = [];
      const capture = (bytes: Uint8Array) => updates.push(new Uint8Array(bytes));
      candidate.on('update', capture);
      try {
        const html = candidate.getText('html');
        const meta = candidate.getMap('meta');
        const pieces = [...contentPieces(diff.insert)];
        if (diff.remove || pieces.length || next) {
          let offset = diff.start;
          for (let index = 0; index < Math.max(1, pieces.length); index++) {
            const piece = pieces[index] ?? '';
            candidate.transact(() => {
              if (index === 0 && diff.remove) html.delete(diff.start, diff.remove);
              if (index === 0 && next) {
                if (next.length) meta.set('attachments', JSON.parse(JSON.stringify(next)));
                else meta.delete('attachments');
              }
              if (piece) html.insert(offset, piece);
            });
            offset += piece.length;
          }
          validateContentUpdates(updates);
        }
      } finally {
        candidate.off('update', capture);
      }
      const expected: typeof before = { ...before, source: command.source };
      if (next) {
        if (next.length) expected.attachments = next;
        else delete expected.attachments;
      }
      if (!sameContent({ ...project(candidate), own: before.own }, expected))
        throw new Error('Content preparation projection mismatch');
      replayContent(committed, updates, expected);
      prepared = updates.length
        ? { kind: 'updates', projection: expected, updates }
        : { kind: 'noop', projection: expected };
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
      for (const root of ['threads', 'intents', 'messages'])
        for (const [key, value] of Object.entries(roots[root])) {
          if (
            ![
              'thread',
              'comment',
              'thread-status',
              'thread-notification',
              'proposal-decision',
              'attachment-publication',
              'edit-request',
            ].includes(value?.kind)
          )
            continue;
          if (JSON.stringify(value) !== JSON.stringify(ownProjection[writer][root][key]))
            throw new Error('Own record is immutable');
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
    if (prepared) self.postMessage({ id, type: 'prepared-content', ...prepared });
    else self.postMessage({ id, ...projection, own: ownProjection, update });
  } catch {
    candidate.destroy();
    changed.forEach((doc) => doc.destroy());
    self.postMessage({ id, error: 'Rejected content update' });
  }
};
