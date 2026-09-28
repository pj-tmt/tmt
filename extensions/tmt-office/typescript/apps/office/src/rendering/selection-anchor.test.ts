import { expect, it } from 'vitest';
import {
  selectionAnchor,
  selectionTarget,
  placeSelectionPanel,
  sameSelectionTarget,
} from './selection-anchor.js';

it('retains the complete visible target and viewport for collision-aware placement', () => {
  const viewport = { width: 1536, height: 1024 };
  const target = selectionTarget(
    { x: 20, y: 10, width: 48, height: 41 },
    { x: 100, y: 50, scale: 3 },
    viewport
  )!;
  expect(target).toEqual({ bounds: { x: 160, y: 80, width: 144, height: 123 }, viewport });
  expect(sameSelectionTarget(target, structuredClone(target))).toBe(true);
  expect(sameSelectionTarget(target, { ...target, viewport: { width: 390, height: 844 } })).toBe(
    false
  );
  expect(sameSelectionTarget(undefined, undefined)).toBe(true);
});

it('keeps measured cards clear of artwork at every viewport edge and on narrow screens', () => {
  for (const viewport of [
    { width: 1536, height: 1024 },
    { width: 390, height: 844 },
  ]) {
    for (const bounds of [
      { x: 20, y: 170, width: 110, height: 130 },
      { x: viewport.width - 130, y: 170, width: 110, height: 130 },
      { x: viewport.width / 2 - 55, y: viewport.height - 240, width: 110, height: 130 },
    ]) {
      const size = { width: 240, height: 224 };
      const panel = placeSelectionPanel({ bounds, viewport }, size);
      expect(panel.x).toBeGreaterThanOrEqual(12);
      expect(panel.x + size.width).toBeLessThanOrEqual(viewport.width - 12);
      expect(panel.y).toBeGreaterThanOrEqual(164);
      expect(panel.y + size.height).toBeLessThanOrEqual(viewport.height - 100);
      expect(
        panel.x + size.width <= bounds.x ||
          panel.x >= bounds.x + bounds.width ||
          panel.y + size.height <= bounds.y ||
          panel.y >= bounds.y + bounds.height
      ).toBe(true);
    }
  }
});

it('bounds oversized cards without inventing an off-screen placement', () => {
  const result = placeSelectionPanel(
    { bounds: { x: 0, y: 0, width: 320, height: 300 }, viewport: { width: 320, height: 300 } },
    { width: 240, height: 400 }
  );
  expect(result.maxHeight).toBe(186);
  expect(result.y).toBe(60);
});

it('uses measured wrapped toolbar and save feedback boundaries instead of fixed HUD offsets', () => {
  const target = {
    bounds: { x: 230, y: 230, width: 140, height: 180 },
    viewport: { width: 390, height: 844 },
  };
  for (const clearance of [
    { top: 305, bottom: 720 },
    { top: 340, bottom: 650 },
  ]) {
    const result = placeSelectionPanel(target, { width: 280, height: 500 }, clearance);
    expect(result.y).toBe(clearance.top);
    expect(result.maxHeight).toBe(clearance.bottom - clearance.top);
    expect(result.y + result.maxHeight).toBe(clearance.bottom);
    expect(result.x).toBeGreaterThanOrEqual(12);
    expect(result.x + 280).toBeLessThanOrEqual(378);
  }
});

it('projects the selected footprint through the existing camera and follows pan and zoom', () => {
  const item = { x: 20, y: 20, width: 8, height: 6 };
  const viewport = { width: 1280, height: 900 };
  expect(selectionAnchor(item, { x: 100, y: 50, scale: 10 }, viewport)).toEqual({ x: 340, y: 322 });
  expect(selectionAnchor(item, { x: 130, y: 70, scale: 10 }, viewport)).toEqual({ x: 370, y: 342 });
  expect(selectionAnchor(item, { x: 100, y: 50, scale: 20 }, viewport)).toEqual({ x: 580, y: 582 });
});

it('keeps the HUD within the viewport and removes it when its object leaves view', () => {
  const camera = { x: 0, y: 0, scale: 1 };
  const viewport = { width: 1280, height: 900 };
  expect(selectionAnchor({ x: -10, y: 870, width: 20, height: 40 }, camera, viewport)).toEqual({
    x: 124,
    y: 724,
  });
  expect(
    selectionAnchor({ x: 1280, y: 300, width: 20, height: 40 }, camera, viewport)
  ).toBeUndefined();
  expect(
    selectionAnchor({ x: 20, y: -40, width: 20, height: 40 }, camera, viewport)
  ).toBeUndefined();
  expect(
    selectionAnchor({ x: 20, y: 20, width: 20, height: 40 }, camera, { width: 0, height: 0 })
  ).toBeUndefined();
});
