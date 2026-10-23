import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { text } from '../src/strings.js';
import { capturePath } from './captures.js';
import { expectOnePrimaryPerGroup } from './primary-groups.js';

/**
 * The loud action of a group is the accent fill (#4475): each Colab surface that has actions
 * holds at most one enabled primary per group. These mount the surfaces the v2 look repainted and
 * read the real DOM; with COLAB_CAPTURE_DIR set they also leave the review captures.
 */
const askPage = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture: askPage, method, argument },
  );
const views = [
  { width: 1440, scheme: 'light' },
  { width: 390, scheme: 'dark' },
] as const;
const capture = async (page: Page, name: string, width: number, scheme: string) => {
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({ path: capturePath(`primary-${name}-${width}-${scheme}.png`) });
};

for (const { width, scheme } of views) {
  const look = async (page: Page) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
  };

  test(`a proposal card has one primary, Approve, at ${width} ${scheme}`, async ({ page }) => {
    await look(page);
    await page.goto('/');
    await run(page, 'mount');
    await run(page, 'proposal', 'inline');
    const card = page.locator('[data-inline-proposal]').first();
    await expect(card.getByRole('button', { name: 'Approve', exact: true })).toHaveAttribute(
      'data-variant',
      'primary',
    );
    await expect(card.getByRole('button', { name: 'Decline', exact: true })).toHaveAttribute(
      'data-variant',
      'secondary',
    );
    await expectOnePrimaryPerGroup(page, 'proposal card');
    await capture(page, 'proposal', width, scheme);
  });

  test(`the work controls keep one primary per group at ${width} ${scheme}`, async ({ page }) => {
    await look(page);
    await page.goto('/');
    await page.evaluate(async () => {
      const path = '/test/work-page-browser.tsx';
      (await import(path)).mountWorkPage(true, true);
    });
    await expect(page.locator('.page-header-actions:visible')).toBeVisible();
    const band = page.getByRole('region', { name: 'Work on this page' });
    await band.getByRole('button', { name: /Show \d+ tasks/ }).click();
    for (const id of ['T-3', 'T-2']) {
      await band.locator('.work-row', { hasText: id }).locator('button').click();
      await expect(band.locator('.work-controls')).toBeVisible();
      // Every form has its input, so each submit is enabled: the panel is one group, and only
      // the task's decision (Sign off) is primary.
      await band.getByRole('textbox', { name: 'Name' }).fill('Atlas');
      await band.getByRole('textbox', { name: 'New task' }).fill('Ship it');
      await expectOnePrimaryPerGroup(page, `work controls of ${id}`);
      await capture(page, `work-${id}`, width, scheme);
    }
    await band.getByRole('button', { name: 'Edit', exact: true }).click();
    await expectOnePrimaryPerGroup(page, 'work task edit');
    await capture(page, 'work-edit', width, scheme);
  });

  test(`the export drawer has one primary download at ${width} ${scheme}`, async ({ page }) => {
    await look(page);
    await page.goto('/');
    await run(page, 'mount', { exportAttachments: true });
    await (await pageAction(page, 'Export page')).click();
    const drawer = page.locator('.tmt-ui-drawer[data-panel=export][open]');
    await expect(drawer).toBeVisible();
    await expect(drawer.locator('[data-variant="primary"]')).toHaveCount(1);
    await expectOnePrimaryPerGroup(page, 'export drawer');
    await capture(page, 'export', width, scheme);
  });

  test(`the new project dialog has one primary at ${width} ${scheme}`, async ({ page }) => {
    await look(page);
    await page.goto('/');
    await run(page, 'mount', { projects: { names: ['alpha', 'beta'], current: [] } });
    await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toBeVisible();
    await (await pageAction(page.locator('#ask-page-fixture'), 'Projects…')).click();
    await page.getByRole('menuitem', { name: text.pageList.newProjectItem }).click();
    const dialog = page.getByRole('dialog', { name: text.pageList.newProject });
    await dialog.getByLabel(text.pageList.nameField).fill('delta');
    await expectOnePrimaryPerGroup(page, 'new project dialog');
    await capture(page, 'project-dialog', width, scheme);
  });

  test(`the page link bar keeps one primary at ${width} ${scheme}`, async ({ page }) => {
    await look(page);
    for (const [href, name] of [
      ['https://outside.example/a/complete/path', 'open'],
      ['javascript:void(0)', 'refused'],
    ] as const) {
      await page.goto('/');
      await page.evaluate(async (source) => {
        const path = '/test/page-link-browser.tsx';
        (await import(path)).mount(source);
      }, `<a href="${href}">A link</a>`);
      await expect(page.locator('#page-link-fixture .status')).toContainText('Live');
      await page.frameLocator('#page-link-fixture iframe').getByRole('link').click();
      await expect(page.getByRole('dialog')).toBeVisible();
      await expectOnePrimaryPerGroup(page, `page link bar, ${name}`);
      await capture(page, `link-bar-${name}`, width, scheme);
    }
  });
}
