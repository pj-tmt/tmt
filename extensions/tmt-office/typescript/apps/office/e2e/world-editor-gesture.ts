import type { Page } from '@playwright/test';

interface Rectangle {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Use independently specified projected fixture extents, not the renderer's
 * own camera or geometry. Coordinates accepted by the result are scene points.
 */
export async function fitWorldCoordinates(page: Page, fittedBounds: Rectangle) {
  await page.getByRole('button', { name: 'Fit office', exact: true }).click();
  const view = (await page.locator('.office-canvas').boundingBox())!;
  const margin = Math.min(view.height * 0.2, Math.max(96, view.height * 0.08));
  const scale = Math.min(
    (view.width * 0.84) / fittedBounds.width,
    (view.height - 2 * margin) / fittedBounds.height
  );
  return (x: number, y: number) => ({
    x: view.x + (view.width - fittedBounds.width * scale) / 2 + (x - fittedBounds.x) * scale,
    y: view.y + (view.height - fittedBounds.height * scale) / 2 + (y - fittedBounds.y) * scale,
  });
}
