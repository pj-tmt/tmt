import { expect, it } from 'vitest';
import {
  fitOfficeCamera,
  resizeOfficeCamera,
  scenePoint,
  zoomOfficeCamera,
  wheelOfficeCamera,
} from './office-geometry.js';

it('pans ordinary wheel input without changing scale, including line and page deltas', () => {
  const camera = { x: 100, y: 200, scale: 3 };
  for (const [deltaMode, unit] of [
    [0, 1],
    [1, 16],
    [2, 800],
  ]) {
    expect(
      wheelOfficeCamera(
        camera,
        { deltaX: 2, deltaY: -3, deltaMode: deltaMode!, ctrlKey: false },
        300,
        250,
        800,
        1,
        8
      )
    ).toEqual({ x: 100 - 2 * unit!, y: 200 + 3 * unit!, scale: 3 });
  }
});

it('zooms pinch input responsively around the gesture anchor and preserves camera limits', () => {
  const camera = { x: 100, y: 200, scale: 3 };
  const input = { deltaX: 0, deltaY: -20, deltaMode: 0, ctrlKey: true };
  const next = wheelOfficeCamera(camera, input, 300, 250, 800, 1, 8);
  expect(next.scale).toBeCloseTo(3 * Math.exp(0.2));
  const anchor = scenePoint(camera, 300, 250);
  expect(scenePoint(next, 300, 250).x).toBeCloseTo(anchor.x, 12);
  expect(scenePoint(next, 300, 250).y).toBeCloseTo(anchor.y, 12);
  expect(wheelOfficeCamera(camera, { ...input, deltaY: -1e6 }, 300, 250, 800, 1, 8).scale).toBe(32);
});

it.each([
  { x: 0, y: 0, width: 1280, height: 900 },
  { x: 8, y: 12, width: 390, height: 844 },
])('fits signed world bounds beneath the full HUD viewport %#', (viewport) => {
  const bounds = { x: -80, y: -120, width: 108, height: 102 };
  const camera = fitOfficeCamera(bounds, viewport),
    epsilon = 1e-8;
  expect(camera.x + bounds.x * camera.scale).toBeGreaterThanOrEqual(viewport.x - epsilon);
  expect(camera.y + bounds.y * camera.scale).toBeGreaterThanOrEqual(viewport.y - epsilon);
  expect(camera.x + (bounds.x + bounds.width) * camera.scale).toBeLessThanOrEqual(
    viewport.x + viewport.width + epsilon
  );
  expect(camera.y + (bounds.y + bounds.height) * camera.scale).toBeLessThanOrEqual(
    viewport.y + viewport.height + epsilon
  );
  expect(fitOfficeCamera(bounds, { ...viewport, width: 0 })).toEqual({ x: 0, y: 0, scale: 1 });
});
it.each([600, 806, 900])('keeps tall platform layouts below the HUD at height %i', (height) => {
  const bounds = { x: -4, y: -46, width: 192, height: 173 };
  const viewport = { x: 12, y: 8, width: 1512, height };
  const camera = fitOfficeCamera(bounds, viewport);
  const top = camera.y + bounds.y * camera.scale;
  const bottom = camera.y + (bounds.y + bounds.height) * camera.scale;
  expect(top).toBeGreaterThanOrEqual(viewport.y + 96 - 1e-8);
  expect(bottom).toBeLessThanOrEqual(viewport.y + height - 96 + 1e-8);
});

it('keeps Fit usable in a short viewport', () => {
  const camera = fitOfficeCamera(
    { x: 0, y: 0, width: 100, height: 100 },
    { x: 0, y: 0, width: 600, height: 200 }
  );
  expect(camera.scale).toBeCloseTo(1.2);
  expect(camera.y).toBeCloseTo(40);
});

it('keeps the pointer invariant when zooming a panned camera', () => {
  const camera = { x: -143, y: 59, scale: 6 };
  const zoomed = zoomOfficeCamera(camera, 2, 217, 299, 6);
  expect(scenePoint(zoomed, 217, 299)).toEqual(scenePoint(camera, 217, 299));
  expect(zoomed).toEqual({ x: -503, y: -181, scale: 12 });
});
it('keeps a fitted office visible when resizing from desktop to portrait and back', () => {
  const bounds = { x: -80, y: -120, width: 108, height: 102 };
  const desktop = { x: 0, y: 0, width: 1440, height: 1000 };
  const portrait = { x: 0, y: 0, width: 390, height: 844 };
  const initial = fitOfficeCamera(bounds, desktop);
  const narrow = resizeOfficeCamera(initial, bounds, desktop, portrait);
  const expected = fitOfficeCamera(bounds, portrait);
  expect(narrow.x).toBeCloseTo(expected.x);
  expect(narrow.y).toBeCloseTo(expected.y);
  expect(narrow.scale).toBeCloseTo(expected.scale);
  const restored = resizeOfficeCamera(narrow, bounds, portrait, desktop);
  expect(restored.x).toBeCloseTo(initial.x);
  expect(restored.y).toBeCloseTo(initial.y);
  expect(restored.scale).toBeCloseTo(initial.scale);
});
it('preserves a manually panned center and relative zoom instead of resetting to Fit on resize', () => {
  const bounds = { x: -80, y: -120, width: 108, height: 102 };
  const before = { x: 0, y: 0, width: 1440, height: 1000 };
  const after = { x: 0, y: 0, width: 390, height: 844 };
  const camera = { x: -143, y: 59, scale: 18 };
  const resized = resizeOfficeCamera(camera, bounds, before, after);
  const center = scenePoint(camera, before.width / 2, before.height / 2);
  const nextCenter = scenePoint(resized, after.width / 2, after.height / 2);
  expect(nextCenter.x).toBeCloseTo(center.x);
  expect(nextCenter.y).toBeCloseTo(center.y);
  expect(resized.scale / fitOfficeCamera(bounds, after).scale).toBeCloseTo(
    camera.scale / fitOfficeCamera(bounds, before).scale
  );
});
it('bounds relative zoom without reversing Zoom in on large displays or Zoom out on large maps', () => {
  const camera = fitOfficeCamera(
    { x: 0, y: 0, width: 36, height: 56 },
    { x: 0, y: 0, width: 3840, height: 2160 }
  );
  expect(camera.scale).toBeGreaterThan(30);
  const closer = zoomOfficeCamera(camera, 1.2, 1920, 1080, camera.scale);
  expect(closer.scale).toBeCloseTo(camera.scale * 1.2);
  expect(scenePoint(closer, 1920, 1080).x).toBeCloseTo(scenePoint(camera, 1920, 1080).x, 10);
  expect(scenePoint(closer, 1920, 1080).y).toBeCloseTo(scenePoint(camera, 1920, 1080).y, 10);
  expect(zoomOfficeCamera(camera, 1e6, 1920, 1080, camera.scale).scale).toBe(camera.scale * 4);
  expect(zoomOfficeCamera(camera, 1e-6, 1920, 1080, camera.scale).scale).toBe(camera.scale / 4);
  expect(zoomOfficeCamera({ x: 0, y: 0, scale: 1 }, 0.5, 400, 300, 1, 6).scale).toBe(0.5);
});
