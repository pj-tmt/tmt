import { expect, it } from 'vitest';
import { componentActionBounds, componentAt } from './scene-component-geometry.js';
import type { SceneComponent } from './scene-component-geometry.js';
import { fitOfficeCamera } from './office-geometry.js';
import { worldObjectRect } from './world-geometry.js';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';

const entry: SceneComponent = {
  id: 'board',
  label: 'Open board',
  available: true,
  bounds: { x: 3, y: 47, width: 12, height: 8 },
};
function hitAreas(components: SceneComponent[], scale = 16) {
  return components.map((component) => ({
    id: component.id,
    available: component.available,
    body: component.bounds,
    action: componentActionBounds(component, scale),
  }));
}

it('picks painted bodies and plaques, with unavailable front objects preventing click-through', () => {
  expect(componentAt(hitAreas([entry]), 4, 50)).toBe('board');
  expect(componentAt(hitAreas([entry]), 15, 50)).toBeUndefined();
  const plaque = componentActionBounds(entry, 16);
  expect(componentAt(hitAreas([entry]), plaque.x + 1, plaque.y + 0.5)).toBe('board');
  expect(componentAt(hitAreas([{ ...entry, available: false }]), 4, 50)).toBeUndefined();
  expect(componentAt(hitAreas([entry, { ...entry, id: 'front' }]), 4, 50)).toBe('front');
  expect(componentAt(hitAreas([entry, { ...entry, available: false }]), 4, 50)).toBeUndefined();
});

it('uses the same projected wall rectangle for interaction, not the unprojected floor coordinates', () => {
  const object = officeWorldFixture().layout.objects[0]!;
  for (const axis of ['horizontal', 'vertical'] as const) {
    const bounds = worldObjectRect({
      ...object,
      surface: { type: 'wall', axis, face: 'positive', elevation: 8 },
    });
    const component = { ...entry, bounds };
    expect(
      componentAt(hitAreas([component]), bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
    ).toBe('board');
    expect(
      componentAt(hitAreas([component]), object.placement.x + 1, object.placement.y + 1)
    ).toBeUndefined();
  }
});

it('keeps idle badges within object width and expands measured labels at fitted zoom', () => {
  for (const size of [36, 512]) {
    for (const width of [390, 1440]) {
      const camera = fitOfficeCamera(
        { x: -20, y: -20, width: size, height: size },
        { x: 0, y: 0, width, height: 844 }
      );
      const plaque = componentActionBounds(entry, camera.scale);
      expect(plaque.width).toBeLessThanOrEqual(entry.bounds.width);
      expect(plaque.height).toBe(plaque.width);
      const expanded = componentActionBounds(entry, camera.scale, {
        expanded: true,
        contentHeight: 8,
      });
      expect(expanded.width * camera.scale).toBeGreaterThanOrEqual(200 - 1e-8);
      expect(expanded.height).toBeGreaterThan(8);
      expect(plaque.x + plaque.width).toBeCloseTo(entry.bounds.x + entry.bounds.width);
      expect(plaque.y + plaque.height).toBeCloseTo(entry.bounds.y + entry.bounds.height);
      expect(expanded.y + expanded.height).toBeCloseTo(entry.bounds.y - 0.2);
    }
  }
});

it('keeps an attached idle control within a shallow object and preserves its hover target', () => {
  const component = { ...entry, bounds: { x: 0, y: 0, width: 12, height: 2 } };
  const idle = componentActionBounds(component, 1);
  expect(idle).toEqual({ x: 10, y: 0, width: 2, height: 2 });
  const x = idle.x + idle.width / 2;
  const y = idle.y + idle.height / 2;
  expect(componentAt(hitAreas([component], 1), x, y)).toBe(component.id);
  const expanded = {
    ...hitAreas([component], 1)[0]!,
    action: componentActionBounds(component, 1, { expanded: true }),
  };
  // Expanding the label cannot leave the pointer over empty space and flicker.
  expect(componentAt([expanded], x, y)).toBe(component.id);
});

it('fits measured labels and keeps their right edge attached instead of extending into the next room', () => {
  for (const scale of [1, 2, 4, 8]) {
    const measured = componentActionBounds(entry, scale, {
      expanded: true,
      contentWidth: 96 / scale,
      contentHeight: 12 / scale,
    });
    expect(measured.x + measured.width).toBeCloseTo(15);
    expect(measured.width * scale).toBeCloseTo(Math.max(12 * scale, 112));
    expect(measured.y + measured.height).toBeCloseTo(46.8);
    expect(
      componentAt(
        [
          {
            id: entry.id,
            available: true,
            body: entry.bounds,
            action: measured,
          },
        ],
        measured.x + measured.width / 2,
        measured.y + measured.height / 2
      )
    ).toBe(entry.id);
  }
});

it('keeps independent object actions separate without growing or mutating their rectangles', () => {
  const board = { ...entry, bounds: { x: 3, y: 5, width: 12, height: 8 } };
  const whiteboard = { ...entry, id: 'whiteboard', bounds: { x: 16, y: 5, width: 12, height: 8 } };
  const before = structuredClone([board, whiteboard]);
  expect(componentAt(hitAreas([board, whiteboard]), 9, 9)).toBe('board');
  expect(componentAt(hitAreas([board, whiteboard]), 22, 9)).toBe('whiteboard');
  expect(componentAt(hitAreas([board, whiteboard]), 15.5, 9)).toBeUndefined();
  for (const scale of [1, 2, 4, 8, 16]) {
    const first = componentActionBounds(board, scale);
    const second = componentActionBounds(whiteboard, scale);
    expect(first.x + first.width).toBeLessThan(second.x);
  }
  expect([board, whiteboard]).toEqual(before);
});
