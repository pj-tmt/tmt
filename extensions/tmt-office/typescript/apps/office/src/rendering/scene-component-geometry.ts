import type { SceneRect } from './office-geometry.js';

/** Inert render inputs. Capability names and handlers never enter the renderer. */
export interface SceneAction {
  id: string;
  label: string;
  available: boolean;
}
/** Painting, selection and actions consume the same projected world rectangle. */
export interface SceneComponent extends SceneAction {
  bounds: SceneRect;
}

export function componentActionBounds(
  component: SceneComponent,
  scale: number,
  {
    expanded = false,
    contentHeight = 0,
    contentWidth,
  }: { expanded?: boolean; contentHeight?: number; contentWidth?: number } = {}
): SceneRect {
  const { bounds } = component;
  const badgeSize = Math.min(bounds.width, bounds.height, Math.max(1.2, 18 / scale));
  const height = expanded
    ? Math.max(1.8, 26 / scale, contentHeight + 2 * Math.max(0.2, 5 / scale))
    : badgeSize;
  const width = expanded
    ? Math.max(
        bounds.width,
        26 / scale,
        Math.min(200 / scale, (contentWidth ?? 200 / scale) + 2 * Math.max(0.5, 8 / scale))
      )
    : badgeSize;
  return {
    x: bounds.x + bounds.width - width,
    y: expanded ? bounds.y - height - 0.2 : bounds.y + bounds.height - badgeSize,
    width,
    height,
  };
}

/** The renderer publishes exactly the rectangles it painted, in paint order. */
export interface ComponentHitArea {
  id: string;
  available: boolean;
  body: SceneRect;
  action: SceneRect;
}

export function componentAt(components: readonly ComponentHitArea[], x: number, y: number) {
  // Unavailable front objects still occlude those behind them; never click through.
  for (let index = components.length - 1; index >= 0; index--) {
    const component = components[index]!;
    if (
      [component.body, component.action].some(
        (area) => x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
      )
    )
      return component.available ? component.id : undefined;
  }
  return undefined;
}
