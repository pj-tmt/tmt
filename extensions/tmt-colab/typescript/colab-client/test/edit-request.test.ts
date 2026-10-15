import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { editRequest as r } from '../src/index.js';
const corpus = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/edit-request-v1.json', import.meta.url), 'utf8'),
);
/** Expands `{"fill": text, "count": n}` to the text repeated n times, anywhere in the value. */
function expand(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(expand);
  if (value !== null && typeof value === 'object') {
    const map = value as Record<string, unknown>;
    if (Object.keys(map).length === 2 && 'fill' in map && 'count' in map)
      return (map.fill as string).repeat(map.count as number);
    return Object.fromEntries(Object.entries(map).map(([k, v]) => [k, expand(v)]));
  }
  return value;
}
it('shares its bounds with the vectors and with Rust', () => {
  expect([r.SOURCE_BYTES, r.HUNK_BYTES, r.HUNKS]).toEqual([
    corpus.sourceBytes,
    corpus.hunkBytes,
    corpus.hunks,
  ]);
});
it('admits and refuses exactly what the frozen vectors list', async () => {
  for (const c of corpus.cases) {
    let admitted = true;
    try {
      await r.verifiedEditRequest(expand(c.record), c.root, c.key);
    } catch {
      admitted = false;
    }
    expect(admitted, c.name).toBe(c.valid);
  }
});
