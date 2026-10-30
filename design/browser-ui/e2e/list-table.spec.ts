import { expect, test } from '@playwright/test';

test('table headers and rows share tracks and phone-hidden cells are hidden', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  const table = page.getByRole('table', { name: 'People' }).first();
  const header = table.getByRole('columnheader', { name: 'Role' });
  const cell = table.getByRole('cell', { name: 'Owner' });
  const h = (await header.boundingBox())!;
  const c = (await cell.boundingBox())!;
  expect(Math.abs(h.x - c.x)).toBeLessThan(1);
  expect(Math.abs(h.width - c.width)).toBeLessThan(1);
  await page.setViewportSize({ width: 761, height: 900 });
  await expect(header).toBeVisible();
  await page.setViewportSize({ width: 760, height: 900 });
  await expect(header).toBeHidden();
  await page.setViewportSize({ width: 390, height: 900 });
  await expect(header).toBeHidden();
  await expect(cell).toBeHidden();
  await expect(table.getByRole('cell', { name: 'Ben' })).toBeVisible();
});
