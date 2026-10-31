import { expect, test, type Locator } from '@playwright/test';
import { browserElements } from '../src/elements-manifest.js';

const controls: Record<string, string> = {
  'tmt-ui-button': '.tmt-ui-action',
  'tmt-ui-icon-button': '.tmt-ui-icon-action',
  'tmt-ui-field': '.tmt-ui-field',
  'tmt-ui-toggle': '.tmt-ui-toggle',
  'tmt-ui-checkbox': '.tmt-ui-checkbox',
  'tmt-ui-menu': '.tmt-ui-menu-trigger',
  'tmt-ui-tabs': '.tmt-ui-tabs',
  'tmt-ui-dialog': '.tmt-ui-action',
  'tmt-ui-chips': '.tmt-ui-chips',
  'tmt-ui-chip': '.tmt-ui-chip',
  'tmt-ui-notice': '.tmt-ui-notice',
  'tmt-ui-list-table': '.tmt-ui-list-table',
};
async function paint(control: Locator) {
  return control.evaluate((node) => {
    const style = getComputedStyle(node);
    return Object.fromEntries(
      [
        'color',
        'backgroundColor',
        'borderColor',
        'borderRadius',
        'fontFamily',
        'fontSize',
        'fontWeight',
        'lineHeight',
        'paddingTop',
        'paddingRight',
        'minHeight',
        'height',
      ].map((key) => [
        key,
        style.getPropertyValue(key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)),
      ]),
    );
  });
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark']) {
    for (const entry of browserElements)
      for (const example of entry.examples) {
        test(`element ${entry.tag} / ${example.state} / ${width} ${theme}`, async ({ page }) => {
          await page.setViewportSize({ width, height: 1000 });
          await page.goto(
            `/elements-frame.html?${new URLSearchParams({ tag: entry.tag, state: example.state, theme })}`,
          );
          const custom = page.locator('#custom');
          const reference = page.locator('#reference');
          await expect(custom.getByRole('alert')).toHaveCount(0);
          const selector = controls[entry.tag]!;
          await expect(custom.locator(selector).first()).toBeVisible();
          await expect(reference.locator(selector).first()).toBeVisible();
          expect(await paint(custom.locator(selector).first())).toEqual(
            await paint(reference.locator(selector).first()),
          );
          if (entry.tag === 'tmt-ui-button' && example.state !== 'compact') {
            expect(
              await custom
                .locator(selector)
                .first()
                .evaluate((node) => parseFloat(getComputedStyle(node).paddingTop)),
            ).toBeGreaterThan(0);
          }
          if (entry.tag === 'tmt-ui-list-table') {
            for (const root of [custom, reference]) {
              await expect(root.locator('[role="columnheader"]')).toHaveText(['Page', 'State']);
              const rows = root.locator('[role="rowgroup"] > [role="row"]');
              await expect(rows).toHaveCount(example.state === 'empty' ? 0 : 2);
              if (example.state !== 'empty') {
                await expect(rows.first().getByRole('cell')).toHaveText(['Project notes', 'Draft']);
                const cells = await rows
                  .first()
                  .getByRole('cell')
                  .evaluateAll((nodes) => nodes.map((node) => node.getBoundingClientRect().top));
                expect(cells[0] === cells[1]).toBe(width === 1440);
              }
            }
          }
          if (entry.tag === 'tmt-ui-list-table' && example.state !== 'empty') {
            await custom
              .locator('tmt-ui-list-table')
              .evaluate((node) =>
                node.setAttribute('rows', JSON.stringify([['<b>literal</b>', 'Ready']])),
              );
            await expect(custom.getByRole('cell')).toHaveText(['<b>literal</b>', 'Ready']);
            await expect(custom.locator('b')).toHaveCount(0);
          }
          if (entry.tag === 'tmt-ui-toggle') {
            for (const root of [custom, reference]) await root.getByRole('button').click();
            expect(await custom.getByRole('button').getAttribute('aria-pressed')).toBe(
              await reference.getByRole('button').getAttribute('aria-pressed'),
            );
          }
          if (entry.tag === 'tmt-ui-checkbox') {
            for (const root of [custom, reference]) await root.getByRole('checkbox').click();
            expect(await custom.getByRole('checkbox').isChecked()).toBe(
              await reference.getByRole('checkbox').isChecked(),
            );
          }
          if (entry.tag === 'tmt-ui-tabs') {
            for (const root of [custom, reference])
              await root.getByRole('tab', { name: 'Done' }).click();
            for (const root of [custom, reference])
              await expect(root.getByRole('tabpanel')).toHaveText('Completed tasks');
          }
          if (entry.tag === 'tmt-ui-menu') {
            for (const root of [custom, reference]) {
              await root.getByRole('button', { name: 'Sort' }).click();
              await root.getByRole('menuitemradio', { name: 'Title' }).click();
              await expect(root.locator('.tmt-ui-menu-value')).toHaveText('Title');
            }
          }
          if (entry.tag === 'tmt-ui-dialog') {
            for (const root of [custom, reference]) {
              await root.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
              await root.getByRole('textbox', { name: 'Name' }).fill('Kept draft');
              await root.getByRole('dialog').press('Escape');
              await expect(root.getByRole('dialog')).toBeHidden();
              await root.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
              await expect(root.getByRole('textbox', { name: 'Name' })).toHaveValue('Kept draft');
              await root.getByRole('dialog').press('Escape');
            }
          }
          await expect(page.locator('[role="alert"]')).toHaveCount(0);
        });
      }
  }

test('elements refuse invalid input and release dialog ownership on disconnect', async ({
  page,
}) => {
  await page.goto('/elements-frame.html?tag=tmt-ui-dialog&state=default&theme=light');
  const custom = page.locator('#custom');
  await custom.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
  const field = custom.getByRole('textbox', { name: 'Name' });
  await field.fill('Draft survives attribute updates');
  await custom.locator('tmt-ui-dialog').evaluate((node) => node.setAttribute('size', 'wide'));
  await expect(field).toBeFocused();
  await expect(field).toHaveValue('Draft survives attribute updates');
  await custom.locator('tmt-ui-dialog').evaluate((node) => node.remove());
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.tmt-ui-overlay')).toHaveCount(0);
  await custom.evaluate((node) => {
    node.innerHTML =
      '<tmt-ui-button variant="invented">Bad</tmt-ui-button><tmt-ui-tabs><section slot="panel" data-value="same" data-label="One">One</section><section slot="panel" data-value="same" data-label="Two">Two</section></tmt-ui-tabs>';
  });
  await expect(custom.getByRole('alert')).toHaveCount(2);
  await expect(custom.getByRole('button')).toHaveCount(0);
  await expect(custom.getByRole('tab')).toHaveCount(0);
});
