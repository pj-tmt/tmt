const plain = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' &&
  value !== null &&
  !Array.isArray(value) &&
  Object.getPrototypeOf(value) === Object.prototype;

/** `next`, except that every part equal to the same part of `previous` is `previous`'s own object.
 * An admitted update arrives as a fresh copy of the whole page's state; passing it through here
 * leaves the parts that did not change with the identity they had, so a section keyed on a part
 * does not run again for a change elsewhere. Parts are compared by value, never by position
 * alone: an array keeps its identity only when every element does. */
export function shareView<T>(previous: unknown, next: T): T {
  if (Object.is(previous, next)) return next;
  if (Array.isArray(previous) && Array.isArray(next)) {
    const merged = next.map((value, index) =>
      index < previous.length ? shareView(previous[index], value) : value,
    );
    return (
      merged.length === previous.length && merged.every((value, index) => value === previous[index])
        ? previous
        : merged
    ) as T;
  }
  if (previous instanceof Uint8Array && next instanceof Uint8Array)
    return (
      previous.length === next.length && previous.every((byte, index) => byte === next[index])
        ? previous
        : next
    ) as T;
  if (plain(previous) && plain(next)) {
    const keys = Object.keys(next);
    let same = keys.length === Object.keys(previous).length;
    const merged: Record<string, unknown> = {};
    for (const key of keys) {
      const value = shareView(previous[key], next[key]);
      merged[key] = value;
      if (!Object.hasOwn(previous, key) || value !== previous[key]) same = false;
    }
    return (same ? previous : merged) as T;
  }
  return next;
}
