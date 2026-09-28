import { footprint } from '../blocks/block-contract.js';
import type { Furniture } from '../blocks/block-contract.js';

/** Reverse paint order selects the visible top piece; edges belong to one tile. */
export function furnitureAt(objects: readonly Furniture[], x: number, y: number) {
  for (let index = objects.length - 1; index >= 0; index--) {
    const item = objects[index]!;
    const size = footprint(item);
    if (x >= item.x && y >= item.y && x < item.x + size.width && y < item.y + size.height)
      return index;
  }
  return undefined;
}
