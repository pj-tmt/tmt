import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

/** #5517: GUI approved ListTable and PageSubheader at the Linux CI measurement:
 * 21,978 -> 22,267 bytes, https://github.com/tmt-ninja/tmt/actions/runs/41143551311/job/124363432535.
 * #5800: GUI approved the one-level rail tree at 22,466 bytes (#5805).
 * #5794: the editorial type ramp's custom properties: 22,466 -> 22,657 bytes, pending GUI approval with
 * the Linux CI measurement on its PR.
 * #5820: the work-panel tokens (row-height, subheader-height, control-size-l, dialog-narrow-width):
 * 22,657 -> 22,685 bytes, approved by the GUI lead.
 * No headroom; later shared CSS changes require a new measurement and decision. */
const approvedGzipBytes = 22_685;
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
