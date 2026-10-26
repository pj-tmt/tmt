/**
 * The global search seam (#4841). Each module offers the navigation items it already knows, the
 * root collects them in one registry, and the one search dialog shows them grouped by module.
 * The registry holds providers, their order and their latest snapshots. It has no query, no
 * keyboard handling and no presentation, reads nothing of its own and activates nothing: an item
 * is an existing route that the host's own navigation opens.
 */

/** One result: where it goes and how it reads. */
export interface SearchItem {
  /** Stable within its provider; a selection finds the item again by this key. */
  key: string;
  label: string;
  /** A second line (a kind, a state); shown and matched as text. */
  detail?: string;
  /** Extra words a query can match that the label and detail do not carry. */
  words?: readonly string[];
  /** A glyph name from the leaf's glyph set; the dialog draws its own default for another name. */
  icon?: string;
  /** An existing route of this origin: `/path`, with an optional query or hash. */
  href: string;
}

/**
 * `unavailable`: there is no admitted current view of this module (not read yet, refused, not
 * paired). `ready`: the module's items, where an empty list is a confirmed empty.
 */
export type SearchSnapshot =
  | { readonly state: 'unavailable' }
  | { readonly state: 'ready'; readonly items: readonly SearchItem[] };

/** What one module contributes, from data its surface already holds and already filters. */
export interface SearchProvider {
  /** The module's stable key, unique in a registry. */
  readonly key: string;
  /** The group heading. */
  readonly label: string;
  /** The current snapshot: the same object until it changes. */
  getSnapshot(): SearchSnapshot;
  /** Calls `listener` after each change of the snapshot; returns the unsubscribe. */
  subscribe(listener: () => void): () => void;
}

/** A provider as the dialog lists it: unavailable groups have no items. */
export interface SearchGroup {
  readonly key: string;
  readonly label: string;
  readonly state: SearchSnapshot['state'];
  readonly items: readonly SearchItem[];
}

export interface SearchRegistry {
  /**
   * Adds a provider after its current snapshot is admitted; returns the withdrawal. A key that is
   * already registered, or an invalid snapshot, throws and leaves the registry as it was.
   */
  register(provider: SearchProvider): () => void;
  /** The groups in registration order: the same array until something changes. */
  groups(): readonly SearchGroup[];
  /** The item as the latest snapshot has it, or nothing once it is withdrawn or replaced. */
  find(groupKey: string, itemKey: string): SearchItem | undefined;
  /** Calls `listener` after the groups change; returns the unsubscribe. */
  subscribe(listener: () => void): () => void;
  /** Withdraws every provider and listener. Later calls are inert. */
  dispose(): void;
}

const noop = () => {};

function isRoute(href: string): boolean {
  return (
    href.startsWith('/') &&
    !href.startsWith('//') &&
    !href.startsWith('/\\') &&
    !/\s/.test(href) &&
    ![...href].some((char) => char.charCodeAt(0) < 0x20 || char.charCodeAt(0) === 0x7f)
  );
}

const isText = (value: unknown): value is string => typeof value === 'string' && value !== '';

function itemProblem(item: unknown): string | undefined {
  if (typeof item !== 'object' || item === null) return 'an item is not an object';
  const { key, label, detail, words, icon, href } = item as Record<string, unknown>;
  if (!isText(key)) return 'an item has no key';
  if (!isText(label)) return `item ${key} has no label`;
  if (typeof href !== 'string' || !isRoute(href)) return `item ${key} has no route of this origin`;
  if (detail !== undefined && typeof detail !== 'string') return `item ${key} has a bad detail`;
  if (icon !== undefined && typeof icon !== 'string') return `item ${key} has a bad icon`;
  if (words !== undefined && (!Array.isArray(words) || words.some((w) => typeof w !== 'string')))
    return `item ${key} has bad words`;
  return undefined;
}

/** Why a snapshot is not admitted, or nothing when it is. Provider tests can assert on it. */
export function searchSnapshotProblem(snapshot: unknown): string | undefined {
  if (typeof snapshot !== 'object' || snapshot === null) return 'the snapshot is not an object';
  const { state, items } = snapshot as { state?: unknown; items?: unknown };
  if (state === 'unavailable') return undefined;
  if (state !== 'ready' || !Array.isArray(items))
    return 'the snapshot is neither unavailable nor ready with items';
  const keys = new Set<string>();
  for (const item of items) {
    const problem = itemProblem(item);
    if (problem) return problem;
    const { key } = item as SearchItem;
    if (keys.has(key)) return `item ${key} appears twice`;
    keys.add(key);
  }
  return undefined;
}

/** A group never shows rows the provider did not admit: a bad or throwing snapshot is unavailable. */
function admitted(provider: SearchProvider): SearchSnapshot {
  try {
    const snapshot = provider.getSnapshot();
    if (!searchSnapshotProblem(snapshot)) return snapshot;
  } catch {
    // An unreadable provider contributes nothing.
  }
  return { state: 'unavailable' };
}

function groupOf(provider: SearchProvider, snapshot: SearchSnapshot): SearchGroup {
  return {
    key: provider.key,
    label: provider.label,
    state: snapshot.state,
    items: snapshot.state === 'ready' ? snapshot.items : [],
  };
}

export function createSearchRegistry(): SearchRegistry {
  // Each entry keeps its group until its own provider notifies, so a change reads one provider.
  const entries = new Map<
    string,
    { provider: SearchProvider; stop(): void; group: SearchGroup | undefined }
  >();
  const listeners = new Set<() => void>();
  let cache: readonly SearchGroup[] | undefined;
  let disposed = false;
  const changed = () => {
    cache = undefined;
    for (const listener of Array.from(listeners)) listener();
  };
  const groups = () =>
    (cache ??= [...entries.values()].map(
      (entry) => (entry.group ??= groupOf(entry.provider, admitted(entry.provider))),
    ));
  return {
    register(provider) {
      if (disposed) return noop;
      if (!isText(provider.key) || !isText(provider.label))
        throw new Error('A search provider needs a key and a label');
      if (entries.has(provider.key))
        throw new Error(`A search provider is already registered for ${provider.key}`);
      const snapshot = provider.getSnapshot();
      const problem = searchSnapshotProblem(snapshot);
      if (problem) throw new Error(`Search provider ${provider.key}: ${problem}`);
      const stop = provider.subscribe(() => {
        const own = entries.get(provider.key);
        if (own?.provider !== provider) return;
        own.group = undefined;
        changed();
      });
      const entry = { provider, stop, group: groupOf(provider, snapshot) };
      entries.set(provider.key, entry);
      changed();
      return () => {
        if (entries.get(provider.key) !== entry) return;
        entries.delete(provider.key);
        stop();
        changed();
      };
    },
    groups,
    find: (groupKey, itemKey) =>
      groups()
        .find((group) => group.key === groupKey)
        ?.items.find((item) => item.key === itemKey),
    subscribe(listener) {
      if (disposed) return noop;
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      for (const { stop } of entries.values()) stop();
      entries.clear();
      listeners.clear();
      cache = undefined;
    },
  };
}
