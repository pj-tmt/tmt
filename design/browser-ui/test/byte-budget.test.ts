import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

/** Shipped CSS, gzip level 9. GUI approved 21,756 B for the 16px rail action icons and 14px add-row plus
 * (+29 B over the 21,727 B cap); no headroom or UI foundation budget change.
 * Further increases require a lead decision. Raw sizes belong in the PR evidence.
 */
const approvedGzipBytes = 21_756;

it('keeps the shipped stylesheets within the approved gzip cap', () => {
  const size = ['static', 'ui']
    .map((name) =>
      gzipSync(readFileSync(new URL(`../generated/${name}.css`, import.meta.url)), { level: 9 }),
    )
    .reduce((sum, bytes) => sum + bytes.length, 0);
  expect(size).toBeLessThanOrEqual(approvedGzipBytes);
});
