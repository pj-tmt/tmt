import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
const fixture = '/test/ask-page-browser.tsx';
for (const width of [1440, 390])
  for (const theme of ['light', 'dark']) {
    for (const surface of ['index', 'detail', 'terminal', 'document']) {
      test(`Design ${surface} ${width} ${theme}`, async ({ page }, info) => {
        await page.setViewportSize({ width, height: 1000 });
        await page.route('**/src/main.tsx', (route) =>
          route.fulfill({ contentType: 'text/javascript', body: '' }),
        );
        await page.goto('/');
        await page.evaluate(
          async ({ fixture, theme }) => {
            document.documentElement.dataset.theme = theme;
            await (await import(fixture)).mount();
          },
          { fixture, theme },
        );
        if (surface === 'document') {
          await page.evaluate(
            async (fixture) =>
              (await import(fixture)).change(
                '<h1>Widget document</h1><tmt-ui-toggle label="Follow updates"></tmt-ui-toggle><tmt-question data-id="00000000-0000-4000-8000-000000000090" data-kind="short" data-agent="Agent 1"><p data-question>Keep this question draft?</p></tmt-question>',
              ),
            fixture,
          );
          const frame = page.frameLocator('#ask-page-fixture iframe');
          await expect(frame.getByRole('heading', { name: 'Widget document' })).toBeVisible();
          const control = frame.getByRole('button', { name: 'Follow updates' });
          await page.locator('.question-group-card .question-line').click();
          const answer = page
            .locator('.question-group-card')
            .getByRole('textbox', { name: 'Your answer' });
          await answer.fill('Retained beside the live widget');
          await control.click();
          await expect(answer).toHaveValue('Retained beside the live widget');
          await expect(control).toHaveAttribute('aria-pressed', 'true');
          expect(
            await page.evaluate(async (fixture) => (await import(fixture)).proof(), fixture),
          ).toEqual({ sends: [], actions: [] });
        } else {
          const path =
            surface === 'detail'
              ? '/design/gui/tmt-ui-dialog'
              : surface === 'terminal'
                ? '/design/tui'
                : '/design/gui';
          // The real app router owns these routes independently of the menu entry.
          await page.evaluate((path) => {
            location.hash = path;
          }, path);
          const catalog = page.locator('.design-page');
          await expect(catalog).toBeVisible();
          if (surface === 'terminal') {
            await expect(catalog.getByRole('table', { name: 'TUI components' })).toBeVisible();
            await expect(catalog).toContainText('not available in this build');
            await expect(catalog.locator('iframe')).toHaveCount(0);
          } else {
            const previews = catalog.locator('[data-design-ready]');
            await expect(previews).toHaveCount(surface === 'index' ? 12 : 1);
            await expect(catalog.getByRole('alert')).toHaveCount(0);
            if (surface === 'detail') {
              const frame = catalog.frameLocator('iframe');
              const trigger = frame
                .getByRole('button', { name: 'Manage shortcuts', exact: true })
                .first();
              expect(
                await trigger.evaluate((node) => parseFloat(getComputedStyle(node).paddingTop)),
              ).toBeGreaterThan(0);
              await trigger.click();
              await expect(frame.getByRole('dialog').first()).toBeVisible();
              await frame.getByRole('dialog').first().press('Escape');
            }
            await expect(catalog.getByLabel('Indexed coverage').first()).toContainText('✓ Test');
          }
        }
        await info.attach(`design-${surface}-${width}-${theme}`, {
          body: await page.screenshot({ fullPage: true }),
          contentType: 'image/png',
        });
      });
    }
  }

test('widget runtime admission matches the shared native export vectors', async ({ page }) => {
  const { cases } = JSON.parse(
    readFileSync(
      new URL('../../../contracts/vectors/runtime-marker-v1.json', import.meta.url),
      'utf8',
    ),
  ) as { cases: { name: string; source: string; optsIn: boolean }[] };
  await page.goto('/');
  const answers = await page.evaluate(
    async (sources) => {
      const path = '/src/page-runtime.ts';
      const { wantsRuntime } = await import(path);
      return sources.map((source: string) => wantsRuntime(source));
    },
    cases.map((entry) => entry.source),
  );
  expect(cases.map((entry, index) => [entry.name, answers[index]])).toEqual(
    cases.map((entry) => [entry.name, entry.optsIn]),
  );
});

test('reader loads script-free widgets and remounts replaced widget source', async ({ page }) => {
  const fixture = '/test/reader-metadata-browser.tsx';
  await page.goto('/');
  await page.evaluate(async (fixture) => {
    const owner = await import(fixture);
    await owner.mount('viewer');
    owner.source('<h1>Reader widgets</h1><tmt-ui-toggle label="Follow updates"></tmt-ui-toggle>');
  }, fixture);
  const frame = page.frameLocator('#reader-metadata-fixture iframe');
  const control = frame.getByRole('button', { name: 'Follow updates' });
  await control.click();
  await expect(control).toHaveAttribute('aria-pressed', 'true');
  const previous = await page.locator('#reader-metadata-fixture iframe').elementHandle();
  await page.evaluate(
    async (fixture) =>
      (await import(fixture)).source(
        '<h1>Changed reader widgets</h1><tmt-ui-toggle label="Follow updates"></tmt-ui-toggle>',
      ),
    fixture,
  );
  await expect(frame.getByRole('heading', { name: 'Changed reader widgets' })).toBeVisible();
  expect(await previous!.evaluate((node) => node.isConnected)).toBe(false);
  await expect(control).toHaveAttribute('aria-pressed', 'false');
});
