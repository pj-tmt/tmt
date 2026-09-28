import type { Point, WhiteboardElement } from './scene-contract.js';
import { WHITEBOARD_LIMITS } from './scene-contract.js';

export function documentPoint(
  clientX: number,
  clientY: number,
  rect: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>
): Point {
  return [
    Math.round(
      Math.max(
        0,
        Math.min(
          WHITEBOARD_LIMITS.width,
          ((clientX - rect.left) / rect.width) * WHITEBOARD_LIMITS.width
        )
      )
    ),
    Math.round(
      Math.max(
        0,
        Math.min(
          WHITEBOARD_LIMITS.height,
          ((clientY - rect.top) / rect.height) * WHITEBOARD_LIMITS.height
        )
      )
    ),
  ];
}

export function elementBounds(element: WhiteboardElement) {
  if (!('points' in element))
    return { x: element.x, y: element.y, width: element.width, height: element.height };
  const xs = element.points.map(([x]) => x);
  const ys = element.points.map(([, y]) => y);
  // Avoid spreading a potentially 65,536-point stroke into function arguments.
  const x = xs.reduce((a, b) => Math.min(a, b));
  const y = ys.reduce((a, b) => Math.min(a, b));
  return {
    x,
    y,
    width: xs.reduce((a, b) => Math.max(a, b)) - x,
    height: ys.reduce((a, b) => Math.max(a, b)) - y,
  };
}

function segmentDistance(point: Point, from: Point, to: Point): number {
  const dx = to[0] - from[0],
    dy = to[1] - from[1];
  const length = dx * dx + dy * dy;
  const t =
    length === 0
      ? 0
      : Math.max(0, Math.min(1, ((point[0] - from[0]) * dx + (point[1] - from[1]) * dy) / length));
  return Math.hypot(point[0] - from[0] - t * dx, point[1] - from[1] - t * dy);
}

export function hitElement(
  elements: readonly WhiteboardElement[],
  point: Point
): WhiteboardElement | undefined {
  function contains(element: WhiteboardElement): boolean {
    if ('points' in element)
      return element.points
        .slice(1)
        .some(
          (to, index) =>
            segmentDistance(point, element.points[index]!, to) <= element.strokeWidth / 2 + 6
        );
    const box = elementBounds(element);
    if (element.kind === 'ellipse')
      return (
        ((point[0] - box.x - box.width / 2) / (box.width / 2)) ** 2 +
          ((point[1] - box.y - box.height / 2) / (box.height / 2)) ** 2 <=
        1
      );
    return (
      point[0] >= box.x &&
      point[1] >= box.y &&
      point[0] <= box.x + box.width &&
      point[1] <= box.y + box.height
    );
  }
  for (let index = elements.length - 1; index >= 0; index--) {
    if (contains(elements[index]!)) return elements[index];
  }
  return undefined;
}

export function moveElement(element: WhiteboardElement, dx: number, dy: number): WhiteboardElement {
  const box = elementBounds(element);
  dx = Math.round(Math.max(-box.x, Math.min(WHITEBOARD_LIMITS.width - box.x - box.width, dx)));
  dy = Math.round(Math.max(-box.y, Math.min(WHITEBOARD_LIMITS.height - box.y - box.height, dy)));
  if ('points' in element)
    return { ...element, points: element.points.map(([x, y]) => [x + dx, y + dy]) };
  return { ...element, x: element.x + dx, y: element.y + dy };
}

export function dragBox(from: Point, to: Point) {
  return {
    x: Math.min(from[0], to[0]),
    y: Math.min(from[1], to[1]),
    width: Math.abs(to[0] - from[0]),
    height: Math.abs(to[1] - from[1]),
  };
}
