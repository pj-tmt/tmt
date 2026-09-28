import { footprint } from '../blocks/block-contract.js';
import type { Furniture } from '../blocks/block-contract.js';
import { exactRecord } from '../contracts/record.js';
import type { WorldObject } from './world-contract.js';

/** Physical support within the unrotated art envelope, not a drawing or hit box. */
export interface FloorBase {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export function decodeFloorBase(value: unknown, placement: Furniture): FloorBase {
  const base = exactRecord(value, ['x', 'y', 'width', 'height'], 'floor base');
  if (
    !Object.values(base).every(Number.isInteger) ||
    Number(base.x) < 0 ||
    Number(base.y) < 0 ||
    Number(base.width) < 1 ||
    Number(base.height) < 1 ||
    Number(base.x) + Number(base.width) > placement.footprint.width ||
    Number(base.y) + Number(base.height) > placement.footprint.height
  )
    throw new Error('Floor base must fit inside the artwork envelope.');
  return {
    x: Number(base.x),
    y: Number(base.y),
    width: Number(base.width),
    height: Number(base.height),
  };
}

/** World-space support, independent of the upright art projection and catalog. */
export function floorObjectBounds(object: WorldObject): FloorBase {
  const p = object.placement;
  const base = object.surface.type === 'floor' ? object.surface.base : undefined;
  if (!base) return { x: p.x, y: p.y, ...footprint(p) };
  const { x, y, width, height } = base;
  const w = p.footprint.width,
    h = p.footprint.height;
  const rotated =
    p.rotation === 1
      ? { x: h - y - height, y: x, width: height, height: width }
      : p.rotation === 2
        ? { x: w - x - width, y: h - y - height, width, height }
        : p.rotation === 3
          ? { x: y, y: w - x - width, width: height, height: width }
          : base;
  return { ...rotated, x: p.x + rotated.x, y: p.y + rotated.y };
}
