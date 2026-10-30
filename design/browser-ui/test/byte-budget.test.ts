import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

/** Shipped CSS, gzip level 9. GUI approved 21,727 B for the shared row controls and drop
 * indicator (+80 B over the 21,647 B v2 cap); no headroom or UI foundation budget change.
 * Further increases require a lead decision. Raw sizes belong in the PR evidence.
 */
const approvedGzipBytes = 21_727;

it('keeps the shipped stylesheets within the approved gzip cap', () => {
  const size = ['static', 'ui']
    .map((name) =>
      gzipSync(readFileSync(new URL(`../generated/${name}.css`, import.meta.url)), { level: 9 }),
    )
    .reduce((sum, bytes) => sum + bytes.length, 0);
  expect(size).toBeLessThanOrEqual(approvedGzipBytes);
});
