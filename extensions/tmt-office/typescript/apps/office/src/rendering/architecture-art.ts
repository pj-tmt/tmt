import type { SceneRect } from './office-geometry.js';

/** Reviewed source-pixel bounds, not inferred equal cells in a generated sheet. */
export const ARCHITECTURE_SOURCE_SIZE = 1254;
// Front and rear elevations are the same wall. Nine-slicing changes only infill;
// the crown and base retain their authored proportions across supported bounds.
// The infill is opaque on source rows 129..288. Exclude alpha padding so the
// painted crown and base meet neighboring surfaces without background seams.
const STRAIGHT_WALL = { x: 90, y: 129, width: 217, height: 160 } as const;
const STRAIGHT_WALL_BORDERS = [0, 0, 35, 21] as const;
export const ARCHITECTURE_FRAMES = {
  // Straight infill excludes terminal posts: side returns and portals own those
  // silhouettes. Repeating source end caps beside every door duplicates columns.
  back: STRAIGHT_WALL,
  sill: STRAIGHT_WALL,
  // Circulation edges use only the metal crown, never a compressed room facade.
  rail: { x: 90, y: 130, width: 217, height: 40 },
  frontPost: { x: 391, y: 212, width: 54, height: 91 },
  left: { x: 751, y: 26, width: 122, height: 280 },
  right: { x: 1013, y: 26, width: 123, height: 280 },
  leftBody: { x: 751, y: 120, width: 122, height: 115 },
  rightBody: { x: 1013, y: 120, width: 123, height: 115 },
  leftCrown: { x: 751, y: 26, width: 122, height: 100 },
  rightCrown: { x: 1013, y: 26, width: 123, height: 100 },
  portal: { x: 33, y: 656, width: 276, height: 218 },
  sidePortal: { x: 395, y: 633, width: 156, height: 265 },
  // An opaque interior patch excludes the illustrated tile's edge shadow.
  floor: { x: 677, y: 693, width: 198, height: 184 },
  // Existing illuminated bridge art, excluding end posts and transparent margins.
  bridgeDeck: { x: 112, y: 1052, width: 106, height: 43 },
  bridgeRail: { x: 112, y: 1018, width: 106, height: 24 },
  bridgeDock: { x: 421, y: 934, width: 100, height: 58 },
} as const satisfies Record<string, SceneRect>;

export type ArchitectureFrame = keyof typeof ARCHITECTURE_FRAMES;

/** Nine-slice cuts retain structural trim; plain infill has no terminal posts. */
export const ARCHITECTURE_BORDERS = {
  back: STRAIGHT_WALL_BORDERS,
  sill: STRAIGHT_WALL_BORDERS,
  rail: [0, 0, 8, 8],
  frontPost: [10, 10, 25, 15],
  left: [38, 35, 78, 35],
  right: [35, 38, 78, 35],
  leftBody: [38, 35, 0, 0],
  rightBody: [35, 38, 0, 0],
  leftCrown: [0, 0, 0, 0],
  rightCrown: [0, 0, 0, 0],
  portal: [66, 66, 64, 20],
  sidePortal: [40, 40, 78, 26],
} as const;
