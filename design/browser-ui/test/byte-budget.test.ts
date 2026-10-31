import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

/** #5815 / #5840: GUI approved each PR's exact measured delta, without headroom.
 * Shared rail, attention dot and metric roles: 22,685 -> 23,006 bytes (+321), Linux CI:
 * https://github.com/tmt-ninja/tmt/actions/runs/41228247407/job/124619513775.
 * A second-merging overlap rebases and re-measures; later growth requires owner approval. */
const approvedGzipBytes = 23_006;
/** #5794: the editorial layer is its own entry, loaded only by prose surfaces, so it has its own cap.
 * Pending GUI approval with the Linux CI measurement on its PR; no headroom. */
const approvedEditorialGzipBytes = 951;

const gzipped = (name: string) =>
  gzipSync(readFileSync(new URL(`../generated/${name}.css`, import.meta.url)), { level: 9 }).length;

it('keeps the shipped stylesheets within the approved gzip cap', () => {
  expect(gzipped('static') + gzipped('ui')).toBeLessThanOrEqual(approvedGzipBytes);
});

it('keeps the editorial layer within its approved gzip cap', () => {
  expect(gzipped('editorial')).toBeLessThanOrEqual(approvedEditorialGzipBytes);
});
