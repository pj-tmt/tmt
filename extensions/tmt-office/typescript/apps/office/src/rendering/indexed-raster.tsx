import { indexedRuns } from './indexed-art.js';
import type { IndexedArt } from './indexed-art.js';
import { INSET_TEXT_FONT, insetTextGeometry } from './inset-text.js';

/** Inert indexed pixels shared by trusted character art and admitted props. */
export function IndexedRaster({
  pixels,
  palette,
  indexWidth = 1,
  width,
  height,
  text,
}: {
  width: number;
  height: number;
} & IndexedArt) {
  // One inert path per visible color bounds DOM size independently of cell count.
  const paths = new Map<string, string[]>();
  for (const { x, y, width: runWidth, color } of indexedRuns({ pixels, palette, indexWidth })) {
    const segments = paths.get(color) ?? [];
    segments.push(`M${x} ${y}h${runWidth}v1h-${runWidth}z`);
    paths.set(color, segments);
  }
  return (
    <svg
      x="0"
      y="0"
      width={width}
      height={height}
      viewBox={`0 0 ${pixels[0]!.length / indexWidth} ${pixels.length}`}
      preserveAspectRatio="none"
      overflow="hidden"
      aria-hidden="true"
      shapeRendering="crispEdges"
    >
      {Array.from(paths, ([color, segments]) => (
        <path key={color} d={segments.join('')} fill={color} />
      ))}
      {text && (
        <svg x={text.x} y={text.y} width={text.width} height={text.height} overflow="hidden">
          <text
            x={text.width / 2}
            y={text.height / 2}
            textAnchor="middle"
            dominantBaseline="central"
            fontFamily={INSET_TEXT_FONT}
            fontSize={insetTextGeometry(text).fontSize}
            fill={text.color}
          >
            {text.value}
          </text>
        </svg>
      )}
    </svg>
  );
}
