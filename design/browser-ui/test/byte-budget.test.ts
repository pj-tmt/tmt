import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

/**
 * What the browser downloads of the two shipped stylesheets (`generated/static.css` plus
 * `generated/ui.css`, gzip level 9) may grow by 15% over main before the v2 style (commit
 * 91441380f: static 14,206 B + ui 4,618 B). The GUI lead approved 15% (21,647 B) for the
 * scoped v2 panel, dialog/confirm, action, notice and shell work; the original 10% predates
 * that scope. Encapsulation is retained; any further increase requires the lead's decision.
 * Raw sizes are reported in each pull request, not gated.
 */
const baselineGzipBytes = 14_206 + 4_618;

it('keeps the shipped stylesheets within 15% of the pre-v2 gzip size', () => {
  const size = ['static', 'ui']
    .map((name) =>
      gzipSync(readFileSync(new URL(`../generated/${name}.css`, import.meta.url)), { level: 9 }),
    )
    .reduce((sum, bytes) => sum + bytes.length, 0);
  expect(size).toBeLessThanOrEqual(Math.floor(baselineGzipBytes * 1.15));
});
