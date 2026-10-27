import { expect, test } from '@playwright/test';

test('storage shows the estimate, confirms, cancels and reports the actual cleanup', async ({
  page,
}) => {
  await page.goto('/test/command-browser.html?storage&history=current');
  const dialog = page.getByRole('dialog').first();
  await expect(dialog.getByText('Storage on this Mac')).toBeVisible();
  await expect(dialog.getByText('Page content')).toBeVisible();
  await expect(dialog.getByText('Edit history')).toBeVisible();
  await expect(dialog.getByText(/can be freed/)).toBeVisible();
  await expect(dialog.getByText('Attachments')).toBeVisible();
  await expect(dialog.getByText('Comments', { exact: true })).toBeVisible();
  await expect(dialog.getByText('Kept: comments, resolved threads, archived state.')).toBeVisible();
  await expect(dialog.getByText(/removed after 30 days/)).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Clean up now' }).click();
  const confirm = page.getByRole('dialog', { name: 'Clean up “Review notes” now?' });
  await expect(confirm).toBeVisible();
  await expect(confirm).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(
    confirm.getByText(
      'Deleted text and old edit steps are removed. The page, its comments and attachments stay as they are. Other devices catch up from the current version.',
    ),
  ).toBeVisible();
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog.getByText(/can be freed/)).toBeVisible();
  await dialog.getByRole('button', { name: 'Clean up now' }).click();
  await page
    .getByRole('dialog', { name: 'Clean up “Review notes” now?' })
    .getByRole('button', { name: 'Clean up', exact: true })
    .click();
  await expect(dialog.getByText(/Freed .* The page is unchanged\./)).toBeVisible();
  await expect(dialog.getByText(/can be freed/)).toHaveCount(0);
});

test('partial and unknown cleanup errors preserve the page message', async ({ page }) => {
  for (const suffix of ['', '&storage-unknown']) {
    await page.goto(`/test/command-browser.html?storage&history=current&storage-error${suffix}`);
    const dialog = page.getByRole('dialog').first();
    await dialog.getByRole('button', { name: 'Clean up now' }).click();
    await page
      .getByRole('dialog', { name: 'Clean up “Review notes” now?' })
      .getByRole('button', { name: 'Clean up', exact: true })
      .click();
    await expect(dialog.getByRole('alert')).toContainText('The page is unchanged.');
    if (suffix) await expect(dialog.getByRole('alert')).not.toContainText('Freed');
    else await expect(dialog.getByRole('alert')).toContainText('Freed');
  }
});

test('known attachment allocation is shown without an unused or expiry claim', async ({ page }) => {
  await page.goto('/test/command-browser.html?storage&history=current&storage-known');
  const section = page.getByRole('region', { name: 'Storage on this Mac' });
  await expect(section.getByText('6.1 MB', { exact: true })).toBeVisible();
  await expect(section.getByText(/unused|removed after 30 days/)).toHaveCount(0);
});

test('skipped pages are read-only and report no new history', async ({ page }) => {
  await page.goto('/test/command-browser.html?storage&history=current&storage-skipped');
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('button', { name: 'Clean up now' })).toBeDisabled();
  await expect(dialog.getByText('There is no new history to clean up.')).toBeVisible();
});

test('a refresh failure does not erase a confirmed cleanup result', async ({ page }) => {
  await page.goto('/test/command-browser.html?storage&history=current&storage-refresh-fail');
  await page.getByRole('button', { name: 'Clean up now' }).click();
  await page
    .getByRole('dialog', { name: 'Clean up “Review notes” now?' })
    .getByRole('button', { name: 'Clean up', exact: true })
    .click();
  await expect(page.getByText('Freed 3.9 MB. The page is unchanged.')).toBeVisible();
  await expect(
    page.getByText('Cleanup could not be confirmed. The result is unknown.'),
  ).toHaveCount(0);
});

test('storage and its confirmation fit a narrow dark viewport', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/test/command-browser.html?storage&history=current');
  const section = page.getByRole('region', { name: 'Storage on this Mac' });
  await expect(section).toBeVisible();
  await page.getByRole('button', { name: 'Clean up now' }).click();
  const confirm = page.getByRole('dialog', { name: 'Clean up “Review notes” now?' });
  const bounds = await confirm.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  await page.keyboard.press('Escape');
  await expect(confirm).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Clean up now' })).toBeFocused();
});
