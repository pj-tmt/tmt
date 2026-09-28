import type { WorldObject } from '../world-map/world-contract.js';

/** Transient scene projection, never a layout writer. */
export interface CatalogPlacement {
  preview(
    object: WorldObject,
    point: { clientX: number; clientY: number }
  ): { object: WorldObject; problem?: string } | undefined;
  clear(): void;
}
