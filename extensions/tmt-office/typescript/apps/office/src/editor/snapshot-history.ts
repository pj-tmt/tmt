// Retained draft work is bounded independently of each document's admission cap.
const HISTORY_BYTES = 16 * 1024 * 1024;
const HISTORY_ENTRIES = 64;

interface Entry<T> {
  readonly value: T;
  readonly bytes: number;
}
export interface SnapshotHistory<T> {
  readonly entries: readonly Entry<T>[];
  readonly cursor: number;
}

/** A decoder must validate and take ownership of input values, without side effects. */
export function snapshotHistory<T>(decode: (value: T) => T) {
  function entry(value: T): Entry<T> {
    const admitted = decode(value);
    return { value: admitted, bytes: new TextEncoder().encode(JSON.stringify(admitted)).length };
  }
  function current(history: SnapshotHistory<T>): T {
    return history.entries[history.cursor]!.value;
  }
  return {
    create: (value: T): SnapshotHistory<T> => ({ entries: [entry(value)], cursor: 0 }),
    current,
    /** Commit once per completed gesture. Cancellation does not commit. */
    commit(history: SnapshotHistory<T>, value: T): SnapshotHistory<T> {
      const next = entry(value);
      if (JSON.stringify(next.value) === JSON.stringify(current(history))) return history;
      const entries = [...history.entries.slice(0, history.cursor + 1), next];
      let bytes = entries.reduce((total, item) => total + item.bytes, 0);
      while (entries.length > 1 && (entries.length > HISTORY_ENTRIES || bytes > HISTORY_BYTES)) {
        bytes -= entries.shift()!.bytes;
      }
      return { entries, cursor: entries.length - 1 };
    },
    undo: (history: SnapshotHistory<T>): SnapshotHistory<T> =>
      history.cursor > 0 ? { ...history, cursor: history.cursor - 1 } : history,
    redo: (history: SnapshotHistory<T>): SnapshotHistory<T> =>
      history.cursor + 1 < history.entries.length
        ? { ...history, cursor: history.cursor + 1 }
        : history,
  };
}
