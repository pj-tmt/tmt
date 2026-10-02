import { expect, it } from 'vite-plus/test';
import vectors from '../../../../../contracts/snapshot-reference-vectors.json';
import { resolveSnapshotReference, snapshotReference } from './snapshot-reference.js';

it('shares exact local reference admission with native readers', () => {
  for (const item of vectors.valid) {
    expect(resolveSnapshotReference(item.input)).toBe(item.id);
    expect(snapshotReference(item.id)).toBe(`tmt:whiteboard:snapshot:${item.id}`);
  }
  for (const value of vectors.invalid) expect(() => resolveSnapshotReference(value)).toThrow();
});
