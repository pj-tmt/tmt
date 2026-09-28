import { memo } from 'react';
import type { PropDefinition, PropPack } from './prop-contract.js';
import { propFrame } from './prop-contract.js';
import { indexedRuns } from '../rendering/indexed-art.js';
import { IndexedProp } from './indexed-prop.js';

/** Fit catalog art, not its transparent placement footprint, into a UI thumbnail. */
export const PropThumbnail = memo(function PropThumbnail({
  pack,
  prop,
}: {
  pack: PropPack;
  prop: PropDefinition;
}) {
  const frame = propFrame(pack, prop);
  let left = Infinity,
    top = Infinity,
    right = -Infinity,
    bottom = -Infinity;
  for (const run of indexedRuns(frame)) {
    left = Math.min(left, run.x);
    top = Math.min(top, run.y);
    right = Math.max(right, run.x + run.width);
    bottom = Math.max(bottom, run.y + 1);
  }
  const scaleX = frame.width / (frame.pixels[0]!.length / frame.indexWidth);
  const scaleY = frame.height / frame.pixels.length;
  const viewBox = Number.isFinite(left)
    ? `${left * scaleX} ${top * scaleY} ${(right - left) * scaleX} ${(bottom - top) * scaleY}`
    : `0 0 ${frame.width} ${frame.height}`;
  return (
    <svg aria-hidden="true" viewBox={viewBox}>
      <IndexedProp pack={pack} prop={prop} />
    </svg>
  );
});
