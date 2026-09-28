import * as Y from 'yjs';
import { decodeWorldDocument } from './world-contract.js';
import type { WorldDocument, WorldObject } from './world-contract.js';

type MapSource = WorldDocument['map'];
type FreeformMap = Extract<MapSource, { version: 1 }>;
type ModuleMap = Exclude<MapSource, FreeformMap>;
type LayoutCell =
  | Omit<FreeformMap, 'areas'>
  | Omit<ModuleMap, 'modules'>
  | FreeformMap['areas'][number]
  | ModuleMap['modules'][number]
  | WorldObject
  | null;

/** The only owner of raw layout shared types. Callers exchange detached domain
 * values; they cannot bypass admission by mutating a stored JSON object.
 * Transactions batch observation, not exception rollback: prepare before writing.
 */
export class WorldYjsDocument {
  readonly #doc = new Y.Doc();
  readonly #values = this.#doc.getMap<LayoutCell>('layout');
  readonly #areas = this.#doc.getArray<string>('areaOrder');
  readonly #objects = this.#doc.getArray<string>('objectOrder');

  constructor(initial: WorldDocument, origin: symbol) {
    this.replace(initial, origin);
  }

  createHistory(origin: symbol): Y.UndoManager {
    return new Y.UndoManager([this.#values, this.#areas, this.#objects], {
      trackedOrigins: new Set([origin]),
      captureTimeout: 500,
    });
  }

  encodedSize(): number {
    return Y.encodeStateAsUpdate(this.#doc).byteLength;
  }

  onUpdate(listener: (update: Uint8Array) => void): () => void {
    this.#doc.on('update', listener);
    return () => this.#doc.off('update', listener);
  }

  replace(input: WorldDocument, origin: symbol): void {
    const value = decodeWorldDocument(structuredClone(input));
    const next = new Map<string, LayoutCell>();
    const { map } = value;
    if (map.version === 1) {
      const { areas, ...topology } = map;
      next.set('topology', topology);
      for (const area of areas) next.set(`area:${area.id}`, area);
    } else {
      const { modules, ...topology } = map;
      next.set('topology', topology);
      for (const module of modules) next.set(`module:${module.area.id}`, module);
    }
    for (const object of value.objects) next.set(`object:${object.id}`, object);
    // Prepare the complete delta before entering the non-rollback transaction.
    const changes = new Map<string, LayoutCell>();
    for (const key of this.#values.keys())
      if (!next.has(key) && this.#values.get(key) !== null) changes.set(key, null);
    for (const [key, cell] of next)
      if (JSON.stringify(this.#values.get(key)) !== JSON.stringify(cell)) changes.set(key, cell);
    const objectOrder = value.objects.map((object) => object.id);
    const areaOrder =
      map.version === 1
        ? map.areas.map((area) => area.id)
        : map.modules.map((module) => module.area.id);
    this.#doc.transact(() => {
      // Explicit tombstones prevent an old move's undo resurrecting an entity
      // deleted by a later untracked observation.
      for (const [key, cell] of changes) this.#values.set(key, cell);
      this.#writeOrder(this.#objects, objectOrder);
      this.#writeOrder(this.#areas, areaOrder);
    }, origin);
  }

  snapshot(): WorldDocument {
    const topology = this.#values.get('topology');
    if (!topology || !('version' in topology)) throw new Error('Missing layout topology.');
    return decodeWorldDocument(
      structuredClone({
        version: 1,
        map: {
          ...topology,
          ...(topology.version === 1
            ? { areas: this.#ordered('area:', this.#areas) }
            : { modules: this.#ordered('module:', this.#areas) }),
        },
        objects: this.#ordered('object:', this.#objects),
      })
    );
  }

  destroy(): void {
    this.#doc.destroy();
  }

  #ordered(prefix: string, order: Y.Array<string>): LayoutCell[] {
    const entries = new Map(
      [...this.#values.entries()]
        .filter(([key, value]) => key.startsWith(prefix) && value !== null)
        .map(([key, value]) => [key.slice(prefix.length), value])
    );
    const ids = [...new Set([...order.toArray(), ...[...entries.keys()].sort()])];
    return ids.flatMap((id) => {
      const value = entries.get(id);
      return value === undefined ? [] : [value];
    });
  }

  #writeOrder(target: Y.Array<string>, next: string[]): void {
    const previous = target.toArray();
    let start = 0;
    while (start < previous.length && start < next.length && previous[start] === next[start])
      start++;
    let end = 0;
    while (
      end < previous.length - start &&
      end < next.length - start &&
      previous[previous.length - end - 1] === next[next.length - end - 1]
    )
      end++;
    const removed = previous.length - start - end;
    if (removed) target.delete(start, removed);
    const inserted = next.slice(start, next.length - end);
    if (inserted.length) target.insert(start, inserted);
  }
}
