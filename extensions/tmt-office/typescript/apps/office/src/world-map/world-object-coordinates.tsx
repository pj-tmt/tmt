import { MAP_LIMITS } from './map-contract.js';
import { WORLD_LIMITS } from './world-contract.js';
import type { WorldObject } from './world-contract.js';
import { footprint } from '../blocks/block-contract.js';

/** Browser input text stays local until one complete coordinate edit is applied. */
export function WorldObjectCoordinates({
  object,
  change,
}: {
  object: WorldObject;
  change: (object: WorldObject) => void;
}) {
  const maximumElevation = WORLD_LIMITS.wallHeight - footprint(object.placement).height;
  return (
    <form
      aria-label="Object coordinates"
      onSubmit={(event) => {
        event.preventDefault();
        if (!event.currentTarget.reportValidity()) return;
        const input = new FormData(event.currentTarget);
        const x = Number(input.get('x'));
        const y = Number(input.get('y'));
        const elevation = Number(input.get('elevation'));
        if (
          ![x, y].every(
            (value) => Number.isInteger(value) && Math.abs(value) <= MAP_LIMITS.coordinate
          ) ||
          (object.surface.type === 'wall' &&
            (!Number.isInteger(elevation) || elevation < 0 || elevation > maximumElevation))
        )
          return;
        change({
          ...object,
          placement: { ...object.placement, x, y },
          surface:
            object.surface.type === 'wall' ? { ...object.surface, elevation } : object.surface,
        });
      }}
    >
      {(['x', 'y'] as const).map((axis) => (
        <label key={axis}>
          {axis.toUpperCase()}
          <input
            name={axis}
            type="number"
            required
            step="1"
            min={-MAP_LIMITS.coordinate}
            max={MAP_LIMITS.coordinate}
            defaultValue={object.placement[axis]}
          />
        </label>
      ))}
      {object.surface.type === 'wall' && (
        <label>
          Elevation
          <input
            name="elevation"
            type="number"
            required
            step="1"
            min="0"
            max={maximumElevation}
            defaultValue={object.surface.elevation}
          />
        </label>
      )}
      <button type="submit">Apply coordinates</button>
      <p>Apply coordinates to update this object. Changes are saved automatically.</p>
    </form>
  );
}
