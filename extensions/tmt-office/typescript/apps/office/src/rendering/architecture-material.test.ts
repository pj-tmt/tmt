import { expect, it } from 'vitest';
import { architectureTone, recolorArchitecture } from './architecture-material.js';
import { ARCHITECTURE_SOURCE_SIZE } from './architecture-art.js';

it('separates cool trim, pale plaster, warm metal and floor timber in the reviewed source', () => {
  expect(architectureTone(65, 85, 80, false)).toBe('structure');
  expect(architectureTone(220, 200, 160, false)).toBe('plaster');
  expect(architectureTone(180, 110, 35, false)).toBe('metal');
  expect(architectureTone(180, 110, 35, true)).toBe('wood');
});

it('changes surface colors, retains source relief and leaves alpha and pixels outside reviewed frames exact', () => {
  const width = ARCHITECTURE_SOURCE_SIZE;
  const original = new Uint8ClampedArray(width * width * 4);
  const at = (x: number, y: number) => (y * width + x) * 4;
  const samples = [
    [100, 130, 65, 85, 80, 255], // Structural crown.
    [800, 122, 65, 85, 80, 255], // Shared side/crown/body crop pixel.
    [100, 200, 220, 200, 160, 255], // Plaster.
    [100, 285, 180, 110, 35, 127], // Warm base trim.
    [700, 720, 180, 110, 35, 255], // Timber.
    [701, 720, 90, 55, 17, 255], // Same timber in shadow.
    [200, 700, 90, 55, 17, 0], // Transparent portal center.
    [1, 1, 65, 85, 80, 255], // Outside admitted frames.
  ];
  for (const [x, y, r, g, b, a] of samples) original.set([r!, g!, b!, a!], at(x!, y!));
  const moonlight = original.slice(),
    copper = original.slice();
  recolorArchitecture(moonlight, width, 'moonlight');
  recolorArchitecture(copper, width, 'copper');
  const pixel = (data: Uint8ClampedArray, x: number, y: number) =>
    Array.from(data.subarray(at(x, y), at(x, y) + 4));
  for (const candidate of [moonlight, copper]) {
    expect(pixel(candidate, 800, 122)).toEqual(pixel(candidate, 100, 130));
    for (let i = 3; i < original.length; i += 4) {
      if (candidate[i] !== original[i]) throw new Error(`Changed alpha at byte ${i}`);
    }
    expect(pixel(candidate, 200, 700)).toEqual(pixel(original, 200, 700));
    expect(pixel(candidate, 1, 1)).toEqual(pixel(original, 1, 1));
    expect(candidate[at(700, 720)]).toBeGreaterThan(candidate[at(701, 720)]!);
    expect(pixel(candidate, 700, 720)).not.toEqual(pixel(original, 700, 720));
  }
  // Blue structural metal versus warm copper, not a uniform whole-room tint.
  const blue = pixel(moonlight, 100, 130),
    warm = pixel(copper, 100, 130);
  expect(blue[2]).toBeGreaterThan(blue[0]!);
  expect(warm[0]).toBeGreaterThan(warm[2]!);
  const plaster = pixel(moonlight, 100, 200);
  expect(Math.max(...plaster.slice(0, 3)) - Math.min(...plaster.slice(0, 3))).toBeLessThan(30);
  expect(pixel(moonlight, 100, 285)).not.toEqual(pixel(moonlight, 700, 720));
});
