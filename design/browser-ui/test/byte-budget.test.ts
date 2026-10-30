import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

/** Shipped CSS, gzip level 9. GUI approved 21,955 B for grouped menus (+194 B
 * over 21,761 B), without headroom or a UI foundation budget change.
 * Measurement: https://github.com/tmt-ninja/tmt/actions/runs/41126465620/job/124308229899
 * Further increases require a lead decision. Raw sizes belong in the PR evidence.
 */
const approvedGzipBytes = 21_955;

it('keeps the shipped stylesheets within the approved gzip cap', () => {
  const size = ['static', 'ui']
    .map((name) =>
      gzipSync(readFileSync(new URL(`../generated/${name}.css`, import.meta.url)), { level: 9 }),
    )
    .reduce((sum, bytes) => sum + bytes.length, 0);
  expect(size).toBeLessThanOrEqual(approvedGzipBytes);
});
