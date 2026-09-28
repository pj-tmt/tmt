import type { OfficeCamera, SceneRect } from './office-geometry.js';

export interface SelectionAnchor {
  x: number;
  y: number;
}

export interface SelectionTarget {
  bounds: SceneRect;
  viewport: { width: number; height: number };
}

/** Full screen footprint for measured panels that must avoid the selected artwork. */
export function selectionTarget(
  bounds: SceneRect,
  camera: OfficeCamera,
  viewport: SelectionTarget['viewport']
): SelectionTarget | undefined {
  const projected = {
    x: bounds.x * camera.scale + camera.x,
    y: bounds.y * camera.scale + camera.y,
    width: bounds.width * camera.scale,
    height: bounds.height * camera.scale,
  };
  if (
    viewport.width <= 0 ||
    viewport.height <= 0 ||
    projected.x + projected.width <= 0 ||
    projected.x >= viewport.width ||
    projected.y + projected.height <= 0 ||
    projected.y >= viewport.height
  )
    return;
  return { bounds: projected, viewport };
}

export function sameSelectionTarget(
  a: SelectionTarget | undefined,
  b: SelectionTarget | undefined
): boolean {
  return (
    a === b ||
    Boolean(
      a &&
      b &&
      a.bounds.x === b.bounds.x &&
      a.bounds.y === b.bounds.y &&
      a.bounds.width === b.bounds.width &&
      a.bounds.height === b.bounds.height &&
      a.viewport.width === b.viewport.width &&
      a.viewport.height === b.viewport.height
    )
  );
}

/** Prefer a non-overlapping side; clamp to the HUD-safe viewport when space is tight. */
export function placeSelectionPanel(
  target: SelectionTarget,
  size: { width: number; height: number },
  clearance?: { top: number; bottom: number }
) {
  const { bounds, viewport } = target;
  const inset = 12;
  const top = clearance ? Math.max(inset, clearance.top) : Math.min(164, viewport.height * 0.2);
  const bottom = clearance
    ? Math.max(inset, viewport.height - clearance.bottom)
    : Math.min(100, viewport.height * 0.18);
  const maxHeight = Math.max(1, viewport.height - top - bottom);
  const width = Math.min(size.width, Math.max(1, viewport.width - 2 * inset));
  const height = Math.min(size.height, maxHeight);
  const centerX = bounds.x + bounds.width / 2;
  const centerY = bounds.y + bounds.height / 2;
  const candidates = [
    { x: bounds.x + bounds.width + inset, y: centerY - height / 2 },
    { x: bounds.x - width - inset, y: centerY - height / 2 },
    { x: centerX - width / 2, y: bounds.y - height - inset },
    { x: centerX - width / 2, y: bounds.y + bounds.height + inset },
  ].map((candidate) => {
    const x = Math.max(inset, Math.min(viewport.width - width - inset, candidate.x));
    const y = Math.max(top, Math.min(viewport.height - bottom - height, candidate.y));
    const overlap =
      Math.max(0, Math.min(x + width, bounds.x + bounds.width) - Math.max(x, bounds.x)) *
      Math.max(0, Math.min(y + height, bounds.y + bounds.height) - Math.max(y, bounds.y));
    return { x, y, overlap, shift: Math.abs(x - candidate.x) + Math.abs(y - candidate.y) };
  });
  candidates.sort((a, b) => a.overlap - b.overlap || a.shift - b.shift);
  const { x, y } = candidates[0]!;
  return { x, y, maxHeight };
}

/** Project the current selection, not another camera or editable coordinate system. */
export function selectionAnchor(
  bounds: SceneRect,
  camera: OfficeCamera,
  viewport: { width: number; height: number }
): SelectionAnchor | undefined {
  const target = selectionTarget(bounds, camera, viewport);
  if (!target) return;
  const { x: left, y: top, width, height } = target.bounds;
  const right = left + width;
  const bottom = top + height;
  const inset = Math.min(124, viewport.width / 2);
  return {
    x: Math.max(inset, Math.min(viewport.width - inset, (left + right) / 2)),
    y: Math.min(
      Math.max(8, viewport.height - 176),
      Math.max(86, bottom + 76 < viewport.height - 100 ? bottom + 12 : top - 76)
    ),
  };
}
