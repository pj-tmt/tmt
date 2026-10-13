import { expect, it } from 'vite-plus/test';
import { sortedJson } from '../src/sorted-json.js';

it('keeps the value and drops the order an object happens to hold its keys in', () => {
  const listed = { objectId: 'a', filename: 'dot.png', nested: { z: 1, y: [{ b: 2, a: 1 }] } };
  const again = { nested: { y: [{ a: 1, b: 2 }], z: 1 }, filename: 'dot.png', objectId: 'a' };
  expect(sortedJson([{ kind: 'document' }, [listed]])).toBe(
    sortedJson([{ kind: 'document' }, [again]]),
  );
  // An array is an order, and a different value is a different key.
  expect(sortedJson([1, 2])).not.toBe(sortedJson([2, 1]));
  expect(sortedJson({ a: 1 })).not.toBe(sortedJson({ a: 2 }));
  expect(sortedJson({ a: null, b: [] })).toBe('{"a":null,"b":[]}');
});
