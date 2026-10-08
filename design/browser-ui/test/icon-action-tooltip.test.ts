import { expect, it } from 'vite-plus/test';
import { iconActionTooltipPosition } from '../src/icon-action-tooltip.js';

it('centers below the action and clamps long labels and both viewport edges', () => {
  const viewport = { left: 0, top: 0, width: 390, height: 844 };
  expect(
    iconActionTooltipPosition({ left: 180, right: 214, bottom: 50 }, 100, viewport, 8),
  ).toEqual({ left: 147, top: 50, maxWidth: 374, maxHeight: 786 });
  expect(iconActionTooltipPosition({ left: 0, right: 34, bottom: 50 }, 100, viewport, 8).left).toBe(
    8,
  );
  expect(
    iconActionTooltipPosition({ left: 356, right: 390, bottom: 50 }, 100, viewport, 8).left,
  ).toBe(282);
  expect(
    iconActionTooltipPosition({ left: 356, right: 390, bottom: 50 }, 800, viewport, 8).left,
  ).toBe(8);
});

it('uses the visual viewport offset and bounds the remaining space without flipping above', () => {
  const viewport = { left: 20, top: 100, width: 320, height: 400 };
  expect(iconActionTooltipPosition({ left: 20, right: 54, bottom: 480 }, 400, viewport, 8)).toEqual(
    { left: 28, top: 480, maxWidth: 304, maxHeight: 12 },
  );
  expect(
    iconActionTooltipPosition({ left: 20, right: 54, bottom: 510 }, 40, viewport, 8).maxHeight,
  ).toBe(0);
});
