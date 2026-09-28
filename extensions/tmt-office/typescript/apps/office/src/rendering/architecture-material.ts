import type { ModuleMaterial } from '../world-map/module-contract.js';
import { ARCHITECTURE_FRAMES } from './architecture-art.js';

type Tone = 'structure' | 'plaster' | 'metal' | 'wood';
type Color = readonly [number, number, number];

/** Bundled surface parameters; no geometry, lighting loop or user executable theme. */
const FINISHES: Record<Exclude<ModuleMaterial, 'workshop'>, Record<Tone, Color>> = {
  moonlight: {
    structure: [68, 99, 128],
    plaster: [192, 204, 213],
    metal: [151, 154, 155],
    wood: [136, 132, 122],
  },
  copper: {
    structure: [105, 78, 59],
    plaster: [214, 188, 151],
    metal: [197, 106, 54],
    wood: [113, 68, 38],
  },
};
const SOURCE_LUMA: Record<Tone, number> = { structure: 83, plaster: 202, metal: 133, wood: 122 };

/** Reviewed Workshop art uses cool structural trim, pale plaster and warm metal. */
export function architectureTone(r: number, g: number, b: number, floor: boolean): Tone {
  if (floor) return 'wood';
  if (g >= r * 0.98 && b >= r * 0.75) return 'structure';
  if (r - g < 48 && g - b < 70 && 0.2126 * r + 0.7152 * g + 0.0722 * b > 110) return 'plaster';
  return 'metal';
}

/** Preserve source relief and exact alpha. Only reviewed frame pixels are sampled. */
export function recolorArchitecture(
  pixels: Uint8ClampedArray,
  width: number,
  material: Exclude<ModuleMaterial, 'workshop'>
): void {
  const finish = FINISHES[material];
  const painted = new Uint8Array(pixels.length / 4);
  for (const [key, frame] of Object.entries(ARCHITECTURE_FRAMES)) {
    // Full frames and derived trim crops can overlap; recolor each source pixel once.
    for (let y = frame.y; y < frame.y + frame.height; y++) {
      for (let x = frame.x; x < frame.x + frame.width; x++) {
        const at = (y * width + x) * 4;
        if (!pixels[at + 3] || painted[at / 4]) continue;
        painted[at / 4] = 1;
        const r = pixels[at]!,
          g = pixels[at + 1]!,
          b = pixels[at + 2]!;
        const tone = architectureTone(r, g, b, key === 'floor');
        const shade = (0.2126 * r + 0.7152 * g + 0.0722 * b) / SOURCE_LUMA[tone];
        const target = finish[tone];
        pixels[at] = target[0] * shade;
        pixels[at + 1] = target[1] * shade;
        pixels[at + 2] = target[2] * shade;
      }
    }
  }
}
