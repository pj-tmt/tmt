import { useEffect, useId, useState } from 'react';
import architectureUrl from '../rendering/assets/modular-architecture-v1.png';
import { ARCHITECTURE_FRAMES, ARCHITECTURE_SOURCE_SIZE } from '../rendering/architecture-art.js';
import { recolorArchitecture } from '../rendering/architecture-material.js';
import type { ModuleMaterial } from './module-contract.js';

const finishes = [
  { value: 'workshop', label: 'Workshop', description: 'Oak and brass' },
  { value: 'moonlight', label: 'Moonlight', description: 'Slate and silver' },
  { value: 'copper', label: 'Copper', description: 'Walnut and copper' },
] as const;

/** Static finish samples use the same atlas and recoloring as the world, without
 * allocating another WebGL scene or maintaining a second set of theme colors.
 */
async function materialSamples(platform: boolean): Promise<Record<ModuleMaterial, string>> {
  const image = new Image();
  image.src = architectureUrl;
  await image.decode();
  if (
    image.naturalWidth !== ARCHITECTURE_SOURCE_SIZE ||
    image.naturalHeight !== ARCHITECTURE_SOURCE_SIZE
  )
    throw new Error('Room finish previews are unavailable.');
  const source = document.createElement('canvas');
  source.width = source.height = ARCHITECTURE_SOURCE_SIZE;
  const context = source.getContext('2d', { willReadFrequently: true });
  const preview = document.createElement('canvas');
  preview.width = 200;
  preview.height = 112;
  const target = preview.getContext('2d');
  if (!context || !target) throw new Error('Room finish previews are unavailable.');
  target.imageSmoothingEnabled = false;
  const samples = {} as Record<ModuleMaterial, string>;
  for (const finish of finishes) {
    context.clearRect(0, 0, source.width, source.height);
    context.drawImage(image, 0, 0);
    if (finish.value !== 'workshop') {
      const pixels = context.getImageData(0, 0, source.width, source.height);
      recolorArchitecture(pixels.data, source.width, finish.value);
      context.putImageData(pixels, 0, 0);
    }
    target.clearRect(0, 0, preview.width, preview.height);
    const floor = ARCHITECTURE_FRAMES.floor;
    const edge = platform ? ARCHITECTURE_FRAMES.rail : ARCHITECTURE_FRAMES.back;
    target.drawImage(
      source,
      floor.x,
      floor.y,
      floor.width,
      floor.height,
      8,
      platform ? 8 : 40,
      184,
      platform ? 88 : 64
    );
    target.drawImage(
      source,
      edge.x,
      edge.y,
      edge.width,
      edge.height,
      8,
      platform ? 96 : 8,
      184,
      platform ? 8 : 54
    );
    samples[finish.value] = preview.toDataURL('image/png');
  }
  return samples;
}

export function RoomMaterialChoices({
  value,
  change,
  platform = false,
}: {
  value: ModuleMaterial;
  change: (value: ModuleMaterial) => void;
  platform?: boolean;
}) {
  const name = useId();
  const [samples, setSamples] = useState<Partial<Record<ModuleMaterial, string>>>({});
  const [error, setError] = useState(false);
  useEffect(() => {
    let active = true;
    void materialSamples(platform).then(
      (result) => {
        if (active) setSamples(result);
      },
      () => {
        if (active) setError(true);
      }
    );
    return () => {
      active = false;
    };
  }, [platform]);
  return (
    <fieldset className="room-material-choices">
      <legend>{platform ? 'Floor & frame' : 'Room style'}</legend>
      <div>
        {finishes.map((finish) => (
          <label key={finish.value}>
            <input
              type="radio"
              aria-label={finish.label}
              name={name}
              value={finish.value}
              checked={value === finish.value}
              onChange={() => change(finish.value)}
            />
            {samples[finish.value] && (
              <img
                src={samples[finish.value]}
                alt={`${finish.label} ${platform ? 'platform' : 'wall and floor'} preview`}
              />
            )}
            <strong>{finish.label}</strong>
            <span>{finish.description}</span>
          </label>
        ))}
      </div>
      {error && <p>Preview unavailable. You can still choose a finish by name.</p>}
    </fieldset>
  );
}
