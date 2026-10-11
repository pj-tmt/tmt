import { iconActionTooltipPosition } from './icon-action-tooltip.js';

/** Menus share the leaf's below/flip/clamp rule, aligning to the trigger's end. */
export function browserMenuPosition(
  anchor: Pick<DOMRect, 'left' | 'right' | 'top' | 'bottom'>,
  width: number,
  height: number,
  viewport: { left: number; top: number; width: number; height: number },
  gutter = 8,
) {
  const placed = iconActionTooltipPosition(anchor, width, height, viewport, gutter);
  return {
    ...placed,
    left: Math.max(
      viewport.left + gutter,
      Math.min(
        anchor.right - Math.min(width, placed.maxWidth),
        viewport.left + viewport.width - gutter - Math.min(width, placed.maxWidth),
      ),
    ),
  };
}
