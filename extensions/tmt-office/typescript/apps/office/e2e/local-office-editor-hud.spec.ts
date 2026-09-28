import { expect, test } from '@playwright/test';
import { furnishedOfficeFixture } from './furnished-office-fixture.js';
import { openOfficeDirectory } from './office-navigation.js';

test('keeps automatic change status and contextual properties visible without an editing mode', async ({
  page,
}, info) => {
  const fixture = await furnishedOfficeFixture(page);
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await expect(page.getByRole('button', { name: 'Directory · 3' })).toBeHidden();
  await expect(page.getByRole('button', { name: 'Meeting rooms', exact: true })).toBeHidden();
  await expect(page.getByRole('button', { name: 'Add meeting room', exact: true })).toHaveCount(0);
  await openOfficeDirectory(page);
  await page.getByRole('button', { name: 'Lobby · lobby', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Edit this area', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Edit layout', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Save layout', exact: true })).toHaveCount(0);
  for (const viewport of [
    { width: 1440, height: 1000 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    const draft = page.getByRole('region', { name: 'Layout changes' });
    const tools = page.getByRole('complementary', { name: 'Layout tools' });
    await expect(draft.getByRole('status')).toHaveText('All changes applied');
    await expect(draft.getByRole('button', { name: 'Undo' })).toBeInViewport();
    await expect(tools.getByLabel('Area name')).toBeVisible();
    await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
    await page.getByRole('button', { name: 'Fit office', exact: true }).click();
    const header = (await draft.boundingBox())!;
    const body = (await tools.boundingBox())!;
    await page.screenshot({ path: info.outputPath(`editor-${viewport.width}.png`) });
    expect(
      header.y + header.height,
      JSON.stringify({ viewport, header, body })
    ).toBeLessThanOrEqual(body.y);
    expect(header.x).toBeGreaterThanOrEqual(0);
    expect(header.x + header.width).toBeLessThanOrEqual(viewport.width);
    expect(await page.locator('.office-canvas').boundingBox()).toMatchObject({
      x: 0,
      y: 0,
      ...viewport,
    });
    await page.screenshot({ path: info.outputPath(`editor-${viewport.width}.png`) });
  }
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: 'Furniture & devices' })).toBeVisible();
  expect(fixture.writes).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
