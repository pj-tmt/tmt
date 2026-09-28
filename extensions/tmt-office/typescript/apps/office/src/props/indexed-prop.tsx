import type { PropDefinition, PropPack } from './prop-contract.js';
import { propFrame } from './prop-contract.js';
import { IndexedRaster } from '../rendering/indexed-raster.js';
import type { PropCustomization } from './prop-customization.js';

export function IndexedProp({
  pack,
  prop,
  rotation = 0,
  customization,
}: {
  pack: PropPack;
  prop: PropDefinition;
  rotation?: number;
  customization?: PropCustomization;
}) {
  const frame = propFrame(pack, prop, rotation, customization);
  const width = rotation % 2 ? prop.footprint.height : prop.footprint.width;
  const height = rotation % 2 ? prop.footprint.width : prop.footprint.height;
  return (
    <g
      transform={`translate(${width / 2} ${height / 2}) rotate(${frame.rotation * 90}) translate(${-frame.width / 2} ${-frame.height / 2})`}
    >
      <IndexedRaster {...frame} />
    </g>
  );
}
