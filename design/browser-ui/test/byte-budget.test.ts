import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

/** #5517: GUI approved ListTable and PageSubheader at the Linux CI measurement:
 * 21,978 -> 22,267 bytes, https://github.com/tmt-ninja/tmt/actions/runs/41143551311/job/124363432535.
 * No headroom; later shared CSS changes require a new measurement and decision. */
/** #5800: the one-level rail tree, its disclosure and unread emphasis: 22,267 -> 22,466 bytes,
 * pending GUI approval with the Linux CI measurement on #5805. */
const approvedGzipBytes = 22_466;

it('keeps the shipped stylesheets within the approved gzip cap', () => {
  const size = ['static', 'ui']
    .map((name) =>
      gzipSync(readFileSync(new URL(`../generated/${name}.css`, import.meta.url)), { level: 9 }),
    )
    .reduce((sum, bytes) => sum + bytes.length, 0);
  expect(size).toBeLessThanOrEqual(approvedGzipBytes);
});
