import { describe, expect, it } from 'vitest';
import { documentPoint, dragBox, elementBounds, hitElement, moveElement } from './geometry.js';
import type { WhiteboardElement } from './scene-contract.js';

const box: WhiteboardElement = {
  id: 'box',
  kind: 'rectangle',
  x: 100,
  y: 100,
  width: 200,
  height: 100,
  color: '#000000',
  fill: 'none',
  strokeWidth: 2,
};
const line: WhiteboardElement = {
  id: 'line',
  kind: 'stroke',
  points: [
    [0, 0],
    [100, 100],
    [200, 0],
  ],
  color: '#000000',
  strokeWidth: 4,
};
describe('whiteboard geometry', () => {
  it('maps scaled client coordinates into clamped document pixels', () => {
    const rect = { left: 20, top: 40, width: 800, height: 500 };
    expect(documentPoint(420, 290, rect)).toEqual([800, 500]);
    expect(documentPoint(-10, 1000, rect)).toEqual([0, 1000]);
    expect(dragBox([100, 200], [50, 20])).toEqual({ x: 50, y: 20, width: 50, height: 180 });
  });
  it('picks the frontmost real line or ellipse, not their rectangular empty area', () => {
    expect(hitElement([box, { ...box, id: 'front' }], [150, 150])?.id).toBe('front');
    expect(hitElement([line], [50, 52])?.id).toBe('line');
    expect(hitElement([line], [100, 0])).toBeUndefined();
    expect(hitElement([{ ...box, kind: 'ellipse' }], [101, 101])).toBeUndefined();
    expect(hitElement([{ ...box, kind: 'ellipse' }], [200, 150])?.id).toBe('box');
  });
  it('clamps translation without stretching a path or mutating the original', () => {
    expect(moveElement(box, 2000, 2000)).toMatchObject({
      x: 1400,
      y: 900,
      width: 200,
      height: 100,
    });
    expect(moveElement(line, -100, -100)).toEqual(line);
    expect(moveElement(line, 20, 30)).toMatchObject({
      points: [
        [20, 30],
        [120, 130],
        [220, 30],
      ],
    });
    expect(elementBounds(line)).toEqual({ x: 0, y: 0, width: 200, height: 100 });
  });
});
