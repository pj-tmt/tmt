import { expect, it } from 'vitest';
import {
  ARCHITECTURE_BORDERS,
  ARCHITECTURE_FRAMES,
  ARCHITECTURE_SOURCE_SIZE,
} from './architecture-art.js';

it('uses one wall construction for rear elevations and foreground cutaways', () => {
  expect(ARCHITECTURE_FRAMES.sill).toBe(ARCHITECTURE_FRAMES.back);
  expect(ARCHITECTURE_BORDERS.sill).toBe(ARCHITECTURE_BORDERS.back);
});

it('keeps every reviewed crop within the bundled image and every fixed cap within its own crop', () => {
  for (const [key, frame] of Object.entries(ARCHITECTURE_FRAMES)) {
    expect(frame.x).toBeGreaterThanOrEqual(0);
    expect(frame.y).toBeGreaterThanOrEqual(0);
    expect(frame.width).toBeGreaterThan(0);
    expect(frame.height).toBeGreaterThan(0);
    expect(frame.x + frame.width).toBeLessThanOrEqual(ARCHITECTURE_SOURCE_SIZE);
    expect(frame.y + frame.height).toBeLessThanOrEqual(ARCHITECTURE_SOURCE_SIZE);
    if (!(key in ARCHITECTURE_BORDERS)) continue;
    const [left, right, top, bottom] =
      ARCHITECTURE_BORDERS[key as keyof typeof ARCHITECTURE_BORDERS];
    expect(left + right).toBeLessThan(frame.width);
    expect(top + bottom).toBeLessThan(frame.height);
  }
});
