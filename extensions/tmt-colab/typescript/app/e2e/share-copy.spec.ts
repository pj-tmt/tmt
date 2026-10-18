import { expect, test } from '@playwright/test';
import { capturePath } from './captures.js';

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark']) {
    test(`Share consequences at ${width}px ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/test/command-browser.html?share-copy');
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      const dialog = page.getByRole('dialog', { name: 'Review notes', exact: true });
      const body = dialog.locator('.management-dialog-body');
      async function capture(name: string) {
        expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
        if (process.env.COLAB_CAPTURE_DIR)
          await page.screenshot({ path: capturePath(`share-${name}-${width}-${theme}.png`) });
      }
      async function cancel() {
        await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
      }
      async function confirm(button: string, title: string, consequence: string) {
        await dialog.getByRole('button', { name: button, exact: true }).click();
        await expect(dialog.getByRole('heading', { name: title, exact: true })).toBeVisible();
        await expect(dialog).toContainText(consequence);
        await capture(title.replace(/[^a-z]+/gi, '-').toLowerCase());
        await cancel();
      }
      async function choose(label: string, option: string) {
        await dialog.getByRole('button', { name: new RegExp(`^${label}`) }).click();
        const menu = dialog.getByRole('menu', { name: label });
        await capture(label.replace(/[^a-z]+/gi, '-').toLowerCase());
        await menu.getByRole('menuitemradio', { name: option, exact: true }).click();
      }
      await expect(dialog.getByRole('button', { name: 'Start new history…' })).toBeVisible();
      await capture('ready-top');
      const keyHistory = dialog.getByText(
        'Later joins receive the current key and up to 63 earlier keys.',
        { exact: true },
      );
      await expect(keyHistory).toBeHidden();
      await dialog.getByText('Details', { exact: true }).click();
      await expect(keyHistory).toBeVisible();
      await capture('details');
      await dialog.getByText('Details', { exact: true }).click();
      await confirm(
        'Start new history…',
        "Start the page's history from now?",
        'Existing links keep working, and anything already shared stays shared.',
      );
      await confirm(
        'Set access expiry',
        'Set access expiry',
        'Access expires after 30 days without a page update.',
      );
      await dialog.getByLabel('Keep until I delete it').check();
      await confirm(
        'Set access expiry',
        'Set access expiry',
        'Keep this page until it is explicitly deleted.',
      );
      await confirm(
        'Archive and stop edits…',
        'Archive and stop edits',
        'there is no restore action here.',
      );
      await confirm('Delete page…', 'Delete page', 'this page cannot be restored.');
      await choose('Who can open this page', 'Named members only');
      await expect(dialog).toContainText('Existing links stop working on these pages:');
      await capture('stop-link-access');
      await cancel();
      await choose('Who can open this page', 'Anyone on this computer');
      await expect(dialog).toContainText('Content made public cannot be taken back.');
      await capture('public-history');
      await cancel();
      await choose('What people who join later see', 'Current page only');
      await expect(dialog).toContainText('Access already given stays.');
      await capture('history-choice');
      await cancel();
      await choose('What this member can do', 'Edit');
      await expect(dialog).toContainText('Content already received cannot be taken back.');
      await capture('member-permission');
      await cancel();
      await confirm(
        'Remove this member’s access…',
        "Remove this member's access",
        'a separate link they still hold can continue to grant access.',
      );
      await dialog.getByLabel('Member ID', { exact: true }).fill('new-reader');
      await dialog.getByLabel('Signing public key').fill('fixture-signing-key');
      await dialog.getByLabel('Encryption public key').fill('fixture-encryption-key');
      await choose('Permission for this new recipient', 'Read');
      await confirm(
        'Give this member access…',
        'Give this member access',
        'not access to send requests to your agents.',
      );
      await confirm(
        'Create a sharing link…',
        'Create a sharing link',
        'the browser opens it read-only whatever its permission.',
      );
      await confirm(
        'Replace link…',
        'Replace link',
        'Share the new link only with intended readers.',
      );
      await confirm(
        'Stop access through this link…',
        'Stop access through this link',
        'no replacement link is created.',
      );
      await body.evaluate((node) => (node.scrollTop = node.scrollHeight));
      await capture('ready-bottom');
      await page.goto('/test/command-browser.html?share-copy&history=current');
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      await expect(dialog).toContainText(
        'A link reads everything since the last history start, including comments and replies.',
      );
      await expect(keyHistory).toHaveCount(0);
      await dialog.getByRole('button', { name: 'Start new history…' }).scrollIntoViewIfNeeded();
      await capture('current-history');
    });
  }
}
