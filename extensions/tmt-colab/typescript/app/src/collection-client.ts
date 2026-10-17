import { exactKeys, generatedId, requireValue } from '@tmt/colab-client';
import { validateHomeSearch, type HomeSearch } from './home-search.js';
import { jsonResponse } from './registration.js';
import {
  COLLECTIONS,
  COLLECTION_ITEMS,
  COLLECTION_NOTE_BYTES,
  COLLECTION_PERSON_BYTES,
  collectionNameError,
  validSectionId,
  type Collection,
  type CollectionDetail,
  type CollectionItem,
  type CollectionSort,
} from './collections.js';

export const COLLECTION_REPLY_BYTES = 1024 * 1024;
export type CollectionChange =
  | { op: 'rename'; id: string; name: string }
  | { op: 'maintainer'; id: string; maintainer: string }
  | { op: 'sort'; id: string; sort: CollectionSort }
  | { op: 'archive'; id: string; archived: boolean }
  | { op: 'seen'; id: string; target: string }
  | { op: 'clear-seen'; id: string };
export interface CollectionPort {
  directory(signal?: AbortSignal): Promise<Collection[]>;
  read(id: string, signal?: AbortSignal): Promise<CollectionDetail>;
  change(
    command: CollectionChange,
    signal?: AbortSignal,
  ): Promise<{ detail: CollectionDetail; changed: boolean }>;
}
export class CollectionError extends Error {
  constructor(readonly code: string) {
    super(code);
  }
}
function uuid(value: unknown): asserts value is string {
  requireValue(typeof value === 'string');
  generatedId(value);
}
function display(value: unknown, max: number): asserts value is string {
  requireValue(
    typeof value === 'string' &&
      new TextEncoder().encode(value).length <= max &&
      !/\p{Cc}/u.test(value),
  );
}
function sort(value: unknown): asserts value is CollectionSort {
  requireValue(value === 'manual' || value === 'updated' || value === 'title');
}
function savedSearch(value: unknown): HomeSearch {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  const fields = ['q', 'label', 'by', 'sort', 'archived'];
  requireValue(Object.keys(value).every((key) => fields.includes(key)));
  const validated = validateHomeSearch(value as Record<string, unknown>);
  for (const key of fields) {
    if (!Object.hasOwn(value, key)) continue;
    const raw = (value as Record<string, unknown>)[key];
    const parsed = validated[key as keyof HomeSearch];
    // Empty arrays are the omitted default in the native model as well.
    if ((key === 'label' || key === 'by') && Array.isArray(raw) && raw.length === 0) continue;
    requireValue(raw !== undefined && raw !== null);
    requireValue(
      Array.isArray(raw)
        ? Array.isArray(parsed) &&
            raw.length === parsed.length &&
            raw.every((v, i) => v === parsed[i])
        : raw === parsed,
    );
  }
  return Object.fromEntries(Object.entries(validated).filter(([, v]) => v !== undefined));
}
function collection(value: unknown): Collection {
  exactKeys(value, ['id', 'name', 'maintainer', 'archived', 'definition']);
  uuid(value.id);
  requireValue(typeof value.name === 'string' && collectionNameError(value.name) === undefined);
  display(value.maintainer, COLLECTION_PERSON_BYTES);
  requireValue(typeof value.archived === 'boolean');
  const definition = value.definition;
  requireValue(definition !== null && typeof definition === 'object' && !Array.isArray(definition));
  let parsed: Collection['definition'];
  if ('kind' in definition && definition.kind === 'manual') {
    exactKeys(definition, ['kind', 'sort']);
    requireValue('sort' in definition);
    sort(definition.sort);
    parsed = { kind: 'manual', sort: definition.sort };
  } else {
    const keys = ['kind', 'search'];
    if (Object.hasOwn(definition, 'needsYou')) keys.push('needsYou');
    exactKeys(definition, keys);
    requireValue(definition.kind === 'smart');
    if (Object.hasOwn(definition, 'needsYou')) requireValue(definition.needsYou === true);
    parsed = {
      kind: 'smart',
      search: savedSearch(definition.search),
      ...(definition.needsYou === true ? { needsYou: true as const } : {}),
    };
  }
  return {
    id: value.id,
    name: value.name,
    maintainer: value.maintainer,
    archived: value.archived,
    definition: parsed,
  };
}
function item(value: unknown): CollectionItem {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  const keys = ['id', 'pageId', 'note', 'audience'];
  if (Object.hasOwn(value, 'anchor')) keys.push('anchor');
  exactKeys(value, keys);
  uuid(value.id);
  uuid(value.pageId);
  if (Object.hasOwn(value, 'anchor'))
    requireValue(typeof value.anchor === 'string' && validSectionId(value.anchor));
  display(value.note, COLLECTION_NOTE_BYTES);
  display(value.audience, COLLECTION_PERSON_BYTES);
  return {
    id: value.id,
    pageId: value.pageId,
    note: value.note,
    audience: value.audience,
    ...(typeof value.anchor === 'string' ? { anchor: value.anchor } : {}),
  };
}
export function collectionDirectory(value: unknown): Collection[] {
  exactKeys(value, ['kind', 'collections']);
  requireValue(
    value.kind === 'directory' &&
      Array.isArray(value.collections) &&
      value.collections.length <= COLLECTIONS,
  );
  const result = value.collections.map(collection);
  requireValue(new Set(result.map((c) => c.id)).size === result.length);
  return result;
}
export function collectionDetail(
  value: unknown,
  id: string,
): { detail: CollectionDetail; changed: boolean } {
  generatedId(id);
  exactKeys(value, ['kind', 'detail', 'changed']);
  requireValue(value.kind === 'detail' && typeof value.changed === 'boolean');
  exactKeys(value.detail, ['collection', 'items', 'seen']);
  const c = collection(value.detail.collection);
  requireValue(
    c.id === id &&
      Array.isArray(value.detail.items) &&
      value.detail.items.length <= COLLECTION_ITEMS,
  );
  const items = value.detail.items.map(item);
  requireValue(
    new Set(items.map((i) => i.id)).size === items.length &&
      (c.definition.kind === 'manual' || items.length === 0),
  );
  requireValue(
    Array.isArray(value.detail.seen) &&
      value.detail.seen.length <= (c.definition.kind === 'manual' ? COLLECTION_ITEMS : 1000),
  );
  const seen = value.detail.seen.map((s: unknown) => {
    exactKeys(s, ['id', 'atMs']);
    uuid(s.id);
    requireValue(typeof s.atMs === 'number' && Number.isSafeInteger(s.atMs) && s.atMs >= 0);
    requireValue(c.definition.kind === 'smart' || items.some((i) => i.id === s.id));
    return { id: s.id, atMs: s.atMs };
  });
  requireValue(new Set(seen.map((s) => s.id)).size === seen.length);
  return { detail: { collection: c, items, seen }, changed: value.changed };
}
function validateChange(command: CollectionChange) {
  generatedId(command.id);
  const fields = {
    rename: ['op', 'id', 'name'],
    maintainer: ['op', 'id', 'maintainer'],
    sort: ['op', 'id', 'sort'],
    archive: ['op', 'id', 'archived'],
    seen: ['op', 'id', 'target'],
    'clear-seen': ['op', 'id'],
  };
  requireValue(Object.hasOwn(fields, command.op));
  exactKeys(command, fields[command.op]);
  switch (command.op) {
    case 'rename':
      requireValue(
        typeof command.name === 'string' && collectionNameError(command.name) === undefined,
      );
      break;
    case 'maintainer':
      display(command.maintainer, COLLECTION_PERSON_BYTES);
      break;
    case 'sort':
      sort(command.sort);
      break;
    case 'archive':
      requireValue(typeof command.archived === 'boolean');
      break;
    case 'seen':
      generatedId(command.target);
      break;
    case 'clear-seen':
      break;
  }
}
/** Mounted owner transport only. The caller owns Session/generation fencing and trusted actions. */
export class CollectionClient implements CollectionPort {
  constructor(
    readonly mount: URL,
    private readonly request: typeof fetch = fetch,
  ) {}
  async directory(signal?: AbortSignal): Promise<Collection[]> {
    return collectionDirectory(await this.exchange('api/collections', undefined, signal));
  }
  async read(id: string, signal?: AbortSignal): Promise<CollectionDetail> {
    generatedId(id);
    return collectionDetail(await this.exchange(`api/collections/${id}`, undefined, signal), id)
      .detail;
  }
  async change(command: CollectionChange, signal?: AbortSignal) {
    validateChange(command);
    const frozen = structuredClone(command);
    try {
      return collectionDetail(await this.exchange('api/collections', frozen, signal), frozen.id);
    } catch (error) {
      if (error instanceof CollectionError) throw error;
      throw new CollectionError('COLAB_OUTCOME_UNKNOWN');
    }
  }
  private async exchange(
    path: string,
    command?: CollectionChange,
    signal?: AbortSignal,
  ): Promise<unknown> {
    let response: Response;
    try {
      response = await this.request(new URL(path, this.mount), {
        ...(command
          ? {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ version: 1, command }),
            }
          : {}),
        signal,
      });
    } catch (error) {
      if (signal?.aborted) throw error;
      throw new CollectionError(command ? 'COLAB_OUTCOME_UNKNOWN' : 'COLAB_UNAVAILABLE');
    }
    const doubtful = command ? 'COLAB_OUTCOME_UNKNOWN' : 'COLAB_UNAVAILABLE';
    let value: unknown;
    try {
      value = await jsonResponse(
        new Response(response.body, { status: 200 }),
        COLLECTION_REPLY_BYTES,
      );
    } catch {
      throw new CollectionError(doubtful);
    }
    if (!response.ok) {
      try {
        exactKeys(value, ['code', 'message']);
        const statuses: Record<string, number> = {
          COLAB_INPUT_INVALID: 400,
          COLAB_COLLECTION_NOT_FOUND: 404,
          COLAB_CONFLICT: 409,
          COLAB_CAPACITY: 503,
          COLAB_SERVER_MISMATCH: 503,
          COLAB_UNAVAILABLE: 503,
          INVALID: 400,
          CONFLICT: 409,
          UNAVAILABLE: 503,
          CAPACITY: 503,
          DENIED: 403,
          EXPIRED: 403,
        };
        requireValue(
          typeof value.code === 'string' &&
            Object.hasOwn(statuses, value.code) &&
            statuses[value.code] === response.status,
        );
        throw new CollectionError(value.code);
      } catch (error) {
        if (error instanceof CollectionError) throw error;
        throw new CollectionError(doubtful);
      }
    }
    return value;
  }
}
