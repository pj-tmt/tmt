import { Container, Graphics } from 'pixi.js';
import { sceneLabel } from './scene-label.js';
import type { SceneRect } from './office-geometry.js';

/** Keep one line within its own room; full names remain in the accessible HUD. */
export function fitNameplateText(value: string, width: number, measure: (text: string) => number) {
  if (measure(value) <= width) return value;
  if (measure('…') > width) return '';
  const characters = Array.from(value);
  let low = 0;
  let high = characters.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (measure(`${characters.slice(0, middle).join('')}…`) <= width) low = middle;
    else high = middle - 1;
  }
  return `${characters.slice(0, low).join('')}…`;
}

/** Shared non-interactive identity/room label; repaint only when camera scale changes. */
export function sceneNameplate(
  parent: Container,
  value: string,
  x: number,
  y: number,
  width: number,
  anchor: 'top-left' | 'bottom-center' = 'top-left',
  tone: 'name' | 'status' = 'name',
  meeting = false
) {
  const plate = new Graphics();
  parent.addChild(plate);
  const text = sceneLabel(
    parent,
    value,
    x,
    y,
    1.05,
    tone === 'status' ? '#214b41' : '#f6edcf',
    width
  );
  text.style.wordWrap = false;
  if (tone === 'name') text.style.fontFamily = 'ui-monospace, SFMono-Regular, Menlo, monospace';
  let scale: number | undefined;
  let bounds: SceneRect = { x, y, width: 0, height: 0 };
  return {
    zoom(next: number) {
      if (next === scale) return bounds;
      scale = next;
      const size = Math.max(1.05, 12 / next);
      const padding = Math.max(0.6, 5 / next);
      const iconWidth = meeting ? size * 1.6 : 0;
      text.scale.set(size / 16);
      text.text = fitNameplateText(
        value,
        Math.max(0, width - padding * 2 - iconWidth),
        (candidate) => {
          text.text = candidate;
          return text.width;
        }
      );
      const centered = anchor === 'bottom-center';
      const drawnWidth = Math.min(width, text.width + padding * 2 + iconWidth);
      const height = size * 1.4 + padding;
      const left = centered ? x - drawnWidth / 2 : x;
      const top = centered ? y - height : y;
      text.position.set(left + padding, top + padding / 2);
      const color = tone === 'status' ? '#f6f2d9' : '#252623';
      plate.clear();
      if (tone === 'name')
        plate
          .roundRect(left + 0.3, top + 0.5, drawnWidth, height, 0.7)
          .fill({ color: '#000000', alpha: 0.45 });
      plate.roundRect(left, top, drawnWidth, height, 0.6).fill(color);
      if (tone === 'name')
        plate.stroke({ color: '#a99972', alpha: 0.8, width: Math.max(0.12, 0.7 / next) });
      if (meeting) {
        const unit = size / 8;
        const start = left + drawnWidth - padding - iconWidth + unit;
        const mid = top + height / 2;
        for (const offset of [0, 6]) {
          plate
            .rect(start + (offset + 1) * unit, mid - 3 * unit, 3 * unit, 3 * unit)
            .rect(start + offset * unit, mid + unit, 5 * unit, 3 * unit)
            .fill('#c999f2');
        }
      }
      if (tone === 'status')
        plate
          .poly([
            x - 0.45,
            top + height - 0.05,
            x,
            top + height + 0.45,
            x + 0.45,
            top + height - 0.05,
          ])
          .fill(color);
      bounds = { x: left, y: top, width: drawnWidth, height };
      return bounds;
    },
  };
}

/** One actor label stack with shared sizing; conversation attention replaces status upstream. */
export function sceneActorLabel(
  parent: Container,
  name: string,
  activity: string | undefined,
  x: number,
  y: number
) {
  const nameplate = sceneNameplate(parent, name, x, y, 16, 'bottom-center');
  const presence = new Graphics();
  parent.addChild(presence);
  const bubbleLayer = activity ? new Container() : undefined;
  if (bubbleLayer) parent.addChild(bubbleLayer);
  const bubble =
    bubbleLayer && activity
      ? sceneNameplate(bubbleLayer, activity, x, 0, 12, 'bottom-center', 'status')
      : undefined;
  return {
    zoom(next: number) {
      const bounds = nameplate.zoom(next);
      const radius = Math.max(0.18, 2.5 / next);
      presence
        .clear()
        .circle(bounds.x - radius * 2, bounds.y + bounds.height / 2, radius)
        .fill('#6ee4a2')
        .stroke({ color: '#173c32', width: radius / 3 });
      if (bubbleLayer && bubble) {
        bubbleLayer.y = bounds.y - 0.8;
        bubble.zoom(next);
      }
      return bounds;
    },
  };
}
