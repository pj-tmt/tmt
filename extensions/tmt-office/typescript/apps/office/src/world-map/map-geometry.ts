import type { SceneRect } from '../rendering/office-geometry.js';
import type { MapEdge } from './map-contract.js';
import { mapGeometry } from './map-source.js';
import type { MapSource } from './map-source.js';
import { indexFloor } from './floor-index.js';

export interface MapBoundary {
  readonly edge: MapEdge;
  readonly kind: 'exterior' | 'partition';
  readonly open: boolean;
}
export interface MapGeometry {
  readonly bounds: SceneRect | null;
  readonly tileCount: number;
  readonly boundaries: readonly MapBoundary[];
  /** undefined = outside, null = common circulation floor. */
  areaAt(x: number, y: number): string | null | undefined;
  boundaryAt(edge: MapEdge): MapBoundary | undefined;
}

function edgeKey(edge: MapEdge): string {
  return `${edge.axis}:${edge.x},${edge.y}`;
}

/** The tile inside a mounted wall face, independent of its painted elevation. */
export function wallInteriorTile(edge: MapEdge, face: 'positive' | 'negative') {
  return {
    x: edge.x - Number(edge.axis === 'vertical' && face === 'negative'),
    y: edge.y - Number(edge.axis === 'horizontal' && face === 'negative'),
  };
}

/**
 * Disposable sparse rendering index. Rebuild once per draft change, not per
 * frame. No resources, memberships or saved wall list are owned by the renderer.
 * Invalid connectivity is still previewable; native admission gates Save.
 */
export function projectMap(source: MapSource): MapGeometry {
  const document = mapGeometry(source);
  const floor = indexFloor(document.floor);
  const doors = new Set(document.doors.map(edgeKey));
  const boundaries = new Map<string, MapBoundary>();
  for (const span of document.floor) {
    for (let x = span.start; x < span.end; x++) {
      const y = span.y;
      const neighbors: [number, number, MapEdge][] = [
        [x, y - 1, { x, y, axis: 'horizontal' }],
        [x, y + 1, { x, y: y + 1, axis: 'horizontal' }],
        [x - 1, y, { x, y, axis: 'vertical' }],
        [x + 1, y, { x: x + 1, y, axis: 'vertical' }],
      ];
      for (const [nx, ny, edge] of neighbors) {
        const other = floor.areaAt(nx, ny);
        if (other === span.areaId) continue;
        const key = edgeKey(edge);
        const kind = other === undefined ? 'exterior' : 'partition';
        boundaries.set(key, { edge, kind, open: kind === 'partition' && doors.has(key) });
      }
    }
  }
  return {
    ...floor,
    boundaries: [...boundaries.values()].sort(
      (a, b) =>
        a.edge.x - b.edge.x ||
        a.edge.y - b.edge.y ||
        (a.edge.axis === b.edge.axis ? 0 : a.edge.axis === 'horizontal' ? -1 : 1)
    ),
    boundaryAt: (edge) => boundaries.get(edgeKey(edge)),
  };
}
