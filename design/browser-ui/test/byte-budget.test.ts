import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

/** #5517: GUI approved ListTable and PageSubheader at the Linux CI measurement:
 * 21,761 -> 22,003 bytes, https://github.com/tmt-ninja/tmt/actions/runs/41122384789/job/124291864289.
 * No headroom; later shared CSS changes require a new measurement and decision. */
const approvedGzipBytes = 22_003;

it('keeps the shipped stylesheets within the approved gzip cap', () => {
  const size = ['static', 'ui']
    .map((name) =>
      gzipSync(readFileSync(new URL(`../generated/${name}.css`, import.meta.url)), { level: 9 }),
    )
    .reduce((sum, bytes) => sum + bytes.length, 0);
  expect(size).toBeLessThanOrEqual(approvedGzipBytes);
});
