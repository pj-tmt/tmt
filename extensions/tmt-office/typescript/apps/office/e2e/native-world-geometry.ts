/**
 * Independent screen coordinates for the installation's unsaved platform fixture.
 * The v8 starter spans source x=0..104 and y=-48..136. The fixed lattice maps
 * columns to 72-unit steps and rows to 64-unit steps before 7/8 depth.
 * The painted floor spans x=0..120 and y=-56..147; framing adds eight units
 * horizontally and twelve vertically, giving (-4,-60,128,215).
 * No dedicated meeting-wing preview expands the fitted bounds.
 * Do not import the production camera or hit-test implementation as its oracle.
 */
export function installationWorldPoint(
  viewport: { x: number; y: number; width: number; height: number },
  x: number,
  y: number
) {
  const margin = Math.min(viewport.height * 0.2, Math.max(96, viewport.height * 0.08));
  const scale = Math.min((viewport.width * 0.84) / 128, (viewport.height - 2 * margin) / 215);
  return {
    x: viewport.x + (viewport.width - 128 * scale) / 2 + (x + 4) * scale,
    y: viewport.y + (viewport.height - 215 * scale) / 2 + (y + 60) * scale,
  };
}
